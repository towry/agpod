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
        if let Some(cases) = value.get("cases").and_then(|v| v.as_array()) {
            render_case_list(cases, value.get("query").and_then(|v| v.as_str()));
        }
        if let Some(next) = value.get("next") {
            render_next_text(next);
        }
        return;
    }

    // Case list (list / recall)
    if let Some(cases) = value.get("cases").and_then(|v| v.as_array()) {
        render_case_list(cases, value.get("query").and_then(|v| v.as_str()));
    }

    // Case info header
    if let Some(case) = value.get("case") {
        render_case_header(case);
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

    // Step (single, for step add)
    if let Some(step) = value.get("step") {
        render_single_step(step);
    }

    // Resume
    if let Some(resume) = value.get("resume") {
        render_resume(resume);
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
    for case in cases {
        let id = case.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let status = case.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let goal = case.get("goal").and_then(|v| v.as_str()).unwrap_or("?");
        println!("{id}  [{status}]  {goal}");
    }
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

fn render_event(event: &Value) {
    let seq = event.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
    let entry_type = event
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let summary = event.get("summary").and_then(|v| v.as_str());

    println!("\n  event #{seq} ({entry_type})");
    if let Some(s) = summary {
        println!("    {s}");
    }

    if entry_type == "redirect" {
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

fn render_timestamp_line(label: &str, timestamps: &Value, key: &str) {
    let local_key = format!("{key}_local");
    let utc_key = format!("{key}_utc");
    let local = timestamps.get(&local_key).and_then(|v| v.as_str());
    let utc = timestamps.get(&utc_key).and_then(|v| v.as_str());
    match (local, utc) {
        (Some(local), Some(utc)) => println!("{label}: {local} (utc: {utc})"),
        (Some(local), None) => println!("{label}: {local}"),
        (None, Some(utc)) => println!("{label}_utc: {utc}"),
        (None, None) => {}
    }
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
}
