//! Command execution dispatch.
//!
//! Keywords: flow commands, execute, dispatch

use crate::cli::*;
use crate::config::FlowDocsConfig;
use crate::error::FlowError;
use crate::frontmatter;
use crate::graph;
use crate::recent;
use crate::repo_id::RepoIdentity;
use crate::session;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn execute(args: FlowArgs) -> Result<()> {
    match args.command {
        FlowCommand::Init => cmd_init(args.json),
        FlowCommand::Rebuild => cmd_rebuild(args.json),
        FlowCommand::Recent { limit, days } => cmd_recent(limit, days, args.json),
        FlowCommand::Tree { root } => cmd_tree(root, args.json),
        FlowCommand::Session { command } => cmd_session(command, args.session, args.json),
        FlowCommand::Status => cmd_status(args.session, args.json),
        FlowCommand::Focus { task } => cmd_focus(args.session, &task, args.json),
        FlowCommand::Fork {
            from,
            checkpoint,
            no_switch,
        } => cmd_fork(args.session, from.as_deref(), &checkpoint, no_switch),
        FlowCommand::Parent => cmd_parent(args.session),
        FlowCommand::Doc { command } => cmd_doc(command, args.session, args.json),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct TaskDocSummary {
    doc_id: String,
    path: String,
    doc_type: String,
}

fn load_task_docs(identity: &RepoIdentity, task_id: &str) -> Result<Vec<TaskDocSummary>> {
    let task_graph = match graph::load(identity) {
        Ok(g) => g,
        Err(FlowError::Other(msg)) if msg.contains("graph.json not found") => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut docs: Vec<TaskDocSummary> = task_graph
        .docs
        .values()
        .filter(|d| d.task_id == task_id)
        .map(|d| TaskDocSummary {
            doc_id: d.doc_id.clone(),
            path: d.path.clone(),
            doc_type: d.doc_type.clone(),
        })
        .collect();
    docs.sort_by(|a, b| a.path.cmp(&b.path).then(a.doc_id.cmp(&b.doc_id)));
    Ok(docs)
}

fn session_json_with_docs(
    session: &session::Session,
    docs: &[TaskDocSummary],
) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(session)?;
    if let serde_json::Value::Object(ref mut map) = value {
        map.insert("docs".to_string(), serde_json::to_value(docs)?);
    }
    Ok(value)
}

fn get_repo_root() -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Not in a git repository");
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn require_session(session: Option<String>) -> Result<String> {
    session.ok_or_else(|| anyhow::anyhow!("This command requires -s <session-id>"))
}

// --- Stateless commands ---

fn cmd_init(json: bool) -> Result<()> {
    let repo_root = get_repo_root()?;
    let config_path = repo_root.join(".agpod.flow.toml");
    if config_path.exists() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "initialized": true,
                    "created": false,
                    "config_path": config_path,
                }))?
            );
        } else {
            println!("Already initialized: {}", config_path.display());
        }
    } else {
        let content = r#"[flow.docs]
root = "docs"
include_globs = ["**/*.md", "**/*.mdx"]
exclude_globs = ["**/node_modules/**", "**/.git/**", "**/dist/**"]
frontmatter_required = true
follow_symlinks = false
"#;
        std::fs::write(&config_path, content)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "initialized": true,
                    "created": true,
                    "config_path": config_path,
                }))?
            );
        } else {
            println!("Created: {}", config_path.display());
        }
    }

    let config = FlowDocsConfig::load(&repo_root)?;
    let flow_root = config.ensure_flow_root(&repo_root)?;
    if !json {
        println!("Flow docs root: {}", flow_root.display());
    }

    Ok(())
}

fn cmd_rebuild(json: bool) -> Result<()> {
    let repo_root = get_repo_root()?;
    let identity = RepoIdentity::resolve_from(&repo_root)?;
    let config = FlowDocsConfig::load(&repo_root)?;
    let _ = config.ensure_flow_root(&repo_root)?;

    let (task_graph, diagnostics) = graph::rebuild(&repo_root, &identity, &config)?;
    graph::save(&identity, &task_graph, &diagnostics)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&task_graph)?);
    } else {
        println!(
            "Graph rebuilt: {} tasks, {} docs, {} edges",
            task_graph.tasks.len(),
            task_graph.docs.len(),
            task_graph.edges.len()
        );
        if !diagnostics.items.is_empty() {
            eprintln!("{} diagnostics:", diagnostics.items.len());
            for d in &diagnostics.items {
                let path_info = d.path.as_deref().unwrap_or("-");
                eprintln!("  [{}] {}: {}", d.level, path_info, d.message);
            }
        }
    }

    Ok(())
}

