//! Command dispatch and implementation.
//!
//! Keywords: commands, execute, dispatch, open, record, decide, redirect, close, step

use crate::cli::{CaseArgs, CaseCommand, CaseStatusArg, ContextScopeArg, OpenModeArg, StepCommand};
use crate::client::CaseClient;
use crate::config::{CaseConfig, CaseOverrides};
use crate::context::{CaseContextProvider, LocalCaseContextProvider};
use crate::error::{CaseError, CaseResult};
use crate::events::{CaseDomainEvent, CaseEventEnvelope};
use crate::honcho::HonchoBackend;
use crate::hooks::{CaseDispatchReport, CaseEventDispatcher, CaseHookStatus};
use crate::output;
use crate::repo_id::RepoIdentity;
use crate::search::{CaseSearchBackend, ContextScope, LocalTextSearchBackend};
use crate::server_client::execute_via_server;
use crate::types::*;
use crate::GoalDriftFlag;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use tracing::{debug, warn};
use uuid::Uuid;

async fn dispatch_event(client: &CaseClient, event: CaseDomainEvent) -> CaseDispatchReport {
    let (dispatcher, mut report) = build_dispatcher(client);
    let envelope = CaseEventEnvelope::new(client, event);
    let mut dispatched = dispatcher.dispatch(&envelope).await;
    report.statuses.append(&mut dispatched.statuses);
    report
}

fn append_dispatch_report(value: &mut serde_json::Value, report: &CaseDispatchReport) {
    if report.is_empty() {
        return;
    }

    value["hooks"] = json!(report);
    if report.has_failures() {
        value["warnings"] = json!(report.warnings());
    }
}

fn build_dispatcher(client: &CaseClient) -> (CaseEventDispatcher, CaseDispatchReport) {
    let mut sinks: Vec<std::sync::Arc<dyn crate::hooks::CaseEventSink>> = Vec::new();
    let mut report = CaseDispatchReport::default();
    if client.config().honcho_enabled && client.config().honcho_sync_enabled {
        match HonchoBackend::from_config(client.config()) {
            Ok(Some(honcho)) => {
                debug!("honcho sink enabled for case event dispatch");
                sinks.push(std::sync::Arc::new(honcho));
            }
            Ok(None) => {}
            Err(error) => {
                warn!(error = %error, "failed to initialize honcho sink");
                report.statuses.push(CaseHookStatus {
                    sink: "honcho".to_string(),
                    ok: false,
                    message: Some(error.to_string()),
                });
            }
        }
    }
    (CaseEventDispatcher::new(sinks), report)
}

fn context_provider_for_client(client: &CaseClient) -> CaseResult<Box<dyn CaseContextProvider>> {
    if client.config().honcho_enabled && client.config().semantic_recall_enabled {
        if let Some(honcho) = HonchoBackend::from_config(client.config())? {
            debug!("using honcho context provider");
            return Ok(Box::new(honcho));
        }
    }
    debug!("using local case context provider");
    Ok(Box::new(LocalCaseContextProvider::new(client.clone())))
}

