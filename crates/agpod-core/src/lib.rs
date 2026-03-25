//! Configuration management for agpod.
//!
//! Supports feature-specific configuration sections:
//! - [diff] - Git diff minimization settings
//! - [case] - Case server settings

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Current configuration version.
pub const CURRENT_CONFIG_VERSION: &str = "1";

/// Supported configuration versions.
pub const SUPPORTED_CONFIG_VERSIONS: &[&str] = &["1"];

/// Root configuration structure.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Configuration version for tracking schema changes.
    #[serde(default = "default_config_version")]
    pub version: String,

    /// Diff minimization configuration.
    #[serde(default)]
    pub diff: Option<DiffConfig>,

    /// Case workflow configuration.
    #[serde(default)]
    pub case: Option<CaseConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: default_config_version(),
            diff: None,
            case: None,
        }
    }
}

/// Configuration for case server / client access.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseConfig {
    #[serde(default)]
    pub data_dir: Option<String>,

    #[serde(default)]
    pub server_addr: Option<String>,

    #[serde(default)]
    pub auto_start: Option<bool>,

    #[serde(default)]
    pub access_mode: Option<String>,

    #[serde(default)]
    pub semantic_recall_enabled: Option<bool>,

    #[serde(default)]
    pub vector_digest_job_enabled: Option<bool>,

    #[serde(default)]
    pub honcho_enabled: Option<bool>,

    #[serde(default)]
    pub honcho_sync_enabled: Option<bool>,

    #[serde(default)]
    pub honcho_base_url: Option<String>,

    #[serde(default)]
    pub honcho_workspace_id: Option<String>,

    #[serde(default)]
    pub honcho_api_key: Option<String>,

    #[serde(default)]
    pub honcho_api_key_env: Option<String>,

    #[serde(default)]
    pub honcho_peer_id: Option<String>,

    #[serde(default)]
    pub plugins: Option<CasePluginsConfig>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CasePluginsConfig {
    #[serde(default)]
    pub honcho: Option<CaseHonchoPluginConfig>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaseHonchoPluginConfig {
    #[serde(default)]
    pub enabled: Option<bool>,

    #[serde(default)]
    pub sync_enabled: Option<bool>,

    #[serde(default)]
    pub base_url: Option<String>,

    #[serde(default)]
    pub workspace_id: Option<String>,

    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(default)]
    pub api_key_env: Option<String>,

    #[serde(default)]
    pub peer_id: Option<String>,
}

/// Configuration for diff minimization.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffConfig {
    /// Default output directory for saved chunks.
    #[serde(default = "default_diff_output_dir")]
    pub output_dir: String,

    /// Threshold for considering a file "large" by change count.
    #[serde(default = "default_large_file_changes_threshold")]
    pub large_file_changes_threshold: usize,

    /// Threshold for considering a file "large" by total line count.
    #[serde(default = "default_large_file_lines_threshold")]
    pub large_file_lines_threshold: usize,

    /// Maximum consecutive empty lines to keep.
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

fn default_config_version() -> String {
    CURRENT_CONFIG_VERSION.to_string()
}

