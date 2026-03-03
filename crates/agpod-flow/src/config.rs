//! Flow configuration from `.agpod.flow.toml`.
//!
//! Keywords: flow config, doc roots, include globs, exclude globs

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
    #[serde(default = "default_roots")]
    pub roots: Vec<String>,

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
            roots: default_roots(),
            include_globs: default_include_globs(),
            exclude_globs: default_exclude_globs(),
            frontmatter_required: true,
            follow_symlinks: false,
        }
    }
}

fn default_roots() -> Vec<String> {
    vec!["llm".into(), "docs".into(), "notes".into()]
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

    /// Absolute root directories that actually exist.
    pub fn absolute_roots(&self, repo_root: &Path) -> Vec<PathBuf> {
        self.roots
            .iter()
            .map(|r| repo_root.join(r))
            .filter(|p| p.is_dir())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let c = FlowDocsConfig::default();
        assert_eq!(c.roots, vec!["llm", "docs", "notes"]);
        assert!(c.frontmatter_required);
        assert!(!c.follow_symlinks);
    }

    #[test]
    fn test_parse_toml() {
        let s = r#"
[flow.docs]
roots = ["docs", "specs"]
include_globs = ["**/*.md"]
frontmatter_required = true
"#;
        let f: FlowConfigFile = toml::from_str(s).unwrap();
        assert_eq!(f.flow.docs.roots, vec!["docs", "specs"]);
    }
}
