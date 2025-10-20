//! Configuration management for agpod
//!
//! Supports feature-specific configuration sections:
//! - [kiro] - PR draft workflow settings
//! - [diff] - Git diff minimization settings

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Current configuration version
pub const CURRENT_CONFIG_VERSION: &str = "1";

/// Supported configuration versions
pub const SUPPORTED_CONFIG_VERSIONS: &[&str] = &["1"];

/// Root configuration structure supporting multiple features
#[allow(dead_code)] // Public API for library users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Configuration version for tracking schema changes
    #[serde(default = "default_config_version")]
    pub version: String,

    /// Kiro workflow configuration
    #[serde(default)]
    pub kiro: Option<KiroConfig>,

    /// Diff minimization configuration
    #[serde(default)]
    pub diff: Option<DiffConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: default_config_version(),
            kiro: None,
            diff: None,
        }
    }
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

    #[serde(default)]
    pub plugins: KiroPluginConfig,

    #[serde(default)]
    pub rendering: KiroRenderingConfig,

    #[serde(default)]
    pub templates: std::collections::HashMap<String, KiroTemplateConfig>,
}

impl Default for KiroConfig {
    fn default() -> Self {
        Self {
            base_dir: default_kiro_base_dir(),
            templates_dir: default_templates_dir(),
            plugins_dir: default_plugins_dir(),
            template: default_template(),
            summary_lines: default_summary_lines(),
            plugins: KiroPluginConfig::default(),
            rendering: KiroRenderingConfig::default(),
            templates: std::collections::HashMap::new(),
        }
    }
}

/// Plugin configuration for Kiro
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KiroPluginConfig {
    #[serde(default)]
    pub name: KiroBranchNamePlugin,
}

/// Branch name plugin configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroBranchNamePlugin {
    #[serde(default = "default_plugin_enabled")]
    pub enabled: bool,

    #[serde(default = "default_plugin_command")]
    pub command: String,

    #[serde(default = "default_plugin_timeout")]
    pub timeout_secs: u64,

    #[serde(default)]
    pub pass_env: Vec<String>,
}

impl Default for KiroBranchNamePlugin {
    fn default() -> Self {
        Self {
            enabled: true,
            command: default_plugin_command(),
            timeout_secs: default_plugin_timeout(),
            pass_env: vec![
                "AGPOD_*".to_string(),
                "GIT_*".to_string(),
                "USER".to_string(),
                "HOME".to_string(),
            ],
        }
    }
}

/// Rendering configuration for Kiro
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroRenderingConfig {
    #[serde(default = "default_rendering_files")]
    pub files: Vec<String>,

    #[serde(default)]
    pub extra: Vec<String>,

    #[serde(default = "default_missing_policy")]
    pub missing_policy: String,
}

impl Default for KiroRenderingConfig {
    fn default() -> Self {
        Self {
            files: default_rendering_files(),
            extra: vec![],
            missing_policy: default_missing_policy(),
        }
    }
}

/// Template-specific configuration for Kiro
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroTemplateConfig {
    #[serde(default)]
    pub description: String,

    #[serde(default = "default_rendering_files")]
    pub files: Vec<String>,

    #[serde(default = "default_missing_policy")]
    pub missing_policy: String,
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

// Default value functions for root Config
fn default_config_version() -> String {
    CURRENT_CONFIG_VERSION.to_string()
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

fn default_plugin_enabled() -> bool {
    true
}

fn default_plugin_command() -> String {
    "name.sh".to_string()
}

fn default_plugin_timeout() -> u64 {
    3
}

fn default_rendering_files() -> Vec<String> {
    vec!["DESIGN.md.j2".to_string(), "TASK.md.j2".to_string()]
}

fn default_missing_policy() -> String {
    "error".to_string()
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
    /// Check if the configuration version is supported
    #[allow(dead_code)] // Public API for library users
    pub fn is_version_supported(&self) -> bool {
        SUPPORTED_CONFIG_VERSIONS.contains(&self.version.as_str())
    }

    /// Get a warning message for unsupported versions
    #[allow(dead_code)] // Public API for library users
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

    /// Load configuration from file
    #[allow(dead_code)] // Public API for library users
    pub fn load_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;

        // Warn if version is not supported
        if let Some(warning) = config.version_warning() {
            eprintln!("{}", warning);
        }

        // Set to current version if empty or missing
        if config.version.is_empty() {
            config.version = CURRENT_CONFIG_VERSION.to_string();
        }

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
        // Prefer the other version if it's not the default
        if other.version != CURRENT_CONFIG_VERSION || !other.version.is_empty() {
            self.version = other.version;
        }

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
        assert_eq!(config.version, "1");
        assert!(config.kiro.is_none());
        assert!(config.diff.is_none());
    }

    #[test]
    fn test_config_version_validation() {
        let config = Config {
            version: "1".to_string(),
            kiro: None,
            diff: None,
        };
        assert!(config.is_version_supported());
        assert!(config.version_warning().is_none());

        let unsupported_config = Config {
            version: "999".to_string(),
            kiro: None,
            diff: None,
        };
        assert!(!unsupported_config.is_version_supported());
        assert!(unsupported_config.version_warning().is_some());
    }

    #[test]
    fn test_parse_config_with_version() {
        let toml_str = r#"
version = "1"

[kiro]
base_dir = "custom/kiro"
template = "rust"

[diff]
output_dir = "custom/diff"
large_file_changes_threshold = 200
"#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.version, "1");
        assert!(config.is_version_supported());
        assert!(config.kiro.is_some());
        assert!(config.diff.is_some());

        let kiro = config.kiro.unwrap();
        assert_eq!(kiro.base_dir, "custom/kiro");
        assert_eq!(kiro.template, "rust");

        let diff = config.diff.unwrap();
        assert_eq!(diff.output_dir, "custom/diff");
        assert_eq!(diff.large_file_changes_threshold, 200);
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
