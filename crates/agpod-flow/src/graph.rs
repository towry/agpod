//! Task graph data model and rebuild logic.
//!
//! Keywords: graph.json, task graph, rebuild, edges, doc node, task node

use crate::config::FlowDocsConfig;
use crate::error::{FlowError, FlowResult};
use crate::frontmatter::{parse_frontmatter, validate_frontmatter};
use crate::repo_id::RepoIdentity;
use crate::scanner;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const BOOTSTRAP_TASK_ID: &str = "T-001";

/// The full graph cache structure (graph.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraph {
    pub version: u32,
    pub repo_id: String,
    pub generated_at: String,
    pub tasks: HashMap<String, TaskNode>,
    pub docs: HashMap<String, DocNode>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub task_id: String,
    pub root_task_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub children: Vec<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocNode {
    pub doc_id: String,
    pub doc_type: String,
    pub task_id: String,
    pub path: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    #[serde(rename = "type")]
    pub edge_type: String,
    pub from: String,
    pub to: String,
}

/// Diagnostic entry for rebuild issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub level: String, // "error" | "warning"
    pub path: Option<String>,
    pub message: String,
}

/// Diagnostics report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    pub version: u32,
    pub generated_at: String,
    pub items: Vec<Diagnostic>,
}

/// Rebuild graph from documents.
pub fn rebuild(
    repo_root: &Path,
    identity: &RepoIdentity,
    config: &FlowDocsConfig,
) -> FlowResult<(TaskGraph, DiagnosticsReport)> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let files = scanner::scan_documents(repo_root, config)?;

    let mut tasks: HashMap<String, TaskNode> = HashMap::new();
    let mut docs: HashMap<String, DocNode> = HashMap::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    for file_path in &files {
        let rel_path = file_path
            .strip_prefix(repo_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                diagnostics.push(Diagnostic {
                    level: "error".into(),
                    path: Some(rel_path),
                    message: format!("Failed to read file: {e}"),
                });
                continue;
            }
        };

        // Parse frontmatter (fail-fast if required and missing)
        let fm = match parse_frontmatter(&content, &rel_path) {
            Ok(fm) => fm,
            Err(e) => {
                if config.frontmatter_required {
                    return Err(e);
                }
                diagnostics.push(Diagnostic {
                    level: "warning".into(),
                    path: Some(rel_path),
                    message: e.to_string(),
                });
                continue;
            }
        };

        // Validate (fail-fast on doc_id missing)
        validate_frontmatter(&fm, &rel_path)?;

        let doc_id = fm.doc_id.as_ref().unwrap().clone();
        let task_id = fm.task_id.as_ref().unwrap().clone();
        let doc_type = fm.doc_type.as_ref().unwrap().clone();

        // Handle duplicate doc_id: keep latest updated_at
        if let Some(existing) = docs.get(&doc_id) {
            let existing_ts = existing.updated_at.as_deref().unwrap_or("");
            let new_ts = fm.updated_at.as_deref().unwrap_or("");
            if new_ts <= existing_ts {
                diagnostics.push(Diagnostic {
                    level: "warning".into(),
                    path: Some(rel_path),
                    message: format!("Duplicate doc_id '{doc_id}', keeping newer version"),
                });
                continue;
            }
            diagnostics.push(Diagnostic {
                level: "warning".into(),
                path: Some(existing.path.clone()),
                message: format!("Duplicate doc_id '{doc_id}', superseded by {}", rel_path),
            });
        }

        // Insert doc node
        docs.insert(
            doc_id.clone(),
            DocNode {
                doc_id: doc_id.clone(),
                doc_type,
                task_id: task_id.clone(),
                path: rel_path,
                created_at: fm.created_at.clone(),
                updated_at: fm.updated_at.clone(),
            },
        );

        let (inferred_root, inferred_parent) = infer_task_hierarchy(&task_id);
        let effective_root = fm.root_task_id.clone().or(Some(inferred_root));
        let effective_parent = fm.parent_task_id.clone().or(inferred_parent);

        // Ensure task node exists
        ensure_task(
            &mut tasks,
            &task_id,
            effective_root.clone(),
            effective_parent.clone(),
            fm.status.clone(),
        );

        // Build edges
        edges.push(Edge {
            edge_type: "doc_task".into(),
            from: doc_id,
            to: task_id.clone(),
        });

        // Parent-child edge
        if let Some(parent_id) = &effective_parent {
            ensure_task_minimal(&mut tasks, parent_id);

            // Add child to parent
            if let Some(parent) = tasks.get_mut(parent_id) {
                if !parent.children.contains(&task_id) {
                    parent.children.push(task_id.clone());
                }
            }

            edges.push(Edge {
                edge_type: "parent_child".into(),
                from: parent_id.clone(),
                to: task_id.clone(),
            });
        } else if let Some(root_id) = &effective_root {
            // If no explicit parent but has root, and task != root, mark as child of root
            if root_id != &task_id {
                ensure_task_minimal(&mut tasks, root_id);
                if let Some(root) = tasks.get_mut(root_id) {
                    if !root.children.contains(&task_id) {
                        root.children.push(task_id.clone());
                    }
                }
                edges.push(Edge {
                    edge_type: "parent_child".into(),
                    from: root_id.clone(),
                    to: task_id,
                });
            }
        }
    }

    // Check for orphan tasks (parent_task_id references non-existent task)
    for task in tasks.values() {
        if let Some(parent_id) = &task.parent_task_id {
            if !tasks.contains_key(parent_id) {
                diagnostics.push(Diagnostic {
                    level: "warning".into(),
                    path: None,
                    message: format!(
                        "Task '{}' references parent '{}' which does not exist (orphan)",
                        task.task_id, parent_id
                    ),
                });
            }
        }
    }

    // Deduplicate edges
    edges.sort_by(|a, b| (&a.edge_type, &a.from, &a.to).cmp(&(&b.edge_type, &b.from, &b.to)));
    edges.dedup_by(|a, b| a.edge_type == b.edge_type && a.from == b.from && a.to == b.to);

    let graph = TaskGraph {
        version: 1,
        repo_id: identity.repo_id.clone(),
        generated_at: now.clone(),
        tasks,
        docs,
        edges,
    };

    let report = DiagnosticsReport {
        version: 1,
        generated_at: now,
        items: diagnostics,
    };

    Ok((graph, report))
}

