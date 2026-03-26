//! Output formatting for JSON and human-readable text.
//!
//! Keywords: output, json, text, render, format, display

use crate::types::*;
use chrono::{DateTime, Local};
use serde_json::{json, Value};
use termtree::Tree;

/// Render the result either as JSON or human-readable text.
pub fn render(json_mode: bool, value: &Value) {
    if json_mode {
        render_json(value);
    } else {
        render_text(value);
    }
}

fn render_json(value: &Value) {
    let mut printable = value.clone();
    if let Some(obj) = printable.as_object_mut() {
        obj.remove("_meta");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&printable).unwrap_or_else(|_| "{}".to_string())
    );
}

fn render_text(value: &Value) {
    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

    if !ok {
        if let Some(msg) = value.get("message").and_then(|v| v.as_str()) {
            eprintln!("Error: {msg}");
        }
        if let Some(case) = value.get("case") {
            render_case_header(case);
        }
        if let Some(dir) = value.get("direction") {
            render_direction(dir);
        }
        if let Some(steps) = value.get("steps") {
            render_steps(steps, false);
        }
        if let Some(unfinished) = value.get("unfinished_steps").and_then(|v| v.as_array()) {
            if !unfinished.is_empty() {
                println!("unfinished_steps:");
                for step in unfinished {
                    let order = step.get("order").and_then(|v| v.as_u64()).unwrap_or(0);
                    let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = step.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    let title = step.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                    println!("  {order}. {id}  [{status}]  {title}");
                }
            }
        }
        if let Some(cases) = value.get("cases").and_then(|v| v.as_array()) {
            render_case_list(cases, value.get("query").and_then(|v| v.as_str()));
        }
        if let Some(next) = value.get("next") {
            render_next_text(next);
        }
        return;
    }

    if let Some(state) = value.get("state").and_then(|v| v.as_str()) {
        if value.get("kind").and_then(|v| v.as_str()) == Some("case_current_state") {
            println!("{state}");
            return;
        }
    }

    // Case list (list / recall)
    if let Some(cases) = value.get("cases").and_then(|v| v.as_array()) {
        render_case_list(cases, value.get("query").and_then(|v| v.as_str()));
    }

    // Case info header
    if let Some(case) = value.get("case") {
        render_case_header(case);
    }

    if let Some(message) = value.get("message").and_then(|v| v.as_str()) {
        println!("message: {message}");
    }
    if let Some(spill) = value.get("spill") {
        render_spill_info(spill);
    }

    // Event receipt
    if let Some(event) = value.get("event") {
        render_event(event);
    }

    // Direction
    if let Some(dir) = value.get("direction") {
        render_direction(dir);
    }

    // Direction history + steps (show command) — unified tree
    let rendered_direction_tree = if let (Some(history), Some(sbd)) = (
        value.get("direction_history").and_then(|v| v.as_array()),
        value.get("steps_by_direction"),
    ) {
        render_direction_tree(history, sbd);
        true
    } else {
        // Standalone direction history (non-show contexts)
        if let Some(history) = value.get("direction_history") {
            render_direction_history(history);
        }
        // Standalone steps (non-show contexts)
        if let Some(sbd) = value.get("steps_by_direction") {
            render_steps_by_direction(sbd);
        }
        false
    };

    // Steps
    if let Some(steps) = value.get("steps") {
        render_steps(steps, rendered_direction_tree);
    }

    if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
        render_entries(entries);
    }

    // Step (single, for step add)
    if let Some(step) = value.get("step") {
        render_single_step(step);
    }

    // Resume
    if let Some(resume) = value.get("resume") {
        render_resume(resume);
    }

    if let Some(case_context) = value.get("case_context") {
        render_case_context(case_context);
    }

    // Last fact
    if let Some(fact) = value.get("last_fact").and_then(|v| v.as_str()) {
        println!("last_fact: {fact}");
    }

    // Health
    if let Some(health) = value.get("health").and_then(|v| v.as_str()) {
        println!("health: {health}");
    }
    if let Some(warning) = value.get("warning").and_then(|v| v.as_str()) {
        println!("warning: {warning}");
    }
    if let Some(warnings) = value.get("warnings").and_then(|v| v.as_array()) {
        for warning in warnings {
            if let Some(warning) = warning.as_str() {
                println!("warning: {warning}");
            }
        }
    }
    if let Some(statuses) = value
        .get("hooks")
        .and_then(|v| v.get("statuses"))
        .and_then(|v| v.as_array())
    {
        for status in statuses {
            let sink = status.get("sink").and_then(|v| v.as_str()).unwrap_or("?");
            let ok = status.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let message = status.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let label = if ok { "ok" } else { "failed" };
            if message.is_empty() {
                println!("hook {sink}: {label}");
            } else {
                println!("hook {sink}: {label} ({message})");
            }
        }
    }
    if let Some(reminder) = value.get("reminder").and_then(|v| v.as_str()) {
        println!("reminder: {reminder}");
    }

    // Next
    if let Some(next) = value.get("next") {
        render_next_text(next);
    }
}