/// Get the configuration home directory, respecting XDG_CONFIG_HOME.
#[allow(dead_code)]
pub fn get_config_home() -> Option<PathBuf> {
    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        if !xdg_config_home.is_empty() {
            return Some(PathBuf::from(xdg_config_home));
        }
    }

    dirs::home_dir().map(|h| h.join(".config"))
}

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
    /// Check if the configuration version is supported.
    #[allow(dead_code)]
    pub fn is_version_supported(&self) -> bool {
        SUPPORTED_CONFIG_VERSIONS.contains(&self.version.as_str())
    }

    /// Get a warning message for unsupported versions.
    #[allow(dead_code)]
    pub fn version_warning(&self) -> Option<String> {
        if !self.is_version_supported() {
            Some(format!(
                "Warning: Configuration version '{}' is not supported. Supported versions: {}. Using defaults where needed.",
                self.version,
                SUPPORTED_CONFIG_VERSIONS.join(", ")
            ))
        } else {
            None
        }
    }

    /// Load configuration from file.
    #[allow(dead_code)]
    pub fn load_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;

        if let Some(warning) = config.version_warning() {
            eprintln!("{}", warning);
        }

        if config.version.is_empty() {
            config.version = CURRENT_CONFIG_VERSION.to_string();
        }

        Ok(config)
    }

    /// Get the default config directory path.
    #[allow(dead_code)]
    pub fn get_config_dir() -> Option<PathBuf> {
        get_config_home().map(|h| h.join("agpod"))
    }

    /// Load configuration with priority:
    /// 1. Defaults
    /// 2. Global config
    /// 3. Repo config (.agpod.toml)
    #[allow(dead_code)]
    pub fn load() -> Self {
        let mut config = Self::default();

        if let Some(config_dir) = Self::get_config_dir() {
            let global_config = config_dir.join("config.toml");
            if global_config.exists() {
                if let Ok(loaded) = Self::load_from_file(&global_config) {
                    config = config.merge(loaded);
                }
            }
        }

        let repo_config = PathBuf::from(".agpod.toml");
        if repo_config.exists() {
            if let Ok(loaded) = Self::load_from_file(&repo_config) {
                config = config.merge(loaded);
            }
        }

        config
    }

    /// Merge another config into this one.
    #[allow(dead_code)]
    pub fn merge(mut self, other: Config) -> Self {
        if other.version != CURRENT_CONFIG_VERSION || !other.version.is_empty() {
            self.version = other.version;
        }

        if other.diff.is_some() {
            self.diff = other.diff;
        }

        if other.case.is_some() {
            self.case = other.case;
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.version, "1");
        assert!(config.diff.is_none());
        assert!(config.case.is_none());
    }

    #[test]
    fn test_config_version_validation() {
        let config = Config {
            version: "1".to_string(),
            diff: None,
            case: None,
        };
        assert!(config.is_version_supported());
        assert!(config.version_warning().is_none());

        let unsupported_config = Config {
            version: "999".to_string(),
            diff: None,
            case: None,
        };
        assert!(!unsupported_config.is_version_supported());
        assert!(unsupported_config.version_warning().is_some());
    }

    #[test]
    fn test_parse_config_with_version() {
        let toml_str = r#"
version = "1"

[diff]
output_dir = "custom/diff"
large_file_changes_threshold = 200

[case]
server_addr = "127.0.0.1:6142"
"#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.version, "1");
        assert!(config.is_version_supported());
        assert!(config.diff.is_some());
        assert!(config.case.is_some());

        let diff = config.diff.unwrap();
        assert_eq!(diff.output_dir, "custom/diff");
        assert_eq!(diff.large_file_changes_threshold, 200);
        let case = config.case.unwrap();
        assert_eq!(case.server_addr.as_deref(), Some("127.0.0.1:6142"));
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
    fn test_parse_config_with_sections() {
        let toml_str = r#"
[diff]
output_dir = "custom/diff"
large_file_changes_threshold = 200
"#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.diff.is_some());

        let diff = config.diff.unwrap();
        assert_eq!(diff.output_dir, "custom/diff");
        assert_eq!(diff.large_file_changes_threshold, 200);
    }

    #[test]
    fn test_xdg_config_home_support() {
        let _guard = ENV_LOCK.lock().unwrap();

        env::set_var("XDG_CONFIG_HOME", "/tmp/test_config");
        let config_dir = Config::get_config_dir();
        assert!(config_dir.is_some());
        assert_eq!(
            config_dir.unwrap().to_str().unwrap(),
            "/tmp/test_config/agpod"
        );
        env::remove_var("XDG_CONFIG_HOME");

        env::set_var("XDG_CONFIG_HOME", "");
        let config_dir = Config::get_config_dir();
        assert!(config_dir.is_some());
        let path = config_dir.unwrap();
        let path_str = path.to_str().unwrap();
        assert!(path_str.ends_with(".config/agpod"));
        env::remove_var("XDG_CONFIG_HOME");
    }
}