fn ensure_task(
    tasks: &mut HashMap<String, TaskNode>,
    task_id: &str,
    root_task_id: Option<String>,
    parent_task_id: Option<String>,
    status: Option<String>,
) {
    let entry = tasks
        .entry(task_id.to_string())
        .or_insert_with(|| TaskNode {
            task_id: task_id.to_string(),
            root_task_id: None,
            parent_task_id: None,
            children: Vec::new(),
            status: None,
        });

    // Update with frontmatter data (later doc wins)
    if root_task_id.is_some() {
        entry.root_task_id = root_task_id;
    }
    if parent_task_id.is_some() {
        entry.parent_task_id = parent_task_id;
    }
    if status.is_some() {
        entry.status = status;
    }
}

fn ensure_task_minimal(tasks: &mut HashMap<String, TaskNode>, task_id: &str) {
    tasks
        .entry(task_id.to_string())
        .or_insert_with(|| TaskNode {
            task_id: task_id.to_string(),
            root_task_id: None,
            parent_task_id: None,
            children: Vec::new(),
            status: None,
        });
}

/// Save graph and diagnostics to disk.
pub fn save(
    identity: &RepoIdentity,
    graph: &TaskGraph,
    diagnostics: &DiagnosticsReport,
) -> FlowResult<()> {
    let dir = crate::storage::ensure_repo_data_dir(identity)?;

    // Save repo-meta.json
    let meta = serde_json::json!({
        "repo_id": identity.repo_id,
        "repo_label": identity.repo_label,
        "generated_at": graph.generated_at,
    });
    let meta_path = dir.join("repo-meta.json");
    crate::storage::write_atomic(&meta_path, &serde_json::to_string_pretty(&meta)?)?;

    // Save graph.json
    let graph_path = dir.join("graph.json");
    crate::storage::write_atomic(&graph_path, &serde_json::to_string_pretty(graph)?)?;

    // Save diagnostics.json
    let diagnostics_path = dir.join("diagnostics.json");
    crate::storage::write_atomic(
        &diagnostics_path,
        &serde_json::to_string_pretty(diagnostics)?,
    )?;

    Ok(())
}

