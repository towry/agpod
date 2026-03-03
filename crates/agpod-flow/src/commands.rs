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
use std::path::PathBuf;

pub fn execute(args: FlowArgs) -> Result<()> {
    match args.command {
        FlowCommand::Rebuild => cmd_rebuild(args.json),
        FlowCommand::Recent { limit, days } => cmd_recent(limit, days, args.json),
        FlowCommand::Tree { root } => cmd_tree(root, args.json),
        FlowCommand::Session { command } => cmd_session(command, args.session, args.json),
        FlowCommand::Status => cmd_status(args.session, args.json),
        FlowCommand::Focus { task } => cmd_focus(args.session, &task),
        FlowCommand::Fork {
            to,
            from,
            no_switch,
        } => cmd_fork(args.session, &to, from.as_deref(), no_switch),
        FlowCommand::Parent => cmd_parent(args.session),
        FlowCommand::Doc { command } => cmd_doc(command, args.session, args.json),
    }
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

fn cmd_rebuild(json: bool) -> Result<()> {
    let repo_root = get_repo_root()?;
    let identity = RepoIdentity::resolve_from(&repo_root)?;
    let config = FlowDocsConfig::load(&repo_root)?;

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
    let label = if let Some(task) = g.tasks.get(task_id) {
        let icon = status_icon(task.status.as_deref());
        let status = task.status.as_deref().unwrap_or("unknown");
        format!("{icon} {} [{status}]", task.task_id)
    } else {
        format!("? {task_id} [not found]")
    };

    let mut tree = termtree::Tree::new(label);

    // Attach docs as leaves
    if let Some(docs) = docs_by_task.get(task_id) {
        for doc in docs {
            tree.push(termtree::Tree::new(format!(
                "📄 {} ({}) [{}]",
                doc.doc_id, doc.path, doc.doc_type
            )));
        }
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

    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
    } else {
        let task = s.active_task_id.as_deref().unwrap_or("(none)");
        println!("Session:     {}", s.session_id);
        println!("Active task: {task}");
        println!("Updated:     {}", s.updated_at);
    }

    Ok(())
}

fn cmd_focus(session_arg: Option<String>, task_id: &str) -> Result<()> {
    let sid = require_session(session_arg)?;
    let s = session::focus(&sid, task_id)?;
    println!(
        "Focused on task: {}",
        s.active_task_id.as_deref().unwrap_or("?")
    );
    Ok(())
}

fn cmd_fork(
    session_arg: Option<String>,
    new_task_id: &str,
    from: Option<&str>,
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
    graph::add_fork_task(&identity, &parent_task, new_task_id)?;

    println!("Created {} (parent: {})", new_task_id, parent_task);

    if no_switch {
        let current = s.active_task_id.as_deref().unwrap_or("(none)");
        println!("Staying on: {current}");
    } else {
        session::focus(&sid, new_task_id)?;
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
            session::focus(&sid, parent_id)?;
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
    match command {
        DocCommand::Init {
            path,
            task,
            doc_type,
        } => {
            let file_path = PathBuf::from(&path);
            let existing = frontmatter::read_existing_frontmatter(&file_path)?;
            let fm = frontmatter::upsert_frontmatter(existing, &task, Some(&doc_type));
            frontmatter::write_frontmatter(&file_path, &fm)?;
            println!("Initialized frontmatter in: {path}");
            println!("  doc_id:  {}", fm.doc_id.as_deref().unwrap_or("?"));
            println!("  task_id: {task}");
            println!("  type:    {doc_type}");
        }
        DocCommand::Add {
            path,
            task,
            doc_type,
        } => {
            let sid = require_session(session_arg)?;
            let s = session::load(&sid)?;
            let task_id = task
                .as_deref()
                .or(s.active_task_id.as_deref())
                .ok_or_else(|| FlowError::NoActiveTask {
                    session_id: sid.clone(),
                })?;

            let file_path = PathBuf::from(&path);
            let existing = frontmatter::read_existing_frontmatter(&file_path)?;
            let dtype = doc_type.as_deref();
            let fm = frontmatter::upsert_frontmatter(existing, task_id, dtype);
            frontmatter::write_frontmatter(&file_path, &fm)?;
            println!("Added document: {path} -> task {task_id}");
        }
    }

    Ok(())
}
