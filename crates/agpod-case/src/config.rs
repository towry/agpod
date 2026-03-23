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
    pub semantic_recall_enabled: bool,
    pub vector_digest_job_enabled: bool,
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
            semantic_recall_enabled: false,
            vector_digest_job_enabled: false,
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
    pub semantic_recall_enabled: Option<bool>,
    pub vector_digest_job_enabled: Option<bool>,
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
        if let Some(enabled) = file.semantic_recall_enabled {
            self.semantic_recall_enabled = enabled;
        }
        if let Some(enabled) = file.vector_digest_job_enabled {
            self.vector_digest_job_enabled = enabled;
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
        std::env::set_var("AGPOD_CASE_SEMANTIC_RECALL", "true");

        let config = CaseConfig::load(CaseOverrides::default());
        assert!(!config.auto_start);
        assert_eq!(config.access_mode, CaseAccessMode::Remote);
        assert!(config.semantic_recall_enabled);

        std::env::remove_var("AGPOD_CASE_AUTO_START");
        std::env::remove_var("AGPOD_CASE_ACCESS_MODE");
        std::env::remove_var("AGPOD_CASE_SEMANTIC_RECALL");
    }
}
