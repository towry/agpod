//! Configuration management for agpod
//!
//! Supports feature-specific configuration sections:
//! - [kiro] - PR draft workflow settings
//! - [diff] - Git diff minimization settings

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Root configuration structure supporting multiple features
#[allow(dead_code)] // Public API for library users
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Kiro workflow configuration
    #[serde(default)]
    pub kiro: Option<KiroConfig>,

    /// Diff minimization configuration
    #[serde(default)]
    pub diff: Option<DiffConfig>,
}

/// Configuration for Kiro workflow
#[allow(dead_code)] // Public API for library users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroConfig {
    #[serde(default = "default_kiro_base_dir")]
    pub base_dir: String,

    #[serde(default = "default_templates_dir")]
    pub templates_dir: String,

    #[serde(default = "default_plugins_dir")]
    pub plugins_dir: String,

    #[serde(default = "default_template")]
    pub template: String,

    #[serde(default = "default_summary_lines")]
    pub summary_lines: usize,

    // For backward compatibility, keep existing nested structures
    #[serde(flatten)]
    pub legacy_fields: serde_json::Value,
}

impl Default for KiroConfig {
    fn default() -> Self {
        Self {
            base_dir: default_kiro_base_dir(),
            templates_dir: default_templates_dir(),
            plugins_dir: default_plugins_dir(),
            template: default_template(),
            summary_lines: default_summary_lines(),
            legacy_fields: serde_json::Value::Null,
        }
    }
}

/// Configuration for diff minimization
#[allow(dead_code)] // Public API for library users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffConfig {
    /// Default output directory for saved chunks
    #[serde(default = "default_diff_output_dir")]
    pub output_dir: String,

    /// Threshold for considering a file "large" (number of changes)
    #[serde(default = "default_large_file_changes_threshold")]
    pub large_file_changes_threshold: usize,

    /// Threshold for considering a file "large" (total lines)
    #[serde(default = "default_large_file_lines_threshold")]
    pub large_file_lines_threshold: usize,

    /// Maximum consecutive empty lines to keep
    #[serde(default = "default_max_consecutive_empty_lines")]
    pub max_consecutive_empty_lines: usize,
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self {
            output_dir: default_diff_output_dir(),
            large_file_changes_threshold: default_large_file_changes_threshold(),
            large_file_lines_threshold: default_large_file_lines_threshold(),
            max_consecutive_empty_lines: default_max_consecutive_empty_lines(),
        }
    }
}

// Default value functions for Kiro
fn default_kiro_base_dir() -> String {
    "llm/kiro".to_string()
}

fn default_templates_dir() -> String {
    if let Some(home_dir) = dirs::home_dir() {
        home_dir
            .join(".config")
            .join("agpod")
            .join("templates")
            .to_string_lossy()
            .to_string()
    } else {
        "~/.config/agpod/templates".to_string()
    }
}

fn default_plugins_dir() -> String {
    if let Some(home_dir) = dirs::home_dir() {
        home_dir
            .join(".config")
            .join("agpod")
            .join("plugins")
            .to_string_lossy()
            .to_string()
    } else {
        "~/.config/agpod/plugins".to_string()
    }
}

fn default_template() -> String {
    "default".to_string()
}

fn default_summary_lines() -> usize {
    3
}

// Default value functions for Diff
fn default_diff_output_dir() -> String {
    "llm/diff".to_string()
}

fn default_large_file_changes_threshold() -> usize {
    100
}

fn default_large_file_lines_threshold() -> usize {
    500
}

fn default_max_consecutive_empty_lines() -> usize {
    2
}

impl Config {
    /// Load configuration from file
    #[allow(dead_code)] // Public API for library users
    pub fn load_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Get the default config directory path
    #[allow(dead_code)] // Public API for library users
    pub fn get_config_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("agpod"))
    }

    /// Load configuration with priority:
    /// 1. Defaults
    /// 2. Global config (~/.config/agpod/config.toml)
    /// 3. Repo config (.agpod.toml)
    #[allow(dead_code)] // Public API for library users
    pub fn load() -> Self {
        let mut config = Self::default();

        // Try to load global config
        if let Some(config_dir) = Self::get_config_dir() {
            let global_config = config_dir.join("config.toml");
            if global_config.exists() {
                if let Ok(loaded) = Self::load_from_file(&global_config) {
                    config = config.merge(loaded);
                }
            }
        }

        // Try to load repo config
        let repo_config = PathBuf::from(".agpod.toml");
        if repo_config.exists() {
            if let Ok(loaded) = Self::load_from_file(&repo_config) {
                config = config.merge(loaded);
            }
        }

        config
    }

    /// Merge another config into this one (other takes precedence)
    #[allow(dead_code)] // Public API for library users
    pub fn merge(mut self, other: Config) -> Self {
        if other.kiro.is_some() {
            self.kiro = other.kiro;
        }
        if other.diff.is_some() {
            self.diff = other.diff;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.kiro.is_none());
        assert!(config.diff.is_none());
    }

    #[test]
    fn test_diff_config_defaults() {
        let diff_config = DiffConfig::default();
        assert_eq!(diff_config.output_dir, "llm/diff");
        assert_eq!(diff_config.large_file_changes_threshold, 100);
        assert_eq!(diff_config.large_file_lines_threshold, 500);
        assert_eq!(diff_config.max_consecutive_empty_lines, 2);
    }

    #[test]
    fn test_kiro_config_defaults() {
        let kiro_config = KiroConfig::default();
        assert_eq!(kiro_config.base_dir, "llm/kiro");
        assert_eq!(kiro_config.template, "default");
        assert_eq!(kiro_config.summary_lines, 3);
    }

    #[test]
    fn test_parse_config_with_sections() {
        let toml_str = r#"
[kiro]
base_dir = "custom/kiro"
template = "rust"

[diff]
output_dir = "custom/diff"
large_file_changes_threshold = 200
"#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.kiro.is_some());
        assert!(config.diff.is_some());

        let kiro = config.kiro.unwrap();
        assert_eq!(kiro.base_dir, "custom/kiro");
        assert_eq!(kiro.template, "rust");

        let diff = config.diff.unwrap();
        assert_eq!(diff.output_dir, "custom/diff");
        assert_eq!(diff.large_file_changes_threshold, 200);
    }
}