/// Load graph from cache.
pub fn load(identity: &RepoIdentity) -> FlowResult<TaskGraph> {
    let path = crate::storage::graph_path(identity)?;
    let content = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            FlowError::Other(format!(
                "graph.json not found for repo '{}'. Run `agpod flow rebuild` first.",
                identity.repo_label
            ))
        } else {
            FlowError::Io(e)
        }
    })?;
    let graph: TaskGraph = serde_json::from_str(&content)?;
    Ok(graph)
}

/// Load graph if present; otherwise initialize an empty graph cache.
pub fn load_or_init(identity: &RepoIdentity) -> FlowResult<TaskGraph> {
    match load(identity) {
        Ok(graph) => Ok(graph),
        Err(FlowError::Other(msg)) if msg.contains("graph.json not found") => {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let graph = TaskGraph {
                version: 1,
                repo_id: identity.repo_id.clone(),
                generated_at: now.clone(),
                tasks: HashMap::new(),
                docs: HashMap::new(),
                edges: Vec::new(),
            };
            let diagnostics = DiagnosticsReport {
                version: 1,
                generated_at: now,
                items: Vec::new(),
            };
            save(identity, &graph, &diagnostics)?;
            Ok(graph)
        }
        Err(e) => Err(e),
    }
}

/// Ensure a task exists in graph. If missing, initialize it as a root-level task.
pub fn ensure_task_exists(identity: &RepoIdentity, task_id: &str) -> FlowResult<()> {
    let mut graph = load_or_init(identity)?;
    if graph.tasks.contains_key(task_id) {
        return Ok(());
    }

    graph.tasks.insert(
        task_id.to_string(),
        TaskNode {
            task_id: task_id.to_string(),
            root_task_id: Some(task_id.to_string()),
            parent_task_id: None,
            children: Vec::new(),
            status: Some("todo".to_string()),
        },
    );
    graph.generated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let diagnostics = DiagnosticsReport {
        version: 1,
        generated_at: graph.generated_at.clone(),
        items: Vec::new(),
    };
    save(identity, &graph, &diagnostics)
}

/// Add a forked task to existing graph and persist.
pub fn add_fork_task(identity: &RepoIdentity, parent_task_id: &str) -> FlowResult<String> {
    let _lock = crate::storage::acquire_repo_lock(identity)?;
    let mut graph = load(identity)?;

    if !graph.tasks.contains_key(parent_task_id) {
        return Err(FlowError::TaskNotFound(parent_task_id.to_string()));
    }

    let new_task_id = allocate_next_child_task_id(&graph, parent_task_id);

    let root_task_id = infer_root_task_id(&graph, parent_task_id);
    graph.tasks.insert(
        new_task_id.to_string(),
        TaskNode {
            task_id: new_task_id.to_string(),
            root_task_id: Some(root_task_id),
            parent_task_id: Some(parent_task_id.to_string()),
            children: Vec::new(),
            status: Some("todo".to_string()),
        },
    );

    if let Some(parent) = graph.tasks.get_mut(parent_task_id) {
        if !parent.children.contains(&new_task_id.to_string()) {
            parent.children.push(new_task_id.to_string());
        }
    }

    let edge = Edge {
        edge_type: "parent_child".to_string(),
        from: parent_task_id.to_string(),
        to: new_task_id.clone(),
    };
    if !graph
        .edges
        .iter()
        .any(|e| e.edge_type == edge.edge_type && e.from == edge.from && e.to == edge.to)
    {
        graph.edges.push(edge);
    }

    graph.generated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let diagnostics = DiagnosticsReport {
        version: 1,
        generated_at: graph.generated_at.clone(),
        items: Vec::new(),
    };
    save(identity, &graph, &diagnostics)?;
    Ok(new_task_id)
}