pub async fn execute(args: CaseArgs) -> Result<()> {
    let value = execute_json(args).await;
    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let json_mode = value
        .get("_meta")
        .and_then(|meta| meta.get("json_mode"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    output::render(json_mode, &value);
    if ok {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

pub async fn execute_json(args: CaseArgs) -> serde_json::Value {
    let json_mode = args.json;
    debug!(
        json_mode,
        has_repo_root = args.repo_root.is_some(),
        has_data_dir = args.data_dir.is_some(),
        has_server_addr = args.server_addr.is_some(),
        "starting case command"
    );
    let setup = setup_runtime(
        args.data_dir.as_deref(),
        args.server_addr.as_deref(),
        args.repo_root.as_deref(),
    )
    .await;

    let (config, identity) = match setup {
        Ok(runtime) => runtime,
        Err(e) => {
            warn!(error = %e, "case runtime setup failed");
            let mut err_value = output::error_json("error", &e.to_string(), None);
            err_value["_meta"] = json!({ "json_mode": json_mode });
            return err_value;
        }
    };

    match execute_via_server(&config, identity, args.command.clone()).await {
        Ok(mut value) => {
            debug!("case command completed via server");
            value["_meta"] = json!({ "json_mode": json_mode });
            value
        }
        Err(e) => {
            warn!(error = %e, "case command failed via server");
            let mut err_value = output::error_json("error", &e.to_string(), None);
            err_value["_meta"] = json!({ "json_mode": json_mode });
            err_value
        }
    }
}

pub async fn execute_json_batch(
    data_dir: Option<&str>,
    server_addr: Option<&str>,
    repo_root: Option<&str>,
    commands: Vec<CaseCommand>,
) -> Vec<serde_json::Value> {
    let setup = setup_runtime(data_dir, server_addr, repo_root).await;

    let (config, identity) = match setup {
        Ok(runtime) => runtime,
        Err(e) => {
            let mut err_value = output::error_json("error", &e.to_string(), None);
            err_value["_meta"] = json!({ "json_mode": true });
            return vec![err_value];
        }
    };

    let mut values = Vec::with_capacity(commands.len());
    for command in commands {
        match execute_via_server(&config, identity.clone(), command).await {
            Ok(mut value) => {
                value["_meta"] = json!({ "json_mode": true });
                let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                values.push(value);
                if !ok {
                    break;
                }
            }
            Err(e) => {
                let mut err_value = output::error_json("error", &e.to_string(), None);
                err_value["_meta"] = json!({ "json_mode": true });
                values.push(err_value);
                break;
            }
        }
    }

    values
}

#[cfg(test)]
async fn execute_json_batch_with_client(
    client: &CaseClient,
    commands: Vec<CaseCommand>,
    json_mode: bool,
) -> Vec<serde_json::Value> {
    let mut values = Vec::with_capacity(commands.len());
    for command in commands {
        let value = finish_json_value(
            execute_command_json(client, &command).await,
            client,
            &command,
            json_mode,
        )
        .await;
        let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        values.push(value);
        if !ok {
            break;
        }
    }

    values
}

async fn setup_runtime(
    data_dir: Option<&str>,
    server_addr: Option<&str>,
    repo_root: Option<&str>,
) -> Result<(CaseConfig, RepoIdentity)> {
    let config = CaseConfig::load(CaseOverrides {
        data_dir,
        server_addr,
    });
    let root = match repo_root {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_dir()?,
    };
    let identity = RepoIdentity::resolve_from(&root)?;
    debug!(
        repo_id = %identity.repo_id,
        repo_label = %identity.repo_label,
        server_addr = %config.server_addr,
        data_dir = %config.data_dir.to_string_lossy(),
        honcho_enabled = config.honcho_enabled,
        semantic_recall_enabled = config.semantic_recall_enabled,
        "resolved case runtime"
    );
    Ok((config, identity))
}

pub(crate) async fn finish_json_value(
    result: CaseResult<serde_json::Value>,
    client: &CaseClient,
    command: &CaseCommand,
    json_mode: bool,
) -> serde_json::Value {
    match result {
        Ok(mut value) => {
            value["_meta"] = json!({ "json_mode": json_mode });
            value
        }
        Err(e) => {
            let mut err_value = build_error_value(client, command, &e).await;
            err_value["_meta"] = json!({ "json_mode": json_mode });
            err_value
        }
    }
}

pub(crate) async fn execute_command_json(
    client: &CaseClient,
    command: &CaseCommand,
) -> CaseResult<serde_json::Value> {
    match command {
        CaseCommand::Open {
            mode,
            case_id,
            goal,
            direction,
            goal_constraints,
            constraints,
            success_condition,
            abort_condition,
        } => {
            cmd_open(
                client,
                OpenRequest {
                    mode: *mode,
                    reopen_case_id: case_id.as_deref(),
                    goal: goal.as_deref(),
                    direction: direction.as_deref(),
                    goal_constraint_strs: goal_constraints,
                    constraint_strs: constraints,
                    success_condition: success_condition.as_deref(),
                    abort_condition: abort_condition.as_deref(),
                },
            )
            .await
        }
        CaseCommand::Current { state } => cmd_current(client, *state).await,
        CaseCommand::Record {
            id,
            summary,
            kind,
            goal_constraints,
            files,
            context,
        } => {
            let file_list: Vec<String> = files
                .as_ref()
                .map(|f| f.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();
            cmd_record(
                client,
                id,
                summary,
                kind,
                goal_constraints,
                &file_list,
                context.as_deref(),
            )
            .await
        }
        CaseCommand::Decide {
            id,
            summary,
            reason,
        } => cmd_decide(client, id, summary, reason).await,
        CaseCommand::Redirect {
            id,
            direction,
            reason,
            context,
            is_drift_from_goal,
            constraints,
            success_condition,
            abort_condition,
        } => {
            cmd_redirect(
                client,
                id,
                direction,
                reason,
                context,
                *is_drift_from_goal,
                constraints,
                success_condition,
                abort_condition,
            )
            .await
        }
        CaseCommand::Show { id } => cmd_show(client, id.as_deref()).await,
        CaseCommand::Close {
            id,
            summary,
            confirm_token,
        } => cmd_close(client, id, summary, confirm_token.as_deref()).await,
        CaseCommand::Abandon {
            id,
            summary,
            confirm_token,
        } => cmd_abandon(client, id, summary, confirm_token.as_deref()).await,
        CaseCommand::Step { command } => cmd_step(client, command).await,
        CaseCommand::Recall {
            query,
            status,
            limit,
            recent_days,
        } => {
            cmd_recall(
                client,
                query,
                CaseListOptions::new(*status, *limit, *recent_days),
            )
            .await
        }
        CaseCommand::Context {
            id,
            scope,
            query,
            limit,
            token_limit,
        } => {
            cmd_context(
                client,
                id.as_deref(),
                *scope,
                query.as_deref(),
                *limit,
                *token_limit,
            )
            .await
        }
        CaseCommand::List {
            status,
            limit,
            recent_days,
        } => cmd_list(client, CaseListOptions::new(*status, *limit, *recent_days)).await,
        CaseCommand::Resume { id } => cmd_resume(client, id.as_deref()).await,
    }
}

async fn build_error_value(
    client: &CaseClient,
    command: &CaseCommand,
    error: &CaseError,
) -> serde_json::Value {
    let mut err_value = output::error_json("error", &error.to_string(), error_next_action(error));
    err_value["state"] = json!(error_state(error));

    if let Some(case_id) = command_case_id(command) {
        err_value["requested_case_id"] = json!(case_id);
    }
    if let Some(step_id) = command_step_id(command) {
        err_value["requested_step_id"] = json!(step_id);
    }
    if let Some(before_id) = command_before_step_id(command) {
        err_value["requested_before_step_id"] = json!(before_id);
    }

    match error {
        CaseError::CloseConfirmationRequired {
            case_id,
            action,
            summary,
            confirm_token,
        } => {
            err_value["confirmation"] = json!({
                "required": true,
                "case_id": case_id,
                "action": action,
                "summary": summary,
                "confirm_token": confirm_token,
                "message": format!(
                    "Case closure is destructive. Re-run `{action}` with the same summary and `confirm_token` only if you intend to end this case."
                )
            });
            err_value["message"] = json!(format!(
                "confirmation required before {action}; retry with confirm_token if ending this case is intentional"
            ));
        }
        CaseError::InvalidCloseConfirmationToken { case_id, action } => {
            err_value["confirmation"] = json!({
                "required": true,
                "case_id": case_id,
                "action": action,
                "message": "confirm_token was missing, stale, or did not match the requested action and summary"
            });
        }
        _ => {}
    }

    if let Ok(mut cases) = client.list_cases().await {
        cases.sort_by(compare_case_recency);
        if !cases.is_empty() {
            err_value["cases"] = json!(cases.iter().map(output::case_json).collect::<Vec<_>>());
        }
    }

    if let Some(case) = load_context_case(client, command, error).await {
        err_value["case"] = output::case_json(&case);
        err_value["context"] = output::context_json(&case.id, case.current_direction_seq);

        if let Ok(direction) = client
            .get_current_direction(&case.id, case.current_direction_seq)
            .await
        {
            err_value["direction"] = output::direction_json(&direction);
        }

        if let Ok(steps) = client.get_steps(&case.id, case.current_direction_seq).await {
            err_value["steps"] = output::steps_json(&steps);
            if matches!(error, CaseError::UnfinishedSteps) {
                err_value["unfinished_steps"] = json!(steps
                    .iter()
                    .filter(|step| !matches!(step.status, StepStatus::Done))
                    .map(output::step_json)
                    .collect::<Vec<_>>());
            }
        }
    }

    err_value
}

fn error_state(error: &CaseError) -> &'static str {
    match error {
        CaseError::RepoHasOpenCase(_) => "conflict",
        CaseError::GoalDriftRequiresNewCase => "goal_drift",
        CaseError::UnfinishedSteps => "unfinished_steps",
        CaseError::CloseConfirmationRequired { .. } => "confirmation_required",
        CaseError::InvalidCloseConfirmationToken { .. } => "invalid_confirmation",
        CaseError::NoOpenCase => "none",
        CaseError::CaseNotFound(_) | CaseError::StepNotFound(_) => "missing",
        CaseError::CaseNotOpen(_) => "not_open",
        _ => "error",
    }
}

fn error_next_action(error: &CaseError) -> Option<NextAction> {
    match error {
        CaseError::RepoHasOpenCase(_) => Some(NextAction {
            suggested_command: "resume".to_string(),
            why: "an open case already exists for this repository".to_string(),
        }),
        CaseError::GoalDriftRequiresNewCase => Some(NextAction {
            suggested_command: "open".to_string(),
            why: "goal drift means this work now belongs in a new case, not a redirect".to_string(),
        }),
        CaseError::NoOpenCase => Some(NextAction {
            suggested_command: "open".to_string(),
            why: "there is no active case; first decide whether this task actually needs case tracking, then open one only if warranted".to_string(),
        }),
        CaseError::CaseNotFound(_) => Some(NextAction {
            suggested_command: "list".to_string(),
            why: "inspect available case IDs before retrying".to_string(),
        }),
        CaseError::UnfinishedSteps => Some(NextAction {
            suggested_command: "step done".to_string(),
            why: "review unfinished steps, then mark them done or blocked before closing the case"
                .to_string(),
        }),
        CaseError::CloseConfirmationRequired { action, .. } => Some(NextAction {
            suggested_command: action.clone(),
            why: "retry with the returned confirm_token only if closing this case is truly intended"
                .to_string(),
        }),
        CaseError::InvalidCloseConfirmationToken { action, .. } => Some(NextAction {
            suggested_command: action.clone(),
            why: "request a fresh confirm_token from a new close/abandon attempt, then retry with that token"
                .to_string(),
        }),
        CaseError::InvalidRecordKind { kind, .. } if kind == "decision" => Some(NextAction {
            suggested_command: "decide".to_string(),
            why: "decisions belong in `case_decide`, which also requires a reason".to_string(),
        }),
        CaseError::StepNotFound(_) => Some(NextAction {
            suggested_command: "current".to_string(),
            why: "inspect the latest ordered steps before retrying".to_string(),
        }),
        _ => None,
    }
}

async fn load_context_case(
    client: &CaseClient,
    command: &CaseCommand,
    error: &CaseError,
) -> Option<Case> {
    if let Some(case_id) = command_case_id(command) {
        if let Ok(case) = client.get_case(case_id).await {
            return Some(case);
        }
    }

    if matches!(error, CaseError::RepoHasOpenCase(_) | CaseError::NoOpenCase)
        || matches!(
            command,
            CaseCommand::Open { .. }
                | CaseCommand::Current { .. }
                | CaseCommand::Show { id: None }
                | CaseCommand::Resume { id: None }
        )
    {
        return client.find_open_case().await.ok().flatten();
    }

    None
}

fn command_case_id(command: &CaseCommand) -> Option<&str> {
    match command {
        CaseCommand::Record { id, .. }
        | CaseCommand::Decide { id, .. }
        | CaseCommand::Redirect { id, .. }
        | CaseCommand::Close { id, .. }
        | CaseCommand::Abandon { id, .. } => Some(id.as_str()),
        CaseCommand::Show { id } | CaseCommand::Resume { id } => id.as_deref(),
        CaseCommand::Step { command } => match command {
            StepCommand::Add { id, .. }
            | StepCommand::Start { id, .. }
            | StepCommand::Done { id, .. }
            | StepCommand::Move { id, .. }
            | StepCommand::Block { id, .. } => Some(id.as_str()),
        },
        CaseCommand::Context { id, .. } => id.as_deref(),
        CaseCommand::Open { .. }
        | CaseCommand::Current { .. }
        | CaseCommand::Recall { .. }
        | CaseCommand::List { .. } => None,
    }
}

fn command_step_id(command: &CaseCommand) -> Option<&str> {
    match command {
        CaseCommand::Step { command } => match command {
            StepCommand::Start { step_id, .. }
            | StepCommand::Done { step_id, .. }
            | StepCommand::Move { step_id, .. }
            | StepCommand::Block { step_id, .. } => Some(step_id.as_str()),
            StepCommand::Add { .. } => None,
        },
        _ => None,
    }
}

fn command_before_step_id(command: &CaseCommand) -> Option<&str> {
    match command {
        CaseCommand::Step {
            command: StepCommand::Move { before, .. },
        } => Some(before.as_str()),
        _ => None,
    }
}

fn parse_constraints(raw: &[String]) -> CaseResult<Vec<Constraint>> {
    raw.iter().map(|s| parse_constraint(s)).collect()
}

fn parse_constraint(raw: &str) -> CaseResult<Constraint> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CaseError::InvalidConstraint(
            "constraint must not be empty".to_string(),
        ));
    }

    if let Ok(constraint) = serde_json::from_str::<Constraint>(trimmed) {
        return Ok(constraint);
    }

    if let Ok(rule) = serde_json::from_str::<String>(trimmed) {
        return Ok(Constraint { rule, reason: None });
    }

    Ok(Constraint {
        rule: trimmed.to_string(),
        reason: None,
    })
}

fn format_case_id(uuid: Uuid) -> String {
    format!("C-{uuid}")
}

/// Generate case ID: C-<uuid>
async fn generate_case_id(_client: &CaseClient) -> CaseResult<String> {
    Ok(format_case_id(Uuid::new_v4()))
}

/// Generate step ID: {case_id}/S-NNN (case-scoped, globally unique)
async fn generate_step_id(client: &CaseClient, case_id: &str) -> CaseResult<String> {
    let count = client.get_step_count(case_id).await?;
    let seq = count + 1;
    Ok(format!("{case_id}/S-{seq:03}"))
}

/// Get next entry seq for a case.
async fn next_entry_seq(client: &CaseClient, case_id: &str) -> CaseResult<u32> {
    let count = client.get_entry_count(case_id).await?;
    Ok(count + 1)
}

/// Resolve a case ID: use given ID or find the open case.
async fn resolve_case(client: &CaseClient, id: Option<&str>) -> CaseResult<Case> {
    match id {
        Some(id) => client.get_case(id).await,
        None => client.find_open_case().await?.ok_or(CaseError::NoOpenCase),
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

struct OpenRequest<'a> {
    mode: OpenModeArg,
    reopen_case_id: Option<&'a str>,
    goal: Option<&'a str>,
    direction: Option<&'a str>,
    goal_constraint_strs: &'a [String],
    constraint_strs: &'a [String],
    success_condition: Option<&'a str>,
    abort_condition: Option<&'a str>,
}

async fn cmd_open(client: &CaseClient, request: OpenRequest<'_>) -> CaseResult<serde_json::Value> {
    // Check no open case exists
    if let Some(existing) = client.find_open_case().await? {
        return Err(CaseError::RepoHasOpenCase(existing.id));
    }

    match request.mode {
        OpenModeArg::New => {
            let goal = request.goal.ok_or_else(|| {
                CaseError::InvalidOpenMode("`goal` is required when mode is `new`".to_string())
            })?;
            let direction = request.direction.ok_or_else(|| {
                CaseError::InvalidOpenMode("`direction` is required when mode is `new`".to_string())
            })?;
            let goal_constraints = parse_constraints(request.goal_constraint_strs)?;
            let direction_constraints = parse_constraints(request.constraint_strs)?;

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
                    request.success_condition.unwrap_or(""),
                    request.abort_condition.unwrap_or(""),
                    None,
                    None,
                )
                .await?;
            let dispatch = dispatch_event(
                client,
                CaseDomainEvent::CaseOpened {
                    case: case.clone(),
                    direction: dir.clone(),
                },
            )
            .await;

            let next = NextAction {
                suggested_command: "step add".to_string(),
                why: "the case is open but the execution queue is still empty".to_string(),
            };

            let mut value = json!({
                "ok": true,
                "case": output::case_json(&case),
                "direction": output::direction_json(&dir),
                "steps": output::steps_json(&[]),
                "context": output::context_json(&case_id, 1),
                "next": output::next_json(&next)
            });
            append_dispatch_report(&mut value, &dispatch);
            Ok(value)
        }
        OpenModeArg::Reopen => {
            let case_id = request.reopen_case_id.ok_or_else(|| {
                CaseError::InvalidOpenMode(
                    "`case_id` is required when mode is `reopen`".to_string(),
                )
            })?;
            if request.goal.is_some()
                || request.direction.is_some()
                || !request.goal_constraint_strs.is_empty()
                || !request.constraint_strs.is_empty()
                || request.success_condition.is_some()
                || request.abort_condition.is_some()
            {
                return Err(CaseError::InvalidOpenMode(
                    "`goal`, `direction`, constraints, and exit conditions are only allowed when mode is `new`"
                        .to_string(),
                ));
            }

            let case = client.get_case(case_id).await?;
            if case.status == CaseStatus::Open {
                return Err(CaseError::RepoHasOpenCase(case.id));
            }

            client.reopen_case(case_id).await?;
            let reopened = client.get_case(case_id).await?;
            let directions = client.get_directions(case_id).await?;
            let dir = directions
                .iter()
                .find(|direction| direction.seq == reopened.current_direction_seq)
                .cloned()
                .ok_or_else(|| CaseError::Other("no direction found".to_string()))?;
            let steps = client
                .get_steps(case_id, reopened.current_direction_seq)
                .await?;

            let next = suggest_next(
                &reopened,
                steps.iter().find(|step| step.status == StepStatus::Active),
                &steps
                    .iter()
                    .filter(|step| step.status == StepStatus::Pending)
                    .cloned()
                    .collect::<Vec<_>>(),
                &Health::OnTrack,
            );

            let next_entry_seq = client
                .get_latest_entry(case_id)
                .await?
                .map(|entry| entry.seq + 1)
                .unwrap_or(1);
            let reopened_entry = client
                .create_entry(
                    case_id,
                    next_entry_seq,
                    EntryType::Record,
                    Some("note"),
                    "reopened case",
                    None,
                    Some("case reopened via case_open mode=reopen"),
                    &[],
                    &[],
                )
                .await?;
            let dispatch = dispatch_event(
                client,
                CaseDomainEvent::CaseReopened {
                    case: reopened.clone(),
                    direction: dir.clone(),
                    reopened_entry: reopened_entry.clone(),
                },
            )
            .await;

            let mut value = json!({
                "ok": true,
                "case": output::case_json(&reopened),
                "direction": output::direction_json(&dir),
                "steps": output::steps_json(&steps),
                "context": output::context_json(case_id, reopened.current_direction_seq),
                "message": "case reopened",
                "next": output::next_json(&next)
            });
            append_dispatch_report(&mut value, &dispatch);
            Ok(value)
        }
    }
}

#[cfg(test)]
async fn cmd_open_new(
    client: &CaseClient,
    goal: &str,
    direction: &str,
    goal_constraint_strs: &[String],
    constraint_strs: &[String],
    success_condition: Option<&str>,
    abort_condition: Option<&str>,
) -> CaseResult<serde_json::Value> {
    cmd_open(
        client,
        OpenRequest {
            mode: OpenModeArg::New,
            reopen_case_id: None,
            goal: Some(goal),
            direction: Some(direction),
            goal_constraint_strs,
            constraint_strs,
            success_condition,
            abort_condition,
        },
    )
    .await
}

async fn cmd_current(client: &CaseClient, state_only: bool) -> CaseResult<serde_json::Value> {
    let case = client
        .find_open_case()
        .await?
        .ok_or(CaseError::NoOpenCase)?;

    if state_only {
        return Ok(json!({
            "ok": true,
            "kind": "case_current_state",
            "state": case.status.as_str(),
            "case_id": case.id,
        }));
    }

    let directions = client.get_directions(&case.id).await?;
    let all_steps = client.get_all_steps(&case.id).await?;
    let (dir_history, steps_by_dir) =
        build_direction_tree_payload(&directions, &all_steps, Some(case.current_direction_seq));

    let dir = directions
        .iter()
        .find(|direction| direction.seq == case.current_direction_seq)
        .cloned()
        .ok_or_else(|| CaseError::Other("no direction found".to_string()))?;

    let steps: Vec<_> = all_steps
        .iter()
        .filter(|step| step.direction_seq == case.current_direction_seq)
        .cloned()
        .collect();

    let (current_step, pending_steps) = split_steps(&steps);

    let last_entry = client.get_latest_entry(&case.id).await?;
    let last_fact = last_entry.as_ref().map(|e| e.summary.as_str());

    // Health detection
    let health = detect_health(&steps, &last_entry);

    let mut result = json!({
        "ok": true,
        "case": output::case_json(&case),
        "direction": output::direction_json(&dir),
        "direction_history": dir_history,
        "steps_by_direction": steps_by_dir,
        "steps": output::steps_json(&steps),
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
    goal_constraint_strs: &[String],
    files: &[String],
    context: Option<&str>,
) -> CaseResult<serde_json::Value> {
    let mut case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    if summary.trim().is_empty() {
        return Err(CaseError::Other("summary must not be empty".to_string()));
    }

    let record_kind = kind
        .parse::<RecordKind>()
        .map_err(|_| CaseError::invalid_record_kind(kind))?;
    let goal_constraints = parse_constraints(goal_constraint_strs)?;

    if record_kind == RecordKind::GoalConstraintUpdate && goal_constraints.is_empty() {
        return Err(CaseError::GoalConstraintUpdateRequiresConstraints);
    }
    if record_kind != RecordKind::GoalConstraintUpdate && !goal_constraints.is_empty() {
        return Err(CaseError::GoalConstraintsOnlyAllowedForGoalConstraintUpdate);
    }

    if record_kind == RecordKind::GoalConstraintUpdate {
        let mut merged = case.goal_constraints.clone();
        for constraint in goal_constraints.iter().cloned() {
            if !merged.contains(&constraint) {
                merged.push(constraint);
            }
        }
        client
            .update_case_goal_constraints(case_id, &merged)
            .await?;
        case.goal_constraints = merged;
    }

    let seq = next_entry_seq(client, case_id).await?;
    let artifacts = if record_kind == RecordKind::GoalConstraintUpdate {
        goal_constraints
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CaseError::Other(e.to_string()))?
    } else {
        vec![]
    };
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
            &artifacts,
        )
        .await?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::RecordAppended {
            case: case.clone(),
            entry: entry.clone(),
        },
    )
    .await;

    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let (current_step, _) = split_steps(&steps);

    let next = NextAction {
        suggested_command: "record".to_string(),
        why: "the scan step is still gathering evidence".to_string(),
    };

    let mut result = json!({
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
    });

    if record_kind == RecordKind::GoalConstraintUpdate {
        result["event"]["goal_constraints"] = json!(goal_constraints);
        result["case"] = output::case_json(&case);
    }
    append_dispatch_report(&mut result, &dispatch);

    Ok(result)
}