fn render_case_list(cases: &[Value], query: Option<&str>) {
    if cases.is_empty() {
        if let Some(q) = query {
            println!("No cases matching \"{q}\".");
        } else {
            println!("No cases.");
        }
        return;
    }

    let grouped_cases = group_cases_by_status(cases);
    for (index, (label, cases_in_group)) in grouped_cases.iter().enumerate() {
        let case_count = cases_in_group.len();
        let mut group_tree = Tree::new(format!("{label} ({case_count}, newest first)"));
        for (case_index, case) in cases_in_group.iter().enumerate() {
            group_tree.push(render_case_list_item_tree(case, case_index + 1));
        }
        println!("{group_tree}");
        if index + 1 < grouped_cases.len() {
            println!();
        }
    }
}

fn render_case_list_item_tree(case: &Value, index: usize) -> Tree<String> {
    let id = case.get("id").and_then(|v| v.as_str()).unwrap_or("?");
    let goal = case.get("goal").and_then(|v| v.as_str()).unwrap_or("?");
    let updated_at = case
        .get("timestamps")
        .and_then(|timestamps| compact_timestamp_text(timestamps, "updated_at"))
        .unwrap_or_else(|| "?".to_string());

    let mut node = Tree::new(format!("{index}. {id}"));
    node.push(Tree::new(format!("updated_at: {updated_at}")));
    node.push(Tree::new(format!("goal: {goal}")));
    if let Some(timestamps) = case.get("timestamps") {
        if let Some(opened_at) = compact_timestamp_text(timestamps, "opened_at") {
            node.push(Tree::new(format!("opened_at: {opened_at}")));
        }
    }
    if let Some(matches) = case.get("matches").and_then(|v| v.as_array()) {
        for matched in matches.iter().take(3) {
            let scope = matched.get("scope").and_then(|v| v.as_str()).unwrap_or("?");
            let field = matched.get("field").and_then(|v| v.as_str()).unwrap_or("?");
            let excerpt = matched
                .get("excerpt")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            node.push(Tree::new(format!("match {scope}.{field}: {excerpt}")));
        }
    }
    node
}

fn group_cases_by_status(cases: &[Value]) -> Vec<(&'static str, Vec<&Value>)> {
    let mut open_cases = Vec::new();
    let mut closed_cases = Vec::new();
    let mut abandoned_cases = Vec::new();
    let mut other_cases = Vec::new();

    for case in cases {
        match case.get("status").and_then(|v| v.as_str()).unwrap_or("?") {
            "open" => open_cases.push(case),
            "closed" => closed_cases.push(case),
            "abandoned" => abandoned_cases.push(case),
            _ => other_cases.push(case),
        }
    }

    sort_case_values_by_recency(&mut open_cases);
    sort_case_values_by_recency(&mut closed_cases);
    sort_case_values_by_recency(&mut abandoned_cases);
    sort_case_values_by_recency(&mut other_cases);

    let mut groups = Vec::new();
    if !open_cases.is_empty() {
        groups.push(("open cases", open_cases));
    }
    if !closed_cases.is_empty() {
        groups.push(("closed cases", closed_cases));
    }
    if !abandoned_cases.is_empty() {
        groups.push(("abandoned cases", abandoned_cases));
    }
    if !other_cases.is_empty() {
        groups.push(("other cases", other_cases));
    }

    groups
}