fn cmd_recent(limit: usize, days: u32, json: bool) -> Result<()> {
    let repo_root = get_repo_root()?;
    let config = FlowDocsConfig::load(&repo_root)?;
    let _ = config.ensure_flow_root(&repo_root)?;

    let results = recent::recent_tasks(&repo_root, &config, limit, days)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else if results.is_empty() {
        println!("No recent tasks found in the last {days} days");
    } else {
        for r in &results {
            println!(
                "{:<12} score={:<8.2} last_seen={}",
                r.task_id, r.score, r.last_seen_at
            );
            for ev in &r.evidence {
                println!("  └─ {ev}");
            }
            println!("  → {}", r.suggested_command);
        }
    }

    Ok(())
}

fn cmd_tree(root: Option<String>, json: bool) -> Result<()> {
    let repo_root = get_repo_root()?;
    let identity = RepoIdentity::resolve_from(&repo_root)?;
    let task_graph = graph::load(&identity)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&task_graph)?);
        return Ok(());
    }

    // Index docs by task_id for O(1) lookup
    let mut docs_by_task: HashMap<&str, Vec<&graph::DocNode>> = HashMap::new();
    for doc in task_graph.docs.values() {
        docs_by_task
            .entry(doc.task_id.as_str())
            .or_default()
            .push(doc);
    }

    let mut roots: Vec<&str> = if let Some(ref r) = root {
        vec![r.as_str()]
    } else {
        task_graph
            .tasks
            .values()
            .filter(|t| t.parent_task_id.is_none())
            .map(|t| t.task_id.as_str())
            .collect()
    };
    roots.sort();

    for root_id in roots {
        let tree = build_termtree(&task_graph, &docs_by_task, root_id);
        println!("{tree}");
    }

    Ok(())
}

fn status_icon(status: Option<&str>) -> &'static str {
    match status {
        Some("done") => "✓",
        Some("in_progress") => "▶",
        Some("blocked") => "⏸",
        Some("todo") => "○",
        Some("archived") => "⊘",
        _ => "?",
    }
}

fn build_termtree(
    g: &graph::TaskGraph,
    docs_by_task: &HashMap<&str, Vec<&graph::DocNode>>,
    task_id: &str,
) -> termtree::Tree<String> {
    let mut task_docs: Vec<&graph::DocNode> =
        docs_by_task.get(task_id).cloned().unwrap_or_default();
    task_docs.sort_by(|a, b| a.path.cmp(&b.path));
    let doc_hint = task_hint(&task_docs);

    let label = if let Some(task) = g.tasks.get(task_id) {
        let icon = status_icon(task.status.as_deref());
        let status = task.status.as_deref().unwrap_or("unknown");
        if let Some(hint) = doc_hint {
            format!("{icon} {} [{status}] - {hint}", task.task_id)
        } else {
            format!("{icon} {} [{status}]", task.task_id)
        }
    } else {
        format!("? {task_id} [not found]")
    };

    let mut tree = termtree::Tree::new(label);

    // Attach docs as leaves
    for doc in task_docs {
        tree.push(termtree::Tree::new(format!(
            "📄 {} ({}) [{}]",
            doc.doc_id, doc.path, doc.doc_type
        )));
    }

    // Recurse into children
    if let Some(task) = g.tasks.get(task_id) {
        let mut children = task.children.clone();
        children.sort();
        for child_id in &children {
            tree.push(build_termtree(g, docs_by_task, child_id));
        }
    }

    tree
}

