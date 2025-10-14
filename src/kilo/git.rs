use std::process::Command;
use crate::kilo::error::{KiloError, KiloResult};
use crate::kilo::template::GitInfo;

pub struct GitHelper;

impl GitHelper {
    pub fn get_git_info() -> Option<GitInfo> {
        let repo_root = Self::get_repo_root().ok()?;
        let current_branch = Self::get_current_branch().ok();
        let short_sha = Self::get_short_sha().ok();
        
        Some(GitInfo {
            repo_root,
            current_branch,
            short_sha,
        })
    }
    
    pub fn get_repo_root() -> KiloResult<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|e| KiloError::Git(format!("Failed to execute git: {}", e)))?;
        
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(KiloError::Git("Not in a git repository".to_string()))
        }
    }
    
    pub fn get_current_branch() -> KiloResult<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .map_err(|e| KiloError::Git(format!("Failed to execute git: {}", e)))?;
        
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(KiloError::Git("Failed to get current branch".to_string()))
        }
    }
    
    pub fn get_short_sha() -> KiloResult<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .map_err(|e| KiloError::Git(format!("Failed to execute git: {}", e)))?;
        
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(KiloError::Git("Failed to get HEAD SHA".to_string()))
        }
    }
    
    pub fn create_and_checkout_branch(branch_name: &str) -> KiloResult<()> {
        let output = Command::new("git")
            .args(["checkout", "-b", branch_name])
            .output()
            .map_err(|e| KiloError::Git(format!("Failed to execute git: {}", e)))?;
        
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(KiloError::Git(format!("Failed to create branch: {}", stderr)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_git_info() {
        // This test will only work if we're in a git repository
        // We'll make it optional
        if let Some(info) = GitHelper::get_git_info() {
            assert!(!info.repo_root.is_empty());
        }
    }
}