fn sort_case_values_by_recency(cases: &mut Vec<&Value>) {
    cases.sort_by(|left, right| compare_case_value_recency(left, right));
}

fn compare_case_value_recency(left: &Value, right: &Value) -> std::cmp::Ordering {
    case_value_updated_at(right)
        .cmp(&case_value_updated_at(left))
        .then_with(|| case_value_id(right).cmp(case_value_id(left)))
}

fn case_value_updated_at(case: &Value) -> Option<DateTime<chrono::Utc>> {
    case.get("timestamps")
        .and_then(|timestamps| timestamp_utc_value(timestamps, "updated_at"))
        .or_else(|| {
            case.get("timestamps")
                .and_then(|timestamps| timestamp_local_value(timestamps, "updated_at"))
        })
        .and_then(parse_rfc3339_utc)
}

fn case_value_id(case: &Value) -> &str {
    case.get("id").and_then(|v| v.as_str()).unwrap_or("")
}

fn render_case_header(case: &Value) {
    let id = case.get("id").and_then(|v| v.as_str()).unwrap_or("?");
    let status = case.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    let goal = case.get("goal").and_then(|v| v.as_str()).unwrap_or("?");

    println!("case_id: {id}");
    println!("status: {status}");
    println!("goal: {goal}");

    if let Some(constraints) = case.get("goal_constraints").and_then(|v| v.as_array()) {
        if !constraints.is_empty() {
            println!("goal_constraints:");
            for c in constraints {
                if let Some(rule) = c.get("rule").and_then(|v| v.as_str()) {
                    println!("    - {rule}");
                    if let Some(reason) = c.get("reason").and_then(|v| v.as_str()) {
                        println!("      because: {reason}");
                    }
                }
            }
        }
    }

    if let Some(repo) = case.get("repo") {
        if let Some(label) = repo.get("label").and_then(|v| v.as_str()) {
            println!("repo: {label}");
        }
        if let Some(repo_id) = repo.get("id").and_then(|v| v.as_str()) {
            println!("repo_id: {repo_id}");
        }
    }

    if let Some(worktree) = case.get("worktree") {
        if let Some(worktree_id) = worktree.get("id").and_then(|v| v.as_str()) {
            println!("worktree_id: {worktree_id}");
        }
        if let Some(root) = worktree.get("root").and_then(|v| v.as_str()) {
            println!("worktree_root: {root}");
        }
    }

    if let Some(timestamps) = case.get("timestamps") {
        render_timestamp_line("opened_at", timestamps, "opened_at");
        render_timestamp_line("updated_at", timestamps, "updated_at");
        render_timestamp_line("closed_at", timestamps, "closed_at");
        render_timestamp_line("abandoned_at", timestamps, "abandoned_at");
    }
}

fn render_direction(dir: &Value) {
    if let Some(summary) = dir.get("summary").and_then(|v| v.as_str()) {
        println!("direction: {summary}");
    }

    if let Some(constraints) = dir.get("constraints").and_then(|v| v.as_array()) {
        if !constraints.is_empty() {
            println!("constraints:");
            for c in constraints {
                if let Some(rule) = c.get("rule").and_then(|v| v.as_str()) {
                    println!("    - {rule}");
                    if let Some(reason) = c.get("reason").and_then(|v| v.as_str()) {
                        println!("      because: {reason}");
                    }
                }
            }
        }
    }

    if let Some(sc) = dir.get("success_condition").and_then(|v| v.as_str()) {
        if !sc.is_empty() {
            println!("success_condition: {sc}");
        }
    }
    if let Some(ac) = dir.get("abort_condition").and_then(|v| v.as_str()) {
        if !ac.is_empty() {
            println!("abort_condition: {ac}");
        }
    }
}

fn render_direction_history(history: &Value) {
    if let Some(arr) = history.as_array() {
        println!("\n  direction_history:");
        for dir in arr {
            let seq = dir.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
            let summary = dir.get("summary").and_then(|v| v.as_str()).unwrap_or("?");
            println!("    [{seq}] {summary}");
        }
    }
}

