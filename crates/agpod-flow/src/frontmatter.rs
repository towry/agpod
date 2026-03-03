//! YAML frontmatter parsing via gray_matter + serde.
//!
//! Keywords: frontmatter, yaml, doc metadata, doc_id, task_id

use crate::error::{FlowError, FlowResult};
use gray_matter::engine::YAML;
use gray_matter::Matter;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Parsed YAML frontmatter from a flow document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocFrontmatter {
    pub doc_id: Option<String>,
    pub doc_type: Option<String>,
    pub task_id: Option<String>,
    pub root_task_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub status: Option<String>,
    pub branch: Option<String>,
    pub agent_id: Option<String>,
}

const VALID_DOC_TYPES: &[&str] = &[
    "requirement",
    "design",
    "task",
    "impl",
    "bug",
    "decision",
    "note",
    "summary",
];

const VALID_STATUSES: &[&str] = &["todo", "in_progress", "blocked", "done", "archived"];

/// Parse frontmatter from markdown content.
pub fn parse_frontmatter(content: &str, file_path: &str) -> FlowResult<DocFrontmatter> {
    let matter = Matter::<YAML>::new();
    let result = matter.parse(content);

    let pod = result.data.ok_or_else(|| FlowError::FrontmatterMissing {
        path: file_path.to_string(),
    })?;

    let fm: DocFrontmatter = pod
        .deserialize()
        .map_err(|e| FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: format!("Failed to deserialize frontmatter: {e}"),
        })?;

    Ok(fm)
}

/// Validate required fields.
pub fn validate_frontmatter(fm: &DocFrontmatter, file_path: &str) -> FlowResult<()> {
    if fm.doc_id.is_none() {
        return Err(FlowError::DocIdMissing {
            path: file_path.to_string(),
        });
    }

    let doc_type = fm.doc_type.as_deref().unwrap_or("");
    if doc_type.is_empty() {
        return Err(FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: "doc_type is required".into(),
        });
    }
    if !VALID_DOC_TYPES.contains(&doc_type) {
        return Err(FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: format!(
                "invalid doc_type '{doc_type}', expected one of: {}",
                VALID_DOC_TYPES.join(", ")
            ),
        });
    }

    if fm.task_id.is_none() {
        return Err(FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: "task_id is required".into(),
        });
    }

    let status = fm
        .status
        .as_deref()
        .ok_or_else(|| FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: "status is required".into(),
        })?;
    if !VALID_STATUSES.contains(&status) {
        return Err(FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: format!(
                "invalid status '{status}', expected one of: {}",
                VALID_STATUSES.join(", ")
            ),
        });
    }

    let created = fm
        .created_at
        .as_deref()
        .ok_or_else(|| FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: "created_at is required".into(),
        })?;
    let updated = fm
        .updated_at
        .as_deref()
        .ok_or_else(|| FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: "updated_at is required".into(),
        })?;

    let created_dt = chrono::DateTime::parse_from_rfc3339(created).map_err(|e| {
        FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: format!("created_at is not valid RFC3339: {e}"),
        }
    })?;
    let updated_dt = chrono::DateTime::parse_from_rfc3339(updated).map_err(|e| {
        FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: format!("updated_at is not valid RFC3339: {e}"),
        }
    })?;

    if !created.ends_with('Z') || !updated.ends_with('Z') {
        return Err(FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: "created_at/updated_at must be UTC with 'Z' suffix".into(),
        });
    }

    if updated_dt < created_dt {
        return Err(FlowError::InvalidFrontmatter {
            path: file_path.to_string(),
            detail: "updated_at must be >= created_at".into(),
        });
    }

    Ok(())
}

static DOC_ID_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generate a new doc_id with process-local monotonic suffix.
pub fn generate_doc_id() -> String {
    let now = chrono::Utc::now();
    let date = now.format("%Y%m%d");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let seq = DOC_ID_COUNTER.fetch_add(1, Ordering::Relaxed) as u64;
    let mixed = nanos ^ (seq << 12) ^ (std::process::id() as u64);
    let suffix = format!("{:06x}", mixed & 0xFF_FFFF);
    format!("D-{date}-{suffix}")
}

