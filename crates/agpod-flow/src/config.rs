//! Flow configuration from `.agpod.flow.toml`.
//!
//! Keywords: flow config, doc root, include globs, exclude globs

use crate::error::FlowResult;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level config structure for `.agpod.flow.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowConfigFile {
    #[serde(default)]
    pub flow: FlowSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowSection {
    #[serde(default)]
    pub docs: FlowDocsConfig,
}

/// Configuration for document scanning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDocsConfig {
    #[serde(default = "default_root")]
    pub root: String,

    #[serde(default = "default_include_globs")]
    pub include_globs: Vec<String>,

    #[serde(default = "default_exclude_globs")]
    pub exclude_globs: Vec<String>,

    #[serde(default = "default_true")]
    pub frontmatter_required: bool,

    #[serde(default)]
    pub follow_symlinks: bool,
}

impl Default for FlowDocsConfig {
    fn default() -> Self {
        Self {
            root: default_root(),
            include_globs: default_include_globs(),
            exclude_globs: default_exclude_globs(),
            frontmatter_required: true,
            follow_symlinks: false,
        }
    }
}

fn default_root() -> String {
    "docs".into()
}

fn default_include_globs() -> Vec<String> {
    vec!["**/*.md".into(), "**/*.mdx".into()]
}

fn default_exclude_globs() -> Vec<String> {
    vec![
        "**/node_modules/**".into(),
        "**/.git/**".into(),
        "**/dist/**".into(),
    ]
}

fn default_true() -> bool {
    true
}

impl FlowDocsConfig {
    pub const FLOW_SUBDIR: &'static str = "agpod-flow";

    /// Load from repo root. Falls back to defaults if file absent.
    pub fn load(repo_root: &Path) -> FlowResult<Self> {
        let path = repo_root.join(".agpod.flow.toml");
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let file: FlowConfigFile = toml::from_str(&content)?;
            Ok(file.flow.docs)
        } else {
            Ok(Self::default())
        }
    }

    /// Absolute flow docs directory if it exists.
    pub fn absolute_root(&self, repo_root: &Path) -> Option<PathBuf> {
        let path = repo_root.join(&self.root).join(Self::FLOW_SUBDIR);
        if path.is_dir() {
            Some(path)
        } else {
            None
        }
    }

    /// Ensure flow docs root exists.
    /// If missing, initialize `<root>/agpod-flow`.
    pub fn ensure_flow_root(&self, repo_root: &Path) -> FlowResult<PathBuf> {
        if let Some(existing) = self.absolute_root(repo_root) {
            return Ok(existing);
        }

        let path = repo_root.join(&self.root).join(Self::FLOW_SUBDIR);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let c = FlowDocsConfig::default();
        assert_eq!(c.root, "docs");
        assert!(c.frontmatter_required);
        assert!(!c.follow_symlinks);
    }

    #[test]
    fn test_parse_toml() {
        let s = r#"
[flow.docs]
root = "docs"
include_globs = ["**/*.md"]
frontmatter_required = true
"#;
        let f: FlowConfigFile = toml::from_str(s).unwrap();
        assert_eq!(f.flow.docs.root, "docs");
    }
}