async fn cmd_decide(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
    reason: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    if summary.trim().is_empty() {
        return Err(CaseError::Other("summary must not be empty".to_string()));
    }

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
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::DecisionAppended {
            case: case.clone(),
            entry: entry.clone(),
        },
    )
    .await;

    let next = NextAction {
        suggested_command: "step done".to_string(),
        why: "the current decision narrows the step queue rather than changing direction"
            .to_string(),
    };

    let mut value = json!({
        "ok": true,
        "event": {
            "seq": entry.seq,
            "entry_type": "decision",
            "summary": summary,
            "reason": reason
        },
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
async fn cmd_redirect(
    client: &CaseClient,
    case_id: &str,
    direction: &str,
    reason: &str,
    context: &str,
    is_drift_from_goal: GoalDriftFlag,
    constraint_strs: &[String],
    success_condition: &str,
    abort_condition: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;

    if is_drift_from_goal == GoalDriftFlag::Yes {
        return Err(CaseError::GoalDriftRequiresNewCase);
    }

    if success_condition.is_empty() || abort_condition.is_empty() {
        return Err(CaseError::MissingDirectionExitConditions);
    }

    let constraints = parse_constraints(constraint_strs)?;

    // Get previous direction for from_direction
    let prev_dir = client
        .get_current_direction(case_id, case.current_direction_seq)
        .await?;

    let new_seq = case.current_direction_seq + 1;
    if let Some(existing_dir) = client.find_direction(case_id, new_seq).await? {
        // Recover the common half-written redirect case: the next direction already exists,
        // but the case pointer never advanced because the final UPDATE failed.
        if existing_dir.summary == direction
            && existing_dir.constraints == constraints
            && existing_dir.success_condition == success_condition
            && existing_dir.abort_condition == abort_condition
            && existing_dir.reason.as_deref() == Some(reason)
            && existing_dir.context.as_deref() == Some(context)
        {
            client.update_case_direction(case_id, new_seq).await?;
            let updated_case = client.get_case(case_id).await?;
            let dispatch = dispatch_event(
                client,
                CaseDomainEvent::RedirectRecovered {
                    case: updated_case,
                    from_direction: prev_dir.clone(),
                    to_direction: existing_dir.clone(),
                },
            )
            .await;

            let next = NextAction {
                suggested_command: "step add".to_string(),
                why: "the recovered direction needs a fresh execution queue".to_string(),
            };

            let mut value = json!({
                "ok": true,
                "event": {
                    "seq": serde_json::Value::Null,
                    "entry_type": "redirect_recovered",
                    "summary": "recovered previously written redirect direction",
                    "from_direction": prev_dir.summary,
                    "to_direction": existing_dir.summary,
                    "reason": reason,
                    "context": context
                },
                "direction": output::direction_json(&existing_dir),
                "steps": output::steps_json(&[]),
                "context": output::context_json(case_id, new_seq),
                "next": output::next_json(&next)
            });
            append_dispatch_report(&mut value, &dispatch);
            return Ok(value);
        }

        return Err(CaseError::Other(format!(
            "direction seq {new_seq} already exists for case {case_id}; likely a partial redirect residue with different content"
        )));
    }

    // Create redirect entry
    let entry_seq = next_entry_seq(client, case_id).await?;
    let entry = client
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
    let updated_case = client.get_case(case_id).await?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::RedirectCommitted {
            case: updated_case,
            from_direction: prev_dir.clone(),
            to_direction: new_dir.clone(),
            entry: entry.clone(),
        },
    )
    .await;

    let next = NextAction {
        suggested_command: "step add".to_string(),
        why: "the new direction needs a fresh execution queue".to_string(),
    };

    let mut value = json!({
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
        "steps": output::steps_json(&[]),
        "context": output::context_json(case_id, new_seq),
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

async fn cmd_show(client: &CaseClient, id: Option<&str>) -> CaseResult<serde_json::Value> {
    let case = resolve_case(client, id).await?;
    let directions = client.get_directions(&case.id).await?;
    let all_steps = client.get_all_steps(&case.id).await?;
    let entries = client.get_entries(&case.id).await?;
    let (dir_history, steps_by_dir) =
        build_direction_tree_payload(&directions, &all_steps, Some(case.current_direction_seq));

    Ok(json!({
        "ok": true,
        "case": output::case_json(&case),
        "direction_history": dir_history,
        "steps_by_direction": steps_by_dir,
        "entries": entries.iter().map(output::entry_json).collect::<Vec<_>>()
    }))
}

async fn cmd_close(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
    confirm_token: Option<&str>,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;
    ensure_no_unfinished_steps(client, &case).await?;
    ensure_close_confirmation(client, &case, "close", summary, confirm_token).await?;

    client
        .update_case_status(case_id, CaseStatus::Closed, summary)
        .await?;
    let closed_case = client.get_case(case_id).await?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::CaseClosed {
            case: closed_case,
            summary: summary.to_string(),
        },
    )
    .await;

    let next = NextAction {
        suggested_command: "open".to_string(),
        why: "the repository now has no active case; open a new one only if the next task merits case tracking".to_string(),
    };

    let mut value = json!({
        "ok": true,
        "case": {
            "id": case_id,
            "goal": case.goal,
            "status": "closed",
            "close_summary": summary
        },
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

async fn cmd_abandon(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
    confirm_token: Option<&str>,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;
    ensure_no_unfinished_steps(client, &case).await?;
    ensure_close_confirmation(client, &case, "abandon", summary, confirm_token).await?;

    client
        .update_case_status(case_id, CaseStatus::Abandoned, summary)
        .await?;
    let abandoned_case = client.get_case(case_id).await?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::CaseAbandoned {
            case: abandoned_case,
            summary: summary.to_string(),
        },
    )
    .await;

    let next = NextAction {
        suggested_command: "open".to_string(),
        why: "the previous goal has been explicitly abandoned; open a new case only if the next task merits case tracking".to_string(),
    };

    let mut value = json!({
        "ok": true,
        "case": {
            "id": case_id,
            "goal": case.goal,
            "status": "abandoned",
            "abandon_summary": summary
        },
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

async fn ensure_close_confirmation(
    client: &CaseClient,
    case: &Case,
    action: &str,
    summary: &str,
    confirm_token: Option<&str>,
) -> CaseResult<()> {
    match confirm_token {
        Some(token)
            if case.close_confirm_token.as_deref() == Some(token)
                && case.close_confirm_action.as_deref() == Some(action)
                && case.close_confirm_summary.as_deref() == Some(summary) =>
        {
            Ok(())
        }
        Some(_) => {
            let next_token = Uuid::new_v4().to_string();
            client
                .set_close_confirmation(&case.id, action, summary, &next_token)
                .await?;
            Err(CaseError::InvalidCloseConfirmationToken {
                case_id: case.id.clone(),
                action: action.to_string(),
            })
        }
        None => {
            let next_token = Uuid::new_v4().to_string();
            client
                .set_close_confirmation(&case.id, action, summary, &next_token)
                .await?;
            Err(CaseError::CloseConfirmationRequired {
                case_id: case.id.clone(),
                action: action.to_string(),
                summary: summary.to_string(),
                confirm_token: next_token,
            })
        }
    }
}

#[cfg(test)]
async fn confirm_and_close(
    client: &CaseClient,
    case_id: &str,
    summary: &str,
) -> CaseResult<serde_json::Value> {
    let confirm_token = match cmd_close(client, case_id, summary, None).await {
        Err(CaseError::CloseConfirmationRequired { confirm_token, .. }) => confirm_token,
        Err(other) => return Err(other),
        Ok(value) => return Ok(value),
    };

    cmd_close(client, case_id, summary, Some(&confirm_token)).await
}

async fn cmd_step(client: &CaseClient, command: &StepCommand) -> CaseResult<serde_json::Value> {
    match command {
        StepCommand::Add {
            id,
            title,
            reason,
            start,
        } => cmd_step_add(client, id, title, reason.as_deref(), *start).await,
        StepCommand::Start { id, step_id } => cmd_step_start(client, id, step_id).await,
        StepCommand::Done { id, step_id } => cmd_step_done(client, id, step_id).await,
        StepCommand::Move {
            id,
            step_id,
            before,
        } => cmd_step_move(client, id, step_id, before).await,
        StepCommand::Block {
            id,
            step_id,
            reason,
        } => cmd_step_block(client, id, step_id, reason).await,
    }
}

async fn cmd_step_add(
    client: &CaseClient,
    case_id: &str,
    title: &str,
    reason: Option<&str>,
    start: bool,
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

    if start {
        activate_step(client, &case, &step.id).await?;
    }

    let steps = client
        .get_steps(case_id, case.current_direction_seq)
        .await?;
    let step = steps
        .iter()
        .find(|candidate| candidate.id == step.id)
        .cloned()
        .expect("newly created step should be visible after reload");
    let refreshed_case = client.get_case(case_id).await?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::StepAdded {
            case: refreshed_case.clone(),
            step: step.clone(),
        },
    )
    .await;

    let next = if start {
        NextAction {
            suggested_command: "record".to_string(),
            why: "capture findings as you execute the active step".to_string(),
        }
    } else {
        NextAction {
            suggested_command: "step start".to_string(),
            why: "the step exists but is not active yet".to_string(),
        }
    };

    let mut value = json!({
        "ok": true,
        "step": {
            "id": step.id,
            "order": step.order_index,
            "title": step.title,
            "status": step.status.as_str()
        },
        "steps": output::steps_json(&steps),
        "context": output::context_json(case_id, refreshed_case.current_direction_seq),
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

async fn cmd_step_start(
    client: &CaseClient,
    case_id: &str,
    step_id: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;
    let step = client.get_step(step_id).await?;
    ensure_step_belongs_to_current_direction(&step, &case, step_id)?;

    activate_step(client, &case, step_id).await?;
    let refreshed_case = client.get_case(case_id).await?;

    let steps = client
        .get_steps(case_id, refreshed_case.current_direction_seq)
        .await?;
    let started_step = steps
        .iter()
        .find(|step| step.id == step_id)
        .cloned()
        .ok_or_else(|| CaseError::StepNotFound(step_id.to_string()))?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::StepStarted {
            case: refreshed_case.clone(),
            step: started_step,
        },
    )
    .await;

    let next = NextAction {
        suggested_command: "record".to_string(),
        why: "capture findings as you execute the step".to_string(),
    };

    let mut value = json!({
        "ok": true,
        "steps": output::steps_json(&steps),
        "reminder": step_status_reminder(&steps),
        "context": output::context_json(case_id, refreshed_case.current_direction_seq),
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

async fn activate_step(client: &CaseClient, case: &Case, step_id: &str) -> CaseResult<()> {
    // Deactivate any existing active step to maintain "one active at a time" invariant.
    let steps = client
        .get_steps(&case.id, case.current_direction_seq)
        .await?;
    for step in &steps {
        if step.status == StepStatus::Active && step.id != step_id {
            client
                .update_step(&step.id, StepStatus::Pending, None)
                .await?;
        }
    }

    client
        .update_step(step_id, StepStatus::Active, None)
        .await?;
    client.update_case_step(&case.id, step_id).await?;
    Ok(())
}

async fn cmd_step_done(
    client: &CaseClient,
    case_id: &str,
    step_id: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;
    let step = client.get_step(step_id).await?;
    ensure_step_belongs_to_current_direction(&step, &case, step_id)?;

    client.update_step(step_id, StepStatus::Done, None).await?;

    // Clear current_step_id if it was the active one
    if case.current_step_id.as_deref() == Some(step_id) {
        client.update_case_step(case_id, "").await?;
    }
    let refreshed_case = client.get_case(case_id).await?;

    let steps = client
        .get_steps(case_id, refreshed_case.current_direction_seq)
        .await?;
    let done_step = steps
        .iter()
        .find(|step| step.id == step_id)
        .cloned()
        .ok_or_else(|| CaseError::StepNotFound(step_id.to_string()))?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::StepDone {
            case: refreshed_case.clone(),
            step: done_step,
        },
    )
    .await;
    let (_, pending_steps) = split_steps(&steps);

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

    let mut value = json!({
        "ok": true,
        "steps": output::steps_json(&steps),
        "reminder": step_status_reminder(&steps),
        "context": output::context_json(case_id, refreshed_case.current_direction_seq),
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
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
        client.reorder_step(&step.id, (i + 1) as u32).await?;
    }

    // Re-fetch to get updated data
    let refreshed_case = client.get_case(case_id).await?;
    let steps = client
        .get_steps(case_id, refreshed_case.current_direction_seq)
        .await?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::StepsReordered {
            case: refreshed_case.clone(),
            moved_step_id: step_id.to_string(),
            before_step_id: before_id.to_string(),
            steps: steps.clone(),
        },
    )
    .await;

    let next = NextAction {
        suggested_command: "step start".to_string(),
        why: "the reordered blocker-fix step should now run first".to_string(),
    };

    let mut value = json!({
        "ok": true,
        "steps": output::steps_json(&steps),
        "reminder": step_status_reminder(&steps),
        "context": output::context_json(case_id, refreshed_case.current_direction_seq),
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

async fn cmd_step_block(
    client: &CaseClient,
    case_id: &str,
    step_id: &str,
    reason: &str,
) -> CaseResult<serde_json::Value> {
    let case = client.get_case(case_id).await?;
    ensure_open(&case)?;
    let step = client.get_step(step_id).await?;
    ensure_step_belongs_to_current_direction(&step, &case, step_id)?;

    client
        .update_step(step_id, StepStatus::Blocked, Some(reason))
        .await?;
    let refreshed_case = client.get_case(case_id).await?;

    let steps = client
        .get_steps(case_id, refreshed_case.current_direction_seq)
        .await?;
    let blocked_step = steps
        .iter()
        .find(|step| step.id == step_id)
        .cloned()
        .ok_or_else(|| CaseError::StepNotFound(step_id.to_string()))?;
    let dispatch = dispatch_event(
        client,
        CaseDomainEvent::StepBlocked {
            case: refreshed_case.clone(),
            step: blocked_step,
        },
    )
    .await;

    let next = NextAction {
        suggested_command: "step add".to_string(),
        why: "consider adding a step to resolve the blocker".to_string(),
    };

    let mut value = json!({
        "ok": true,
        "steps": output::steps_json(&steps),
        "reminder": step_status_reminder(&steps),
        "context": output::context_json(case_id, refreshed_case.current_direction_seq),
        "next": output::next_json(&next)
    });
    append_dispatch_report(&mut value, &dispatch);
    Ok(value)
}

async fn ensure_no_unfinished_steps(client: &CaseClient, case: &Case) -> CaseResult<()> {
    let steps = client
        .get_steps(&case.id, case.current_direction_seq)
        .await?;
    if steps
        .iter()
        .any(|step| !matches!(step.status, StepStatus::Done))
    {
        return Err(CaseError::UnfinishedSteps);
    }
    Ok(())
}

fn step_status_reminder(steps: &[Step]) -> serde_json::Value {
    let unfinished: Vec<_> = steps
        .iter()
        .filter(|step| !matches!(step.status, StepStatus::Done))
        .map(output::step_json)
        .collect();

    if unfinished.is_empty() {
        json!("all steps are complete; if the goal is met, you can close the case")
    } else {
        json!(format!(
            "{} unfinished step(s) remain; review them before closing the case",
            unfinished.len()
        ))
    }
}

fn ensure_step_belongs_to_current_direction(
    step: &Step,
    case: &Case,
    step_id: &str,
) -> CaseResult<()> {
    if step.case_id != case.id || step.direction_seq != case.current_direction_seq {
        return Err(CaseError::StepNotFound(step_id.to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct CaseListOptions {
    status: Option<CaseStatusArg>,
    limit: Option<usize>,
    recent_days: Option<u32>,
}

impl CaseListOptions {
    fn new(status: Option<CaseStatusArg>, limit: Option<usize>, recent_days: Option<u32>) -> Self {
        Self {
            status,
            limit,
            recent_days,
        }
    }
}

// TODO: recall currently uses weighted text matching only (no semantic search).
// Phase 4 will add vector search via CaseSearchIndex.
async fn cmd_recall(
    client: &CaseClient,
    query: &str,
    options: CaseListOptions,
) -> CaseResult<serde_json::Value> {
    validate_list_options(options)?;
    validate_recall_query(query)?;

    let mut cases = LocalTextSearchBackend::new(client.clone())
        .recall_cases(query)
        .await?;
    filter_recall_results(&mut cases, options);
    cases.sort_by(|left, right| compare_recall_results(left, right, query));
    if let Some(limit) = options.limit {
        cases.truncate(limit);
    }

    let case_list: Vec<_> = cases.iter().map(output::case_search_json).collect();

    Ok(json!({
        "ok": true,
        "cases": case_list,
        "query": query,
        "_meta": list_meta_json(options)
    }))
}

async fn cmd_context(
    client: &CaseClient,
    id: Option<&str>,
    scope: ContextScopeArg,
    query: Option<&str>,
    limit: Option<usize>,
    token_limit: Option<u32>,
) -> CaseResult<serde_json::Value> {
    if matches!(limit, Some(0)) {
        return Err(CaseError::InvalidListOption(
            "limit must be at least 1".to_string(),
        ));
    }
    let default_limit = limit.unwrap_or(5);

    match scope {
        ContextScopeArg::Case => {
            let case = resolve_case(client, id).await?;
            let provider = context_provider_for_client(client)?;
            let result = provider
                .get_context(
                    ContextScope::Case { case_id: &case.id },
                    query,
                    default_limit,
                    token_limit,
                )
                .await?;

            Ok(json!({
                "ok": true,
                "case": output::case_json(&case),
                "case_context": output::case_context_json(&result),
                "context": output::context_json(&case.id, case.current_direction_seq),
            }))
        }
        ContextScopeArg::Repo => {
            let result =
                if client.config().honcho_enabled && client.config().semantic_recall_enabled {
                    if let Some(honcho) = HonchoBackend::from_config(client.config())? {
                        honcho
                            .get_repo_context(client.repo_id(), query, default_limit, token_limit)
                            .await?
                    } else {
                        LocalCaseContextProvider::new(client.clone())
                            .get_context(ContextScope::Repo, query, default_limit, token_limit)
                            .await?
                    }
                } else {
                    LocalCaseContextProvider::new(client.clone())
                        .get_context(ContextScope::Repo, query, default_limit, token_limit)
                        .await?
                };

            Ok(json!({
                "ok": true,
                "repo": {
                    "id": client.repo_id(),
                    "label": client.repo_label(),
                    "worktree_id": client.worktree_id(),
                    "worktree_root": client.worktree_root(),
                },
                "case_context": output::case_context_json(&result),
            }))
        }
    }
}

async fn cmd_list(client: &CaseClient, options: CaseListOptions) -> CaseResult<serde_json::Value> {
    validate_list_options(options)?;

    let mut cases = client.list_cases().await?;
    filter_cases(&mut cases, options);
    cases.sort_by(compare_case_recency);
    if let Some(limit) = options.limit {
        cases.truncate(limit);
    }

    let case_list: Vec<_> = cases.iter().map(output::case_json).collect();

    Ok(json!({
        "ok": true,
        "cases": case_list,
        "_meta": list_meta_json(options)
    }))
}

async fn cmd_resume(client: &CaseClient, id: Option<&str>) -> CaseResult<serde_json::Value> {
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
        .find(|e| e.entry_type == EntryType::Record && e.kind.as_deref() == Some("evidence"))
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
    let current = steps
        .iter()
        .find(|s| s.status == StepStatus::Active)
        .cloned();
    let pending: Vec<Step> = steps
        .iter()
        .filter(|s| s.status == StepStatus::Pending)
        .cloned()
        .collect();
    (current, pending)
}

fn validate_list_options(options: CaseListOptions) -> CaseResult<()> {
    if matches!(options.limit, Some(0)) {
        return Err(CaseError::InvalidListOption(
            "limit must be at least 1".to_string(),
        ));
    }

    if matches!(options.recent_days, Some(0)) {
        return Err(CaseError::InvalidListOption(
            "recent_days must be at least 1".to_string(),
        ));
    }

    Ok(())
}

fn validate_recall_query(query: &str) -> CaseResult<()> {
    if query.trim().is_empty() {
        return Err(CaseError::InvalidQuery(
            "recall query must not be empty".to_string(),
        ));
    }

    Ok(())
}

fn filter_recall_results(results: &mut Vec<CaseSearchResult>, options: CaseListOptions) {
    results.retain(|result| matches_case_filters(&result.case, options));
}

fn filter_cases(cases: &mut Vec<Case>, options: CaseListOptions) {
    cases.retain(|case| matches_case_filters(case, options));
}

fn matches_case_filters(case: &Case, options: CaseListOptions) -> bool {
    if let Some(status) = options.status {
        let expected = match status {
            CaseStatusArg::Open => CaseStatus::Open,
            CaseStatusArg::Closed => CaseStatus::Closed,
            CaseStatusArg::Abandoned => CaseStatus::Abandoned,
        };
        if case.status != expected {
            return false;
        }
    }

    if let Some(recent_days) = options.recent_days {
        let Some(updated_at) = parse_case_timestamp(&case.updated_at) else {
            return false;
        };
        let cutoff = Utc::now() - Duration::days(recent_days as i64);
        if updated_at < cutoff {
            return false;
        }
    }

    true
}

fn compare_recall_results(
    left: &CaseSearchResult,
    right: &CaseSearchResult,
    query: &str,
) -> std::cmp::Ordering {
    let left_score = recall_score(left, query);
    let right_score = recall_score(right, query);

    right_score
        .cmp(&left_score)
        .then_with(|| compare_case_recency(&left.case, &right.case))
        .then_with(|| left.case.id.cmp(&right.case.id))
}

fn recall_score(result: &CaseSearchResult, query: &str) -> i64 {
    let query_lower = query.to_lowercase();
    let exact_goal_match = i64::from(result.case.goal.to_lowercase().contains(&query_lower)) * 40;
    let match_score: i64 = result
        .matches
        .iter()
        .map(
            |matched| match (matched.scope.as_str(), matched.field.as_str()) {
                ("case", "goal") => 12,
                ("case", "close_summary" | "abandon_summary") => 8,
                ("direction", "summary") => 7,
                ("direction", "success_condition" | "abort_condition") => 6,
                ("entry", "summary") => 5,
                ("entry", "context") => 4,
                ("direction", "context" | "reason") => 3,
                ("entry", "reason" | "kind") => 2,
                _ => 1,
            },
        )
        .sum();
    let recency_bonus = parse_case_timestamp(&result.case.updated_at)
        .map(|updated_at| {
            let age_days = (Utc::now() - updated_at).num_days();
            (30 - age_days).clamp(0, 30)
        })
        .unwrap_or(0);

    exact_goal_match + match_score + recency_bonus
}

fn compare_case_recency(left: &Case, right: &Case) -> std::cmp::Ordering {
    parse_case_timestamp(&right.updated_at)
        .cmp(&parse_case_timestamp(&left.updated_at))
        .then_with(|| right.id.cmp(&left.id))
}

fn parse_case_timestamp(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn list_meta_json(options: CaseListOptions) -> serde_json::Value {
    json!({
        "status": options.status.map(|value| match value {
            CaseStatusArg::Open => "open",
            CaseStatusArg::Closed => "closed",
            CaseStatusArg::Abandoned => "abandoned",
        }),
        "limit": options.limit,
        "recent_days": options.recent_days
    })
}

fn build_direction_tree_payload(
    directions: &[Direction],
    all_steps: &[Step],
    current_direction_seq: Option<u32>,
) -> (
    Vec<serde_json::Value>,
    serde_json::Map<String, serde_json::Value>,
) {
    let mut steps_by_dir: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();

    for dir in directions {
        let dir_steps: Vec<_> = all_steps
            .iter()
            .filter(|step| step.direction_seq == dir.seq)
            .map(output::step_json)
            .collect();
        if !dir_steps.is_empty() {
            steps_by_dir.insert(dir.seq.to_string(), json!(dir_steps));
        }
    }

    let dir_history = directions
        .iter()
        .map(|dir| {
            let mut value = output::direction_json(dir);
            if current_direction_seq == Some(dir.seq) {
                value["is_current"] = json!(true);
            }
            value
        })
        .collect();

    (dir_history, steps_by_dir)
}

/// Detect health status based on recent entries and steps.
fn detect_health(steps: &[Step], _last_entry: &Option<Entry>) -> (Health, Option<String>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DbConfig;
    use tempfile::TempDir;

    fn temp_db_config(temp_dir: &TempDir) -> DbConfig {
        let db_path = temp_dir.path().join("case.db");
        DbConfig::from_data_dir(Some(
            db_path
                .to_str()
                .expect("temporary database path should be valid UTF-8"),
        ))
    }

    #[tokio::test]
    async fn shared_database_generates_distinct_case_ids_per_repository() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let identity_a = RepoIdentity {
            repo_id: "aaaaaaaaaaaaaaaa".to_string(),
            repo_label: "github.com/example/repo-a".to_string(),
            worktree_id: "1111111111111111".to_string(),
            worktree_root: "/tmp/repo-a".to_string(),
        };
        let identity_b = RepoIdentity {
            repo_id: "bbbbbbbbbbbbbbbb".to_string(),
            repo_label: "github.com/example/repo-b".to_string(),
            worktree_id: "2222222222222222".to_string(),
            worktree_root: "/tmp/repo-b".to_string(),
        };
        let client_a = CaseClient::new(&config, identity_a.clone())
            .await
            .expect("repo A client should initialize");
        let client_b = client_a.clone_with_identity(identity_b);

        let result_a = cmd_open_new(&client_a, "goal a", "direction a", &[], &[], None, None)
            .await
            .expect("repo A should open its first case");
        let result_b = cmd_open_new(&client_b, "goal b", "direction b", &[], &[], None, None)
            .await
            .expect("repo B should open its first case on the same shared DB");

        let case_id_a = result_a["case"]["id"]
            .as_str()
            .expect("repo A case id should exist");
        let case_id_b = result_b["case"]["id"]
            .as_str()
            .expect("repo B case id should exist");

        assert!(case_id_a.starts_with("C-"));
        assert!(case_id_b.starts_with("C-"));
        assert!(Uuid::parse_str(&case_id_a[2..]).is_ok());
        assert!(Uuid::parse_str(&case_id_b[2..]).is_ok());
        assert_ne!(case_id_a, case_id_b);
    }

    #[tokio::test]
    async fn get_case_is_scoped_to_current_repository() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let identity_a = RepoIdentity {
            repo_id: "aaaaaaaaaaaaaaaa".to_string(),
            repo_label: "github.com/example/repo-a".to_string(),
            worktree_id: "1111111111111111".to_string(),
            worktree_root: "/tmp/repo-a".to_string(),
        };
        let identity_b = RepoIdentity {
            repo_id: "bbbbbbbbbbbbbbbb".to_string(),
            repo_label: "github.com/example/repo-b".to_string(),
            worktree_id: "2222222222222222".to_string(),
            worktree_root: "/tmp/repo-b".to_string(),
        };
        let client_a = CaseClient::new(&config, identity_a)
            .await
            .expect("repo A client should initialize");
        let client_b = client_a.clone_with_identity(identity_b);

        let result_a = cmd_open_new(&client_a, "goal a", "direction a", &[], &[], None, None)
            .await
            .expect("repo A should open its case");
        let case_id_a = result_a["case"]["id"]
            .as_str()
            .expect("repo A case id should exist");

        let error = client_b
            .get_case(case_id_a)
            .await
            .expect_err("repo B should not resolve repo A case by explicit id");
        assert!(matches!(error, CaseError::CaseNotFound(id) if id == case_id_a));
    }

    #[tokio::test]
    async fn current_state_only_returns_open_with_case_id() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let result = cmd_current(&client, true)
            .await
            .expect("state-only current should succeed");

        assert_eq!(result["kind"].as_str(), Some("case_current_state"));
        assert_eq!(result["state"].as_str(), Some("open"));
        assert_eq!(result["case_id"].as_str(), Some(case_id.as_str()));
        assert!(result.get("direction").is_none());
        assert!(result.get("steps").is_none());
    }

    #[tokio::test]
    async fn current_state_only_without_open_case_returns_no_open_case() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let error = cmd_current(&client, true)
            .await
            .expect_err("state-only current should fail without open case");

        assert!(matches!(error, CaseError::NoOpenCase));
    }

    #[tokio::test]
    async fn step_add_can_start_immediately() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let result = cmd_step_add(&client, &case_id, "run verification", None, true)
            .await
            .expect("step add with start should succeed");

        assert_eq!(result["step"]["status"].as_str(), Some("active"));
        assert_eq!(result["next"]["suggested_command"].as_str(), Some("record"));
        assert_eq!(
            result["steps"]["current"]["title"].as_str(),
            Some("run verification")
        );
    }

    #[tokio::test]
    async fn execute_json_batch_adds_six_steps_with_shared_client() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let commands = vec![
            ("step 1", None, true),
            ("step 2", Some("reason 2".to_string()), false),
            ("step 3", Some("reason 3".to_string()), false),
            ("step 4", Some("reason 4".to_string()), false),
            ("step 5", Some("reason 5".to_string()), false),
            ("step 6", Some("reason 6".to_string()), false),
        ]
        .into_iter()
        .map(|(title, reason, start)| CaseCommand::Step {
            command: StepCommand::Add {
                id: case_id.clone(),
                title: title.to_string(),
                reason,
                start,
            },
        })
        .collect();

        let values = execute_json_batch_with_client(&client, commands, true).await;

        assert_eq!(values.len(), 6);
        assert!(values
            .iter()
            .all(|value| value.get("ok").and_then(|v| v.as_bool()) == Some(true)));
        assert_eq!(values[0]["step"]["status"].as_str(), Some("active"));
        assert_eq!(
            values[5]["steps"]["ordered"].as_array().map(Vec::len),
            Some(6)
        );
        assert_eq!(values[5]["step"]["title"].as_str(), Some("step 6"));
    }

    #[tokio::test]
    async fn recall_matches_record_summary_and_context() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(
            &client,
            "stabilize inference rollout",
            "inspect prod readiness",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        cmd_record(
            &client,
            &case_id,
            "sample report shows one toxic audit outlier",
            "finding",
            &[],
            &[],
            Some("audit csv was only a sample, not the full pool"),
        )
        .await
        .expect("record should succeed");

        let recalled = cmd_recall(&client, "audit", CaseListOptions::new(None, None, None))
            .await
            .expect("recall should succeed");
        let cases = recalled["cases"]
            .as_array()
            .expect("cases should be returned");

        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0]["id"].as_str(), Some(case_id.as_str()));
        let matches = cases[0]["matches"]
            .as_array()
            .expect("recall should include match details");
        assert!(
            matches.iter().any(|m| {
                m["scope"].as_str() == Some("entry")
                    && m["field"].as_str() == Some("summary")
                    && m["excerpt"]
                        .as_str()
                        .is_some_and(|excerpt| excerpt.contains("audit outlier"))
            }),
            "record summary match should be surfaced"
        );
        assert!(
            matches.iter().any(|m| {
                m["scope"].as_str() == Some("entry")
                    && m["field"].as_str() == Some("context")
                    && m["excerpt"]
                        .as_str()
                        .is_some_and(|excerpt| excerpt.contains("audit csv"))
            }),
            "record context match should be surfaced"
        );
    }

    #[tokio::test]
    async fn list_filters_by_status_and_limit() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let first = cmd_open_new(&client, "goal a", "direction a", &[], &[], None, None)
            .await
            .expect("first case should open");
        let first_id = first["case"]["id"]
            .as_str()
            .expect("first case id should exist")
            .to_string();
        confirm_and_close(&client, &first_id, "done")
            .await
            .expect("first case should close");

        cmd_open_new(&client, "goal b", "direction b", &[], &[], None, None)
            .await
            .expect("second case should open");

        let listed = cmd_list(
            &client,
            CaseListOptions::new(Some(CaseStatusArg::Open), Some(1), None),
        )
        .await
        .expect("list should succeed");
        let cases = listed["cases"]
            .as_array()
            .expect("cases should be an array");

        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0]["status"].as_str(), Some("open"));
        assert_eq!(listed["_meta"]["limit"].as_u64(), Some(1));
        assert_eq!(listed["_meta"]["status"].as_str(), Some("open"));
    }

    #[tokio::test]
    async fn recall_prioritizes_goal_matches_and_applies_limit() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let direct = cmd_open_new(
            &client,
            "financial coverage decision",
            "inspect coverage",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("direct case should open");
        let direct_id = direct["case"]["id"]
            .as_str()
            .expect("direct case id should exist")
            .to_string();
        confirm_and_close(&client, &direct_id, "done")
            .await
            .expect("direct case should close");

        let indirect = cmd_open_new(
            &client,
            "audit follow-up",
            "inspect notes",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("indirect case should open");
        let indirect_id = indirect["case"]["id"]
            .as_str()
            .expect("indirect case id should exist")
            .to_string();
        cmd_record(
            &client,
            &indirect_id,
            "captured a mention of financial coverage in notes",
            "finding",
            &[],
            &[],
            None,
        )
        .await
        .expect("record should succeed");
        confirm_and_close(&client, &indirect_id, "done")
            .await
            .expect("indirect case should close");

        let recalled = cmd_recall(
            &client,
            "financial coverage",
            CaseListOptions::new(None, Some(1), None),
        )
        .await
        .expect("recall should succeed");
        let cases = recalled["cases"]
            .as_array()
            .expect("cases should be an array");

        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0]["id"].as_str(), Some(direct_id.as_str()));
        assert_eq!(recalled["_meta"]["limit"].as_u64(), Some(1));
    }

    #[tokio::test]
    async fn list_rejects_zero_limit() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let error = cmd_list(&client, CaseListOptions::new(None, Some(0), None))
            .await
            .expect_err("zero limit should be rejected");

        assert!(
            matches!(error, CaseError::InvalidListOption(message) if message.contains("limit"))
        );
    }

    #[tokio::test]
    async fn list_rejects_zero_recent_days() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let error = cmd_list(&client, CaseListOptions::new(None, None, Some(0)))
            .await
            .expect_err("zero recent days should be rejected");

        assert!(
            matches!(error, CaseError::InvalidListOption(message) if message.contains("recent_days"))
        );
    }

    #[tokio::test]
    async fn recall_rejects_empty_query() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let error = cmd_recall(&client, "   ", CaseListOptions::new(None, None, None))
            .await
            .expect_err("empty recall query should be rejected");

        assert!(
            matches!(error, CaseError::InvalidQuery(message) if message.contains("must not be empty"))
        );
    }

    #[tokio::test]
    async fn show_includes_record_entries_for_case_review() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        cmd_record(
            &client,
            &case_id,
            "record summary",
            "finding",
            &[],
            &[],
            Some("record context"),
        )
        .await
        .expect("record should succeed");

        let shown = cmd_show(&client, Some(&case_id))
            .await
            .expect("show should succeed");
        let entries = shown["entries"]
            .as_array()
            .expect("show should include entries");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["summary"].as_str(), Some("record summary"));
        assert_eq!(entries[0]["context"].as_str(), Some("record context"));
        assert_eq!(entries[0]["kind"].as_str(), Some("finding"));
    }

    #[tokio::test]
    async fn close_rejects_when_any_step_is_unfinished() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        cmd_step_add(&client, &case_id, "unfinished step", None, false)
            .await
            .expect("step add should succeed");

        let error = cmd_close(&client, &case_id, "done", None)
            .await
            .expect_err("close should reject unfinished steps");

        assert!(matches!(error, CaseError::UnfinishedSteps));

        let error_value = build_error_value(
            &client,
            &CaseCommand::Close {
                id: case_id.clone(),
                summary: "done".to_string(),
                confirm_token: None,
            },
            &error,
        )
        .await;

        let unfinished = error_value["unfinished_steps"]
            .as_array()
            .expect("unfinished steps should be present");
        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0]["title"].as_str(), Some("unfinished step"));
    }

    #[tokio::test]
    async fn close_requires_confirmation_then_succeeds_with_matching_token() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let error = cmd_close(&client, &case_id, "done", None)
            .await
            .expect_err("first close should require confirmation");

        let confirm_token = match &error {
            CaseError::CloseConfirmationRequired { confirm_token, .. } => confirm_token.clone(),
            other => panic!("unexpected error: {other}"),
        };

        let error_value = build_error_value(
            &client,
            &CaseCommand::Close {
                id: case_id.clone(),
                summary: "done".to_string(),
                confirm_token: None,
            },
            &error,
        )
        .await;

        assert_eq!(error_value["state"].as_str(), Some("confirmation_required"));
        assert_eq!(
            error_value["confirmation"]["confirm_token"].as_str(),
            Some(confirm_token.as_str())
        );

        let closed = cmd_close(&client, &case_id, "done", Some(&confirm_token))
            .await
            .expect("second close with matching token should succeed");
        assert_eq!(closed["case"]["status"].as_str(), Some("closed"));
    }

    #[tokio::test]
    async fn close_rejects_stale_confirmation_token() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let _ = cmd_close(&client, &case_id, "done", None)
            .await
            .expect_err("first close should require confirmation");

        let error = cmd_close(&client, &case_id, "done", Some("stale-token"))
            .await
            .expect_err("stale token should be rejected");

        assert!(matches!(
            error,
            CaseError::InvalidCloseConfirmationToken { .. }
        ));
    }

    #[tokio::test]
    async fn open_mode_reopen_reopens_closed_case_without_new_tool() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        confirm_and_close(&client, &case_id, "done")
            .await
            .expect("close should succeed");

        let reopened = cmd_open(
            &client,
            OpenRequest {
                mode: OpenModeArg::Reopen,
                reopen_case_id: Some(&case_id),
                goal: None,
                direction: None,
                goal_constraint_strs: &[],
                constraint_strs: &[],
                success_condition: None,
                abort_condition: None,
            },
        )
        .await
        .expect("reopen should succeed");

        assert_eq!(reopened["case"]["id"].as_str(), Some(case_id.as_str()));
        assert_eq!(reopened["case"]["status"].as_str(), Some("open"));
        assert_eq!(reopened["message"].as_str(), Some("case reopened"));
    }

    #[tokio::test]
    async fn step_done_returns_reminder_before_case_close() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let added = cmd_step_add(&client, &case_id, "only step", None, true)
            .await
            .expect("step add should succeed");
        let step_id = added["step"]["id"]
            .as_str()
            .expect("step id should exist")
            .to_string();

        let done = cmd_step_done(&client, &case_id, &step_id)
            .await
            .expect("step done should succeed");

        assert_eq!(
            done["reminder"].as_str(),
            Some("all steps are complete; if the goal is met, you can close the case")
        );
        assert_eq!(done["next"]["suggested_command"].as_str(), Some("close"));
    }

    #[tokio::test]
    async fn record_rejects_decision_kind_with_decide_hint() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let error = cmd_record(
            &client,
            &case_id,
            "decision summary",
            "decision",
            &[],
            &[],
            None,
        )
        .await
        .expect_err("decision kind should be rejected in record");

        assert!(matches!(
            error,
            CaseError::InvalidRecordKind { ref kind, .. } if kind == "decision"
        ));
        let next = error_next_action(&error).expect("decision misuse should provide hint");
        assert_eq!(next.suggested_command, "decide");
        assert!(next.why.contains("case_decide"));
    }

    #[tokio::test]
    async fn goal_constraint_update_record_appends_case_constraints_and_logs_payload() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let initial_constraints =
            vec![r#"{"rule":"先证据后推断","reason":"避免臆断"}"#.to_string()];
        let opened = cmd_open_new(
            &client,
            "goal",
            "direction",
            &initial_constraints,
            &[],
            None,
            None,
        )
        .await
        .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let added_constraints = vec![
            r#"{"rule":"保持最小改动","reason":"控制范围"}"#.to_string(),
            r#"{"rule":"先证据后推断","reason":"避免臆断"}"#.to_string(),
        ];

        let recorded = cmd_record(
            &client,
            &case_id,
            "补充全局约束",
            "goal_constraint_update",
            &added_constraints,
            &[],
            Some("新增后续执行边界"),
        )
        .await
        .expect("goal constraint update should succeed");

        let current_case = client.get_case(&case_id).await.expect("case should reload");
        assert_eq!(current_case.goal_constraints.len(), 2);
        assert_eq!(
            recorded["case"]["goal_constraints"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            recorded["event"]["goal_constraints"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );

        let shown = cmd_show(&client, Some(&case_id))
            .await
            .expect("show should succeed");
        let entries = shown["entries"]
            .as_array()
            .expect("show should include entries");
        assert_eq!(entries[0]["kind"].as_str(), Some("goal_constraint_update"));
        assert_eq!(entries[0]["artifacts"].as_array().map(Vec::len), Some(2));
    }

    #[tokio::test]
    async fn goal_constraint_update_requires_goal_constraint_payload() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let error = cmd_record(
            &client,
            &case_id,
            "补充全局约束",
            "goal_constraint_update",
            &[],
            &[],
            None,
        )
        .await
        .expect_err("missing goal_constraint payload should fail");

        assert!(matches!(
            error,
            CaseError::GoalConstraintUpdateRequiresConstraints
        ));
    }

    #[tokio::test]
    async fn regular_record_rejects_goal_constraint_payload() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let error = cmd_record(
            &client,
            &case_id,
            "普通记录",
            "finding",
            &[r#"{"rule":"保持最小改动"}"#.to_string()],
            &[],
            None,
        )
        .await
        .expect_err("non-goal-constraint record should reject payload");

        assert!(matches!(
            error,
            CaseError::GoalConstraintsOnlyAllowedForGoalConstraintUpdate
        ));
    }

    #[tokio::test]
    async fn redirect_rejects_goal_drift_and_points_to_new_case() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let error = cmd_redirect(
            &client,
            &case_id,
            "new direction",
            "topic changed",
            "work drifted",
            GoalDriftFlag::Yes,
            &[],
            "success",
            "abort",
        )
        .await
        .expect_err("goal drift should force a new case instead of redirect");

        assert!(matches!(error, CaseError::GoalDriftRequiresNewCase));
    }

    #[tokio::test]
    async fn redirect_recovers_when_next_direction_already_exists() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        client
            .create_direction(
                &case_id,
                2,
                "new direction",
                &[],
                "success",
                "abort",
                Some("topic changed"),
                Some("work drifted"),
            )
            .await
            .expect("residual direction should be inserted");

        let redirected = cmd_redirect(
            &client,
            &case_id,
            "new direction",
            "topic changed",
            "work drifted",
            GoalDriftFlag::No,
            &[],
            "success",
            "abort",
        )
        .await
        .expect("redirect should recover from matching residual direction");

        assert_eq!(
            redirected["event"]["entry_type"].as_str(),
            Some("redirect_recovered")
        );
        assert_eq!(
            redirected["context"]["current_direction_seq"].as_u64(),
            Some(2)
        );
        let case = client
            .get_case(&case_id)
            .await
            .expect("case should still exist");
        assert_eq!(case.current_direction_seq, 2);
    }

    #[tokio::test]
    async fn redirect_rejects_conflicting_residual_direction() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        client
            .create_direction(
                &case_id,
                2,
                "stale direction",
                &[],
                "success",
                "abort",
                Some("old reason"),
                Some("old context"),
            )
            .await
            .expect("conflicting residual direction should be inserted");

        let error = cmd_redirect(
            &client,
            &case_id,
            "new direction",
            "topic changed",
            "work drifted",
            GoalDriftFlag::No,
            &[],
            "success",
            "abort",
        )
        .await
        .expect_err("conflicting residual direction should be rejected");

        let message = error.to_string();
        assert!(message.contains("partial redirect residue"));
    }

    #[tokio::test]
    async fn redirect_rejects_residual_direction_with_different_reason_or_context() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        client
            .create_direction(
                &case_id,
                2,
                "new direction",
                &[],
                "success",
                "abort",
                Some("stale reason"),
                Some("work drifted"),
            )
            .await
            .expect("residual direction should be inserted");

        let error = cmd_redirect(
            &client,
            &case_id,
            "new direction",
            "topic changed",
            "work drifted",
            GoalDriftFlag::No,
            &[],
            "success",
            "abort",
        )
        .await
        .expect_err("different reason should block residual recovery");

        let message = error.to_string();
        assert!(message.contains("partial redirect residue"));
    }

    #[tokio::test]
    async fn step_start_rejects_old_direction_step_before_mutation() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction a", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let added = cmd_step_add(&client, &case_id, "old step", None, false)
            .await
            .expect("step add should succeed");
        let step_id = added["step"]["id"]
            .as_str()
            .expect("step id should exist")
            .to_string();

        cmd_redirect(
            &client,
            &case_id,
            "direction b",
            "need new direction",
            "shift scope",
            GoalDriftFlag::No,
            &[],
            "done",
            "stop",
        )
        .await
        .expect("redirect should succeed");

        let error = cmd_step_start(&client, &case_id, &step_id)
            .await
            .expect_err("old direction step should be rejected");
        assert!(matches!(error, CaseError::StepNotFound(ref id) if id == &step_id));

        let stale_step = client
            .get_step(&step_id)
            .await
            .expect("step should still exist");
        assert_eq!(stale_step.status, StepStatus::Pending);

        let current_case = client.get_case(&case_id).await.expect("case should reload");
        assert_eq!(current_case.current_direction_seq, 2);
        assert_eq!(current_case.current_step_id.as_deref(), None);
    }

    #[tokio::test]
    async fn step_done_and_block_reject_old_direction_step_before_mutation() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(&client, "goal", "direction a", &[], &[], None, None)
            .await
            .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let added = cmd_step_add(&client, &case_id, "old step", None, false)
            .await
            .expect("step add should succeed");
        let step_id = added["step"]["id"]
            .as_str()
            .expect("step id should exist")
            .to_string();

        cmd_redirect(
            &client,
            &case_id,
            "direction b",
            "need new direction",
            "shift scope",
            GoalDriftFlag::No,
            &[],
            "done",
            "stop",
        )
        .await
        .expect("redirect should succeed");

        let done_error = cmd_step_done(&client, &case_id, &step_id)
            .await
            .expect_err("old direction step done should be rejected");
        assert!(matches!(done_error, CaseError::StepNotFound(ref id) if id == &step_id));

        let blocked_error = cmd_step_block(&client, &case_id, &step_id, "blocked")
            .await
            .expect_err("old direction step block should be rejected");
        assert!(matches!(blocked_error, CaseError::StepNotFound(ref id) if id == &step_id));

        let stale_step = client
            .get_step(&step_id)
            .await
            .expect("step should still exist");
        assert_eq!(stale_step.status, StepStatus::Pending);
        assert_eq!(stale_step.reason.as_deref(), None);
    }

    #[tokio::test]
    async fn context_uses_local_provider_by_default() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(
            &client,
            "honcho integration",
            "inspect docs",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        cmd_record(
            &client,
            &case_id,
            "Honcho session context supports token limit",
            "evidence",
            &[],
            &[],
            Some("官方文档言 summary 与 recent messages 混合"),
        )
        .await
        .expect("record should succeed");

        let context = cmd_context(
            &client,
            Some(&case_id),
            ContextScopeArg::Case,
            Some("token limit"),
            Some(3),
            Some(128),
        )
        .await
        .expect("context should succeed");

        assert_eq!(
            context["case_context"]["backend"].as_str(),
            Some("local_text")
        );
        assert_eq!(
            context["case_context"]["query"].as_str(),
            Some("token limit")
        );
        assert!(context["case_context"]["context"]
            .as_str()
            .is_some_and(|text| text.contains("Honcho")));
    }

    #[tokio::test]
    async fn recall_stays_local_when_honcho_flags_are_enabled() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let mut config = temp_db_config(&temp_dir);
        config.honcho_enabled = true;
        config.semantic_recall_enabled = true;
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(
            &client,
            "semantic recall",
            "inspect docs",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        cmd_record(
            &client,
            &case_id,
            "workspace search is future work",
            "finding",
            &[],
            &[],
            Some("recall should still use local text"),
        )
        .await
        .expect("record should succeed");

        let recalled = cmd_recall(
            &client,
            "future work",
            CaseListOptions::new(None, None, None),
        )
        .await
        .expect("recall should succeed");
        assert_eq!(recalled["cases"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn context_fails_fast_on_invalid_honcho_config() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let mut config = temp_db_config(&temp_dir);
        config.honcho_enabled = true;
        config.semantic_recall_enabled = true;
        config.honcho_workspace_id = None;
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(
            &client,
            "semantic context",
            "inspect docs",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let error = cmd_context(
            &client,
            Some(&case_id),
            ContextScopeArg::Case,
            Some("query"),
            Some(5),
            Some(256),
        )
        .await
        .expect_err("invalid honcho config should fail fast");
        assert!(matches!(error, CaseError::HonchoConfig(_)));
    }

    #[tokio::test]
    async fn open_reports_honcho_warning_when_sync_config_invalid() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let mut config = temp_db_config(&temp_dir);
        config.honcho_enabled = true;
        config.honcho_sync_enabled = true;
        config.honcho_workspace_id = None;
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(
            &client,
            "sync warnings",
            "inspect hooks",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("case should open");
        let warnings = opened["warnings"]
            .as_array()
            .expect("warnings should be present");
        assert!(!warnings.is_empty());
    }

    #[tokio::test]
    async fn context_rejects_zero_limit() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened = cmd_open_new(
            &client,
            "semantic context",
            "inspect docs",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("case should open");
        let case_id = opened["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();

        let error = cmd_context(
            &client,
            Some(&case_id),
            ContextScopeArg::Case,
            Some("query"),
            Some(0),
            Some(128),
        )
        .await
        .expect_err("zero limit should be rejected");
        assert!(matches!(error, CaseError::InvalidListOption(_)));
    }

    #[tokio::test]
    async fn repo_scope_context_aggregates_across_sessions_locally() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened_a = cmd_open_new(
            &client,
            "first case goal",
            "inspect docs",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("first case should open");
        let case_a = opened_a["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();
        cmd_record(
            &client,
            &case_a,
            "shared repo memory about vector digest",
            "note",
            &[],
            &[],
            Some("cross session evidence"),
        )
        .await
        .expect("record on first case should succeed");
        let close_token_a = match cmd_close(&client, &case_a, "done", None).await {
            Err(CaseError::CloseConfirmationRequired { confirm_token, .. }) => confirm_token,
            other => panic!("unexpected close response: {other:?}"),
        };
        cmd_close(&client, &case_a, "done", Some(&close_token_a))
            .await
            .expect("first case should close");

        let opened_b = cmd_open_new(
            &client,
            "second case goal",
            "inspect memory",
            &[],
            &[],
            None,
            None,
        )
        .await
        .expect("second case should open");
        let case_b = opened_b["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();
        cmd_record(
            &client,
            &case_b,
            "current session also mentions vector digest",
            "finding",
            &[],
            &[],
            Some("fresh evidence"),
        )
        .await
        .expect("record on current case should succeed");

        let context = cmd_context(
            &client,
            None,
            ContextScopeArg::Repo,
            Some("vector digest"),
            Some(6),
            Some(256),
        )
        .await
        .expect("repo context should succeed");

        assert_eq!(context["case_context"]["scope"].as_str(), Some("repo"));
        assert_eq!(
            context["case_context"]["repo_id"].as_str(),
            Some("aaaaaaaaaaaaaaaa")
        );
        let hits = context["case_context"]["hits"]
            .as_array()
            .expect("hits should exist");
        assert!(hits.len() >= 2);
        assert!(hits
            .iter()
            .any(|hit| hit["case_id"].as_str() == Some(&case_a)));
        assert!(hits
            .iter()
            .any(|hit| hit["case_id"].as_str() == Some(&case_b)));
    }

    #[tokio::test]
    async fn repo_scope_can_be_used_as_default_shape() {
        let temp_dir = TempDir::new().expect("temporary directory should be created");
        let config = temp_db_config(&temp_dir);
        let client = CaseClient::new(
            &config,
            RepoIdentity {
                repo_id: "aaaaaaaaaaaaaaaa".to_string(),
                repo_label: "github.com/example/repo-a".to_string(),
                worktree_id: "1111111111111111".to_string(),
                worktree_root: "/tmp/repo-a".to_string(),
            },
        )
        .await
        .expect("client should initialize");

        let opened_a = cmd_open_new(&client, "goal a", "dir a", &[], &[], None, None)
            .await
            .expect("first case should open");
        let case_a = opened_a["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();
        cmd_record(
            &client,
            &case_a,
            "alpha vector digest",
            "note",
            &[],
            &[],
            None,
        )
        .await
        .expect("record should succeed");
        let close_token_a = match cmd_close(&client, &case_a, "done", None).await {
            Err(CaseError::CloseConfirmationRequired { confirm_token, .. }) => confirm_token,
            other => panic!("unexpected close response: {other:?}"),
        };
        cmd_close(&client, &case_a, "done", Some(&close_token_a))
            .await
            .expect("first case should close");

        let opened_b = cmd_open_new(&client, "goal b", "dir b", &[], &[], None, None)
            .await
            .expect("second case should open");
        let case_b = opened_b["case"]["id"]
            .as_str()
            .expect("case id should exist")
            .to_string();
        cmd_record(
            &client,
            &case_b,
            "beta vector digest",
            "finding",
            &[],
            &[],
            None,
        )
        .await
        .expect("record should succeed");

        let context = cmd_context(
            &client,
            Some(&case_b),
            ContextScopeArg::Repo,
            Some("vector digest"),
            Some(5),
            Some(128),
        )
        .await
        .expect("repo context should succeed");

        let hits = context["case_context"]["hits"]
            .as_array()
            .expect("hits should exist");
        assert!(hits
            .iter()
            .any(|hit| hit["case_id"].as_str() == Some(&case_a)));
        assert!(hits
            .iter()
            .any(|hit| hit["case_id"].as_str() == Some(&case_b)));
    }
}