fn render_steps(steps: &Value, current_only: bool) {
    if let Some(ordered) = steps.get("ordered").and_then(|v| v.as_array()) {
        if !ordered.is_empty() {
            if current_only {
                println!("current_steps:");
            } else {
                println!("steps:");
            }
            for step in ordered {
                let order = step.get("order").and_then(|v| v.as_u64()).unwrap_or(0);
                let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let status = step.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                let title = step.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {order}. {id}  [{status}]  {title}");
            }
            return;
        }
    }

    if let Some(current) = steps.get("current") {
        if !current.is_null() {
            let id = current.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let title = current.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            println!("current_step: {id} | {title}");
        }
    }
    if let Some(pending) = steps.get("pending").and_then(|v| v.as_array()) {
        if !pending.is_empty() {
            println!("pending_steps:");
            for s in pending {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  - {id} | {title}");
            }
        }
    }
}

fn render_direction_tree(history: &[Value], sbd: &Value) {
    let steps_map = sbd.as_object();
    let mut root = Tree::new("direction_tree:".to_string());

    for dir in history {
        let seq = dir.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
        let summary = dir.get("summary").and_then(|v| v.as_str()).unwrap_or("?");
        let is_current = dir
            .get("is_current")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let step_count = steps_map
            .and_then(|obj| obj.get(&seq.to_string()))
            .and_then(|v| v.as_array())
            .map_or(0, Vec::len);
        let label = if is_current {
            format!("[{seq}] {summary} (current) ({step_count} steps)")
        } else {
            format!("[{seq}] {summary} ({step_count} steps)")
        };
        let dir_node = Tree::new(label);

        root.push(dir_node);
    }

    println!("{root}");
}

fn render_steps_by_direction(sbd: &Value) {
    if let Some(obj) = sbd.as_object() {
        for (dir_seq, steps) in obj {
            println!("\n  steps (direction {dir_seq}):");
            if let Some(arr) = steps.as_array() {
                for s in arr {
                    let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = s.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    println!("    {id}  [{status}]  {title}");
                }
            }
        }
    }
}

fn render_single_step(step: &Value) {
    let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("?");
    let order = step.get("order").and_then(|v| v.as_u64()).unwrap_or(0);
    let title = step.get("title").and_then(|v| v.as_str()).unwrap_or("?");
    let status = step.get("status").and_then(|v| v.as_str()).unwrap_or("?");

    println!("Step added.");
    println!("\n  step_id:  {id}");
    println!("  order:    {order}");
    println!("  title:    {title}");
    println!("  status:   {status}");
}

fn render_entries(entries: &[Value]) {
    if entries.is_empty() {
        return;
    }

    println!("entries:");
    for entry in entries {
        let seq = entry.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
        let entry_type = entry
            .get("entry_type")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let kind = entry.get("kind").and_then(|v| v.as_str());
        let summary = entry.get("summary").and_then(|v| v.as_str()).unwrap_or("?");
        match kind {
            Some(kind) if !kind.is_empty() => println!("  {seq}. {entry_type}/{kind}: {summary}"),
            _ => println!("  {seq}. {entry_type}: {summary}"),
        }
        if let Some(context) = entry.get("context").and_then(|v| v.as_str()) {
            if !context.is_empty() {
                println!("     context: {context}");
            }
        }
    }
}

fn render_event(event: &Value) {
    let seq = event.get("seq").and_then(|v| v.as_u64());
    let entry_type = event
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let summary = event.get("summary").and_then(|v| v.as_str());

    match seq {
        Some(seq) => println!("\n  event #{seq} ({entry_type})"),
        None => println!("\n  event ({entry_type})"),
    }
    if let Some(s) = summary {
        println!("    {s}");
    }

    if matches!(entry_type, "redirect" | "redirect_recovered") {
        if let Some(from) = event.get("from_direction").and_then(|v| v.as_str()) {
            println!("\n  from:  {from}");
        }
        if let Some(to) = event.get("to_direction").and_then(|v| v.as_str()) {
            println!("  to:    {to}");
        }
    }
}

