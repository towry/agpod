//! Output formatting for JSON and human-readable text.
//!
//! Keywords: output, json, text, render, format, display

use crate::types::*;
use serde_json::{json, Value};

/// Render the result either as JSON or human-readable text.
pub fn render(json_mode: bool, value: &Value) {
    if json_mode {
        render_json(value);
    } else {
        render_text(value);
    }
}

fn render_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
    );
}

fn render_text(value: &Value) {
    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

    if !ok {
        if let Some(msg) = value.get("message").and_then(|v| v.as_str()) {
            eprintln!("Error: {msg}");
        }
        if let Some(next) = value.get("next") {
            render_next_text(next);
        }
        return;
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

    // Direction history
    if let Some(history) = value.get("direction_history") {
        render_direction_history(history);
    }

    // Steps
    if let Some(steps) = value.get("steps") {
        render_steps(steps);
    }

    // Steps by direction (show command)
    if let Some(sbd) = value.get("steps_by_direction") {
        render_steps_by_direction(sbd);
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
        println!("\n  last_fact:");
        println!("    {fact}");
    }

    // Health
    if let Some(health) = value.get("health").and_then(|v| v.as_str()) {
        println!("\n  health: {health}");
    }
    if let Some(warning) = value.get("warning").and_then(|v| v.as_str()) {
        println!("  warning: {warning}");
    }

    // Next
    if let Some(next) = value.get("next") {
        render_next_text(next);
    }
}

fn render_case_header(case: &Value) {
    let id = case.get("id").and_then(|v| v.as_str()).unwrap_or("?");
    let status = case.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    let goal = case.get("goal").and_then(|v| v.as_str()).unwrap_or("?");

    println!("{id}  [{status}]");
    println!("\n  goal:  {goal}");

    if let Some(constraints) = case.get("goal_constraints").and_then(|v| v.as_array()) {
        if !constraints.is_empty() {
            println!("\n  goal_constraints:");
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
}

fn render_direction(dir: &Value) {
    if let Some(summary) = dir.get("summary").and_then(|v| v.as_str()) {
        println!("\n  current_direction:");
        println!("    {summary}");
    }

    if let Some(constraints) = dir.get("constraints").and_then(|v| v.as_array()) {
        if !constraints.is_empty() {
            println!("\n  direction_constraints:");
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
            println!("\n  success_condition:");
            println!("    {sc}");
        }
    }
    if let Some(ac) = dir.get("abort_condition").and_then(|v| v.as_str()) {
        if !ac.is_empty() {
            println!("\n  abort_condition:");
            println!("    {ac}");
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

fn render_steps(steps: &Value) {
    if let Some(current) = steps.get("current") {
        if !current.is_null() {
            let id = current.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let title = current.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            println!("\n  current_step:");
            println!("    {id}  {title}");
        }
    }
    if let Some(pending) = steps.get("pending").and_then(|v| v.as_array()) {
        if !pending.is_empty() {
            println!("\n  pending_steps:");
            for s in pending {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                println!("    - {id}  {title}");
            }
        }
    }
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
    println!("Resume brief:\n");

    if let Some(goal) = resume.get("goal").and_then(|v| v.as_str()) {
        println!("  goal:");
        println!("    {goal}");
    }

    if let Some(constraints) = resume.get("goal_constraints").and_then(|v| v.as_array()) {
        if !constraints.is_empty() {
            println!("\n  goal_constraints:");
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
        println!("\n  current_direction:");
        println!("    {dir}");
    }

    if let Some(constraints) = resume
        .get("direction_constraints")
        .and_then(|v| v.as_array())
    {
        if !constraints.is_empty() {
            println!("\n  direction_constraints:");
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
            println!("\n  current_step:");
            println!("    {id}  {title}");
        }
    }

    if let Some(pending) = resume.get("next_pending_steps").and_then(|v| v.as_array()) {
        if !pending.is_empty() {
            println!("\n  next_pending_steps:");
            for s in pending {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                println!("    - {id}  {title}");
            }
        }
    }

    if let Some(d) = resume.get("last_decision").and_then(|v| v.as_str()) {
        println!("\n  last_decision:");
        println!("    {d}");
    }
    if let Some(e) = resume.get("last_evidence").and_then(|v| v.as_str()) {
        println!("\n  last_evidence:");
        println!("    {e}");
    }

    if let Some(sc) = resume.get("success_condition").and_then(|v| v.as_str()) {
        println!("\n  success_condition:");
        println!("    {sc}");
    }
    if let Some(ac) = resume.get("abort_condition").and_then(|v| v.as_str()) {
        println!("\n  abort_condition:");
        println!("    {ac}");
    }
}

fn render_next_text(next: &Value) {
    if let Some(cmd) = next.get("suggested_command").and_then(|v| v.as_str()) {
        let why = next.get("why").and_then(|v| v.as_str()).unwrap_or("");
        println!("\nNext:");
        println!("  {cmd}  \u{2014} {why}");
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
    json!({
        "id": case.id,
        "goal": case.goal,
        "goal_constraints": case.goal_constraints,
        "status": case.status.as_str()
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

pub fn steps_json(current: Option<&Step>, pending: &[Step]) -> Value {
    json!({
        "current": current.map(|s| step_json(s)),
        "pending": pending.iter().map(step_json).collect::<Vec<_>>()
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
