//! SurrealDB embedded client for case operations.
//!
//! Keywords: surrealdb client, case client, database, query

use crate::config::DbConfig;
use crate::error::{CaseError, CaseResult};
use crate::types::*;
use chrono::Utc;
use serde_json::{json, Value};
use surrealdb::engine::local::{Db, RocksDb};
use surrealdb::Surreal;

pub struct CaseClient {
    db: Surreal<Db>,
    repo_id: String,
}

impl CaseClient {
    pub async fn new(config: &DbConfig, repo_id: String) -> CaseResult<Self> {
        // Ensure parent directory exists
        if let Some(parent) = config.data_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CaseError::DbInit(format!("failed to create data directory: {e}")))?;
        }

        let db = Surreal::new::<RocksDb>(config.data_dir.to_string_lossy().as_ref())
            .await
            .map_err(|e| CaseError::DbConnection(format!("{e}")))?;

        db.use_ns("agpod")
            .use_db("case")
            .await
            .map_err(|e| CaseError::DbInit(format!("namespace/db init: {e}")))?;

        let client = Self { db, repo_id };
        client.ensure_schema().await?;
        Ok(client)
    }

    async fn ensure_schema(&self) -> CaseResult<()> {
        let schema = "
            DEFINE TABLE IF NOT EXISTS case SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS case_id ON case TYPE string;
            DEFINE FIELD IF NOT EXISTS repo_id ON case TYPE string;
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

    #[allow(dead_code)]
    pub fn repo_id(&self) -> &str {
        &self.repo_id
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
             goal_constraints = $goal_constraints, status = 'open', \
             current_direction_seq = $current_direction_seq, current_step_id = '', \
             opened_at = $opened_at, updated_at = $updated_at, \
             closed_at = '', close_summary = '', abandoned_at = '', abandon_summary = ''",
            json!({
                "case_id": case_id,
                "repo_id": self.repo_id,
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
            .ok_or_else(|| CaseError::Other(format!("step not found: {step_id}")))
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

    pub async fn search_cases(&self, query: &str) -> CaseResult<Vec<Case>> {
        let results = self
            .query_raw(
                "SELECT * FROM case WHERE repo_id = $repo_id AND string::lowercase(goal) CONTAINS string::lowercase($query)",
                json!({ "repo_id": self.repo_id, "query": query }),
            )
            .await?;
        Ok(results.iter().filter_map(parse_case).collect())
    }

    pub async fn get_case(&self, case_id: &str) -> CaseResult<Case> {
        let results = self
            .query_raw(
                "SELECT * FROM case WHERE case_id = $case_id LIMIT 1",
                json!({ "case_id": case_id }),
            )
            .await?;
        results
            .first()
            .and_then(parse_case)
            .ok_or_else(|| CaseError::CaseNotFound(case_id.to_string()))
    }

    pub async fn count_cases_today(&self) -> CaseResult<u32> {
        let today_prefix = format!("C-{}", Utc::now().format("%Y%m%d"));
        let results = self
            .query_raw(
                "SELECT * FROM case WHERE repo_id = $repo_id AND string::starts_with(case_id, $prefix)",
                json!({ "repo_id": self.repo_id, "prefix": today_prefix }),
            )
            .await?;
        Ok(results.len() as u32)
    }
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
