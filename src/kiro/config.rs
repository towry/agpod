use crate::config::get_config_home;
use crate::kiro::error::{KiroError, KiroResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_base_dir")]
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
    pub plugins: PluginConfig,

    #[serde(default)]
    pub rendering: RenderingConfig,

    #[serde(default)]
    pub templates: HashMap<String, TemplateConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfig {
    #[serde(default)]
    pub name: BranchNamePlugin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchNamePlugin {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_name_command")]
    pub command: String,

    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    #[serde(default)]
    pub pass_env: Vec<String>,
}

impl Default for BranchNamePlugin {
    fn default() -> Self {
        Self {
            enabled: true,
            command: default_name_command(),
            timeout_secs: default_timeout_secs(),
            pass_env: vec![
                "AGPOD_*".to_string(),
                "GIT_*".to_string(),
                "USER".to_string(),
                "HOME".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderingConfig {
    #[serde(default = "default_rendering_files")]
    pub files: Vec<String>,

    #[serde(default)]
    pub extra: Vec<String>,

    #[serde(default = "default_missing_policy")]
    pub missing_policy: String,
}

impl Default for RenderingConfig {
    fn default() -> Self {
        Self {
            files: default_rendering_files(),
            extra: vec![],
            missing_policy: default_missing_policy(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    #[serde(default)]
    pub description: String,

    #[serde(default = "default_rendering_files")]
    pub files: Vec<String>,

    #[serde(default = "default_missing_policy")]
    pub missing_policy: String,
}

fn default_base_dir() -> String {
    "llm/kiro".to_string()
}

fn default_templates_dir() -> String {
    // Respects XDG_CONFIG_HOME environment variable
    if let Some(config_home) = get_config_home() {
        config_home
            .join("agpod")
            .join("templates")
            .to_string_lossy()
            .to_string()
    } else {
        "~/.config/agpod/templates".to_string()
    }
}

fn default_plugins_dir() -> String {
    // Respects XDG_CONFIG_HOME environment variable
    if let Some(config_home) = get_config_home() {
        config_home
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

fn default_true() -> bool {
    true
}

fn default_name_command() -> String {
    "name.sh".to_string()
}

fn default_timeout_secs() -> u64 {
    3
}

fn default_rendering_files() -> Vec<String> {
    vec!["DESIGN.md.j2".to_string(), "TASK.md.j2".to_string()]
}

fn default_missing_policy() -> String {
    "error".to_string()
}

impl Config {
    /// Get the default config directory path
    /// Respects XDG_CONFIG_HOME environment variable
    pub fn get_config_dir() -> Option<PathBuf> {
        get_config_home().map(|h| h.join("agpod"))
    }

    /// Check if config directory is initialized
    pub fn is_initialized() -> bool {
        if let Some(config_dir) = Self::get_config_dir() {
            config_dir.join("templates").exists()
        } else {
            false
        }
    }

    /// Load configuration with priority:
    /// 1. Defaults
    /// 2. Global config
    /// 3. Repo config
    /// 4. Environment variables
    /// 5. CLI arguments
    pub fn load(
        cli_config: Option<&str>,
        cli_overrides: &crate::kiro::cli::KiroArgs,
    ) -> KiroResult<Self> {
        let mut config = Self::default();

        // Try to load global config from ~/.config/agpod
        if let Some(home_dir) = dirs::home_dir() {
            let global_config = home_dir.join(".config").join("agpod").join("config.toml");
            if global_config.exists() {
                config = config.merge_from_file(&global_config)?;
            }
        }

        // Try to load repo config
        let repo_config = Path::new(".agpod.toml");
        if repo_config.exists() {
            config = config.merge_from_file(repo_config)?;
        }

        // Try to load custom config if specified
        if let Some(custom_config) = cli_config {
            let custom_path = expand_path(custom_config);
            config = config.merge_from_file(Path::new(&custom_path))?;
        }

        // Apply CLI overrides
        if let Some(ref base_dir) = cli_overrides.base_dir {
            config.base_dir = base_dir.clone();
        }
        if let Some(ref templates_dir) = cli_overrides.templates_dir {
            config.templates_dir = templates_dir.clone();
        }
        if let Some(ref plugins_dir) = cli_overrides.plugins_dir {
            config.plugins_dir = plugins_dir.clone();
        }

        // Expand paths
        config.base_dir = expand_path(&config.base_dir);
        config.templates_dir = expand_path(&config.templates_dir);
        config.plugins_dir = expand_path(&config.plugins_dir);

        Ok(config)
    }

    fn merge_from_file(&self, path: &Path) -> KiroResult<Self> {
        let content = fs::read_to_string(path).map_err(|e| {
            KiroError::Config(format!(
                "Failed to read config from {}: {}",
                path.display(),
                e
            ))
        })?;

        let file_config: Config = toml::from_str(&content).map_err(|e| {
            KiroError::Config(format!(
                "Failed to parse config from {}: {}",
                path.display(),
                e
            ))
        })?;

        Ok(file_config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_dir: default_base_dir(),
            templates_dir: default_templates_dir(),
            plugins_dir: default_plugins_dir(),
            template: default_template(),
            summary_lines: default_summary_lines(),
            plugins: PluginConfig::default(),
            rendering: RenderingConfig::default(),
            templates: HashMap::new(),
        }
    }
}

/// Expand tilde and environment variables in paths
pub fn expand_path(path: &str) -> String {
    let mut expanded = path.to_string();

    // Expand tilde
    if expanded.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            expanded = expanded.replacen("~/", &format!("{}/", home.display()), 1);
        }
    } else if expanded == "~" {
        if let Some(home) = dirs::home_dir() {
            expanded = home.to_string_lossy().to_string();
        }
    }

    // Expand environment variables
    if expanded.contains('$') {
        let re = regex::Regex::new(r"\$([A-Z_][A-Z0-9_]*)").unwrap();
        expanded = re
            .replace_all(&expanded, |caps: &regex::Captures| {
                let var_name = &caps[1];
                std::env::var(var_name).unwrap_or_else(|_| format!("${}", var_name))
            })
            .to_string();
    }

    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.base_dir, "llm/kiro");
        assert_eq!(config.template, "default");
        assert_eq!(config.summary_lines, 3);
    }

    #[test]
    fn test_expand_path() {
        std::env::set_var("TEST_VAR", "/test/path");

        assert_eq!(expand_path("$TEST_VAR/subdir"), "/test/path/subdir");
        assert_eq!(expand_path("relative/path"), "relative/path");

        std::env::remove_var("TEST_VAR");
    }
}
