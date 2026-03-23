//! SurrealDB embedded client for case operations.
//!
//! Keywords: surrealdb client, case client, database, query

use crate::config::DbConfig;
use crate::error::{CaseError, CaseResult};
use crate::repo_id::RepoIdentity;
use crate::types::*;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use surrealdb::engine::local::{Db, RocksDb};
use surrealdb::Surreal;

const DB_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);
const DB_LOCK_RETRY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct SharedDbHandle {
    db: Surreal<Db>,
    db_lock: Arc<File>,
}

#[derive(Clone)]
pub struct CaseClient {
    db: Surreal<Db>,
    _db_lock: Arc<File>,
    repo_id: String,
    repo_label: String,
    worktree_id: String,
    worktree_root: String,
}

impl CaseClient {
    pub async fn new(config: &DbConfig, identity: RepoIdentity) -> CaseResult<Self> {
        // Ensure parent directory exists
        if let Some(parent) = config.data_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CaseError::DbInit(format!("failed to create data directory: {e}")))?;
        }

        let shared = SharedDbHandle::connect(config).await?;
        Self::from_shared(shared, identity).await
    }

    pub async fn from_shared(shared: SharedDbHandle, identity: RepoIdentity) -> CaseResult<Self> {
        let client = Self {
            db: shared.db,
            _db_lock: shared.db_lock,
            repo_id: identity.repo_id,
            repo_label: identity.repo_label,
            worktree_id: identity.worktree_id,
            worktree_root: identity.worktree_root,
        };
        client.ensure_schema().await?;
        Ok(client)
    }

    pub fn clone_with_identity(&self, identity: RepoIdentity) -> Self {
        Self {
            db: self.db.clone(),
            _db_lock: self._db_lock.clone(),
            repo_id: identity.repo_id,
            repo_label: identity.repo_label,
            worktree_id: identity.worktree_id,
            worktree_root: identity.worktree_root,
        }
    }
}

impl SharedDbHandle {
    pub async fn connect(config: &DbConfig) -> CaseResult<Self> {
        let db_lock = acquire_db_lock(config).await?;
        let db = connect_with_retry(config).await?;
        db.use_ns("agpod")
            .use_db("case")
            .await
            .map_err(|e| CaseError::DbInit(format!("namespace/db init: {e}")))?;

        Ok(Self { db, db_lock })
    }
}

impl CaseClient {
    async fn ensure_schema(&self) -> CaseResult<()> {
        let schema = "
            DEFINE TABLE IF NOT EXISTS case SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS case_id ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS repo_id ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS repo_label ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS worktree_id ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS worktree_root ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS goal ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS goal_constraints ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS status ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS current_direction_seq ON case TYPE int;
            DEFINE FIELD IF NOT EXISTS current_step_id ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS opened_at ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS updated_at ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS closed_at ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS close_summary ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS abandoned_at ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS abandon_summary ON case TYPE string;
            DEFINE INDEX IF NOT EXISTS idx_case_repo_status ON case FIELDS repo_id, status;
            DEFINE INDEX IF NOT EXISTS idx_case_id ON case FIELDS case_id UNIQUE;

            DEFINE TABLE IF NOT EXISTS direction SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS case_id ON direction TYPE string;
            DEFINE FIELD IF NOT EXISTS seq ON direction TYPE int;
            DEFINE FIELD IF NOT EXISTS summary ON direction TYPE string;
            DEFINE FIELD IF NOT EXISTS constraints ON direction TYPE string;
            DEFINE FIELD IF NOT EXISTS success_condition ON direction TYPE string;
            DEFINE FIELD IF NOT EXISTS abort_condition ON direction TYPE string;
            DEFINE FIELD IF NOT EXISTS reason ON direction TYPE string;
            DEFINE FIELD IF NOT EXISTS context ON direction TYPE string;
            DEFINE FIELD IF NOT EXISTS created_at ON direction TYPE string;
            DEFINE INDEX IF NOT EXISTS idx_direction_case ON direction FIELDS case_id;
            DEFINE INDEX IF NOT EXISTS idx_direction_case_seq ON direction FIELDS case_id, seq UNIQUE;

            DEFINE TABLE IF NOT EXISTS step SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS step_id ON step TYPE string;
            DEFINE FIELD IF NOT EXISTS case_id ON step TYPE string;
            DEFINE FIELD IF NOT EXISTS direction_seq ON step TYPE int;
            DEFINE FIELD IF NOT EXISTS order_index ON step TYPE int;
            DEFINE FIELD IF NOT EXISTS title ON step TYPE string;
            DEFINE FIELD IF NOT EXISTS status ON step TYPE string;
            DEFINE FIELD IF NOT EXISTS reason ON step TYPE string;
            DEFINE FIELD IF NOT EXISTS created_at ON step TYPE string;
            DEFINE FIELD IF NOT EXISTS updated_at ON step TYPE string;
            DEFINE INDEX IF NOT EXISTS idx_step_case ON step FIELDS case_id;
            DEFINE INDEX IF NOT EXISTS idx_step_id ON step FIELDS step_id UNIQUE;

            DEFINE TABLE IF NOT EXISTS entry SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS case_id ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS seq ON entry TYPE int;
            DEFINE FIELD IF NOT EXISTS entry_type ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS kind ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS summary ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS reason ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS context ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS files ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS artifacts ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS created_at ON entry TYPE string;
            DEFINE INDEX IF NOT EXISTS idx_entry_case ON entry FIELDS case_id;
        ";

        self.db
            .query(schema)
            .await
            .map_err(|e| CaseError::DbInit(format!("schema init: {e}")))?;

        Ok(())
    }

