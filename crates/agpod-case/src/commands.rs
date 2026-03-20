//! Command dispatch and implementation.
//!
//! Keywords: commands, execute, dispatch, open, record, decide, redirect, close, step

use crate::cli::{CaseArgs, CaseCommand, StepCommand};
use crate::client::CaseClient;
use crate::config::DbConfig;
use crate::error::{CaseError, CaseResult};
use crate::output;
use crate::repo_id::RepoIdentity;
use crate::types::*;
use anyhow::Result;
use chrono::Utc;
use serde_json::json;

pub async fn execute(args: CaseArgs) -> Result<()> {
    let json_mode = args.json;
    let config = DbConfig::from_data_dir(args.data_dir.as_deref());

    let cwd = std::env::current_dir()?;
    let identity = RepoIdentity::resolve_from(&cwd)?;
    let client = CaseClient::new(&config, identity.repo_id).await?;

    let result = match args.command {
        CaseCommand::Open {
            goal,
            direction,
            goal_constraints,
            constraints,
            success_condition,
            abort_condition,
        } => {
            cmd_open(
                &client,
                &goal,
                &direction,
                &goal_constraints,
                &constraints,
                success_condition.as_deref(),
                abort_condition.as_deref(),
            )
            .await
        }
        CaseCommand::Current => cmd_current(&client).await,
        CaseCommand::Record {
            id,
            summary,
            kind,
            files,
            context,
        } => {
            let file_list: Vec<String> = files
                .map(|f| f.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();
            cmd_record(&client, &id, &summary, &kind, &file_list, context.as_deref()).await
        }
        CaseCommand::Decide {
            id,
            summary,
            reason,
        } => cmd_decide(&client, &id, &summary, &reason).await,
        CaseCommand::Redirect {
            id,
            direction,
            reason,
            context,
            constraints,
            success_condition,
            abort_condition,
        } => {
            cmd_redirect(
                &client,
                &id,
                &direction,
                &reason,
                &context,
                &constraints,
                &success_condition,
                &abort_condition,
            )
            .await
        }
        CaseCommand::Show { id } => cmd_show(&client, id.as_deref()).await,
        CaseCommand::Close { id, summary } => cmd_close(&client, &id, &summary).await,
        CaseCommand::Abandon { id, summary } => cmd_abandon(&client, &id, &summary).await,
        CaseCommand::Step { command } => cmd_step(&client, command).await,
        CaseCommand::Recall { query } => cmd_recall(&client, &query).await,
        CaseCommand::List => cmd_list(&client).await,
        CaseCommand::Resume { id } => cmd_resume(&client, id.as_deref()).await,
    };

    match result {
        Ok(value) => {
            output::render(json_mode, &value);
            Ok(())
        }
        Err(e) => {
            let err_value = output::error_json("error", &e.to_string(), None);
            output::render(json_mode, &err_value);
            std::process::exit(1);
        }
    }
}

fn parse_constraints(raw: &[String]) -> CaseResult<Vec<Constraint>> {
    raw.iter()
        .map(|s| {
            serde_json::from_str::<Constraint>(s)
                .map_err(|e| CaseError::InvalidConstraint(format!("{s}: {e}")))
        })
        .collect()
}

/// Generate case ID: C-YYYYMMDD-NN
async fn generate_case_id(client: &CaseClient) -> CaseResult<String> {
    let today = Utc::now().format("%Y%m%d").to_string();
    let count = client.count_cases_today().await.unwrap_or(0);
    let seq = count + 1;
    Ok(format!("C-{today}-{seq:02}"))
}

/// Generate step ID: {case_id}/S-NNN (case-scoped, globally unique)
async fn generate_step_id(client: &CaseClient, case_id: &str) -> CaseResult<String> {
    let count = client.get_step_count(case_id).await.unwrap_or(0);
    let seq = count + 1;
    Ok(format!("{case_id}/S-{seq:03}"))
}

/// Get next entry seq for a case.
async fn next_entry_seq(client: &CaseClient, case_id: &str) -> CaseResult<u32> {
    let count = client.get_entry_count(case_id).await.unwrap_or(0);
    Ok(count + 1)
}

/// Resolve a case ID: use given ID or find the open case.
async fn resolve_case(client: &CaseClient, id: Option<&str>) -> CaseResult<Case> {
    match id {
        Some(id) => client.get_case(id).await,
        None => client
            .find_open_case()
            .await?
            .ok_or(CaseError::NoOpenCase),
    }
}

/// Ensure the case is open.
fn ensure_open(case: &Case) -> CaseResult<()> {
    if case.status != CaseStatus::Open {
        return Err(CaseError::CaseNotOpen(case.id.clone()));
    }
    Ok(())
}

// ── Command implementations ──

async fn cmd_open(
    client: &CaseClient,
    goal: &str,
    direction: &str,
    goal_constraint_strs: &[String],
    constraint_strs: &[String],
    success_condition: Option<&str>,
    abort_condition: Option<&str>,
) -> CaseResult<serde_json::Value> {
    // Check no open case exists
    if let Some(existing) = client.find_open_case().await? {
        return Err(CaseError::RepoHasOpenCase(existing.id));
    }

    let goal_constraints = parse_constraints(goal_constraint_strs)?;
    let direction_constraints = parse_constraints(constraint_strs)?;

    let case_id = generate_case_id(client).await?;
    let case = client
        .create_case(&case_id, goal, &goal_constraints)
        .await?;

    let dir = client
        .create_direction(
            &case_id,
            1,
            direction,
            &direction_constraints,
            success_condition.unwrap_or(""),
            abort_condition.unwrap_or(""),
            None,
            None,
        )
        .await?;

    let next = NextAction {
        suggested_command: "step add".to_string(),
        why: "the case is open but the execution queue is still empty".to_string(),
    };

    Ok(json!({
        "ok": true,
        "case": output::case_json(&case),
        "direction": output::direction_json(&dir),
        "steps": output::steps_json(None, &[]),
        "context": output::context_json(&case_id, 1),
        "next": output::next_json(&next)
    }))
}

async fn cmd_current(client: &CaseClient) -> CaseResult<serde_json::Value> {
    let case = client
        .find_open_case()
        .await?
        .ok_or(CaseError::NoOpenCase)?;

    let dir = client
        .get_current_direction(&case.id, case.current_direction_seq)
        .await?;

    let steps = client
        .get_steps(&case.id, case.current_direction_seq)
        .await?;

    let (current_step, pending_steps) = split_steps(&steps);

    let last_entry = client.get_latest_entry(&case.id).await?;
    let last_fact = last_entry.as_ref().map(|e| e.summary.as_str());

    // Health detection
    let health = detect_health(&steps, &last_entry);

    let mut result = json!({
        "ok": true,
        "case": output::case_json(&case),
        "direction": output::direction_json(&dir),
        "steps": output::steps_json(current_step.as_ref(), &pending_steps),
        "context": output::context_json(&case.id, case.current_direction_seq)
    });

    if let Some(fact) = last_fact {
        result["last_fact"] = json!(fact);
    }
    result["health"] = json!(health.0.as_str());
    if let Some(warning) = health.1 {
        result["warning"] = json!(warning);
    }

    // Suggest next action
    let next = suggest_next(&case, current_step.as_ref(), &pending_steps, &health.0);
    result["next"] = output::next_json(&next);

    Ok(result)
}

async fn cmd_record(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
    kind: &str,
    files: &[String],
    context: Option<&str>,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    RecordKind::from_str(kind)
        .ok_or_else(|| CaseError::Other(format!("invalid record kind: {kind}")))?;

    let seq = next_entry_seq(client, case_id).await?;
    let entry = client
        .create_entry(
            case_id,
            seq,
            EntryType::Record,
            Some(kind),
            summary,
            None,
            context,
            files,
            &[],
        )
        .await?;

    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let (current_step, _) = split_steps(&steps);

    let next = NextAction {
        suggested_command: "record".to_string(),
        why: "the scan step is still gathering evidence".to_string(),
    };

    Ok(json!({
        "ok": true,
        "event": {
            "seq": entry.seq,
            "entry_type": "record",
            "kind": kind,
            "summary": summary,
            "files": files
        },
        "steps": {
            "current": current_step.map(|s| output::step_json(&s))
        },
        "next": output::next_json(&next)
    }))
}

async fn cmd_decide(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
    reason: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    let seq = next_entry_seq(client, case_id).await?;
    let entry = client
        .create_entry(
            case_id,
            seq,
            EntryType::Decision,
            None,
            summary,
            Some(reason),
            None,
            &[],
            &[],
        )
        .await?;

    let next = NextAction {
        suggested_command: "step done".to_string(),
        why: "the current decision narrows the step queue rather than changing direction"
            .to_string(),
    };

    Ok(json!({
        "ok": true,
        "event": {
            "seq": entry.seq,
            "entry_type": "decision",
            "summary": summary,
            "reason": reason
        },
        "next": output::next_json(&next)
    }))
}

async fn cmd_redirect(
    client: &CaseClient,
    case_id: &str,
    direction: &str,
    reason: &str,
    context: &str,
    constraint_strs: &[String],
    success_condition: &str,
    abort_condition: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    if success_condition.is_empty() || abort_condition.is_empty() {
        return Err(CaseError::MissingDirectionExitConditions);
    }

    let constraints = parse_constraints(constraint_strs)?;

    // Get previous direction for from_direction
    let prev_dir = client
        .get_current_direction(case_id, case.current_direction_seq)
        .await?;

    let new_seq = case.current_direction_seq + 1;

    // Create redirect entry
    let entry_seq = next_entry_seq(client, case_id).await?;
    let _entry = client
        .create_entry(
            case_id,
            entry_seq,
            EntryType::Redirect,
            None,
            direction,
            Some(reason),
            Some(context),
            &[],
            &[],
        )
        .await?;

    // Create new direction
    let new_dir = client
        .create_direction(
            case_id,
            new_seq,
            direction,
            &constraints,
            success_condition,
            abort_condition,
            Some(reason),
            Some(context),
        )
        .await?;

    // Update case
    client.update_case_direction(case_id, new_seq).await?;

    let next = NextAction {
        suggested_command: "step add".to_string(),
        why: "the new direction needs a fresh execution queue".to_string(),
    };

    Ok(json!({
        "ok": true,
        "event": {
            "seq": entry_seq,
            "entry_type": "redirect",
            "from_direction": prev_dir.summary,
            "to_direction": direction,
            "reason": reason,
            "context": context
        },
        "direction": output::direction_json(&new_dir),
        "steps": output::steps_json(None, &[]),
        "context": output::context_json(case_id, new_seq),
        "next": output::next_json(&next)
    }))
}

async fn cmd_show(
    client: &CaseClient,
    id: Option<&str>,
) -> CaseResult<serde_json::Value> {
    let case = resolve_case(client, id).await?;
    let directions = client.get_directions(&case.id).await?;
    let all_steps = client.get_all_steps(&case.id).await?;

    // Group steps by direction_seq
    let mut steps_by_dir: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for dir in &directions {
        let dir_steps: Vec<_> = all_steps
            .iter()
            .filter(|s| s.direction_seq == dir.seq)
            .map(|s| output::step_json(s))
            .collect();
        if !dir_steps.is_empty() {
            steps_by_dir.insert(dir.seq.to_string(), json!(dir_steps));
        }
    }

    let dir_history: Vec<_> = directions.iter().map(|d| output::direction_json(d)).collect();

    Ok(json!({
        "ok": true,
        "case": output::case_json(&case),
        "direction_history": dir_history,
        "steps_by_direction": steps_by_dir
    }))
}

async fn cmd_close(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    client
        .update_case_status(case_id, CaseStatus::Closed, summary)
        .await?;

    let next = NextAction {
        suggested_command: "open".to_string(),
        why: "the repository now has no active case".to_string(),
    };

    Ok(json!({
        "ok": true,
        "case": {
            "id": case_id,
            "goal": case.goal,
            "status": "closed",
            "close_summary": summary
        },
        "next": output::next_json(&next)
    }))
}

async fn cmd_abandon(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    client
        .update_case_status(case_id, CaseStatus::Abandoned, summary)
        .await?;

    let next = NextAction {
        suggested_command: "open".to_string(),
        why: "the previous goal has been explicitly abandoned".to_string(),
    };

    Ok(json!({
        "ok": true,
        "case": {
            "id": case_id,
            "goal": case.goal,
            "status": "abandoned",
            "abandon_summary": summary
        },
        "next": output::next_json(&next)
    }))
}

async fn cmd_step(
    client: &CaseClient,
    command: StepCommand,
) -> CaseResult<serde_json::Value> {
    match command {
        StepCommand::Add { id, title, reason } => {
            cmd_step_add(client, &id, &title, reason.as_deref()).await
        }
        StepCommand::Start { id, step_id } => cmd_step_start(client, &id, &step_id).await,
        StepCommand::Done { id, step_id } => cmd_step_done(client, &id, &step_id).await,
        StepCommand::Move {
            id,
            step_id,
            before,
        } => cmd_step_move(client, &id, &step_id, &before).await,
        StepCommand::Block {
            id,
            step_id,
            reason,
        } => cmd_step_block(client, &id, &step_id, &reason).await,
    }
}

async fn cmd_step_add(
    client: &CaseClient,
    case_id: &str,
    title: &str,
    reason: Option<&str>,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let order = steps.len() as u32 + 1;
    let step_id = generate_step_id(client, case_id).await?;

    let step = client
        .create_step(
            &step_id,
            case_id,
            case.current_direction_seq,
            order,
            title,
            reason,
        )
        .await?;

    let next = NextAction {
        suggested_command: "step start".to_string(),
        why: "the step exists but is not active yet".to_string(),
    };

    Ok(json!({
        "ok": true,
        "step": {
            "id": step.id,
            "order": step.order_index,
            "title": step.title,
            "status": step.status.as_str()
        },
        "context": output::context_json(case_id, case.current_direction_seq),
        "next": output::next_json(&next)
    }))
}

async fn cmd_step_start(
    client: &CaseClient,
    case_id: &str,
    step_id: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    // Deactivate any existing active step to maintain "one active at a time" invariant
    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    for s in &steps {
        if s.status == StepStatus::Active && s.id != step_id {
            client
                .update_step(&s.id, StepStatus::Pending, None)
                .await?;
        }
    }

    client
        .update_step(step_id, StepStatus::Active, None)
        .await?;
    client.update_case_step(case_id, step_id).await?;

    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let (current_step, pending_steps) = split_steps(&steps);

    let next = NextAction {
        suggested_command: "record".to_string(),
        why: "capture findings as you execute the step".to_string(),
    };

    Ok(json!({
        "ok": true,
        "steps": output::steps_json(current_step.as_ref(), &pending_steps),
        "context": output::context_json(case_id, case.current_direction_seq),
        "next": output::next_json(&next)
    }))
}

async fn cmd_step_done(
    client: &CaseClient,
    case_id: &str,
    step_id: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    client
        .update_step(step_id, StepStatus::Done, None)
        .await?;

    // Clear current_step_id if it was the active one
    if case.current_step_id.as_deref() == Some(step_id) {
        client.update_case_step(case_id, "").await?;
    }

    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let (current_step, pending_steps) = split_steps(&steps);

    let next = if pending_steps.is_empty() {
        NextAction {
            suggested_command: "close".to_string(),
            why: "all steps are done; consider closing the case if the goal is met".to_string(),
        }
    } else {
        NextAction {
            suggested_command: "step start".to_string(),
            why: "advance to the next pending step".to_string(),
        }
    };

    Ok(json!({
        "ok": true,
        "steps": output::steps_json(current_step.as_ref(), &pending_steps),
        "context": output::context_json(case_id, case.current_direction_seq),
        "next": output::next_json(&next)
    }))
}

async fn cmd_step_move(
    client: &CaseClient,
    case_id: &str,
    step_id: &str,
    before_id: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    let mut steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;

    // Find indices
    let move_idx = steps
        .iter()
        .position(|s| s.id == step_id)
        .ok_or_else(|| CaseError::StepNotFound(step_id.to_string()))?;
    let before_idx = steps
        .iter()
        .position(|s| s.id == before_id)
        .ok_or_else(|| CaseError::StepNotFound(before_id.to_string()))?;

    // Reorder in memory
    let moved = steps.remove(move_idx);
    let insert_at = if move_idx < before_idx {
        before_idx - 1
    } else {
        before_idx
    };
    // Re-find before_id position after removal
    let insert_at = steps
        .iter()
        .position(|s| s.id == before_id)
        .unwrap_or(insert_at);
    steps.insert(insert_at, moved);

    // Update order_index for all steps
    for (i, step) in steps.iter().enumerate() {
        client
            .reorder_step(&step.id, (i + 1) as u32)
            .await?;
    }

    // Re-fetch to get updated data
    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let (current_step, pending_steps) = split_steps(&steps);

    let next = NextAction {
        suggested_command: "step start".to_string(),
        why: "the reordered blocker-fix step should now run first".to_string(),
    };

    Ok(json!({
        "ok": true,
        "steps": output::steps_json(current_step.as_ref(), &pending_steps),
        "next": output::next_json(&next)
    }))
}

async fn cmd_step_block(
    client: &CaseClient,
    case_id: &str,
    step_id: &str,
    reason: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    client
        .update_step(step_id, StepStatus::Blocked, Some(reason))
        .await?;

    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let (current_step, pending_steps) = split_steps(&steps);

    let next = NextAction {
        suggested_command: "step add".to_string(),
        why: "consider adding a step to resolve the blocker".to_string(),
    };

    Ok(json!({
        "ok": true,
        "steps": output::steps_json(current_step.as_ref(), &pending_steps),
        "context": output::context_json(case_id, case.current_direction_seq),
        "next": output::next_json(&next)
    }))
}

// TODO: recall currently lists all cases (no semantic search).
// Phase 4 will add vector search via CaseSearchIndex.
async fn cmd_recall(
    client: &CaseClient,
    query: &str,
) -> CaseResult<serde_json::Value> {
    let cases = client.search_cases(query).await?;

    let case_list: Vec<_> = cases.iter().map(|c| output::case_json(c)).collect();

    Ok(json!({
        "ok": true,
        "cases": case_list,
        "query": query
    }))
}

async fn cmd_list(client: &CaseClient) -> CaseResult<serde_json::Value> {
    let cases = client.list_cases().await?;

    let case_list: Vec<_> = cases.iter().map(|c| output::case_json(c)).collect();

    Ok(json!({
        "ok": true,
        "cases": case_list
    }))
}

async fn cmd_resume(
    client: &CaseClient,
    id: Option<&str>,
) -> CaseResult<serde_json::Value> {
    let case = resolve_case(client, id).await?;

    let dir = client
        .get_current_direction(&case.id, case.current_direction_seq)
        .await?;

    let steps = client
        .get_steps(&case.id, case.current_direction_seq)
        .await?;
    let (current_step, pending_steps) = split_steps(&steps);

    let entries = client.get_entries(&case.id).await?;
    let last_decision = entries
        .iter()
        .rev()
        .find(|e| e.entry_type == EntryType::Decision)
        .map(|e| e.summary.as_str());
    let last_evidence = entries
        .iter()
        .rev()
        .find(|e| {
            e.entry_type == EntryType::Record
                && e.kind.as_deref() == Some("evidence")
        })
        .map(|e| e.summary.as_str());

    let next = suggest_next(
        &case,
        current_step.as_ref(),
        &pending_steps,
        &Health::OnTrack,
    );

    let mut resume = json!({
        "case_id": case.id,
        "goal": case.goal,
        "goal_constraints": case.goal_constraints,
        "current_direction": dir.summary,
        "direction_constraints": dir.constraints,
        "current_step": current_step.as_ref().map(|s| json!({
            "id": s.id,
            "title": s.title
        })),
        "next_pending_steps": pending_steps.iter().map(|s| json!({
            "id": s.id,
            "title": s.title
        })).collect::<Vec<_>>(),
        "success_condition": dir.success_condition,
        "abort_condition": dir.abort_condition
    });

    if let Some(d) = last_decision {
        resume["last_decision"] = json!(d);
    }
    if let Some(e) = last_evidence {
        resume["last_evidence"] = json!(e);
    }

    Ok(json!({
        "ok": true,
        "resume": resume,
        "next": output::next_json(&next)
    }))
}

// ── Helpers ──

/// Split steps into current (active) and pending.
fn split_steps(steps: &[Step]) -> (Option<Step>, Vec<Step>) {
    let current = steps.iter().find(|s| s.status == StepStatus::Active).cloned();
    let pending: Vec<Step> = steps
        .iter()
        .filter(|s| s.status == StepStatus::Pending)
        .cloned()
        .collect();
    (current, pending)
}

/// Detect health status based on recent entries and steps.
fn detect_health(
    steps: &[Step],
    _last_entry: &Option<Entry>,
) -> (Health, Option<String>) {
    // Check for blocked steps
    if steps.iter().any(|s| s.status == StepStatus::Blocked) {
        return (
            Health::Blocked,
            Some("one or more steps are blocked".to_string()),
        );
    }

    // NOTE: Phase 4 will add looping detection (consecutive same-type records).
    // For now, default to on_track.
    (Health::OnTrack, None)
}

/// Suggest the next action based on current state.
fn suggest_next(
    _case: &Case,
    current_step: Option<&Step>,
    pending_steps: &[Step],
    health: &Health,
) -> NextAction {
    if *health == Health::Looping {
        return NextAction {
            suggested_command: "redirect".to_string(),
            why: "the current direction appears to have plateaued".to_string(),
        };
    }

    if current_step.is_some() {
        return NextAction {
            suggested_command: "record".to_string(),
            why: "the active step is collecting evidence".to_string(),
        };
    }

    if !pending_steps.is_empty() {
        return NextAction {
            suggested_command: "step start".to_string(),
            why: "there are pending steps waiting to be started".to_string(),
        };
    }

    NextAction {
        suggested_command: "step add".to_string(),
        why: "the direction is set but no execution step has been added yet".to_string(),
    }
}