fn render_resume(resume: &Value) {
    if let Some(case_id) = resume.get("case_id").and_then(|v| v.as_str()) {
        println!("case_id: {case_id}");
    }
    if let Some(goal) = resume.get("goal").and_then(|v| v.as_str()) {
        println!("goal: {goal}");
    }

    if let Some(constraints) = resume.get("goal_constraints").and_then(|v| v.as_array()) {
        if !constraints.is_empty() {
            println!("goal_constraints:");
            for c in constraints {
                if let Some(rule) = c.get("rule").and_then(|v| v.as_str()) {
                    println!("    - {rule}");
                    if let Some(reason) = c.get("reason").and_then(|v| v.as_str()) {
                        println!("      because: {reason}");
                    }
                }
            }
        }
    }

    if let Some(dir) = resume.get("current_direction").and_then(|v| v.as_str()) {
        println!("direction: {dir}");
    }

    if let Some(constraints) = resume
        .get("direction_constraints")
        .and_then(|v| v.as_array())
    {
        if !constraints.is_empty() {
            println!("direction_constraints:");
            for c in constraints {
                if let Some(rule) = c.get("rule").and_then(|v| v.as_str()) {
                    println!("    - {rule}");
                    if let Some(reason) = c.get("reason").and_then(|v| v.as_str()) {
                        println!("      because: {reason}");
                    }
                }
            }
        }
    }

    if let Some(step) = resume.get("current_step") {
        if !step.is_null() {
            let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let title = step.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            println!("current_step: {id} | {title}");
        }
    }

    if let Some(pending) = resume.get("next_pending_steps").and_then(|v| v.as_array()) {
        if !pending.is_empty() {
            println!("next_pending_steps:");
            for s in pending {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  - {id} | {title}");
            }
        }
    }

    if let Some(d) = resume.get("last_decision").and_then(|v| v.as_str()) {
        println!("last_decision: {d}");
    }
    if let Some(e) = resume.get("last_evidence").and_then(|v| v.as_str()) {
        println!("last_evidence: {e}");
    }

    if let Some(sc) = resume.get("success_condition").and_then(|v| v.as_str()) {
        println!("success_condition: {sc}");
    }
    if let Some(ac) = resume.get("abort_condition").and_then(|v| v.as_str()) {
        println!("abort_condition: {ac}");
    }
}

fn render_next_text(next: &Value) {
    if let Some(cmd) = next.get("suggested_command").and_then(|v| v.as_str()) {
        let why = next.get("why").and_then(|v| v.as_str()).unwrap_or("");
        println!("\nnext: {cmd}");
        if !why.is_empty() {
            println!("why: {why}");
        }
    }
}

// ── JSON builders ──

pub fn error_json(error_code: &str, message: &str, next: Option<NextAction>) -> Value {
    let mut v = json!({
        "ok": false,
        "error": error_code,
        "message": message
    });
    if let Some(n) = next {
        v["next"] = json!({
            "suggested_command": n.suggested_command,
            "why": n.why
        });
    }
    v
}

pub fn case_json(case: &Case) -> Value {
    let mut value = json!({
        "id": case.id,
        "goal": case.goal,
        "goal_constraints": case.goal_constraints,
        "status": case.status.as_str(),
        "repo": {
            "id": case.repo_id,
            "label": case.repo_label
        },
        "worktree": {
            "id": case.worktree_id,
            "root": case.worktree_root
        }
    });
    let timestamps = timestamp_bundle(
        Some(&case.opened_at),
        Some(&case.updated_at),
        case.closed_at.as_deref(),
        case.abandoned_at.as_deref(),
    );
    if !timestamps.is_null() {
        value["timestamps"] = timestamps;
    }
    value
}

pub fn case_search_json(result: &CaseSearchResult) -> Value {
    let mut value = case_json(&result.case);
    value["matches"] = json!(result
        .matches
        .iter()
        .map(search_match_json)
        .collect::<Vec<_>>());
    value
}