/// Read and parse existing frontmatter if present.
pub fn read_existing_frontmatter(file_path: &Path) -> FlowResult<Option<DocFrontmatter>> {
    if !file_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(file_path)?;
    if !has_frontmatter(&content) {
        return Ok(None);
    }
    let path_str = file_path.to_string_lossy().to_string();
    let fm = parse_frontmatter(&content, &path_str)?;
    Ok(Some(fm))
}

/// Merge document metadata for `doc add` / `doc init` style operations.
pub fn upsert_frontmatter(
    existing: Option<DocFrontmatter>,
    task_id: &str,
    doc_type: Option<&str>,
) -> DocFrontmatter {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut fm = existing.unwrap_or_default();

    if fm.doc_id.is_none() {
        fm.doc_id = Some(generate_doc_id());
    }
    fm.task_id = Some(task_id.to_string());
    let (inferred_root, inferred_parent) = infer_task_hierarchy(task_id);
    if fm.root_task_id.is_none() {
        fm.root_task_id = Some(inferred_root);
    }
    if fm.parent_task_id.is_none() {
        fm.parent_task_id = inferred_parent;
    }

    if let Some(dtype) = doc_type {
        fm.doc_type = Some(dtype.to_string());
    } else if fm.doc_type.is_none() {
        fm.doc_type = Some("note".to_string());
    }

    if fm.created_at.is_none() {
        fm.created_at = Some(now.clone());
    }
    fm.updated_at = Some(now);

    if fm.status.is_none() {
        fm.status = Some("todo".to_string());
    }

    fm
}

fn infer_task_hierarchy(task_id: &str) -> (String, Option<String>) {
    let parent = task_id.rsplit_once('.').map(|(p, _)| p.to_string());
    let root = task_id
        .split_once('.')
        .map(|(r, _)| r.to_string())
        .unwrap_or_else(|| task_id.to_string());
    (root, parent)
}

/// Write frontmatter into a markdown file (prepend or replace).
pub fn write_frontmatter(file_path: &Path, fm: &DocFrontmatter) -> FlowResult<()> {
    let content = if file_path.exists() {
        std::fs::read_to_string(file_path)?
    } else {
        String::new()
    };

    let yaml = render_yaml(fm);

    let new_content = if has_frontmatter(&content) {
        // Replace existing frontmatter
        let body = strip_frontmatter(&content);
        format!("---\n{yaml}\n---\n{body}")
    } else if content.is_empty() {
        format!("---\n{yaml}\n---\n")
    } else {
        format!("---\n{yaml}\n---\n\n{content}")
    };

    std::fs::write(file_path, new_content)?;
    Ok(())
}

/// Remove frontmatter from a markdown file while preserving body.
/// Returns true if frontmatter existed and was removed.
pub fn remove_frontmatter(file_path: &Path) -> FlowResult<bool> {
    if !file_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(file_path)?;
    if !has_frontmatter(&content) {
        return Ok(false);
    }

    let body = strip_frontmatter(&content);
    std::fs::write(file_path, body)?;
    Ok(true)
}

fn has_frontmatter(content: &str) -> bool {
    content.trim_start().starts_with("---")
}

/// Remove existing frontmatter, return remaining body.
fn strip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);
    match after_open.find("\n---") {
        Some(pos) => {
            let rest = &after_open[pos + 4..];
            rest.strip_prefix('\n').unwrap_or(rest)
        }
        None => content,
    }
}

