//! Shared configuration for agpod case client/server.
//!
//! Keywords: case config, server config, auto start, remote server, shared config

use agpod_core::Config as CoreConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_CASE_SERVER_ADDR: &str = "127.0.0.1:6142";
pub type DbConfig = CaseConfig;

/// Runtime mode for case access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaseAccessMode {
    #[default]
    LocalServer,
    Remote,
}

/// Shared case configuration loaded by CLI, MCP, and case-server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseConfig {
    pub data_dir: PathBuf,
    pub server_addr: String,
    pub auto_start: bool,
    pub access_mode: CaseAccessMode,
    pub redirect_limit: u32,
    pub semantic_recall_enabled: bool,
    pub vector_digest_job_enabled: bool,
    pub honcho_enabled: bool,
    pub honcho_sync_enabled: bool,
    pub honcho_base_url: Option<String>,
    pub honcho_workspace_id: Option<String>,
    pub honcho_api_key: Option<String>,
    pub honcho_api_key_env: String,
    pub honcho_peer_id: String,
}

impl Default for CaseConfig {
    fn default() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agpod")
            .join("case.db");
        Self {
            data_dir,
            server_addr: DEFAULT_CASE_SERVER_ADDR.to_string(),
            auto_start: true,
            access_mode: CaseAccessMode::LocalServer,
            redirect_limit: 100,
            semantic_recall_enabled: false,
            vector_digest_job_enabled: false,
            honcho_enabled: false,
            honcho_sync_enabled: true,
            honcho_base_url: None,
            honcho_workspace_id: None,
            honcho_api_key: None,
            honcho_api_key_env: "HONCHO_API_KEY".to_string(),
            honcho_peer_id: "agpod-system".to_string(),
        }
    }
}

