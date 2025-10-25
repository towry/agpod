use crate::error::{KiroError, KiroResult};
use agpod_core::get_config_home;
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
        cli_overrides: &crate::cli::KiroArgs,
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

        // Parse as root config with [kiro] section
        let root_config = toml::from_str::<agpod_core::Config>(&content).map_err(|e| {
            KiroError::Config(format!(
                "Failed to parse config from {}: {}",
                path.display(),
                e
            ))
        })?;

        // Extract kiro section
        let kiro_config = root_config.kiro.ok_or_else(|| {
            KiroError::Config(format!(
                "No [kiro] section found in config file: {}",
                path.display()
            ))
        })?;

        // Convert agpod_core::KiroConfig to our Config
        Ok(self.merge_with_core_config(kiro_config))
    }

    fn merge_with_core_config(&self, other: agpod_core::KiroConfig) -> Self {
        Self {
            base_dir: other.base_dir,
            templates_dir: other.templates_dir,
            plugins_dir: other.plugins_dir,
            template: other.template,
            summary_lines: other.summary_lines,
            plugins: PluginConfig {
                name: BranchNamePlugin {
                    enabled: other.plugins.name.enabled,
                    command: other.plugins.name.command,
                    timeout_secs: other.plugins.name.timeout_secs,
                    pass_env: other.plugins.name.pass_env,
                },
            },
            rendering: RenderingConfig {
                files: other.rendering.files,
                extra: other.rendering.extra,
                missing_policy: other.rendering.missing_policy,
            },
            templates: other
                .templates
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        TemplateConfig {
                            description: v.description,
                            files: v.files,
                            missing_policy: v.missing_policy,
                        },
                    )
                })
                .collect(),
        }
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

    #[test]
    fn test_load_config_with_kiro_section() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let config_content = r#"
version = "1"

[kiro]
base_dir = "custom/kiro"
templates_dir = "~/.config/agpod/templates"
plugins_dir = "~/.config/agpod/plugins"
template = "vue"
summary_lines = 5

[kiro.templates.vue]
description = "Vue.js component template"
files = ["design.md.j2", "tasks.md.j2", "component.md.j2"]
missing_policy = "skip"

[kiro.templates.default]
description = "Default template"
files = ["design.md.j2", "tasks.md.j2"]
missing_policy = "error"
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(config_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let default_config = Config::default();
        let loaded_config = default_config.merge_from_file(temp_file.path()).unwrap();

        assert_eq!(loaded_config.base_dir, "custom/kiro");
        assert_eq!(loaded_config.template, "vue");
        assert_eq!(loaded_config.summary_lines, 5);
        assert_eq!(loaded_config.templates.len(), 2);

        let vue_template = loaded_config.templates.get("vue").unwrap();
        assert_eq!(vue_template.description, "Vue.js component template");
        assert_eq!(vue_template.files.len(), 3);
        assert_eq!(vue_template.files[0], "design.md.j2");
        assert_eq!(vue_template.files[1], "tasks.md.j2");
        assert_eq!(vue_template.files[2], "component.md.j2");
        assert_eq!(vue_template.missing_policy, "skip");

        let default_template = loaded_config.templates.get("default").unwrap();
        assert_eq!(default_template.files.len(), 2);
        assert_eq!(default_template.missing_policy, "error");
    }

    #[test]
    fn test_template_config_overrides_rendering() {
        let mut config = Config::default();
        config.rendering.files = vec!["DESIGN.md.j2".to_string(), "TASK.md.j2".to_string()];

        let mut template_config = HashMap::new();
        template_config.insert(
            "custom".to_string(),
            TemplateConfig {
                description: "Custom template".to_string(),
                files: vec!["design.md.j2".to_string(), "tasks.md.j2".to_string()],
                missing_policy: "error".to_string(),
            },
        );
        config.templates = template_config;

        // Template config should be available
        let custom = config.templates.get("custom").unwrap();
        assert_eq!(custom.files[0], "design.md.j2");
        assert_eq!(custom.files[1], "tasks.md.j2");
    }

    #[test]
    fn test_load_config_with_plugin_settings() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let config_content = r#"
version = "1"

[kiro]
base_dir = "llm/kiro"
templates_dir = "~/.config/agpod/templates"
plugins_dir = "~/.config/agpod/plugins"
template = "default"
summary_lines = 3

[kiro.plugins.name]
enabled = true
command = "name.sh"
timeout_secs = 5
pass_env = ["AGPOD_*", "GIT_*", "USER", "HOME"]
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(config_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let default_config = Config::default();
        let loaded_config = default_config.merge_from_file(temp_file.path()).unwrap();

        // Verify plugin configuration was loaded
        assert_eq!(loaded_config.plugins.name.enabled, true);
        assert_eq!(loaded_config.plugins.name.command, "name.sh");
        assert_eq!(loaded_config.plugins.name.timeout_secs, 5);
        assert_eq!(loaded_config.plugins.name.pass_env.len(), 4);
        assert!(loaded_config
            .plugins
            .name
            .pass_env
            .contains(&"AGPOD_*".to_string()));
        assert!(loaded_config
            .plugins
            .name
            .pass_env
            .contains(&"GIT_*".to_string()));
        assert!(loaded_config
            .plugins
            .name
            .pass_env
            .contains(&"USER".to_string()));
        assert!(loaded_config
            .plugins
            .name
            .pass_env
            .contains(&"HOME".to_string()));
    }
}
