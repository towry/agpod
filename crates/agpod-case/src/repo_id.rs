//! repo-id computation from Git remote URL.
//!
//! Keywords: repo-id, repository identity, git remote, normalize url

use crate::error::{CaseError, CaseResult};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Normalized repo identity derived from git remote URL.
#[derive(Debug, Clone)]
pub struct RepoIdentity {
    /// Stable hex hash: hex(sha256("v1:" + normalized))[0..16]
    pub repo_id: String,
    /// Human-readable label, e.g. "github.com/towry/agpod"
    pub repo_label: String,
    /// Stable worktree hash: hex(sha256("wt1:" + canonical_root))[0..16]
    pub worktree_id: String,
    /// Canonical worktree root path.
    pub worktree_root: String,
}

impl RepoIdentity {
    /// Resolve from a given repo root path.
    pub fn resolve_from(repo_root: &Path) -> CaseResult<Self> {
        let url = get_remote_url(Some(repo_root))?;
        let normalized = normalize_git_url(&url);
        let worktree_root = get_worktree_root(Some(repo_root))?;
        let repo_id = compute_repo_id(&normalized);
        let worktree_id = compute_worktree_id(&worktree_root);
        Ok(Self {
            repo_id,
            repo_label: normalized,
            worktree_id,
            worktree_root,
        })
    }
}

/// Try remotes in order: origin -> upstream -> first alphabetically.
fn get_remote_url(cwd: Option<&Path>) -> CaseResult<String> {
    let mut check = Command::new("git");
    check.args(["rev-parse", "--git-dir"]);
    if let Some(dir) = cwd {
        check.current_dir(dir);
    }
    let output = check
        .output()
        .map_err(|e| CaseError::Git(format!("failed to execute git: {e}")))?;
    if !output.status.success() {
        return Err(CaseError::NotGitRepo);
    }

    for name in &["origin", "upstream"] {
        if let Some(url) = try_get_remote(name, cwd) {
            return Ok(url);
        }
    }

    let mut cmd = Command::new("git");
    cmd.args(["remote"]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().map_err(|e| CaseError::Git(e.to_string()))?;
    if output.status.success() {
        let remotes_raw = String::from_utf8_lossy(&output.stdout).to_string();
        let mut remotes: Vec<&str> = remotes_raw
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        remotes.sort();
        if let Some(first) = remotes.first() {
            if let Some(url) = try_get_remote(first, cwd) {
                return Ok(url);
            }
        }
    }

    Err(CaseError::NoGitRemote)
}

fn try_get_remote(name: &str, cwd: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(["remote", "get-url", name]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !url.is_empty() {
            return Some(url);
        }
    }
    None
}

fn get_worktree_root(cwd: Option<&Path>) -> CaseResult<String> {
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--show-toplevel"]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().map_err(|e| CaseError::Git(e.to_string()))?;
    if !output.status.success() {
        return Err(CaseError::NotGitRepo);
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return Err(CaseError::Git(
            "git rev-parse --show-toplevel returned an empty path".to_string(),
        ));
    }

    let canonical = fs::canonicalize(&root)
        .map_err(|e| CaseError::Git(format!("failed to canonicalize worktree root: {e}")))?;
    Ok(canonical.to_string_lossy().to_string())
}

/// Normalize a git remote URL to `host/full_path` form.
///
/// Handles ssh shorthand, ssh://, https://, http:// schemes.
pub fn normalize_git_url(raw: &str) -> String {
    let s = raw.trim();

    // SSH shorthand: git@host:owner/repo.git
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some(colon_pos) = rest.find(':') {
            let host = &rest[..colon_pos];
            let path = &rest[colon_pos + 1..];
            return format_normalized(host, path);
        }
    }

    // Protocol URLs: ssh://git@host/path, https://host/path
    for scheme in &["ssh://", "https://", "http://"] {
        if let Some(without_scheme) = s.strip_prefix(scheme) {
            let without_user = match without_scheme.find('@') {
                Some(pos) => &without_scheme[pos + 1..],
                None => without_scheme,
            };
            if let Some(slash_pos) = without_user.find('/') {
                let host = &without_user[..slash_pos];
                let path = &without_user[slash_pos + 1..];
                return format_normalized(host, path);
            }
        }
    }

    s.to_lowercase()
}

fn format_normalized(host: &str, path: &str) -> String {
    let host = host.to_lowercase();
    let path = path
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_lowercase();
    format!("{host}/{path}")
}

/// repo_id = hex(sha256("v1:" + normalized))[0..16]
fn compute_repo_id(normalized: &str) -> String {
    let source = format!("v1:{normalized}");
    let hash = Sha256::digest(source.as_bytes());
    hash[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// worktree_id = hex(sha256("wt1:" + canonical_root))[0..16]
fn compute_worktree_id(canonical_root: &str) -> String {
    let source = format!("wt1:{canonical_root}");
    let hash = Sha256::digest(source.as_bytes());
    hash[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ssh_shorthand() {
        assert_eq!(
            normalize_git_url("git@github.com:Org/Repo.git"),
            "github.com/org/repo"
        );
    }

    #[test]
    fn normalize_https() {
        assert_eq!(
            normalize_git_url("https://github.com/Org/Repo.git"),
            "github.com/org/repo"
        );
    }

    #[test]
    fn normalize_ssh_protocol() {
        assert_eq!(
            normalize_git_url("ssh://git@github.com/Org/Repo.git"),
            "github.com/org/repo"
        );
    }

    #[test]
    fn normalize_no_git_suffix() {
        assert_eq!(
            normalize_git_url("https://github.com/towry/agpod"),
            "github.com/towry/agpod"
        );
    }

    #[test]
    fn ssh_and_https_produce_same_id() {
        let ssh = normalize_git_url("git@github.com:towry/agpod.git");
        let https = normalize_git_url("https://github.com/towry/agpod.git");
        assert_eq!(ssh, https);
        assert_eq!(compute_repo_id(&ssh), compute_repo_id(&https));
    }

    #[test]
    fn repo_id_is_16_hex_chars() {
        let id = compute_repo_id("github.com/towry/agpod");
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn worktree_id_is_16_hex_chars() {
        let id = compute_worktree_id("/tmp/agpod-worktree");
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn distinct_worktree_paths_produce_distinct_ids() {
        let first = compute_worktree_id("/tmp/agpod-worktree-a");
        let second = compute_worktree_id("/tmp/agpod-worktree-b");
        assert_ne!(first, second);
    }
}