fn task_hint(docs: &[&graph::DocNode]) -> Option<String> {
    let first = docs.first()?;
    let stem = Path::new(&first.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc");
    Some(format!("{stem} ({} doc)", docs.len()))
}

// --- Session commands ---

fn cmd_session(command: SessionCommand, session_arg: Option<String>, json: bool) -> Result<()> {
    match command {
        SessionCommand::New => {
            let repo_root = get_repo_root()?;
            let identity = RepoIdentity::resolve_from(&repo_root)?;
            let s = session::create(&identity.repo_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                println!("Session created: {}", s.session_id);
            }
        }
        SessionCommand::List => {
            let repo_root = get_repo_root()?;
            let identity = RepoIdentity::resolve_from(&repo_root)?;
            let sessions = session::list(&identity.repo_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else if sessions.is_empty() {
                println!("No active sessions");
            } else {
                for s in &sessions {
                    let task = s.active_task_id.as_deref().unwrap_or("(none)");
                    println!(
                        "{} | task: {} | updated: {}",
                        s.session_id, task, s.updated_at
                    );
                }
            }
        }
        SessionCommand::Close => {
            let sid = require_session(session_arg)?;
            session::close(&sid)?;
            println!("Session closed: {sid}");
        }
    }

    Ok(())
}

// --- Stateful commands (require -s) ---

fn cmd_status(session_arg: Option<String>, json: bool) -> Result<()> {
    let sid = require_session(session_arg)?;
    let s = session::load(&sid)?;
    let docs = if let Some(active_task) = s.active_task_id.as_deref() {
        let repo_root = get_repo_root()?;
        let identity = RepoIdentity::resolve_from(&repo_root)?;
        load_task_docs(&identity, active_task)?
    } else {
        Vec::new()
    };

    if json {
        let payload = session_json_with_docs(&s, &docs)?;
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        let task = s.active_task_id.as_deref().unwrap_or("(none)");
        println!("Session:     {}", s.session_id);
        println!("Active task: {task}");
        println!("Updated:     {}", s.updated_at);
        if s.history.is_empty() {
            println!("History:");
            if let Some(active) = &s.active_task_id {
                println!("  - (none) -> {} (resume) @ {}", active, s.updated_at);
            } else {
                println!("  - (empty)");
            }
        } else {
            println!("History:");
            for h in s.history.iter().rev().take(5) {
                let from = h.from_task_id.as_deref().unwrap_or("(none)");
                println!("  - {} -> {} ({}) @ {}", from, h.to_task_id, h.action, h.at);
            }
        }
        println!("Docs:");
        if docs.is_empty() {
            println!("  - (none)");
        } else {
            for d in &docs {
                println!("  - {} ({}) [{}]", d.doc_id, d.path, d.doc_type);
            }
        }
    }

    Ok(())
}

fn cmd_focus(session_arg: Option<String>, task_id: &str, json: bool) -> Result<()> {
    let sid = require_session(session_arg)?;
    let s = session::focus(&sid, task_id)?;
    let active_task = s.active_task_id.as_deref().unwrap_or("?");
    let repo_root = get_repo_root()?;
    let identity = RepoIdentity::resolve_from(&repo_root)?;
    let docs = load_task_docs(&identity, active_task)?;

    if json {
        let payload = session_json_with_docs(&s, &docs)?;
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("Focused on task: {}", active_task);
        println!("Docs:");
        if docs.is_empty() {
            println!("  - (none)");
        } else {
            for d in &docs {
                println!("  - {} ({}) [{}]", d.doc_id, d.path, d.doc_type);
            }
        }
    }
    Ok(())
}

fn cmd_fork(
    session_arg: Option<String>,
    from: Option<&str>,
    checkpoint: &str,
    no_switch: bool,
) -> Result<()> {
    let sid = require_session(session_arg)?;
    let s = session::load(&sid)?;

    // Determine parent: explicit --from, or fall back to active task
    let parent_task = match from {
        Some(id) => id.to_string(),
        None => session::require_active_task(&s)?.to_string(),
    };

    let repo_root = get_repo_root()?;
    let identity = RepoIdentity::resolve_from(&repo_root)?;
    let new_task_id = graph::add_fork_task(&identity, &parent_task)?;

    println!("Created {} (parent: {})", new_task_id, parent_task);
    println!("Checkpoint: {checkpoint}");

    if no_switch {
        let current = s.active_task_id.as_deref().unwrap_or("(none)");
        println!("Staying on: {current}");
    } else {
        let action = format!("fork[{checkpoint}]");
        session::transition(&sid, &new_task_id, &action)?;
        println!("Switched to: {new_task_id}");
    }

    Ok(())
}

fn cmd_parent(session_arg: Option<String>) -> Result<()> {
    let sid = require_session(session_arg)?;
    let s = session::load(&sid)?;
    let current = session::require_active_task(&s)?;

    // Load graph to find parent
    let repo_root = get_repo_root()?;
    let identity = RepoIdentity::resolve_from(&repo_root)?;
    let task_graph = graph::load(&identity)?;

    if let Some(task) = task_graph.tasks.get(current) {
        if let Some(parent_id) = &task.parent_task_id {
            session::transition(&sid, parent_id, "parent")?;
            println!("Moved to parent task: {parent_id}");
        } else {
            eprintln!("Task '{current}' has no parent");
        }
    } else {
        return Err(FlowError::TaskNotFound(current.to_string()).into());
    }

    Ok(())
}

// --- Doc commands ---

fn cmd_doc(command: DocCommand, session_arg: Option<String>, _json: bool) -> Result<()> {
    let repo_root = get_repo_root()?;
    let config = FlowDocsConfig::load(&repo_root)?;
    let flow_root = config.ensure_flow_root(&repo_root)?;

    match command {
        DocCommand::Init {
            path,
            task,
            doc_type,
            content,
            force,
        } => {
            let task_id = resolve_or_init_task(task, session_arg.clone())?;
            let file_path = resolve_doc_path(&repo_root, &config, &flow_root, &path)?;
            let existing = frontmatter::read_existing_frontmatter(&file_path)?;
            if existing.is_some() && !force {
                anyhow::bail!(
                    "Frontmatter already exists in '{}'. Re-run with --force to overwrite.",
                    file_path.display()
                );
            }
            let fm = frontmatter::upsert_frontmatter(existing, &task_id, Some(&doc_type));
            frontmatter::write_document(&file_path, &fm, &content)?;
            println!("Initialized frontmatter in: {}", file_path.display());
            println!("  doc_id:  {}", fm.doc_id.as_deref().unwrap_or("?"));
            println!("  task_id: {task_id}");
            println!("  type:    {doc_type}");
        }
        DocCommand::Add {
            path,
            task,
            doc_type,
            content,
        } => {
            let task_id = resolve_or_init_task(task, session_arg)?;

            let file_path = resolve_doc_path(&repo_root, &config, &flow_root, &path)?;
            let existing = frontmatter::read_existing_frontmatter(&file_path)?;
            let dtype = doc_type.as_deref();
            let fm = frontmatter::upsert_frontmatter(existing, &task_id, dtype);
            frontmatter::write_document(&file_path, &fm, &content)?;
            println!(
                "Added document: {} -> task {}",
                file_path.display(),
                task_id
            );
        }
        DocCommand::Remove { path } => {
            let file_path = resolve_doc_path(&repo_root, &config, &flow_root, &path)?;
            let removed = frontmatter::remove_frontmatter(&file_path)?;
            if removed {
                println!("Removed flow frontmatter: {}", file_path.display());
            } else {
                println!("No frontmatter found: {}", file_path.display());
            }
        }
    }

    Ok(())
}

fn resolve_or_init_task(task: Option<String>, session_arg: Option<String>) -> Result<String> {
    if let Some(task_id) = task {
        let repo_root = get_repo_root()?;
        let identity = RepoIdentity::resolve_from(&repo_root)?;
        graph::ensure_task_exists(&identity, &task_id)?;
        return Ok(task_id);
    }

    if let Some(sid) = session_arg {
        let s = session::load(&sid)?;
        if let Some(active) = s.active_task_id {
            let repo_root = get_repo_root()?;
            let identity = RepoIdentity::resolve_from(&repo_root)?;
            graph::ensure_task_exists(&identity, &active)?;
            return Ok(active);
        }
    }

    let repo_root = get_repo_root()?;
    let identity = RepoIdentity::resolve_from(&repo_root)?;
    graph::ensure_task_exists(&identity, graph::BOOTSTRAP_TASK_ID)?;
    eprintln!(
        "No task provided; bootstrapped default task {}",
        graph::BOOTSTRAP_TASK_ID
    );
    Ok(graph::BOOTSTRAP_TASK_ID.to_string())
}

fn resolve_doc_path(
    _repo_root: &Path,
    config: &FlowDocsConfig,
    flow_root: &Path,
    input_path: &str,
) -> Result<PathBuf> {
    let input = PathBuf::from(input_path);
    if input.is_absolute() {
        anyhow::bail!(
            "Document path must be relative to repo root (for example '{}'), absolute path is not allowed: '{}'",
            Path::new(&config.root)
                .join(FlowDocsConfig::FLOW_SUBDIR)
                .display(),
            input.display()
        );
    }

    let root_prefix = PathBuf::from(&config.root);
    let flow_prefix = root_prefix.join(FlowDocsConfig::FLOW_SUBDIR);

    let normalized_relative = if input.starts_with(&flow_prefix) {
        input
            .strip_prefix(&flow_prefix)
            .unwrap_or(&input)
            .to_path_buf()
    } else if input.starts_with(&root_prefix) {
        input
            .strip_prefix(&root_prefix)
            .unwrap_or(&input)
            .to_path_buf()
    } else if input.starts_with(FlowDocsConfig::FLOW_SUBDIR) {
        input
            .strip_prefix(FlowDocsConfig::FLOW_SUBDIR)
            .unwrap_or(&input)
            .to_path_buf()
    } else {
        input
    };

    Ok(flow_root.join(normalized_relative))
}