pub fn case_context_json(result: &CaseContextResult) -> Value {
    json!({
        "backend": result.backend,
        "scope": result.scope,
        "case_id": result.case_id,
        "repo_id": result.repo_id,
        "query": result.query,
        "token_limit": result.token_limit,
        "generated_at": result.generated_at,
        "context": result.context,
        "hits": result.hits.iter().map(case_context_hit_json).collect::<Vec<_>>()
    })
}

pub fn entry_json(entry: &Entry) -> Value {
    json!({
        "case_id": entry.case_id,
        "seq": entry.seq,
        "entry_type": entry.entry_type.as_str(),
        "kind": entry.kind,
        "summary": entry.summary,
        "reason": entry.reason,
        "context": entry.context,
        "files": entry.files,
        "artifacts": entry.artifacts,
        "created_at": entry.created_at
    })
}

pub fn search_match_json(search_match: &SearchMatch) -> Value {
    json!({
        "scope": search_match.scope,
        "field": search_match.field,
        "excerpt": search_match.excerpt,
        "direction_seq": search_match.direction_seq,
        "entry_seq": search_match.entry_seq,
        "kind": search_match.kind
    })
}

pub fn case_context_hit_json(hit: &CaseContextHit) -> Value {
    json!({
        "case_id": hit.case_id,
        "source": hit.source,
        "field": hit.field,
        "excerpt": hit.excerpt,
        "score": hit.score,
        "direction_seq": hit.direction_seq,
        "entry_seq": hit.entry_seq,
        "step_id": hit.step_id,
        "kind": hit.kind
    })
}

pub fn direction_json(dir: &Direction) -> Value {
    let mut v = json!({
        "summary": dir.summary,
        "constraints": dir.constraints,
        "success_condition": dir.success_condition,
        "abort_condition": dir.abort_condition
    });
    if dir.seq > 0 {
        v["seq"] = json!(dir.seq);
    }
    v
}

pub fn steps_json(steps: &[Step]) -> Value {
    let mut ordered: Vec<&Step> = steps.iter().collect();
    ordered.sort_by_key(|step| step.order_index);

    let current = ordered
        .iter()
        .find(|step| step.status == StepStatus::Active)
        .copied();
    let pending: Vec<&Step> = ordered
        .iter()
        .copied()
        .filter(|step| step.status == StepStatus::Pending)
        .collect();

    json!({
        "ordered": ordered.iter().map(|step| step_json(step)).collect::<Vec<_>>(),
        "current": current.map(step_json),
        "pending": pending.iter().map(|step| step_json(step)).collect::<Vec<_>>()
    })
}

pub fn step_json(step: &Step) -> Value {
    let mut v = json!({
        "id": step.id,
        "order": step.order_index,
        "title": step.title,
        "status": step.status.as_str()
    });
    if let Some(ref r) = step.reason {
        v["reason"] = json!(r);
    }
    v
}

pub fn next_json(action: &NextAction) -> Value {
    json!({
        "suggested_command": action.suggested_command,
        "why": action.why
    })
}

pub fn context_json(case_id: &str, direction_seq: u32) -> Value {
    json!({
        "active_case_id": case_id,
        "current_direction_seq": direction_seq
    })
}

fn render_case_context(value: &Value) {
    if let Some(backend) = value.get("backend").and_then(|v| v.as_str()) {
        println!("context_backend: {backend}");
    }
    if let Some(scope) = value.get("scope").and_then(|v| v.as_str()) {
        println!("context_scope: {scope}");
    }
    if let Some(query) = value.get("query").and_then(|v| v.as_str()) {
        println!("context_query: {query}");
    }
    if let Some(context) = value.get("context").and_then(|v| v.as_str()) {
        println!("context:\n{context}");
    }
    if let Some(hits) = value.get("hits").and_then(|v| v.as_array()) {
        if !hits.is_empty() {
            println!("context_hits:");
            for hit in hits {
                let case_label = hit
                    .get("case_id")
                    .and_then(|v| v.as_str())
                    .map(|case_id| format!("case {case_id} "))
                    .unwrap_or_default();
                let source = hit.get("source").and_then(|v| v.as_str()).unwrap_or("?");
                let field = hit.get("field").and_then(|v| v.as_str()).unwrap_or("?");
                let excerpt = hit.get("excerpt").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  - {case_label}{source}.{field}: {excerpt}");
            }
        }
    }
}

