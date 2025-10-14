use crate::kilo::config::Config;
use crate::kilo::error::KiloResult;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct PluginExecutor {
    config: Config,
}

impl PluginExecutor {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn generate_branch_name(
        &self,
        desc: &str,
        template: &str,
        prefix: &str,
    ) -> KiloResult<String> {
        let plugin_config = &self.config.plugins.branch_name;

        if !plugin_config.enabled {
            return Ok(crate::kilo::slug::generate_branch_name(desc, prefix));
        }

        // Determine plugin path
        let plugin_path = if Path::new(&plugin_config.command).is_absolute() {
            plugin_config.command.clone()
        } else {
            Path::new(&self.config.plugins_dir)
                .join(&plugin_config.command)
                .to_string_lossy()
                .to_string()
        };

        if !Path::new(&plugin_path).exists() {
            eprintln!(
                "Warning: Plugin not found at {}, using default branch name generation",
                plugin_path
            );
            return Ok(crate::kilo::slug::generate_branch_name(desc, prefix));
        }

        // Prepare environment variables
        let mut env_vars = HashMap::new();
        env_vars.insert("AGPOD_DESC".to_string(), desc.to_string());
        env_vars.insert("AGPOD_TEMPLATE".to_string(), template.to_string());
        env_vars.insert("AGPOD_BRANCH_PREFIX".to_string(), prefix.to_string());
        env_vars.insert(
            "AGPOD_TIME_ISO".to_string(),
            chrono::Utc::now().to_rfc3339(),
        );
        env_vars.insert("AGPOD_BASE_DIR".to_string(), self.config.base_dir.clone());

        // Add user
        if let Ok(user) = std::env::var("USER") {
            env_vars.insert("AGPOD_USER".to_string(), user);
        }

        // Add git repo root if available
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
        {
            if output.status.success() {
                if let Ok(repo_root) = String::from_utf8(output.stdout) {
                    env_vars.insert("AGPOD_REPO_ROOT".to_string(), repo_root.trim().to_string());
                }
            }
        }

        // Filter environment variables based on pass_env patterns
        let current_env: HashMap<String, String> = std::env::vars().collect();
        for pattern in &plugin_config.pass_env {
            if pattern.ends_with('*') {
                let prefix = &pattern[..pattern.len() - 1];
                for (key, value) in &current_env {
                    if key.starts_with(prefix) {
                        env_vars.insert(key.clone(), value.clone());
                    }
                }
            } else if let Some(value) = current_env.get(pattern) {
                env_vars.insert(pattern.clone(), value.clone());
            }
        }

        // Execute plugin
        let output = Command::new(&plugin_path)
            .envs(&env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    let branch_name = String::from_utf8_lossy(&output.stdout).trim().to_string();

                    // Validate and sanitize the branch name
                    let sanitized = sanitize_branch_name(&branch_name);

                    if sanitized.is_empty() {
                        eprintln!("Warning: Plugin returned empty branch name, using default");
                        Ok(crate::kilo::slug::generate_branch_name(desc, prefix))
                    } else {
                        Ok(sanitized)
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    eprintln!(
                        "Warning: Plugin failed with exit code {}: {}",
                        output.status.code().unwrap_or(-1),
                        stderr
                    );
                    eprintln!("Falling back to default branch name generation");
                    Ok(crate::kilo::slug::generate_branch_name(desc, prefix))
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to execute plugin: {}", e);
                eprintln!("Falling back to default branch name generation");
                Ok(crate::kilo::slug::generate_branch_name(desc, prefix))
            }
        }
    }
}

/// Sanitize branch name to remove dangerous characters
fn sanitize_branch_name(name: &str) -> String {
    let mut result = String::new();

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric()
            || ch == '-'
            || ch == '_'
            || ch == '.'
            || (!ch.is_ascii() && ch.is_alphabetic())
        // Allow non-ASCII alphabetic (e.g., Chinese)
        {
            result.push(ch);
        } else if ch.is_whitespace() && !result.is_empty() && !result.ends_with('-') {
            result.push('-');
        }
    }

    // Remove trailing hyphens
    while result.ends_with('-') {
        result.pop();
    }

    // Limit length
    if result.len() > 80 {
        result.truncate(80);
        while result.ends_with('-') {
            result.pop();
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_branch_name() {
        assert_eq!(sanitize_branch_name("hello-world"), "hello-world");
        assert_eq!(sanitize_branch_name("hello/world"), "helloworld");
        assert_eq!(sanitize_branch_name("hello..world"), "hello..world");
        assert_eq!(sanitize_branch_name("hello world"), "hello-world");
        assert_eq!(sanitize_branch_name("hello---world"), "hello---world");

        // Remove trailing hyphens
        assert_eq!(sanitize_branch_name("hello-"), "hello");
        assert_eq!(sanitize_branch_name("hello--"), "hello");

        // Length limit
        let long_name = "a".repeat(100);
        let sanitized = sanitize_branch_name(&long_name);
        assert_eq!(sanitized.len(), 80);
    }

    #[test]
    fn test_sanitize_special_chars() {
        assert_eq!(sanitize_branch_name("test!@#$%"), "test");
        assert_eq!(sanitize_branch_name("test & demo"), "test-demo");
    }
}