fn allocate_next_child_task_id(graph: &TaskGraph, parent_task_id: &str) -> String {
    // Use max(existing direct child numeric suffix) + 1 to keep IDs monotonic.
    let mut max_child_index: u32 = 0;
    for task in graph.tasks.values() {
        if task.parent_task_id.as_deref() != Some(parent_task_id) {
            continue;
        }
        let Some(suffix) = task.task_id.strip_prefix(&format!("{parent_task_id}.")) else {
            continue;
        };
        // Only count direct children like T-001.2, not deeper descendants like T-001.2.1
        if suffix.contains('.') {
            continue;
        }
        let Ok(index) = suffix.parse::<u32>() else {
            continue;
        };
        if index > max_child_index {
            max_child_index = index;
        }
    }

    format!("{parent_task_id}.{}", max_child_index + 1)
}

fn infer_root_task_id(graph: &TaskGraph, start_task_id: &str) -> String {
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut current = start_task_id.to_string();

    loop {
        if !visited.insert(current.clone()) {
            return start_task_id.to_string();
        }

        let Some(node) = graph.tasks.get(&current) else {
            return start_task_id.to_string();
        };

        if let Some(root) = &node.root_task_id {
            return root.clone();
        }

        if let Some(parent) = &node.parent_task_id {
            current = parent.clone();
            continue;
        }

        return node.task_id.clone();
    }
}

fn infer_task_hierarchy(task_id: &str) -> (String, Option<String>) {
    let parent = task_id.rsplit_once('.').map(|(p, _)| p.to_string());
    let root = task_id
        .split_once('.')
        .map(|(r, _)| r.to_string())
        .unwrap_or_else(|| task_id.to_string());
    (root, parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_next_child_task_id_uses_max_plus_one() {
        let mut tasks = HashMap::new();
        tasks.insert(
            "T-001".to_string(),
            TaskNode {
                task_id: "T-001".to_string(),
                root_task_id: Some("T-001".to_string()),
                parent_task_id: None,
                children: vec!["T-001.1".to_string(), "T-001.3".to_string()],
                status: Some("todo".to_string()),
            },
        );
        tasks.insert(
            "T-001.1".to_string(),
            TaskNode {
                task_id: "T-001.1".to_string(),
                root_task_id: Some("T-001".to_string()),
                parent_task_id: Some("T-001".to_string()),
                children: vec![],
                status: Some("todo".to_string()),
            },
        );
        tasks.insert(
            "T-001.3".to_string(),
            TaskNode {
                task_id: "T-001.3".to_string(),
                root_task_id: Some("T-001".to_string()),
                parent_task_id: Some("T-001".to_string()),
                children: vec!["T-001.3.1".to_string()],
                status: Some("todo".to_string()),
            },
        );
        tasks.insert(
            "T-001.3.1".to_string(),
            TaskNode {
                task_id: "T-001.3.1".to_string(),
                root_task_id: Some("T-001".to_string()),
                parent_task_id: Some("T-001.3".to_string()),
                children: vec![],
                status: Some("todo".to_string()),
            },
        );

        let graph = TaskGraph {
            version: 1,
            repo_id: "repo".to_string(),
            generated_at: "2026-03-03T00:00:00Z".to_string(),
            tasks,
            docs: HashMap::new(),
            edges: Vec::new(),
        };

        let next = allocate_next_child_task_id(&graph, "T-001");
        assert_eq!(next, "T-001.4");
    }
}
