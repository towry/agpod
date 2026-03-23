//! JSON-line RPC protocol for case-server.
//!
//! Keywords: rpc, jsonl, case server, request response, semantic recall

use crate::cli::CaseCommand;
use crate::repo_id::RepoIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseRequest {
    pub repo: RepoIdentityPayload,
    pub command: CaseCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResponse {
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIdentityPayload {
    pub repo_id: String,
    pub repo_label: String,
    pub worktree_id: String,
    pub worktree_root: String,
}

impl From<RepoIdentity> for RepoIdentityPayload {
    fn from(value: RepoIdentity) -> Self {
        Self {
            repo_id: value.repo_id,
            repo_label: value.repo_label,
            worktree_id: value.worktree_id,
            worktree_root: value.worktree_root,
        }
    }
}

impl From<RepoIdentityPayload> for RepoIdentity {
    fn from(value: RepoIdentityPayload) -> Self {
        Self {
            repo_id: value.repo_id,
            repo_label: value.repo_label,
            worktree_id: value.worktree_id,
            worktree_root: value.worktree_root,
        }
    }
}