fn render_timestamp_line(label: &str, timestamps: &Value, key: &str) {
    let local = timestamp_local_value(timestamps, key);
    let utc = timestamp_utc_value(timestamps, key);
    match (local, utc) {
        (Some(local), Some(utc)) => println!("{label}: {local} (utc: {utc})"),
        (Some(local), None) => println!("{label}: {local}"),
        (None, Some(utc)) => println!("{label}_utc: {utc}"),
        (None, None) => {}
    }
}

fn render_spill_info(value: &Value) {
    if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
        println!("spill_path: {path}");
    }
    if let Some(char_count) = value.get("char_count").and_then(|v| v.as_u64()) {
        println!("spill_char_count: {char_count}");
    }
    if let Some(line_count) = value.get("line_count").and_then(|v| v.as_u64()) {
        println!("spill_line_count: {line_count}");
    }
}

fn compact_timestamp_text(timestamps: &Value, key: &str) -> Option<String> {
    timestamp_local_value(timestamps, key)
        .map(ToOwned::to_owned)
        .or_else(|| timestamp_utc_value(timestamps, key).map(ToOwned::to_owned))
}

fn timestamp_local_value<'a>(timestamps: &'a Value, key: &str) -> Option<&'a str> {
    timestamps
        .get(format!("{key}_local"))
        .and_then(|value| value.as_str())
}

fn timestamp_utc_value<'a>(timestamps: &'a Value, key: &str) -> Option<&'a str> {
    timestamps
        .get(format!("{key}_utc"))
        .and_then(|value| value.as_str())
}

fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<chrono::Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn localize_timestamp(raw: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Local).to_rfc3339())
}

fn timestamp_bundle(
    opened_at: Option<&str>,
    updated_at: Option<&str>,
    closed_at: Option<&str>,
    abandoned_at: Option<&str>,
) -> Value {
    let mut timestamps = serde_json::Map::new();
    timestamps.insert(
        "storage_timezone".to_string(),
        json!("utc_with_local_rendering"),
    );
    insert_timestamp_pair(&mut timestamps, "opened_at", opened_at);
    insert_timestamp_pair(&mut timestamps, "updated_at", updated_at);
    insert_timestamp_pair(&mut timestamps, "closed_at", closed_at);
    insert_timestamp_pair(&mut timestamps, "abandoned_at", abandoned_at);
    Value::Object(timestamps)
}

