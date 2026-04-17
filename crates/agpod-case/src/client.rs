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
use tracing::warn;
use uuid::Uuid;

const DB_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);
const DB_LOCK_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
const SLOW_QUERY_WARN_MS: u128 = 500;

#[derive(Clone)]
pub struct SharedDbHandle {
    db: Surreal<Db>,
    db_lock: Arc<File>,
    config: DbConfig,
}

#[derive(Clone)]
pub struct CaseClient {
    db: Surreal<Db>,
    _db_lock: Arc<File>,
    config: DbConfig,
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
            config: shared.config,
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
            config: self.config.clone(),
            repo_id: identity.repo_id,
            repo_label: identity.repo_label,
            worktree_id: identity.worktree_id,
            worktree_root: identity.worktree_root,
        }
    }

    pub fn config(&self) -> &DbConfig {
        &self.config
    }

    pub fn repo_id(&self) -> &str {
        self.repo_id.as_str()
    }

    pub fn repo_label(&self) -> &str {
        self.repo_label.as_str()
    }

    pub fn worktree_id(&self) -> &str {
        self.worktree_id.as_str()
    }

    pub fn worktree_root(&self) -> &str {
        self.worktree_root.as_str()
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

        Ok(Self {
            db,
            db_lock,
            config: config.clone(),
        })
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
            DEFINE FIELD IF NOT EXISTS close_confirm_token ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS close_confirm_action ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS close_confirm_summary ON case TYPE string;
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
            DEFINE FIELD IF NOT EXISTS step_id ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS summary ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS reason ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS context ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS files ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS artifacts ON entry TYPE string;
            DEFINE FIELD IF NOT EXISTS created_at ON entry TYPE string;
            DEFINE INDEX IF NOT EXISTS idx_entry_case ON entry FIELDS case_id;

            DEFINE TABLE IF NOT EXISTS session_record SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS session_record_id ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS repo_id ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS worktree_id ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS seq ON session_record TYPE int;
            DEFINE FIELD IF NOT EXISTS case_id ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS kind ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS summary ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS context ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS files ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS artifacts ON session_record TYPE string;
            DEFINE FIELD IF NOT EXISTS created_at ON session_record TYPE string;
            DEFINE INDEX IF NOT EXISTS idx_session_record_id ON session_record FIELDS session_record_id UNIQUE;
            DEFINE INDEX IF NOT EXISTS idx_session_record_scope ON session_record FIELDS repo_id, worktree_id, seq UNIQUE;
        ";

        self.db
            .query(schema)
            .await
            .map_err(|e| CaseError::DbInit(format!("schema init: {e}")))?;

        Ok(())
    }

    // ── Internal query helper ──

    async fn query_raw(&self, sql: &str, bindings: Value) -> CaseResult<Vec<Value>> {
        let started = Instant::now();
        let mut response = self
            .db
            .query(sql)
            .bind(bindings)
            .await
            .map_err(|e| CaseError::DbQuery(format!("{e}")))?;

        let result: Vec<Value> = response
            .take(0)
            .map_err(|e| CaseError::DbQuery(format!("take(0): {e}")))?;

        let elapsed = started.elapsed();
        if elapsed.as_millis() >= SLOW_QUERY_WARN_MS {
            let first_line = sql
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or(sql);
            warn!(
                repo_id = %self.repo_id,
                elapsed_ms = elapsed.as_millis(),
                sql = first_line.trim(),
                "case database query executed slowly"
            );
        }

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
             closed_at = '', close_summary = '', abandoned_at = '', abandon_summary = '', \
             close_confirm_token = '', close_confirm_action = '', close_confirm_summary = ''",
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
            close_confirm_token: None,
            close_confirm_action: None,
            close_confirm_summary: None,
        })
    }

    pub async fn set_close_confirmation(
        &self,
        case_id: &str,
        action: &str,
        summary: &str,
        confirm_token: &str,
    ) -> CaseResult<()> {
        let now = Utc::now().to_rfc3339();
        self.query_raw(
            "UPDATE case SET updated_at = $updated_at, \
             close_confirm_token = $confirm_token, close_confirm_action = $action, close_confirm_summary = $summary \
             WHERE case_id = $case_id",
            json!({
                "case_id": case_id,
                "updated_at": now,
                "confirm_token": confirm_token,
                "action": action,
                "summary": summary,
            }),
        )
        .await?;
        Ok(())
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
                 closed_at = $now, close_summary = $summary, \
                 close_confirm_token = '', close_confirm_action = '', close_confirm_summary = '' \
                 WHERE case_id = $case_id"
            }
            CaseStatus::Abandoned => {
                "UPDATE case SET status = $status, updated_at = $updated_at, \
                 abandoned_at = $now, abandon_summary = $summary, \
                 close_confirm_token = '', close_confirm_action = '', close_confirm_summary = '' \
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

    pub async fn reopen_case(&self, case_id: &str) -> CaseResult<()> {
        let now = Utc::now().to_rfc3339();
        self.query_raw(
            "UPDATE case SET status = 'open', updated_at = $updated_at, \
             closed_at = '', close_summary = '', abandoned_at = '', abandon_summary = '', \
             close_confirm_token = '', close_confirm_action = '', close_confirm_summary = '' \
             WHERE case_id = $case_id",
            json!({
                "case_id": case_id,
                "updated_at": now,
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
             updated_at = $updated_at, \
             close_confirm_token = '', close_confirm_action = '', close_confirm_summary = '' \
             WHERE case_id = $case_id",
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

    pub async fn find_direction(&self, case_id: &str, seq: u32) -> CaseResult<Option<Direction>> {
        let results = self
            .query_raw(
                "SELECT * FROM direction WHERE case_id = $case_id AND seq = $seq LIMIT 1",
                json!({ "case_id": case_id, "seq": seq }),
            )
            .await?;
        Ok(results.first().and_then(parse_single_direction))
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
        step_id: Option<&str>,
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
             kind = $kind, step_id = $step_id, summary = $summary, reason = $reason, context = $context, \
             files = $files, artifacts = $artifacts, created_at = $created_at",
            json!({
                "case_id": case_id,
                "seq": seq,
                "entry_type": entry_type.as_str(),
                "kind": kind.unwrap_or(""),
                "step_id": step_id.unwrap_or(""),
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
            step_id: step_id.map(String::from),
            summary: summary.to_string(),
            reason: reason.map(String::from),
            context: context.map(String::from),
            files: files.to_vec(),
            artifacts: artifacts.to_vec(),
            created_at: now,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_session_record(
        &self,
        seq: u32,
        case_id: Option<&str>,
        kind: RecordKind,
        summary: &str,
        context: Option<&str>,
        files: &[String],
        artifacts: &[String],
    ) -> CaseResult<SessionRecord> {
        let now = Utc::now().to_rfc3339();
        let record_id = format!("SR-{}", Uuid::new_v4().simple());
        let files_json =
            serde_json::to_string(files).map_err(|e| CaseError::Other(e.to_string()))?;
        let artifacts_json =
            serde_json::to_string(artifacts).map_err(|e| CaseError::Other(e.to_string()))?;

        self.query_raw(
            "CREATE session_record SET \
             session_record_id = $session_record_id, repo_id = $repo_id, worktree_id = $worktree_id, \
             seq = $seq, case_id = $case_id, kind = $kind, summary = $summary, context = $context, \
             files = $files, artifacts = $artifacts, created_at = $created_at",
            json!({
                "session_record_id": record_id,
                "repo_id": self.repo_id,
                "worktree_id": self.worktree_id,
                "seq": seq,
                "case_id": case_id.unwrap_or(""),
                "kind": kind.as_str(),
                "summary": summary,
                "context": context.unwrap_or(""),
                "files": files_json,
                "artifacts": artifacts_json,
                "created_at": now,
            }),
        )
        .await?;

        Ok(SessionRecord {
            id: record_id,
            repo_id: self.repo_id.clone(),
            worktree_id: self.worktree_id.clone(),
            seq,
            case_id: case_id.map(ToOwned::to_owned),
            kind,
            summary: summary.to_string(),
            context: context.map(ToOwned::to_owned),
            files: files.to_vec(),
            artifacts: artifacts.to_vec(),
            created_at: now,
        })
    }

    pub async fn get_session_record_count(&self) -> CaseResult<u32> {
        let records = self
            .query_raw(
                "SELECT * FROM session_record WHERE repo_id = $repo_id AND worktree_id = $worktree_id",
                json!({ "repo_id": self.repo_id, "worktree_id": self.worktree_id }),
            )
            .await?;
        Ok(records.len() as u32)
    }

    pub async fn list_session_records(&self) -> CaseResult<Vec<SessionRecord>> {
        let records = self
            .query_raw(
                "SELECT * FROM session_record WHERE repo_id = $repo_id AND worktree_id = $worktree_id ORDER BY seq DESC",
                json!({ "repo_id": self.repo_id, "worktree_id": self.worktree_id }),
            )
            .await?;
        Ok(records
            .iter()
            .filter_map(parse_single_session_record)
            .collect())
    }

    pub async fn search_session_records(&self, query: &str) -> CaseResult<Vec<SessionRecord>> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }

        let mut records = self.list_session_records().await?;
        records.retain(|record| {
            record.summary.to_lowercase().contains(&needle)
                || record
                    .context
                    .as_ref()
                    .is_some_and(|context| context.to_lowercase().contains(&needle))
                || record.kind.as_str().contains(&needle)
                || record
                    .files
                    .iter()
                    .any(|path| path.to_lowercase().contains(&needle))
        });
        Ok(records)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn advance_step(
        &self,
        case_id: &str,
        current_direction_seq: u32,
        completed_step_id: &str,
        record_seq: Option<u32>,
        record_kind: Option<&str>,
        record_summary: Option<&str>,
        record_context: Option<&str>,
        record_files: &[String],
        started_step_id: Option<&str>,
    ) -> CaseResult<Option<Entry>> {
        let now = Utc::now().to_rfc3339();
        let files_json =
            serde_json::to_string(record_files).map_err(|e| CaseError::Other(e.to_string()))?;
        let mut sql = String::from(
            "
            BEGIN TRANSACTION;

            UPDATE step SET status = 'done', reason = '', updated_at = $updated_at
            WHERE step_id = $completed_step_id;
            ",
        );
        if record_seq.is_some() {
            sql.push_str(
                "
                CREATE entry SET
                    case_id = $case_id,
                    seq = $record_seq,
                    entry_type = 'record',
                    kind = $record_kind,
                    step_id = $completed_step_id,
                    summary = $record_summary,
                    reason = '',
                    context = $record_context,
                    files = $record_files,
                    artifacts = '[]',
                    created_at = $created_at;
                ",
            );
        }
        sql.push_str(
            "
            UPDATE step SET status = 'pending', reason = '', updated_at = $updated_at
            WHERE case_id = $case_id
              AND direction_seq = $current_direction_seq
              AND status = 'active'
              AND step_id != $completed_step_id;
            ",
        );
        if started_step_id.is_some() {
            sql.push_str(
                "
                UPDATE step SET status = 'active', reason = '', updated_at = $updated_at
                WHERE step_id = $started_step_id;
                ",
            );
        }
        sql.push_str(
            "
            UPDATE case SET
                current_step_id = $next_current_step_id,
                updated_at = $updated_at
            WHERE case_id = $case_id;

            COMMIT TRANSACTION;
            ",
        );

        self.query_raw(
            &sql,
            json!({
                "case_id": case_id,
                "current_direction_seq": current_direction_seq,
                "completed_step_id": completed_step_id,
                "record_seq": record_seq,
                "record_kind": record_kind.unwrap_or(""),
                "record_summary": record_summary.unwrap_or(""),
                "record_context": record_context.unwrap_or(""),
                "record_files": files_json,
                "started_step_id": started_step_id.unwrap_or(""),
                "next_current_step_id": started_step_id.unwrap_or(""),
                "updated_at": now,
                "created_at": now,
            }),
        )
        .await?;
        Ok(record_seq.map(|record_seq| Entry {
            case_id: case_id.to_string(),
            seq: record_seq,
            entry_type: EntryType::Record,
            kind: record_kind.map(str::to_string),
            step_id: Some(completed_step_id.to_string()),
            summary: record_summary.unwrap_or("").to_string(),
            reason: Some(String::new()),
            context: record_context.map(str::to_string),
            files: record_files.to_vec(),
            artifacts: vec![],
            created_at: now,
        }))
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

    pub async fn get_latest_relevant_entry_for_next_action(
        &self,
        case_id: &str,
    ) -> CaseResult<Option<Entry>> {
        let results = self
            .query_raw(
                "SELECT * FROM entry
                 WHERE case_id = $case_id
                   AND (
                     entry_type != 'record'
                     OR step_id = ''
                     OR kind != 'note'
                     OR summary != ''
                   )
                 ORDER BY seq DESC
                 LIMIT 1",
                json!({ "case_id": case_id }),
            )
            .await?;
        Ok(results.first().and_then(parse_single_entry))
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
        close_confirm_token: v
            .get("close_confirm_token")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        close_confirm_action: v
            .get("close_confirm_action")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        close_confirm_summary: v
            .get("close_confirm_summary")
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
        step_id: v
            .get("step_id")
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

fn parse_single_session_record(v: &Value) -> Option<SessionRecord> {
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
    let case_id = v
        .get("case_id")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let kind = v.get("kind")?.as_str()?.parse::<RecordKind>().ok()?;

    Some(SessionRecord {
        id: v.get("session_record_id")?.as_str()?.to_string(),
        repo_id: v.get("repo_id")?.as_str()?.to_string(),
        worktree_id: v.get("worktree_id")?.as_str()?.to_string(),
        seq: v.get("seq")?.as_u64()? as u32,
        case_id,
        kind,
        summary: v.get("summary")?.as_str()?.to_string(),
        context: v
            .get("context")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned),
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

    #[tokio::test]
    async fn update_case_direction_clears_close_confirmation_fields() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        client
            .create_case("C-legacy", "goal", &[])
            .await
            .expect("case should be created");

        client
            .set_close_confirmation("C-legacy", "close", "summary", "token")
            .await
            .expect("close confirmation should be set");

        client
            .update_case_direction("C-legacy", 2)
            .await
            .expect("case update should clear close confirmation fields");

        let raw = client
            .query_raw(
                "SELECT close_confirm_token, close_confirm_action, close_confirm_summary, current_direction_seq \
                 FROM case WHERE case_id = $case_id LIMIT 1",
                json!({ "case_id": "C-legacy" }),
            )
            .await
            .expect("query should succeed");
        let case = raw.first().expect("legacy case should exist");

        assert_eq!(case["close_confirm_token"].as_str(), Some(""));
        assert_eq!(case["close_confirm_action"].as_str(), Some(""));
        assert_eq!(case["close_confirm_summary"].as_str(), Some(""));
        assert_eq!(case["current_direction_seq"].as_u64(), Some(2));
    }
}