fn render_yaml(fm: &DocFrontmatter) -> String {
    let mut lines = Vec::new();
    macro_rules! field {
        ($name:ident) => {
            if let Some(v) = &fm.$name {
                lines.push(format!("{}: {v}", stringify!($name)));
            }
        };
        ($name:ident, null) => {
            match &fm.$name {
                Some(v) => lines.push(format!("{}: {v}", stringify!($name))),
                None => lines.push(format!("{}: null", stringify!($name))),
            }
        };
    }
    field!(doc_id);
    field!(doc_type);
    field!(task_id);
    field!(root_task_id);
    field!(parent_task_id, null);
    field!(created_at);
    field!(updated_at);
    field!(status);
    field!(branch);
    field!(agent_id);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parse_valid_frontmatter() {
        let content = r#"---
doc_id: D-20260303-001
doc_type: design
task_id: T-001.2
root_task_id: T-001
parent_task_id: null
status: in_progress
created_at: "2026-03-03T11:20:00Z"
updated_at: "2026-03-03T12:05:00Z"
---

# Design doc
"#;
        let fm = parse_frontmatter(content, "test.md").unwrap();
        assert_eq!(fm.doc_id.as_deref(), Some("D-20260303-001"));
        assert_eq!(fm.doc_type.as_deref(), Some("design"));
        assert_eq!(fm.task_id.as_deref(), Some("T-001.2"));
    }

    #[test]
    fn missing_frontmatter_errors() {
        let content = "# Just a heading\nSome content";
        let err = parse_frontmatter(content, "no-fm.md").unwrap_err();
        assert!(err.to_string().contains("Frontmatter missing"));
    }

    #[test]
    fn validate_missing_doc_id() {
        let fm = DocFrontmatter {
            doc_type: Some("design".into()),
            task_id: Some("T-001".into()),
            ..Default::default()
        };
        let err = validate_frontmatter(&fm, "test.md").unwrap_err();
        assert!(err.to_string().contains("doc_id missing"));
    }

    #[test]
    fn validate_invalid_status() {
        let fm = DocFrontmatter {
            doc_id: Some("D-001".into()),
            doc_type: Some("design".into()),
            task_id: Some("T-001".into()),
            status: Some("invalid".into()),
            ..Default::default()
        };
        let err = validate_frontmatter(&fm, "test.md").unwrap_err();
        assert!(err.to_string().contains("invalid status"));
    }

    #[test]
    fn generate_doc_id_format() {
        let id = generate_doc_id();
        assert!(id.starts_with("D-"));
        // D-YYYYMMDD-XXXXXX = 17 chars
        assert_eq!(id.len(), 17);
    }

    #[test]
    fn upsert_preserves_existing_root_parent() {
        let existing = DocFrontmatter {
            doc_id: Some("D-20260303-aaaaaa".into()),
            doc_type: Some("design".into()),
            task_id: Some("T-001".into()),
            root_task_id: Some("T-001".into()),
            parent_task_id: Some("T-000".into()),
            created_at: Some("2026-03-03T10:00:00Z".into()),
            updated_at: Some("2026-03-03T10:00:00Z".into()),
            status: Some("in_progress".into()),
            branch: None,
            agent_id: None,
        };

        let merged = upsert_frontmatter(Some(existing), "T-001.2", Some("impl"));
        assert_eq!(merged.root_task_id.as_deref(), Some("T-001"));
        assert_eq!(merged.parent_task_id.as_deref(), Some("T-000"));
        assert_eq!(merged.task_id.as_deref(), Some("T-001.2"));
        assert_eq!(merged.doc_type.as_deref(), Some("impl"));
    }

    #[test]
    fn remove_frontmatter_keeps_body() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("doc.md");
        fs::write(
            &file_path,
            "---\ndoc_id: D-1\ndoc_type: note\ntask_id: T-001\nstatus: todo\ncreated_at: \"2026-03-03T00:00:00Z\"\nupdated_at: \"2026-03-03T00:00:00Z\"\n---\n\n# Body\n",
        )
        .unwrap();

        let removed = remove_frontmatter(&file_path).unwrap();
        let content = fs::read_to_string(&file_path).unwrap();

        assert!(removed);
        assert_eq!(content, "\n# Body\n");
    }

    #[test]
    fn remove_frontmatter_returns_false_when_missing() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("doc.md");
        fs::write(&file_path, "# Body only\n").unwrap();

        let removed = remove_frontmatter(&file_path).unwrap();
        let content = fs::read_to_string(&file_path).unwrap();

        assert!(!removed);
        assert_eq!(content, "# Body only\n");
    }
}