    // ── Internal query helper ──

    async fn query_raw(&self, sql: &str, bindings: Value) -> CaseResult<Vec<Value>> {
        let mut response = self
            .db
            .query(sql)
            .bind(bindings)
            .await
            .map_err(|e| CaseError::DbQuery(format!("{e}")))?;

        let result: Vec<Value> = response
            .take(0)
            .map_err(|e| CaseError::DbQuery(format!("take(0): {e}")))?;

        Ok(result)
    }

    // ── Case operations ──

    pub async fn find_open_case(&self) -> CaseResult<Option<Case>> {
        let results = self
            .query_raw(
                "SELECT * FROM case WHERE repo_id = $repo_id AND status = 'open' LIMIT 1",
                json!({ "repo_id": self.repo_id }),
            )
            .await?;

        Ok(results.first().and_then(parse_case))
    }

    pub async fn create_case(
        &self,
        case_id: &str,
        goal: &str,
        goal_constraints: &[Constraint],
    ) -> CaseResult<Case> {
        let now = Utc::now().to_rfc3339();
        let constraints_json =
            serde_json::to_string(goal_constraints).map_err(|e| CaseError::Other(e.to_string()))?;

        self.query_raw(
            "CREATE case SET case_id = $case_id, repo_id = $repo_id, goal = $goal, \
             repo_label = $repo_label, worktree_id = $worktree_id, worktree_root = $worktree_root, \
             goal_constraints = $goal_constraints, status = 'open', \
             current_direction_seq = $current_direction_seq, current_step_id = '', \
             opened_at = $opened_at, updated_at = $updated_at, \
             closed_at = '', close_summary = '', abandoned_at = '', abandon_summary = ''",
            json!({
                "case_id": case_id,
                "repo_id": self.repo_id,
                "repo_label": self.repo_label,
                "worktree_id": self.worktree_id,
                "worktree_root": self.worktree_root,
                "goal": goal,
                "goal_constraints": constraints_json,
                "current_direction_seq": 1,
                "opened_at": now,
                "updated_at": now,
            }),
        )
        .await?;

        Ok(Case {
            id: case_id.to_string(),
            repo_id: self.repo_id.clone(),
            repo_label: Some(self.repo_label.clone()),
            worktree_id: Some(self.worktree_id.clone()),
            worktree_root: Some(self.worktree_root.clone()),
            goal: goal.to_string(),
            goal_constraints: goal_constraints.to_vec(),
            status: CaseStatus::Open,
            current_direction_seq: 1,
            current_step_id: None,
            opened_at: now.clone(),
            updated_at: now,
            closed_at: None,
            close_summary: None,
            abandoned_at: None,
            abandon_summary: None,
        })
    }