fn insert_timestamp_pair(
    timestamps: &mut serde_json::Map<String, Value>,
    key: &str,
    raw: Option<&str>,
) {
    if let Some(raw) = raw {
        timestamps.insert(format!("{key}_utc"), json!(raw));
        if let Some(local) = localize_timestamp(raw) {
            timestamps.insert(format!("{key}_local"), json!(local));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str, order: u32, status: StepStatus, title: &str) -> Step {
        Step {
            id: id.to_string(),
            case_id: "case-1".to_string(),
            direction_seq: 1,
            order_index: order,
            title: title.to_string(),
            status,
            reason: None,
            created_at: "2026-03-21T00:00:00Z".to_string(),
            updated_at: "2026-03-21T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn steps_json_returns_ordered_full_state() {
        let steps = vec![
            make_step("step-2", 2, StepStatus::Pending, "Second"),
            make_step("step-3", 3, StepStatus::Done, "Third"),
            make_step("step-1", 1, StepStatus::Active, "First"),
        ];

        let result = steps_json(&steps);
        let ordered = result["ordered"].as_array().expect("ordered steps");
        let pending = result["pending"].as_array().expect("pending steps");

        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0]["id"], "step-1");
        assert_eq!(ordered[1]["id"], "step-2");
        assert_eq!(ordered[2]["id"], "step-3");
        assert_eq!(result["current"]["id"], "step-1");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["id"], "step-2");
    }

    #[test]
    fn case_json_includes_repo_worktree_and_localized_timestamps() {
        let case = Case {
            id: "C-550e8400-e29b-41d4-a716-446655440000".to_string(),
            repo_id: "aaaaaaaaaaaaaaaa".to_string(),
            repo_label: Some("github.com/example/agpod".to_string()),
            worktree_id: Some("1111111111111111".to_string()),
            worktree_root: Some("/tmp/agpod-worktree".to_string()),
            goal: "verify timeline rendering".to_string(),
            goal_constraints: vec![Constraint {
                rule: "show local time".to_string(),
                reason: Some("avoid agent confusion".to_string()),
            }],
            status: CaseStatus::Open,
            current_direction_seq: 1,
            current_step_id: None,
            opened_at: "2026-03-21T00:00:00Z".to_string(),
            updated_at: "2026-03-21T01:00:00Z".to_string(),
            closed_at: None,
            close_summary: None,
            abandoned_at: None,
            abandon_summary: None,
            close_confirm_token: None,
            close_confirm_action: None,
            close_confirm_summary: None,
        };

        let result = case_json(&case);

        assert_eq!(result["repo"]["id"], "aaaaaaaaaaaaaaaa");
        assert_eq!(result["worktree"]["id"], "1111111111111111");
        assert_eq!(
            result["timestamps"]["storage_timezone"],
            "utc_with_local_rendering"
        );
        assert_eq!(
            result["timestamps"]["opened_at_utc"],
            "2026-03-21T00:00:00Z"
        );
        assert!(result["timestamps"]["opened_at_local"].as_str().is_some());
    }

    #[test]
    fn compact_timestamp_text_prefers_local_time() {
        let timestamps = json!({
            "updated_at_local": "2026-03-21T09:00:00+08:00",
            "updated_at_utc": "2026-03-21T01:00:00Z"
        });

        let rendered =
            compact_timestamp_text(&timestamps, "updated_at").expect("timestamp should exist");

        assert_eq!(rendered, "2026-03-21T09:00:00+08:00");
    }

    #[test]
    fn group_cases_by_status_orders_sections_open_closed_abandoned() {
        let cases = vec![
            json!({"id":"C-3","status":"abandoned","goal":"third"}),
            json!({"id":"C-1","status":"closed","goal":"first"}),
            json!({"id":"C-2","status":"open","goal":"second"}),
            json!({"id":"C-4","status":"closed","goal":"fourth"}),
        ];

        let grouped = group_cases_by_status(&cases);

        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0].0, "open cases");
        assert_eq!(grouped[0].1[0]["id"], "C-2");
        assert_eq!(grouped[1].0, "closed cases");
        assert_eq!(grouped[1].1.len(), 2);
        assert_eq!(grouped[2].0, "abandoned cases");
        assert_eq!(grouped[2].1[0]["id"], "C-3");
    }

    #[test]
    fn group_cases_by_status_sorts_each_group_by_updated_at_desc() {
        let cases = vec![
            json!({
                "id":"C-older",
                "status":"closed",
                "goal":"older",
                "timestamps":{"updated_at_utc":"2026-03-21T01:00:00Z"}
            }),
            json!({
                "id":"C-newer",
                "status":"closed",
                "goal":"newer",
                "timestamps":{"updated_at_utc":"2026-03-23T01:00:00Z"}
            }),
        ];

        let grouped = group_cases_by_status(&cases);

        assert_eq!(grouped[0].0, "closed cases");
        assert_eq!(grouped[0].1[0]["id"], "C-newer");
        assert_eq!(grouped[0].1[1]["id"], "C-older");
    }

    #[test]
    fn render_case_list_item_tree_includes_goal_and_updated_at() {
        let case = json!({
            "id":"C-1",
            "status":"closed",
            "goal":"improve terminal readability",
            "timestamps":{
                "updated_at_local":"2026-03-23T15:30:00+08:00",
                "opened_at_local":"2026-03-23T14:00:00+08:00"
            }
        });

        let tree = render_case_list_item_tree(&case, 1).to_string();

        assert!(tree.contains("1. C-1"));
        assert!(tree.contains("updated_at: 2026-03-23T15:30:00+08:00"));
        assert!(tree.contains("goal: improve terminal readability"));
        assert!(tree.contains("opened_at: 2026-03-23T14:00:00+08:00"));
    }
}
