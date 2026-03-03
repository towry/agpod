//! Local storage layout: XDG_DATA_HOME + XDG_RUNTIME_DIR paths.
//!
//! Keywords: flow storage, graph cache, session directory, XDG

use crate::error::{FlowError, FlowResult};
use crate::repo_id::RepoIdentity;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Base data directory: $XDG_DATA_HOME/agpod/flow
pub fn data_base_dir() -> FlowResult<PathBuf> {
    let base = match std::env::var("XDG_DATA_HOME") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => dirs::home_dir()
            .map(|h| h.join(".local").join("share"))
            .ok_or_else(|| FlowError::Other("Cannot determine home directory".into()))?,
    };
    Ok(base.join("agpod").join("flow"))
}

/// Repo-specific data directory.
pub fn repo_data_dir(identity: &RepoIdentity) -> FlowResult<PathBuf> {
    Ok(data_base_dir()?.join("repos").join(&identity.repo_id))
}

/// Ensure repo data directory exists.
pub fn ensure_repo_data_dir(identity: &RepoIdentity) -> FlowResult<PathBuf> {
    let dir = repo_data_dir(identity)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Sessions runtime directory.
pub fn sessions_dir() -> FlowResult<PathBuf> {
    let base = match std::env::var("XDG_RUNTIME_DIR") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from("/tmp"),
    };
    Ok(base.join("agpod").join("flow").join("sessions"))
}

/// Ensure sessions directory exists.
pub fn ensure_sessions_dir() -> FlowResult<PathBuf> {
    let dir = sessions_dir()?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Path to graph.json for a repo.
pub fn graph_path(identity: &RepoIdentity) -> FlowResult<PathBuf> {
    Ok(repo_data_dir(identity)?.join("graph.json"))
}

/// Path to a specific session file.
pub fn session_path(session_id: &str) -> FlowResult<PathBuf> {
    Ok(sessions_dir()?.join(format!("{session_id}.json")))
}

/// Write a file atomically: write temp file in same directory and rename.
pub fn write_atomic(path: &Path, content: &str) -> FlowResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| FlowError::Other(format!("Invalid path: {}", path.display())))?;
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| FlowError::Other(format!("Invalid file name: {}", path.display())))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_path = parent.join(format!(".{file_name}.{nonce}.tmp"));

    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}
