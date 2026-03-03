//! Session lifecycle management.
//!
//! Keywords: session, active task, focus, fork, parent, session new

use crate::error::{FlowError, FlowResult};
use crate::storage;
use serde::{Deserialize, Serialize};

/// Session state (stored in runtime directory).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub session_id: String,
    pub repo_id: String,
    pub active_task_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Create a new session. Returns session_id.
pub fn create(repo_id: &str) -> FlowResult<Session> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let session_id = generate_session_id();

    let session = Session {
        version: 1,
        session_id: session_id.clone(),
        repo_id: repo_id.to_string(),
        active_task_id: None,
        created_at: now.clone(),
        updated_at: now,
    };

    save_session(&session)?;
    Ok(session)
}

/// Load session by id.
pub fn load(session_id: &str) -> FlowResult<Session> {
    let path = storage::session_path(session_id)?;
    if !path.exists() {
        return Err(FlowError::SessionNotFound(session_id.to_string()));
    }
    let content = std::fs::read_to_string(&path)?;
    let session: Session = serde_json::from_str(&content)?;
    Ok(session)
}

/// List sessions for a repo.
pub fn list(repo_id: &str) -> FlowResult<Vec<Session>> {
    let dir = storage::sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        if let Ok(session) = serde_json::from_str::<Session>(&content) {
            if session.repo_id == repo_id {
                sessions.push(session);
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

/// Set focus to a task.
pub fn focus(session_id: &str, task_id: &str) -> FlowResult<Session> {
    let mut session = load(session_id)?;
    session.active_task_id = Some(task_id.to_string());
    session.updated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    save_session(&session)?;
    Ok(session)
}

/// Get active task id, or error if none.
pub fn require_active_task(session: &Session) -> FlowResult<&str> {
    session
        .active_task_id
        .as_deref()
        .ok_or_else(|| FlowError::NoActiveTask {
            session_id: session.session_id.clone(),
        })
}

/// Close (delete) a session.
pub fn close(session_id: &str) -> FlowResult<()> {
    let path = storage::session_path(session_id)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

fn save_session(session: &Session) -> FlowResult<()> {
    storage::ensure_sessions_dir()?;
    let path = storage::session_path(&session.session_id)?;
    let json = serde_json::to_string_pretty(session)?;
    storage::write_atomic(&path, &json)?;
    Ok(())
}

fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    // S- prefix + last 6 hex chars of timestamp hash
    let hash = format!("{ts:x}");
    let suffix = if hash.len() >= 6 {
        &hash[hash.len() - 6..]
    } else {
        &hash
    };
    format!("S-{suffix}")
}