    pub async fn update_case_status(
        &self,
        case_id: &str,
        status: CaseStatus,
        summary: &str,
    ) -> CaseResult<()> {
        let now = Utc::now().to_rfc3339();
        let sql = match status {
            CaseStatus::Closed => {
                "UPDATE case SET status = $status, updated_at = $updated_at, \
                 closed_at = $now, close_summary = $summary \
                 WHERE case_id = $case_id"
            }
            CaseStatus::Abandoned => {
                "UPDATE case SET status = $status, updated_at = $updated_at, \
                 abandoned_at = $now, abandon_summary = $summary \
                 WHERE case_id = $case_id"
            }
            _ => {
                "UPDATE case SET status = $status, updated_at = $updated_at \
                 WHERE case_id = $case_id"
            }
        };

        self.query_raw(
            sql,
            json!({
                "case_id": case_id,
                "status": status.as_str(),
                "updated_at": now,
                "now": now,
                "summary": summary,
            }),
        )
        .await?;

        Ok(())
    }

    pub async fn update_case_goal_constraints(
        &self,
        case_id: &str,
        goal_constraints: &[Constraint],
    ) -> CaseResult<()> {
        let now = Utc::now().to_rfc3339();
        let constraints_json =
            serde_json::to_string(goal_constraints).map_err(|e| CaseError::Other(e.to_string()))?;

        self.query_raw(
            "UPDATE case SET goal_constraints = $goal_constraints, updated_at = $updated_at \
             WHERE case_id = $case_id",
            json!({
                "case_id": case_id,
                "goal_constraints": constraints_json,
                "updated_at": now,
            }),
        )
        .await?;

        Ok(())
    }

