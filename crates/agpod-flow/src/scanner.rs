//! Document scanner: walks configured roots and collects candidate files.
//!
//! Keywords: document scanner, walk directory, glob match, scan documents

use crate::config::FlowDocsConfig;
use crate::error::FlowResult;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Build a GlobSet from a list of glob pattern strings.
fn build_globset(patterns: &[String]) -> FlowResult<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p).map_err(|e| {
            crate::error::FlowError::Config(format!("Invalid glob pattern '{p}': {e}"))
        })?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| crate::error::FlowError::Config(format!("Failed to build glob set: {e}")))
}

/// Scan configured roots and return candidate document paths.
pub fn scan_documents(repo_root: &Path, config: &FlowDocsConfig) -> FlowResult<Vec<PathBuf>> {
    let roots = config.absolute_roots(repo_root);
    let include_set = build_globset(&config.include_globs)?;
    let exclude_set = build_globset(&config.exclude_globs)?;

    let mut results = Vec::new();

    for root in &roots {
        let walker = WalkDir::new(root).follow_links(config.follow_symlinks);

        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            let rel = path.strip_prefix(repo_root).unwrap_or(path);
            let rel_str = rel.to_string_lossy();

            if exclude_set.is_match(rel_str.as_ref()) {
                continue;
            }
            if !include_set.is_match(rel_str.as_ref()) {
                continue;
            }

            results.push(path.to_path_buf());
        }
    }

    results.sort();
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_globset_works() {
        let set = build_globset(&["**/*.md".into(), "**/*.mdx".into()]).unwrap();
        assert!(set.is_match("docs/foo.md"));
        assert!(set.is_match("notes/bar.mdx"));
        assert!(!set.is_match("src/main.rs"));
    }

    #[test]
    fn exclude_globset_works() {
        let set = build_globset(&["**/node_modules/**".into()]).unwrap();
        assert!(set.is_match("node_modules/foo/bar.md"));
    }
}