/// File-backed config section under `[case]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaseConfigFile {
    pub data_dir: Option<String>,
    pub server_addr: Option<String>,
    pub auto_start: Option<bool>,
    pub access_mode: Option<CaseAccessMode>,
    pub redirect_limit: Option<u32>,
    pub semantic_recall_enabled: Option<bool>,
    pub vector_digest_job_enabled: Option<bool>,
    pub honcho_enabled: Option<bool>,
    pub honcho_sync_enabled: Option<bool>,
    pub honcho_base_url: Option<String>,
    pub honcho_workspace_id: Option<String>,
    pub honcho_api_key: Option<String>,
    pub honcho_api_key_env: Option<String>,
    pub honcho_peer_id: Option<String>,
    pub plugins: Option<CasePluginsFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CasePluginsFile {
    pub honcho: Option<CaseHonchoPluginFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaseHonchoPluginFile {
    pub enabled: Option<bool>,
    pub sync_enabled: Option<bool>,
    pub base_url: Option<String>,
    pub workspace_id: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub peer_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CaseOverrides<'a> {
    pub data_dir: Option<&'a str>,
    pub server_addr: Option<&'a str>,
}

impl CaseConfig {
    pub fn from_data_dir(data_dir: Option<&str>) -> Self {
        Self::load(CaseOverrides {
            data_dir,
            server_addr: None,
        })
    }

    /// Build config from defaults, shared agpod config files, env, then explicit overrides.
    ///
    /// Priority: explicit overrides > env > file config > defaults.
    pub fn load(overrides: CaseOverrides<'_>) -> Self {
        let mut config = Self::default();

        if let Some(file) = load_case_config_from_core() {
            config.merge_file(file);
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_DATA_DIR") {
            if !value.is_empty() {
                config.data_dir = PathBuf::from(value);
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_SERVER_ADDR") {
            if !value.is_empty() {
                config.server_addr = value;
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_AUTO_START") {
            if let Some(parsed) = parse_bool(&value) {
                config.auto_start = parsed;
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_ACCESS_MODE") {
            if let Some(parsed) = parse_access_mode(&value) {
                config.access_mode = parsed;
            }
        }

        if let Ok(value) = std::env::var("DEBUG_AGPOD_CASE_REDIRECTION_LIMIT") {
            if let Some(parsed) = parse_positive_u32(&value) {
                config.redirect_limit = parsed;
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_SEMANTIC_RECALL") {
            if let Some(parsed) = parse_bool(&value) {
                config.semantic_recall_enabled = parsed;
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_VECTOR_DIGEST_JOB") {
            if let Some(parsed) = parse_bool(&value) {
                config.vector_digest_job_enabled = parsed;
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_HONCHO_ENABLED") {
            if let Some(parsed) = parse_bool(&value) {
                config.honcho_enabled = parsed;
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_HONCHO_SYNC_ENABLED") {
            if let Some(parsed) = parse_bool(&value) {
                config.honcho_sync_enabled = parsed;
            }
        }

        if let Ok(value) = std::env::var("HONCHO_BASE_URL") {
            if !value.is_empty() {
                config.honcho_base_url = Some(value);
            }
        }

        if let Ok(value) = std::env::var("HONCHO_WORKSPACE_ID") {
            if !value.is_empty() {
                config.honcho_workspace_id = Some(value);
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_HONCHO_API_KEY_ENV") {
            if !value.is_empty() {
                config.honcho_api_key = None;
                config.honcho_api_key_env = value;
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_HONCHO_API_KEY") {
            if !value.is_empty() {
                config.honcho_api_key = Some(value);
            }
        }

        if let Ok(value) = std::env::var("AGPOD_CASE_HONCHO_PEER_ID") {
            if !value.is_empty() {
                config.honcho_peer_id = value;
            }
        }

        if let Some(path) = overrides.data_dir {
            config.data_dir = PathBuf::from(path);
        }

        if let Some(addr) = overrides.server_addr {
            config.server_addr = addr.to_string();
        }

        config
    }

    fn merge_file(&mut self, file: CaseConfigFile) {
        if let Some(path) = file.data_dir {
            self.data_dir = PathBuf::from(path);
        }
        if let Some(addr) = file.server_addr {
            self.server_addr = addr;
        }
        if let Some(auto_start) = file.auto_start {
            self.auto_start = auto_start;
        }
        if let Some(mode) = file.access_mode {
            self.access_mode = mode;
        }
        if let Some(limit) = file.redirect_limit {
            self.redirect_limit = limit.max(1);
        }
        if let Some(enabled) = file.semantic_recall_enabled {
            self.semantic_recall_enabled = enabled;
        }
        if let Some(enabled) = file.vector_digest_job_enabled {
            self.vector_digest_job_enabled = enabled;
        }
        if let Some(enabled) = file.honcho_enabled {
            self.honcho_enabled = enabled;
        }
        if let Some(enabled) = file.honcho_sync_enabled {
            self.honcho_sync_enabled = enabled;
        }
        if let Some(base_url) = file.honcho_base_url {
            self.honcho_base_url = Some(base_url);
        }
        if let Some(workspace_id) = file.honcho_workspace_id {
            self.honcho_workspace_id = Some(workspace_id);
        }
        if let Some(api_key_env) = file.honcho_api_key_env {
            self.honcho_api_key_env = api_key_env;
        }
        if let Some(api_key) = file.honcho_api_key {
            self.honcho_api_key = Some(api_key);
        }
        if let Some(peer_id) = file.honcho_peer_id {
            self.honcho_peer_id = peer_id;
        }
        if let Some(plugins) = file.plugins {
            if let Some(honcho) = plugins.honcho {
                if let Some(enabled) = honcho.enabled {
                    self.honcho_enabled = enabled;
                }
                if let Some(sync_enabled) = honcho.sync_enabled {
                    self.honcho_sync_enabled = sync_enabled;
                }
                if let Some(base_url) = honcho.base_url {
                    self.honcho_base_url = Some(base_url);
                }
                if let Some(workspace_id) = honcho.workspace_id {
                    self.honcho_workspace_id = Some(workspace_id);
                }
                if let Some(api_key_env) = honcho.api_key_env {
                    self.honcho_api_key_env = api_key_env;
                }
                if let Some(api_key) = honcho.api_key {
                    self.honcho_api_key = Some(api_key);
                }
                if let Some(peer_id) = honcho.peer_id {
                    self.honcho_peer_id = peer_id;
                }
            }
        }
    }
}

fn load_case_config_from_core() -> Option<CaseConfigFile> {
    let core = CoreConfig::load();
    let value = serde_json::to_value(core).ok()?;
    value
        .get("case")
        .cloned()
        .and_then(|section| serde_json::from_value::<CaseConfigFile>(section).ok())
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_access_mode(raw: &str) -> Option<CaseAccessMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "local_server" | "local-server" | "local" => Some(CaseAccessMode::LocalServer),
        "remote" => Some(CaseAccessMode::Remote),
        _ => None,
    }
}

fn parse_positive_u32(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok().filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_config_has_expected_server_defaults() {
        let config = CaseConfig::default();
        assert!(config.data_dir.ends_with("case.db"));
        assert_eq!(config.server_addr, DEFAULT_CASE_SERVER_ADDR);
        assert!(config.auto_start);
        assert_eq!(config.access_mode, CaseAccessMode::LocalServer);
        assert!(!config.semantic_recall_enabled);
        assert!(!config.vector_digest_job_enabled);
        assert!(!config.honcho_enabled);
        assert!(config.honcho_sync_enabled);
        assert_eq!(config.honcho_api_key, None);
        assert_eq!(config.honcho_api_key_env, "HONCHO_API_KEY");
        assert_eq!(config.honcho_peer_id, "agpod-system");
    }

    #[test]
    fn explicit_overrides_win() {
        let config = CaseConfig::load(CaseOverrides {
            data_dir: Some("/tmp/case.db"),
            server_addr: Some("127.0.0.1:9999"),
        });
        assert_eq!(config.data_dir, PathBuf::from("/tmp/case.db"));
        assert_eq!(config.server_addr, "127.0.0.1:9999");
    }

    #[test]
    fn env_overrides_apply() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        std::env::set_var("AGPOD_CASE_AUTO_START", "false");
        std::env::set_var("AGPOD_CASE_ACCESS_MODE", "remote");
        std::env::set_var("DEBUG_AGPOD_CASE_REDIRECTION_LIMIT", "3");
        std::env::set_var("AGPOD_CASE_SEMANTIC_RECALL", "true");
        std::env::set_var("AGPOD_CASE_HONCHO_ENABLED", "true");
        std::env::set_var("HONCHO_BASE_URL", "https://example.test");
        std::env::set_var("HONCHO_WORKSPACE_ID", "ws-123");
        std::env::set_var("AGPOD_CASE_HONCHO_API_KEY", "direct-secret");

        let config = CaseConfig::load(CaseOverrides::default());
        assert!(!config.auto_start);
        assert_eq!(config.access_mode, CaseAccessMode::Remote);
        assert_eq!(config.redirect_limit, 3);
        assert!(config.semantic_recall_enabled);
        assert!(config.honcho_enabled);
        assert_eq!(
            config.honcho_base_url.as_deref(),
            Some("https://example.test")
        );
        assert_eq!(config.honcho_workspace_id.as_deref(), Some("ws-123"));
        assert_eq!(config.honcho_api_key.as_deref(), Some("direct-secret"));

        std::env::remove_var("AGPOD_CASE_AUTO_START");
        std::env::remove_var("AGPOD_CASE_ACCESS_MODE");
        std::env::remove_var("DEBUG_AGPOD_CASE_REDIRECTION_LIMIT");
        std::env::remove_var("AGPOD_CASE_SEMANTIC_RECALL");
        std::env::remove_var("AGPOD_CASE_HONCHO_ENABLED");
        std::env::remove_var("HONCHO_BASE_URL");
        std::env::remove_var("HONCHO_WORKSPACE_ID");
        std::env::remove_var("AGPOD_CASE_HONCHO_API_KEY");
    }

    #[test]
    fn file_config_can_enable_honcho_settings() {
        let mut config = CaseConfig::default();
        config.merge_file(CaseConfigFile {
            semantic_recall_enabled: Some(true),
            vector_digest_job_enabled: Some(true),
            honcho_enabled: Some(true),
            honcho_sync_enabled: Some(false),
            honcho_base_url: Some("https://api.honcho.dev".to_string()),
            honcho_workspace_id: Some("ws_configured".to_string()),
            honcho_api_key: Some("honcho-inline-secret".to_string()),
            honcho_api_key_env: Some("HONCHO_API_KEY_CUSTOM".to_string()),
            honcho_peer_id: Some("agpod-agent".to_string()),
            ..CaseConfigFile::default()
        });

        assert!(config.semantic_recall_enabled);
        assert!(config.vector_digest_job_enabled);
        assert!(config.honcho_enabled);
        assert!(!config.honcho_sync_enabled);
        assert_eq!(
            config.honcho_base_url.as_deref(),
            Some("https://api.honcho.dev")
        );
        assert_eq!(config.honcho_workspace_id.as_deref(), Some("ws_configured"));
        assert_eq!(
            config.honcho_api_key.as_deref(),
            Some("honcho-inline-secret")
        );
        assert_eq!(config.honcho_api_key_env, "HONCHO_API_KEY_CUSTOM");
        assert_eq!(config.honcho_peer_id, "agpod-agent");
    }

    #[test]
    fn nested_honcho_plugin_config_applies() {
        let mut config = CaseConfig::default();
        config.merge_file(CaseConfigFile {
            semantic_recall_enabled: Some(true),
            plugins: Some(CasePluginsFile {
                honcho: Some(CaseHonchoPluginFile {
                    enabled: Some(true),
                    sync_enabled: Some(false),
                    base_url: Some("https://nested.honcho.dev".to_string()),
                    workspace_id: Some("ws_nested".to_string()),
                    api_key: Some("nested-inline-secret".to_string()),
                    api_key_env: Some("HONCHO_NESTED_KEY".to_string()),
                    peer_id: Some("agpod-nested".to_string()),
                }),
            }),
            ..CaseConfigFile::default()
        });

        assert!(config.semantic_recall_enabled);
        assert!(config.honcho_enabled);
        assert!(!config.honcho_sync_enabled);
        assert_eq!(
            config.honcho_base_url.as_deref(),
            Some("https://nested.honcho.dev")
        );
        assert_eq!(config.honcho_workspace_id.as_deref(), Some("ws_nested"));
        assert_eq!(
            config.honcho_api_key.as_deref(),
            Some("nested-inline-secret")
        );
        assert_eq!(config.honcho_api_key_env, "HONCHO_NESTED_KEY");
        assert_eq!(config.honcho_peer_id, "agpod-nested");
    }

    #[test]
    fn direct_api_key_wins_within_same_nested_config() {
        let mut config = CaseConfig::default();
        config.merge_file(CaseConfigFile {
            plugins: Some(CasePluginsFile {
                honcho: Some(CaseHonchoPluginFile {
                    api_key: Some("inline-secret".to_string()),
                    api_key_env: Some("HONCHO_IGNORED".to_string()),
                    ..CaseHonchoPluginFile::default()
                }),
            }),
            ..CaseConfigFile::default()
        });

        assert_eq!(config.honcho_api_key.as_deref(), Some("inline-secret"));
        assert_eq!(config.honcho_api_key_env, "HONCHO_IGNORED");
    }
}