    pub async fn update_case_direction(&self, case_id: &str, direction_seq: u32) -> CaseResult<()> {
        let now = Utc::now().to_rfc3339();
        self.query_raw(
            "UPDATE case SET current_direction_seq = $seq, current_step_id = '', \
             updated_at = $updated_at WHERE case_id = $case_id",
            json!({
                "case_id": case_id,
                "seq": direction_seq,
                "updated_at": now,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn update_case_step(&self, case_id: &str, step_id: &str) -> CaseResult<()> {
        let now = Utc::now().to_rfc3339();
        self.query_raw(
            "UPDATE case SET current_step_id = $step_id, updated_at = $updated_at \
             WHERE case_id = $case_id",
            json!({
                "case_id": case_id,
                "step_id": step_id,
                "updated_at": now,
            }),
        )
        .await?;
        Ok(())
    }

    // ── Direction operations ──

    #[allow(clippy::too_many_arguments)]
    pub async fn create_direction(
        &self,
        case_id: &str,
        seq: u32,
        summary: &str,
        constraints: &[Constraint],
        success_condition: &str,
        abort_condition: &str,
        reason: Option<&str>,
        context: Option<&str>,
    ) -> CaseResult<Direction> {
        let now = Utc::now().to_rfc3339();
        let constraints_json =
            serde_json::to_string(constraints).map_err(|e| CaseError::Other(e.to_string()))?;

        self.query_raw(
            "CREATE direction SET case_id = $case_id, seq = $seq, summary = $summary, \
             constraints = $constraints, success_condition = $success_condition, \
             abort_condition = $abort_condition, reason = $reason, context = $context, \
             created_at = $created_at",
            json!({
                "case_id": case_id,
                "seq": seq,
                "summary": summary,
                "constraints": constraints_json,
                "success_condition": success_condition,
                "abort_condition": abort_condition,
                "reason": reason.unwrap_or(""),
                "context": context.unwrap_or(""),
                "created_at": now,
            }),
        )
        .await?;

        Ok(Direction {
            case_id: case_id.to_string(),
            seq,
            summary: summary.to_string(),
            constraints: constraints.to_vec(),
            success_condition: success_condition.to_string(),
            abort_condition: abort_condition.to_string(),
            reason: reason.map(String::from),
            context: context.map(String::from),
            created_at: now,
        })
    }

    pub async fn get_directions(&self, case_id: &str) -> CaseResult<Vec<Direction>> {
        let results = self
            .query_raw(
                "SELECT * FROM direction WHERE case_id = $case_id ORDER BY seq",
                json!({ "case_id": case_id }),
            )
            .await?;
        Ok(results.iter().filter_map(parse_single_direction).collect())
    }

    pub async fn get_current_direction(&self, case_id: &str, seq: u32) -> CaseResult<Direction> {
        let results = self
            .query_raw(
                "SELECT * FROM direction WHERE case_id = $case_id AND seq = $seq LIMIT 1",
                json!({ "case_id": case_id, "seq": seq }),
            )
            .await?;
        results
            .first()
            .and_then(parse_single_direction)
            .ok_or_else(|| CaseError::Other("no direction found".to_string()))
    }

    // ── Step operations ──

    pub async fn create_step(
        &self,
        step_id: &str,
        case_id: &str,
        direction_seq: u32,
        order_index: u32,
        title: &str,
        reason: Option<&str>,
    ) -> CaseResult<Step> {
        let now = Utc::now().to_rfc3339();
        self.query_raw(
            "CREATE step SET step_id = $step_id, case_id = $case_id, \
             direction_seq = $direction_seq, order_index = $order_index, \
             title = $title, status = 'pending', reason = $reason, \
             created_at = $created_at, updated_at = $updated_at",
            json!({
                "step_id": step_id,
                "case_id": case_id,
                "direction_seq": direction_seq,
                "order_index": order_index,
                "title": title,
                "reason": reason.unwrap_or(""),
                "created_at": now,
                "updated_at": now,
            }),
        )
        .await?;

        Ok(Step {
            id: step_id.to_string(),
            case_id: case_id.to_string(),
            direction_seq,
            order_index,
            title: title.to_string(),
            status: StepStatus::Pending,
            reason: reason.map(String::from),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub async fn get_steps(&self, case_id: &str, direction_seq: u32) -> CaseResult<Vec<Step>> {
        let results = self
            .query_raw(
                "SELECT * FROM step WHERE case_id = $case_id AND direction_seq = $direction_seq \
                 ORDER BY order_index",
                json!({ "case_id": case_id, "direction_seq": direction_seq }),
            )
            .await?;
        Ok(results.iter().filter_map(parse_single_step).collect())
    }

    pub async fn get_all_steps(&self, case_id: &str) -> CaseResult<Vec<Step>> {
        let results = self
            .query_raw(
                "SELECT * FROM step WHERE case_id = $case_id ORDER BY order_index",
                json!({ "case_id": case_id }),
            )
            .await?;
        Ok(results.iter().filter_map(parse_single_step).collect())
    }

    pub async fn get_step(&self, step_id: &str) -> CaseResult<Step> {
        let results = self
            .query_raw(
                "SELECT * FROM step WHERE step_id = $step_id LIMIT 1",
                json!({ "step_id": step_id }),
            )
            .await?;
        results
            .first()
            .and_then(parse_single_step)
            .ok_or_else(|| CaseError::StepNotFound(step_id.to_string()))
    }

    pub async fn update_step(
        &self,
        step_id: &str,
        status: StepStatus,
        reason: Option<&str>,
    ) -> CaseResult<()> {
        let now = Utc::now().to_rfc3339();
        self.query_raw(
            "UPDATE step SET status = $status, reason = $reason, updated_at = $updated_at \
             WHERE step_id = $step_id",
            json!({
                "step_id": step_id,
                "status": status.as_str(),
                "reason": reason.unwrap_or(""),
                "updated_at": now,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn reorder_step(&self, step_id: &str, new_order_index: u32) -> CaseResult<()> {
        self.query_raw(
            "UPDATE step SET order_index = $order_index WHERE step_id = $step_id",
            json!({
                "step_id": step_id,
                "order_index": new_order_index,
            }),
        )
        .await?;
        Ok(())
    }

    // ── Entry operations ──

    #[allow(clippy::too_many_arguments)]
    pub async fn create_entry(
        &self,
        case_id: &str,
        seq: u32,
        entry_type: EntryType,
        kind: Option<&str>,
        summary: &str,
        reason: Option<&str>,
        context: Option<&str>,
        files: &[String],
        artifacts: &[String],
    ) -> CaseResult<Entry> {
        let now = Utc::now().to_rfc3339();
        let files_json =
            serde_json::to_string(files).map_err(|e| CaseError::Other(e.to_string()))?;
        let artifacts_json =
            serde_json::to_string(artifacts).map_err(|e| CaseError::Other(e.to_string()))?;

        self.query_raw(
            "CREATE entry SET case_id = $case_id, seq = $seq, entry_type = $entry_type, \
             kind = $kind, summary = $summary, reason = $reason, context = $context, \
             files = $files, artifacts = $artifacts, created_at = $created_at",
            json!({
                "case_id": case_id,
                "seq": seq,
                "entry_type": entry_type.as_str(),
                "kind": kind.unwrap_or(""),
                "summary": summary,
                "reason": reason.unwrap_or(""),
                "context": context.unwrap_or(""),
                "files": files_json,
                "artifacts": artifacts_json,
                "created_at": now,
            }),
        )
        .await?;

        Ok(Entry {
            case_id: case_id.to_string(),
            seq,
            entry_type,
            kind: kind.map(String::from),
            summary: summary.to_string(),
            reason: reason.map(String::from),
            context: context.map(String::from),
            files: files.to_vec(),
            artifacts: artifacts.to_vec(),
            created_at: now,
        })
    }

    pub async fn get_entries(&self, case_id: &str) -> CaseResult<Vec<Entry>> {
        let results = self
            .query_raw(
                "SELECT * FROM entry WHERE case_id = $case_id ORDER BY seq",
                json!({ "case_id": case_id }),
            )
            .await?;
        Ok(results.iter().filter_map(parse_single_entry).collect())
    }

    pub async fn get_latest_entry(&self, case_id: &str) -> CaseResult<Option<Entry>> {
        let results = self
            .query_raw(
                "SELECT * FROM entry WHERE case_id = $case_id ORDER BY seq DESC LIMIT 1",
                json!({ "case_id": case_id }),
            )
            .await?;
        Ok(results.first().and_then(parse_single_entry))
    }

    pub async fn get_entry_count(&self, case_id: &str) -> CaseResult<u32> {
        let entries = self.get_entries(case_id).await?;
        Ok(entries.len() as u32)
    }

    pub async fn get_step_count(&self, case_id: &str) -> CaseResult<u32> {
        let steps = self.get_all_steps(case_id).await?;
        Ok(steps.len() as u32)
    }

    // ── List / Search ──

    pub async fn list_cases(&self) -> CaseResult<Vec<Case>> {
        let results = self
            .query_raw(
                "SELECT * FROM case WHERE repo_id = $repo_id",
                json!({ "repo_id": self.repo_id }),
            )
            .await?;
        Ok(results.iter().filter_map(parse_case).collect())
    }

    pub async fn search_cases(&self, query: &str) -> CaseResult<Vec<CaseSearchResult>> {
        let needle = query.trim().to_lowercase();
        let cases = self.list_cases().await?;
        let mut results = Vec::new();

        for case in cases {
            let mut case_matches = Vec::new();
            push_match(
                &mut case_matches,
                "case",
                "goal",
                Some(&case.goal),
                &needle,
                MatchMeta::default(),
            );
            push_match(
                &mut case_matches,
                "case",
                "close_summary",
                case.close_summary.as_deref(),
                &needle,
                MatchMeta::default(),
            );
            push_match(
                &mut case_matches,
                "case",
                "abandon_summary",
                case.abandon_summary.as_deref(),
                &needle,
                MatchMeta::default(),
            );

            let directions = self.get_directions(&case.id).await?;
            for direction in &directions {
                push_match(
                    &mut case_matches,
                    "direction",
                    "summary",
                    Some(&direction.summary),
                    &needle,
                    MatchMeta {
                        direction_seq: Some(direction.seq),
                        ..MatchMeta::default()
                    },
                );
                push_match(
                    &mut case_matches,
                    "direction",
                    "success_condition",
                    Some(&direction.success_condition),
                    &needle,
                    MatchMeta {
                        direction_seq: Some(direction.seq),
                        ..MatchMeta::default()
                    },
                );
                push_match(
                    &mut case_matches,
                    "direction",
                    "abort_condition",
                    Some(&direction.abort_condition),
                    &needle,
                    MatchMeta {
                        direction_seq: Some(direction.seq),
                        ..MatchMeta::default()
                    },
                );
                push_match(
                    &mut case_matches,
                    "direction",
                    "reason",
                    direction.reason.as_deref(),
                    &needle,
                    MatchMeta {
                        direction_seq: Some(direction.seq),
                        ..MatchMeta::default()
                    },
                );
                push_match(
                    &mut case_matches,
                    "direction",
                    "context",
                    direction.context.as_deref(),
                    &needle,
                    MatchMeta {
                        direction_seq: Some(direction.seq),
                        ..MatchMeta::default()
                    },
                );
            }

            let entries = self.get_entries(&case.id).await?;
            for entry in &entries {
                push_match(
                    &mut case_matches,
                    "entry",
                    "summary",
                    Some(&entry.summary),
                    &needle,
                    MatchMeta {
                        entry_seq: Some(entry.seq),
                        kind: entry.kind.clone(),
                        ..MatchMeta::default()
                    },
                );
                push_match(
                    &mut case_matches,
                    "entry",
                    "reason",
                    entry.reason.as_deref(),
                    &needle,
                    MatchMeta {
                        entry_seq: Some(entry.seq),
                        kind: entry.kind.clone(),
                        ..MatchMeta::default()
                    },
                );
                push_match(
                    &mut case_matches,
                    "entry",
                    "context",
                    entry.context.as_deref(),
                    &needle,
                    MatchMeta {
                        entry_seq: Some(entry.seq),
                        kind: entry.kind.clone(),
                        ..MatchMeta::default()
                    },
                );
                push_match(
                    &mut case_matches,
                    "entry",
                    "kind",
                    entry.kind.as_deref(),
                    &needle,
                    MatchMeta {
                        entry_seq: Some(entry.seq),
                        kind: entry.kind.clone(),
                        ..MatchMeta::default()
                    },
                );
            }

            if !case_matches.is_empty() {
                results.push(CaseSearchResult {
                    case,
                    matches: case_matches,
                });
            }
        }

        Ok(results)
    }

    pub async fn get_case(&self, case_id: &str) -> CaseResult<Case> {
        let results = self
            .query_raw(
                "SELECT * FROM case WHERE repo_id = $repo_id AND case_id = $case_id LIMIT 1",
                json!({ "repo_id": self.repo_id, "case_id": case_id }),
            )
            .await?;
        results
            .first()
            .and_then(parse_case)
            .ok_or_else(|| CaseError::CaseNotFound(case_id.to_string()))
    }
}

async fn connect_with_retry(config: &DbConfig) -> CaseResult<Surreal<Db>> {
    let path = config.data_dir.to_string_lossy().to_string();
    let started = Instant::now();

    loop {
        match Surreal::new::<RocksDb>(path.as_str()).await {
            Ok(db) => return Ok(db),
            Err(err) => {
                let message = err.to_string();
                if !is_lock_contention(&message) {
                    return Err(CaseError::DbConnection(message));
                }

                if started.elapsed() >= DB_LOCK_RETRY_TIMEOUT {
                    return Err(CaseError::DbConnection(format!(
                        "timed out waiting for database lock after {} ms: {message}",
                        DB_LOCK_RETRY_TIMEOUT.as_millis()
                    )));
                }

                tokio::time::sleep(DB_LOCK_RETRY_DELAY).await;
            }
        }
    }
}

async fn acquire_db_lock(config: &DbConfig) -> CaseResult<Arc<File>> {
    let lock_path = db_lock_path(config);
    if let Some(existing) = shared_db_lock(&lock_path) {
        return Ok(existing);
    }

    let started = Instant::now();

    loop {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|err| {
                CaseError::DbConnection(format!(
                    "failed to open database access lock {}: {err}",
                    lock_path.display()
                ))
            })?;

        match file.try_lock() {
            Ok(()) => {
                let file = Arc::new(file);
                return Ok(remember_db_lock(lock_path.clone(), file));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                if started.elapsed() >= DB_LOCK_RETRY_TIMEOUT {
                    return Err(CaseError::DbConnection(format!(
                        "timed out waiting for database access lock {} after {} ms",
                        lock_path.display(),
                        DB_LOCK_RETRY_TIMEOUT.as_millis()
                    )));
                }
                tokio::time::sleep(DB_LOCK_RETRY_DELAY).await;
            }
            Err(std::fs::TryLockError::Error(err)) => {
                return Err(CaseError::DbConnection(format!(
                    "failed to acquire database access lock {}: {err}",
                    lock_path.display()
                )));
            }
        }
    }
}

fn db_lock_path(config: &DbConfig) -> PathBuf {
    PathBuf::from(format!("{}.access.lock", config.data_dir.to_string_lossy()))
}

fn shared_db_locks() -> &'static Mutex<HashMap<PathBuf, Weak<File>>> {
    static DB_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<File>>>> = OnceLock::new();
    DB_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shared_db_lock(lock_path: &PathBuf) -> Option<Arc<File>> {
    let mut locks = shared_db_locks()
        .lock()
        .expect("database lock registry should not be poisoned");
    let existing = locks.get(lock_path).and_then(Weak::upgrade);
    if existing.is_none() {
        locks.remove(lock_path);
    }
    existing
}

fn remember_db_lock(lock_path: PathBuf, file: Arc<File>) -> Arc<File> {
    shared_db_locks()
        .lock()
        .expect("database lock registry should not be poisoned")
        .insert(lock_path, Arc::downgrade(&file));
    file
}

fn is_lock_contention(message: &str) -> bool {
    let lowercase = message.to_lowercase();
    lowercase.contains("lock")
        && (lowercase.contains("resource temporarily unavailable")
            || lowercase.contains("no locks available")
            || lowercase.contains("lock hold by current process"))
}

fn text_matches_query(text: Option<&str>, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }

    text.map(|value| value.to_lowercase().contains(query))
        .unwrap_or(false)
}

#[derive(Default)]
struct MatchMeta {
    direction_seq: Option<u32>,
    entry_seq: Option<u32>,
    kind: Option<String>,
}

fn push_match(
    matches: &mut Vec<SearchMatch>,
    scope: &str,
    field: &str,
    text: Option<&str>,
    query: &str,
    meta: MatchMeta,
) {
    let Some(excerpt) = text.filter(|value| text_matches_query(Some(value), query)) else {
        return;
    };

    matches.push(SearchMatch {
        scope: scope.to_string(),
        field: field.to_string(),
        excerpt: excerpt.to_string(),
        direction_seq: meta.direction_seq,
        entry_seq: meta.entry_seq,
        kind: meta.kind,
    });
}

// ── Parsing helpers ──

fn parse_case(v: &Value) -> Option<Case> {
    let id = v.get("case_id")?.as_str()?.to_string();
    let goal_constraints: Vec<Constraint> = v
        .get("goal_constraints")
        .and_then(|gc| {
            if let Some(s) = gc.as_str() {
                serde_json::from_str(s).ok()
            } else {
                serde_json::from_value(gc.clone()).ok()
            }
        })
        .unwrap_or_default();

    Some(Case {
        id,
        repo_id: v.get("repo_id")?.as_str()?.to_string(),
        repo_label: v
            .get("repo_label")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        worktree_id: v
            .get("worktree_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        worktree_root: v
            .get("worktree_root")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        goal: v.get("goal")?.as_str()?.to_string(),
        goal_constraints,
        status: CaseStatus::from_str(v.get("status")?.as_str()?)?,
        current_direction_seq: v.get("current_direction_seq")?.as_u64()? as u32,
        current_step_id: v
            .get("current_step_id")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        opened_at: v.get("opened_at")?.as_str()?.to_string(),
        updated_at: v.get("updated_at")?.as_str()?.to_string(),
        closed_at: v
            .get("closed_at")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        close_summary: v
            .get("close_summary")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        abandoned_at: v
            .get("abandoned_at")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        abandon_summary: v
            .get("abandon_summary")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
    })
}

fn parse_single_direction(v: &Value) -> Option<Direction> {
    let constraints: Vec<Constraint> = v
        .get("constraints")
        .and_then(|c| {
            if let Some(s) = c.as_str() {
                serde_json::from_str(s).ok()
            } else {
                serde_json::from_value(c.clone()).ok()
            }
        })
        .unwrap_or_default();

    Some(Direction {
        case_id: v.get("case_id")?.as_str()?.to_string(),
        seq: v.get("seq")?.as_u64()? as u32,
        summary: v.get("summary")?.as_str()?.to_string(),
        constraints,
        success_condition: v
            .get("success_condition")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        abort_condition: v
            .get("abort_condition")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        reason: v
            .get("reason")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        context: v
            .get("context")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        created_at: v.get("created_at")?.as_str()?.to_string(),
    })
}

fn parse_single_step(v: &Value) -> Option<Step> {
    Some(Step {
        id: v.get("step_id")?.as_str()?.to_string(),
        case_id: v.get("case_id")?.as_str()?.to_string(),
        direction_seq: v.get("direction_seq")?.as_u64()? as u32,
        order_index: v.get("order_index")?.as_u64()? as u32,
        title: v.get("title")?.as_str()?.to_string(),
        status: StepStatus::from_str(v.get("status")?.as_str()?)?,
        reason: v
            .get("reason")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        created_at: v.get("created_at")?.as_str()?.to_string(),
        updated_at: v.get("updated_at")?.as_str()?.to_string(),
    })
}

fn parse_single_entry(v: &Value) -> Option<Entry> {
    let files: Vec<String> = v
        .get("files")
        .and_then(|f| {
            if let Some(s) = f.as_str() {
                serde_json::from_str(s).ok()
            } else {
                serde_json::from_value(f.clone()).ok()
            }
        })
        .unwrap_or_default();
    let artifacts: Vec<String> = v
        .get("artifacts")
        .and_then(|a| {
            if let Some(s) = a.as_str() {
                serde_json::from_str(s).ok()
            } else {
                serde_json::from_value(a.clone()).ok()
            }
        })
        .unwrap_or_default();

    let entry_type = match v.get("entry_type")?.as_str()? {
        "record" => EntryType::Record,
        "decision" => EntryType::Decision,
        "redirect" => EntryType::Redirect,
        _ => return None,
    };

    Some(Entry {
        case_id: v.get("case_id")?.as_str()?.to_string(),
        seq: v.get("seq")?.as_u64()? as u32,
        entry_type,
        kind: v
            .get("kind")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        summary: v.get("summary")?.as_str()?.to_string(),
        reason: v
            .get("reason")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        context: v
            .get("context")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        files,
        artifacts,
        created_at: v.get("created_at")?.as_str()?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_db_config(temp_dir: &TempDir) -> DbConfig {
        let db_path = temp_dir.path().join("case.db");
        DbConfig::from_data_dir(Some(
            db_path
                .to_str()
                .expect("temporary database path should be valid UTF-8"),
        ))
    }

    #[tokio::test]
    async fn database_access_lock_is_shared_within_process() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let first_lock = acquire_db_lock(&config)
            .await
            .expect("first lock should succeed");

        let started = Instant::now();
        let second_lock = acquire_db_lock(&config)
            .await
            .expect("second lock should reuse the process-local lock");

        assert!(started.elapsed() < Duration::from_millis(50));
        assert!(Arc::ptr_eq(&first_lock, &second_lock));
        drop(first_lock);
        drop(second_lock);
    }
}
