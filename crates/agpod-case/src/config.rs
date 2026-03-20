//! SurrealDB embedded database configuration.
//!
//! Keywords: surrealdb config, data dir, database path

use std::path::PathBuf;

/// SurrealDB embedded database configuration.
#[derive(Debug, Clone)]
pub struct DbConfig {
    pub data_dir: PathBuf,
}

impl Default for DbConfig {
    fn default() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agpod")
            .join("case.db");
        Self { data_dir }
    }
}

impl DbConfig {
    /// Build config from CLI flag, env, or defaults.
    ///
    /// Priority: `--data-dir` flag > `AGPOD_CASE_DATA_DIR` env > default.
    pub fn from_data_dir(data_dir: Option<&str>) -> Self {
        match data_dir {
            Some(path) => Self {
                data_dir: PathBuf::from(path),
            },
            None => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_valid_path() {
        let c = DbConfig::default();
        assert!(c.data_dir.ends_with("case.db"));
    }

    #[test]
    fn custom_data_dir() {
        let c = DbConfig::from_data_dir(Some("/tmp/test.db"));
        assert_eq!(c.data_dir, PathBuf::from("/tmp/test.db"));
    }
}
