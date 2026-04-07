//! MCP server for agpod case workflows.
//!
//! Keywords: mcp, model context protocol, case tools, schema, stdio

mod hive;

use agpod_case::{
    CaseArgs, CaseCommand, CaseStatusArg, ContextScopeArg, GoalDriftFlag, OpenModeArg, RecordKind,
    StepCommand,
};
use anyhow::Result;
use hive::{hive_tool_output_schema, HiveRequest, HiveToolEnvelope, HiveToolResponse};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Implementation, InitializeRequestParams, InitializeResult,
        JsonObject, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler, ServiceExt,
};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone)]
pub struct AgpodMcpServer {
    data_dir: Option<String>,
    server_addr: Option<String>,
    readonly: bool,
    tool_router: ToolRouter<Self>,
}

impl AgpodMcpServer {
    pub fn new() -> Self {
        Self::with_case_config(
            std::env::var("AGPOD_CASE_DATA_DIR").ok(),
            std::env::var("AGPOD_CASE_SERVER_ADDR").ok(),
            false,
        )
    }

    pub fn with_options(
        data_dir: Option<String>,
        server_addr: Option<String>,
        readonly: bool,
    ) -> Self {
        Self::with_case_config(data_dir, server_addr, readonly)
    }

    #[cfg(test)]
    fn with_data_dir(data_dir: Option<String>) -> Self {
        Self::with_case_config(data_dir, None, false)
    }

    #[cfg(test)]
    fn readonly() -> Self {
        Self::with_case_config(None, None, true)
    }

    fn with_case_config(
        data_dir: Option<String>,
        server_addr: Option<String>,
        readonly: bool,
    ) -> Self {
        Self {
            data_dir,
            server_addr,
            readonly,
            tool_router: if readonly {
                Self::readonly_tool_router()
            } else {
                Self::full_tool_router()
            },
        }
    }

    pub async fn serve_stdio(self) -> Result<()> {
        let server = self.serve(rmcp::transport::stdio()).await?;
        server.waiting().await?;
        Ok(())
    }

    async fn run_case_tool(
        &self,
        kind: &'static str,
        command: CaseCommand,
        case_id_hint: Option<String>,
    ) -> Result<CallToolResult, ErrorData> {
        let result = self.run_case_command_raw(command).await?;
        Self::case_tool_result(kind, case_id_hint, result)
    }

    async fn run_case_command_raw(
        &self,
        command: CaseCommand,
    ) -> Result<Map<String, Value>, ErrorData> {
        let args = CaseArgs {
            data_dir: self.data_dir.clone(),
            server_addr: self.server_addr.clone(),
            repo_root: None,
            json: true,
            command,
        };
        let mut result = agpod_case::run_json(args).await;
        if let Some(obj) = result.as_object_mut() {
            obj.remove("_meta");
        }
        let result = result.as_object().cloned().ok_or_else(|| {
            ErrorData::internal_error("agpod-case returned a non-object JSON payload", None)
        })?;
        Ok(result)
    }

    async fn run_case_list_request(
        &self,
        req: CaseListRequest,
    ) -> Result<CallToolResult, ErrorData> {
        validate_list_request(req.limit, req.recent_days)?;

        self.run_case_tool(
            "case_list",
            CaseCommand::List {
                status: req.status.map(Into::into),
                limit: req.limit,
                recent_days: req.recent_days,
            },
            None,
        )
        .await
    }

    async fn run_case_recall_request(
        &self,
        req: CaseRecallRequest,
    ) -> Result<CallToolResult, ErrorData> {
        match req.mode.unwrap_or_default() {
            CaseRecallModeInput::Find => {
                if req.query.as_deref().unwrap_or_default().trim().is_empty() {
                    return Err(ErrorData::invalid_params("query must not be empty", None));
                }
                if req.context_id.is_some()
                    || req.context_scope.is_some()
                    || req.context_shortcut.is_some()
                    || req.context_limit.is_some()
                    || req.context_token_limit.is_some()
                {
                    return Err(ErrorData::invalid_params(
                        "`context_*` fields are only supported when mode=`context`",
                        None,
                    ));
                }
                validate_list_request(req.find_limit, req.find_recent_days)?;
                self.run_case_tool(
                    "case_recall",
                    CaseCommand::Recall {
                        query: req.query.unwrap_or_default(),
                        status: req.find_status.map(Into::into),
                        limit: req.find_limit,
                        recent_days: req.find_recent_days,
                    },
                    None,
                )
                .await
            }
            CaseRecallModeInput::Context => {
                if req.context_shortcut.is_some()
                    && !req.query.as_deref().unwrap_or_default().trim().is_empty()
                {
                    return Err(ErrorData::invalid_params(
                        "`query` cannot be combined with `context_shortcut`; use one or the other",
                        None,
                    ));
                }
                let resolved_query = match req.context_shortcut {
                    Some(CaseContextShortcutInput::RecentWork) => {
                        "Summarize the most recent work completed or in progress in this repository. Focus on latest steps, findings, decisions, blockers, and next actions.".to_string()
                    }
                    None => {
                        if req.query.as_deref().unwrap_or_default().trim().is_empty() {
                            return Err(ErrorData::invalid_params(
                                "query must not be empty",
                                None,
                            ));
                        }
                        req.query.clone().unwrap_or_default()
                    }
                };
                if req.find_status.is_some()
                    || req.find_limit.is_some()
                    || req.find_recent_days.is_some()
                {
                    return Err(ErrorData::invalid_params(
                        "`find_*` fields are only supported when mode=`find`",
                        None,
                    ));
                }
                if matches!(req.context_limit, Some(0)) {
                    return Err(ErrorData::invalid_params(
                        "context_limit must be at least 1",
                        None,
                    ));
                }
                let context_scope = match req.context_shortcut {
                    Some(CaseContextShortcutInput::RecentWork) => CaseContextScopeInput::Repo,
                    None => req.context_scope.unwrap_or(CaseContextScopeInput::Repo),
                };
                if matches!(context_scope, CaseContextScopeInput::Case)
                    && req
                        .context_id
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(ErrorData::invalid_params(
                        "`context_id` is required when mode=`context` and context_scope=`case`",
                        None,
                    ));
                }
                let case_id_hint = match context_scope {
                    CaseContextScopeInput::Case => req.context_id.clone(),
                    CaseContextScopeInput::Repo => None,
                };
                self.run_case_tool(
                    "case_recall",
                    CaseCommand::Context {
                        id: req.context_id,
                        scope: context_scope.into(),
                        query: Some(resolved_query),
                        limit: req.context_limit,
                        token_limit: req.context_token_limit,
                    },
                    case_id_hint,
                )
                .await
            }
        }
    }

    async fn run_hive_request(&self, req: HiveRequest) -> Result<CallToolResult, ErrorData> {
        let result = hive::run_hive_request(req).await?;
        HiveToolResponse {
            result: HiveToolEnvelope::from_raw(result),
        }
        .into_call_tool_result()
    }

    fn case_tool_result(
        kind: &'static str,
        case_id_hint: Option<String>,
        result: Map<String, Value>,
    ) -> Result<CallToolResult, ErrorData> {
        ToolResponse {
            result: ToolEnvelope::from_raw(kind, case_id_hint, result),
        }
        .into_call_tool_result()
    }
}

impl Default for AgpodMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

fn case_tool_output_schema() -> Arc<JsonObject> {
    static SCHEMA: OnceLock<Arc<JsonObject>> = OnceLock::new();

    SCHEMA
        .get_or_init(|| {
            Arc::new(
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "result": {
                            "type": "object",
                            "properties": {
                                "kind": {
                                    "type": "string",
                                    "description": "Stable result kind matching the MCP tool name."
                                },
                                "case_id": {
                                    "type": ["string", "null"],
                                    "description": "Case ID when one is known from the request or payload."
                                },
                                "state": {
                                    "type": ["string", "null"],
                                    "description": "Stable high-level state derived from the case payload."
                                },
                                "message": {
                                    "type": ["string", "null"],
                                    "description": "Human-readable message from the underlying case payload, usually on errors."
                                },
                                "raw": {
                                    "type": "object",
                                    "description": "Original agpod case JSON payload.",
                                    "additionalProperties": true
                                }
                            },
                            "required": ["kind", "raw"]
                        }
                    },
                    "required": ["result"],
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "title": "ToolResponse"
                })
                .as_object()
                .expect("output schema should be an object")
                .clone(),
            )
        })
        .clone()
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgpodMcpServer {
    fn get_info(&self) -> ServerInfo {
        let instructions = if self.readonly {
            "agpod case MCP (read-only mode). `case_current`, `case_show`, `case_list`, and `case_recall` are available. They never mutate case state. No tool in this server can open a case, append records, change steps, redirect, finish, resume by id, or otherwise mutate case state."
        } else {
            "agpod case MCP. One open case per repo. First evaluate whether the current task actually needs case tracking; do not call `case_current` or `case_open` by default for trivial or one-off work. Once you decide the task should use case tracking, call `case_current` to inspect active state. If it reports an open case, call `case_resume` before mutating anything; use `case_show` when you need the full case tree and step history. If there is no open case and the task merits one, use `case_open` with `mode=new` to create one, or `mode=reopen` plus `case_id` to reopen a closed or abandoned case. In `mode=new`, `needed_context_query` is optional startup memory input that can ask for how-to topics, docs, pitfalls, and known patterns; it may return `startup_context` with status `ok`, `empty`, or `degraded`, but open itself should still succeed. Use `case_steps_add` to add steps, `case_step_advance` to complete the active step and optionally start the next one, `case_step_mark_as` only to start or block a step, and `case_step_move` to reorder steps. Use `case_record` only for factual notes, evidence, blockers, or goal-constraint updates; use `case_decide` for decisions that require a reason; use `case_redirect` only when the goal is still the same. Use `case_recall` as the unified retrieval entrypoint: use `mode=find` with `query`, `find_status`, `find_limit`, and `find_recent_days` to discover past cases, or `mode=context` with `context_scope=case|repo`, `context_id`, `query`, `context_shortcut`, and `context_token_limit` to get semantic context. In `mode=context`, `query` states the retrieval focus; omit `query` only when `context_shortcut=recent_work`. When `context_scope=case`, `context_id` is required. `context_shortcut=recent_work` is the built-in shortcut for recent repository work. Use `case_finish` to complete or abandon a case; first call it without `confirm_token`, then retry only with the returned token if closing is truly intended. `hive` manages tmux-backed worker sessions for the current tmux pane only: `ensure_session` creates or reuses the pane-derived session, `spawn_agent` opens one worker per window up to 5, `list_agents` reports live workers, `send_prompt` sends a prompt to a chosen worker, and `reset_agent` sends `/new` then waits for session-start hooks to mark the worker idle again. Tool results return structured JSON aligned with `agpod case --json`; prefer stable fields like `result.kind`, `result.case_id`, `result.state`, and `result.raw` when chaining tools."
        };
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
            .with_server_info(Implementation::from_build_env())
            .with_instructions(instructions)
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        Ok(self.get_info())
    }
}

#[tool_router(router = full_tool_router)]
impl AgpodMcpServer {
    #[tool(
        name = "hive",
        description = "Manage a tmux hive session derived from the current tmux pane. Use action=`ensure_session` first, then `spawn_agent`, `list_agents`, `send_prompt`, or `reset_agent` against that session.",
        output_schema = hive_tool_output_schema()
    )]
    async fn hive(
        &self,
        Parameters(req): Parameters<HiveRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_hive_request(req).await
    }

    #[tool(
        name = "case_current",
        description = "Read active case state after you have decided the task should use case tracking, or when resuming known case-aware work.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_current(
        &self,
        Parameters(_req): Parameters<CaseCurrentRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool("case_current", CaseCommand::Current { state: false }, None)
            .await
    }

    #[tool(
        name = "case_open",
        description = "Open a case only after you have decided the task merits case tracking. Use `mode=new` to create a fresh case when `case_current` shows none is open. Use `mode=reopen` with `case_id` to reopen a previously closed or abandoned case. Never call this if another case is already open for the repo.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_open(
        &self,
        Parameters(req): Parameters<CaseOpenRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        if matches!(req.mode, CaseOpenModeInput::Reopen) && req.needed_context_query.is_some() {
            return Err(ErrorData::invalid_params(
                "`needed_context_query` is only allowed when mode is `new`",
                None,
            ));
        }

        self.run_case_tool(
            "case_open",
            CaseCommand::Open {
                mode: match req.mode {
                    CaseOpenModeInput::New => OpenModeArg::New,
                    CaseOpenModeInput::Reopen => OpenModeArg::Reopen,
                },
                case_id: req.case_id,
                goal: req.goal,
                direction: req.direction,
                goal_constraints: encode_constraints(req.goal_constraints),
                constraints: encode_constraints(req.constraints),
                success_condition: req.success_condition,
                abort_condition: req.abort_condition,
                how_to: req
                    .needed_context_query
                    .as_ref()
                    .map(|query| query.how_to.clone())
                    .unwrap_or_default(),
                doc_about: req
                    .needed_context_query
                    .as_ref()
                    .map(|query| query.doc_about.clone())
                    .unwrap_or_default(),
                pitfalls_about: req
                    .needed_context_query
                    .as_ref()
                    .map(|query| query.pitfalls_about.clone())
                    .unwrap_or_default(),
                known_patterns_for: req
                    .needed_context_query
                    .as_ref()
                    .map(|query| query.known_patterns_for.clone())
                    .unwrap_or_default(),
            },
            None,
        )
        .await
    }

    #[tool(
        name = "case_record",
        description = "Append a fact to an open case. Not for decisions or redirects.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_record(
        &self,
        Parameters(req): Parameters<CaseRecordRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool(
            "case_record",
            CaseCommand::Record {
                id: req.id.clone(),
                summary: req.summary,
                kind: req
                    .kind
                    .map(|kind| kind.as_str().to_string())
                    .unwrap_or_else(|| RecordKind::Note.as_str().to_string()),
                goal_constraints: encode_constraints(req.goal_constraints),
                files: req.files.map(|files| files.join(",")),
                context: req.context,
            },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_decide",
        description = "Record an in-direction decision on an open case.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_decide(
        &self,
        Parameters(req): Parameters<CaseDecideRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool(
            "case_decide",
            CaseCommand::Decide {
                id: req.id.clone(),
                summary: req.summary,
                reason: req.reason,
            },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_redirect",
        description = "Change direction on an open case only when the work still fits the same immutable goal. If the work has drifted from the goal, set `is_drift_from_goal` to `yes` and open a new case instead of redirecting.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_redirect(
        &self,
        Parameters(req): Parameters<CaseRedirectRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool(
            "case_redirect",
            CaseCommand::Redirect {
                id: req.id.clone(),
                direction: req.direction,
                reason: req.reason,
                context: req.context,
                is_drift_from_goal: match req.is_drift_from_goal {
                    GoalDriftInput::Yes => GoalDriftFlag::Yes,
                    GoalDriftInput::No => GoalDriftFlag::No,
                },
                constraints: encode_constraints(req.constraints),
                success_condition: req.success_condition,
                abort_condition: req.abort_condition,
            },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_show",
        description = "Show case tree and step history. Use after `case_current` when needed.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_show(
        &self,
        Parameters(req): Parameters<CaseShowRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool("case_show", CaseCommand::Show { id: req.id }, None)
            .await
    }

    #[tool(
        name = "case_finish",
        description = "End an open case. Use outcome \"completed\" when the goal is met, or \"abandoned\" when no longer worth pursuing. First call without `confirm_token` to request confirmation; only retry with the returned token if ending the case is truly intended.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_finish(
        &self,
        Parameters(req): Parameters<CaseFinishRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let command = match req.outcome {
            CaseFinishOutcomeInput::Completed => CaseCommand::Close {
                id: req.id.clone(),
                summary: req.summary,
                confirm_token: req.confirm_token,
            },
            CaseFinishOutcomeInput::Abandoned => CaseCommand::Abandon {
                id: req.id.clone(),
                summary: req.summary,
                confirm_token: req.confirm_token,
            },
        };
        self.run_case_tool("case_finish", command, Some(req.id))
            .await
    }

    #[tool(
        name = "case_list",
        description = "List repo cases with optional status, recency, and limit filters. Safe discovery call.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_list(
        &self,
        Parameters(req): Parameters<CaseListRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_list_request(req).await
    }

    #[tool(
        name = "case_recall",
        description = "Unified case retrieval entrypoint. Use `mode=find` plus `query` to discover past cases. Use `mode=context` to build semantic context: provide `query` to state the retrieval focus, or omit `query` only when `context_shortcut=recent_work`. When `context_scope=case`, `context_id` is required.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_recall(
        &self,
        Parameters(req): Parameters<CaseRecallRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_recall_request(req).await
    }

    #[tool(
        name = "case_resume",
        description = "Get a handoff summary for an open case or a chosen case.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_resume(
        &self,
        Parameters(req): Parameters<CaseShowRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool("case_resume", CaseCommand::Resume { id: req.id }, None)
            .await
    }

    #[tool(
        name = "case_steps_add",
        description = "Add one or more steps to the current direction. Use after `case_open` or `case_redirect`. This batch call may partially succeed; inspect `created_steps`, `created_count`, and any failure details before retrying.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_steps_add(
        &self,
        Parameters(req): Parameters<CaseStepsAddRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        if req.steps.is_empty() {
            return Err(ErrorData::invalid_params(
                "steps array must not be empty",
                None,
            ));
        }

        let case_id = req.id.clone();
        let mut created_steps = Vec::new();
        let mut last_success = None;
        let commands: Vec<CaseCommand> = req
            .steps
            .iter()
            .map(|step| CaseCommand::Step {
                command: StepCommand::Add {
                    id: case_id.clone(),
                    title: step.title().to_string(),
                    reason: step.reason().map(ToOwned::to_owned),
                    start: step.start(),
                },
            })
            .collect();
        let results = agpod_case::run_json_batch(
            self.data_dir.clone(),
            self.server_addr.clone(),
            None,
            commands,
        )
        .await;

        for (index, (step, mut result)) in req.steps.into_iter().zip(results).enumerate() {
            if let Some(obj) = result.as_object_mut() {
                obj.remove("_meta");
            }
            let result = result.as_object().cloned().ok_or_else(|| {
                ErrorData::internal_error("agpod-case returned a non-object JSON payload", None)
            })?;

            if result.get("ok").and_then(Value::as_bool) == Some(true) {
                if let Some(created) = result.get("step").cloned() {
                    created_steps.push(created);
                }
                last_success = Some(result);
                continue;
            }

            let partial =
                build_case_steps_add_partial_error(index + 1, step, created_steps, result);
            return Self::case_tool_result("case_steps_add", Some(case_id), partial);
        }

        let result =
            build_case_steps_add_success(created_steps, last_success.expect("checked non-empty"));
        Self::case_tool_result("case_steps_add", Some(case_id), result)
    }

    #[tool(
        name = "case_step_mark_as",
        description = "Start or block a step. To complete the active step, use `case_step_advance` instead.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_step_mark_as(
        &self,
        Parameters(req): Parameters<CaseStepMarkAsRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let command = match req.status {
            StepStatusInput::Started => StepCommand::Start {
                id: req.id.clone(),
                step_id: req.step_id,
            },
            StepStatusInput::Blocked => StepCommand::Block {
                id: req.id.clone(),
                step_id: req.step_id,
                reason: req.reason.unwrap_or_default(),
            },
        };
        self.run_case_tool(
            "case_step_mark_as",
            CaseCommand::Step { command },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_step_advance",
        description = "Complete the current active step, optionally append one factual record, and optionally start the next step in one call.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_step_advance(
        &self,
        Parameters(req): Parameters<CaseStepAdvanceRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        if req.next_step_id.is_some() && req.next_step_auto {
            return Err(ErrorData::invalid_params(
                "`next_step_id` and `next_step_auto` cannot be combined",
                None,
            ));
        }
        if let Some(record) = req.record.as_ref() {
            if matches!(record.kind, Some(RecordKind::GoalConstraintUpdate)) {
                return Err(ErrorData::invalid_params(
                    "`record.kind` must be one of `note`, `finding`, `evidence`, `blocker`",
                    None,
                ));
            }
        }

        self.run_case_tool(
            "case_step_advance",
            CaseCommand::Step {
                command: StepCommand::Advance {
                    id: req.id.clone(),
                    step_id: req.step_id,
                    record_summary: req.record.as_ref().map(|record| record.summary.clone()),
                    record_kind: req
                        .record
                        .as_ref()
                        .and_then(|record| record.kind.map(|kind| kind.as_str().to_string())),
                    record_files: req
                        .record
                        .as_ref()
                        .map(|record| record.files.clone())
                        .unwrap_or_default(),
                    record_context: req.record.and_then(|record| record.context),
                    next_step_id: req.next_step_id,
                    next_step_auto: req.next_step_auto,
                },
            },
            req.id,
        )
        .await
    }

    #[tool(
        name = "case_step_move",
        description = "Reorder steps within the current direction.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_step_move(
        &self,
        Parameters(req): Parameters<CaseStepMoveRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool(
            "case_step_move",
            CaseCommand::Step {
                command: StepCommand::Move {
                    id: req.id.clone(),
                    step_id: req.step_id,
                    before: req.before,
                },
            },
            Some(req.id),
        )
        .await
    }
}

#[tool_router(router = readonly_tool_router)]
impl AgpodMcpServer {
    #[tool(
        name = "case_current",
        description = "Read active case state for the current open case in this repository.",
        output_schema = case_tool_output_schema()
    )]
    async fn readonly_case_current(
        &self,
        Parameters(_req): Parameters<CaseCurrentRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool("case_current", CaseCommand::Current { state: false }, None)
            .await
    }

    #[tool(
        name = "case_show",
        description = "Show detailed history for the current open case only.",
        output_schema = case_tool_output_schema()
    )]
    async fn readonly_case_show(
        &self,
        Parameters(_req): Parameters<CaseCurrentRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool("case_show", CaseCommand::Show { id: None }, None)
            .await
    }

    #[tool(
        name = "case_list",
        description = "List repo cases with optional status, recency, and limit filters. Safe discovery call.",
        output_schema = case_tool_output_schema()
    )]
    async fn readonly_case_list(
        &self,
        Parameters(req): Parameters<CaseListRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_list_request(req).await
    }

    #[tool(
        name = "case_recall",
        description = "Unified case retrieval entrypoint. Use `mode=find` plus `query` to discover past cases. Use `mode=context` to build semantic context: provide `query` to state the retrieval focus, or omit `query` only when `context_shortcut=recent_work`. When `context_scope=case`, `context_id` is required.",
        output_schema = case_tool_output_schema()
    )]
    async fn readonly_case_recall(
        &self,
        Parameters(req): Parameters<CaseRecallRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_recall_request(req).await
    }
}

fn encode_constraints(constraints: Vec<ConstraintInput>) -> Vec<String> {
    constraints
        .into_iter()
        .map(|constraint| {
            serde_json::to_string(&constraint.into_constraint())
                .expect("constraint should serialize")
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
#[schemars(
    title = "ConstraintInput",
    description = "Accepts either a plain rule string or an object {\"rule\": \"...\", \"reason\": \"...\"}."
)]
pub enum ConstraintInput {
    /// Short form: just the rule text.
    Short(String),
    /// Detailed form: explicit rule plus optional rationale.
    Detailed(ConstraintDetailInput),
}

impl ConstraintInput {
    fn into_constraint(self) -> Value {
        match self {
            Self::Short(rule) => serde_json::json!({
                "rule": rule,
                "reason": null
            }),
            Self::Detailed(detail) => serde_json::json!({
                "rule": detail.rule,
                "reason": detail.reason
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConstraintDetailInput {
    /// Constraint rule text.
    #[schemars(title = "Constraint Rule")]
    pub rule: String,
    /// Optional rationale for the rule.
    #[schemars(title = "Constraint Reason")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ToolResponse {
    pub result: ToolEnvelope,
}

impl ToolResponse {
    fn into_call_tool_result(self) -> Result<CallToolResult, ErrorData> {
        let is_error = self.result.is_error();
        let text = self
            .result
            .message
            .clone()
            .unwrap_or_else(|| self.result.kind.clone());
        structured_tool_result(self, text, is_error)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ToolEnvelope {
    #[serde(skip)]
    is_error: bool,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub raw: Map<String, Value>,
}

impl ToolEnvelope {
    fn from_raw(kind: &str, case_id_hint: Option<String>, raw: Map<String, Value>) -> Self {
        let ok = raw.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let case_id = case_id_hint.or_else(|| extract_case_id(&raw));
        let state = extract_state(&raw, ok);
        let message = raw
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        Self {
            is_error: !ok,
            kind: kind.to_string(),
            case_id,
            state,
            message,
            raw,
        }
    }

    fn is_error(&self) -> bool {
        self.is_error
    }
}

fn extract_case_id(raw: &Map<String, Value>) -> Option<String> {
    raw.get("case")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            raw.get("resume")
                .and_then(|value| value.get("case_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            raw.get("context")
                .and_then(|value| value.get("active_case_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn extract_state(raw: &Map<String, Value>, ok: bool) -> Option<String> {
    raw.get("state")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            if !ok
                && raw
                    .get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message == "no open case in this repository")
            {
                Some("none".to_string())
            } else if !ok {
                Some("error".to_string())
            } else {
                None
            }
        })
        .or_else(|| {
            raw.get("case")
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            if raw.get("resume").is_some() {
                Some("resume".to_string())
            } else if raw.get("cases").is_some() {
                Some("list".to_string())
            } else if raw.get("step").is_some() || raw.get("steps").is_some() {
                Some("step".to_string())
            } else if ok {
                Some("ok".to_string())
            } else {
                Some("error".to_string())
            }
        })
}

fn build_case_steps_add_success(
    created_steps: Vec<Value>,
    last_result: Map<String, Value>,
) -> Map<String, Value> {
    let created_count = created_steps.len() as u64;
    let mut raw = Map::new();
    raw.insert("ok".to_string(), Value::Bool(true));
    raw.insert("created_steps".to_string(), Value::Array(created_steps));
    raw.insert("created_count".to_string(), Value::from(created_count));
    copy_case_steps_add_passthrough_fields(&mut raw, &last_result);
    raw
}

fn build_case_steps_add_partial_error(
    failed_index: usize,
    failed_step: StepInput,
    created_steps: Vec<Value>,
    failed_result: Map<String, Value>,
) -> Map<String, Value> {
    let failed_message = failed_result
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("step add failed");
    let created_count = created_steps.len() as u64;

    let mut raw = Map::new();
    raw.insert("ok".to_string(), Value::Bool(false));
    raw.insert(
        "message".to_string(),
        Value::String(format!(
            "case_steps_add failed at step {failed_index}: {failed_message}"
        )),
    );
    raw.insert("created_steps".to_string(), Value::Array(created_steps));
    raw.insert("created_count".to_string(), Value::from(created_count));
    raw.insert("failed_index".to_string(), Value::from(failed_index as u64));
    raw.insert(
        "failed_input".to_string(),
        serde_json::to_value(failed_step).expect("step input should serialize"),
    );
    raw.insert("failure".to_string(), Value::Object(failed_result.clone()));
    copy_case_steps_add_passthrough_fields(&mut raw, &failed_result);
    raw
}

fn copy_case_steps_add_passthrough_fields(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
) {
    for key in ["steps", "context", "next"] {
        if let Some(value) = source.get(key).cloned() {
            target.insert(key.to_string(), value);
        }
    }
}

fn describe_case_record_kind_schema(schema: &mut schemars::Schema) {
    schema.ensure_object().insert(
        "description".to_string(),
        Value::String(format!(
            "Kind of record to append. Supported values: {}. Omit this field to default to `note`. `decision` is not allowed here; use `case_decide` instead.",
            RecordKind::allowed_values_code_span()
        )),
    );
}

fn describe_case_open_request_schema(_schema: &mut schemars::Schema) {
    // Conditional validation (mode=new requires goal+direction, mode=reopen
    // requires case_id) is enforced server-side. Schema-level allOf/if-then
    // removed for compatibility with providers that reject top-level allOf.
    // `needed_context_query` is optional startup memory input and can be
    // combined with normal open fields in mode=new.
}

fn describe_case_record_request_schema(_schema: &mut schemars::Schema) {
    // Conditional validation (kind=goal_constraint_update requires non-empty
    // goal_constraints) is enforced server-side. Schema-level allOf/if-then
    // removed for compatibility with providers that reject top-level allOf.
}

fn describe_case_step_mark_as_request_schema(_schema: &mut schemars::Schema) {
    // Conditional validation (status=blocked requires reason) is enforced
    // server-side. Schema-level allOf/if-then removed for compatibility
    // with providers that reject top-level allOf.
}

fn deserialize_optional_record_kind<'de, D>(deserializer: D) -> Result<Option<RecordKind>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    match raw.as_deref() {
        None => Ok(None),
        Some("decision") => Err(D::Error::custom(
            "invalid record kind `decision`; use `case_decide` because decisions require a reason",
        )),
        Some(value) => value.parse::<RecordKind>().map(Some).map_err(|_| {
            D::Error::custom(format!(
                "invalid record kind `{value}`; expected one of {}",
                RecordKind::allowed_values_code_span()
            ))
        }),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct CaseCurrentRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(transform = describe_case_open_request_schema)]
pub struct CaseOpenRequest {
    /// Open mode: create a new case or reopen an existing closed/abandoned case.
    #[serde(default)]
    pub mode: CaseOpenModeInput,
    /// Existing case ID to reopen. Required when `mode` is `reopen`.
    pub case_id: Option<String>,
    /// Immutable case goal. Required when `mode` is `new`.
    pub goal: Option<String>,
    /// Initial direction summary. Required when `mode` is `new`.
    pub direction: Option<String>,
    /// Case-wide constraints. Accepts either plain strings like `"先证据后推断"` or objects like `{"rule":"先证据后推断","reason":"避免过早下结论"}`.
    #[serde(default)]
    pub goal_constraints: Vec<ConstraintInput>,
    /// Direction-local constraints. Accepts either plain strings like `"先证据后推断"` or objects like `{"rule":"先证据后推断","reason":"避免过早下结论"}`.
    #[serde(default)]
    pub constraints: Vec<ConstraintInput>,
    /// Condition for success on this direction.
    pub success_condition: Option<String>,
    /// Condition for aborting this direction.
    pub abort_condition: Option<String>,
    /// Startup memory query requested by the opening agent.
    pub needed_context_query: Option<NeededContextQueryInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct NeededContextQueryInput {
    /// How-to topics the new case should know at startup.
    #[serde(default)]
    pub how_to: Vec<String>,
    /// Document topics worth reading first.
    #[serde(default)]
    pub doc_about: Vec<String>,
    /// Pitfall topics to avoid early.
    #[serde(default)]
    pub pitfalls_about: Vec<String>,
    /// Known working pattern topics to inherit.
    #[serde(default)]
    pub known_patterns_for: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum CaseOpenModeInput {
    #[default]
    New,
    Reopen,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(transform = describe_case_record_request_schema)]
pub struct CaseRecordRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// Fact summary.
    pub summary: String,
    /// Kind of record to append.
    #[serde(default, deserialize_with = "deserialize_optional_record_kind")]
    #[schemars(transform = describe_case_record_kind_schema)]
    pub kind: Option<RecordKind>,
    /// Goal constraint payloads. Required and non-empty when `kind` is `goal_constraint_update`; otherwise omit this field.
    #[serde(default)]
    pub goal_constraints: Vec<ConstraintInput>,
    /// Related file paths.
    pub files: Option<Vec<String>>,
    /// Extra context.
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseDecideRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// Decision summary.
    pub summary: String,
    /// Why this decision was made.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseRedirectRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// New direction summary.
    pub direction: String,
    /// Why direction changed.
    pub reason: String,
    /// Context carried from prior work.
    pub context: String,
    /// Required explicit check for goal drift. Use `no` only when the redirect still serves the same immutable case goal. Use `yes` when the work has drifted; the tool will reject the redirect and you should open a new case instead.
    pub is_drift_from_goal: GoalDriftInput,
    /// New direction constraints. Accepts either plain strings like `"先证据后推断"` or objects like `{"rule":"先证据后推断","reason":"避免过早下结论"}`.
    #[serde(default)]
    pub constraints: Vec<ConstraintInput>,
    /// Condition for success on the new direction.
    pub success_condition: String,
    /// Condition for aborting the new direction.
    pub abort_condition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GoalDriftInput {
    Yes,
    No,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseShowRequest {
    /// Case ID. Omit to use the open case returned by `case_current`.
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CaseFinishOutcomeInput {
    Completed,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseFinishRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// Outcome for closing the case.
    pub outcome: CaseFinishOutcomeInput,
    /// Closing or abandonment summary.
    pub summary: String,
    /// Confirmation token returned by a previous `case_finish` attempt. Omit on the first call to request confirmation.
    pub confirm_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseRecallRequest {
    /// Retrieval mode: discover matching cases or assemble semantic context.
    pub mode: Option<CaseRecallModeInput>,
    /// Search query. Required for `mode=find`. Also required for `mode=context` to state retrieval focus, unless `context_shortcut` is used.
    pub query: Option<String>,
    /// Case ID. Used when `mode=context` and `context_scope=case`.
    pub context_id: Option<String>,
    /// Context retrieval scope. Used when `mode=context`.
    pub context_scope: Option<CaseContextScopeInput>,
    /// Shortcut for common `mode=context` retrieval patterns.
    pub context_shortcut: Option<CaseContextShortcutInput>,
    /// Optional case status filter for `mode=find`.
    pub find_status: Option<CaseStatusInput>,
    /// Limit result count for `mode=find`. Must be at least 1 when provided.
    pub find_limit: Option<usize>,
    /// Only include cases updated within the last N days for `mode=find`. Must be at least 1 when provided.
    pub find_recent_days: Option<u32>,
    /// Limit result count for `mode=context`. Must be at least 1 when provided.
    pub context_limit: Option<usize>,
    /// Optional token budget for returned context when `mode=context`.
    pub context_token_limit: Option<u32>,
}

impl schemars::JsonSchema for CaseRecallRequest {
    fn schema_name() -> Cow<'static, str> {
        "CaseRecallRequest".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::CaseRecallRequest").into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Conditional validation (mode-dependent required fields, mutual
        // exclusion of find_* vs context_* params) is enforced server-side.
        // Top-level allOf and nested oneOf removed for compatibility with
        // providers that reject these keywords.
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "description": "Retrieval mode: discover matching cases or assemble semantic context.",
                    "default": "find",
                    "enum": ["find", "context"]
                },
                "query": {
                    "type": "string",
                    "description": "Search query. Required for `mode=find`. Also required for `mode=context` to state retrieval focus, unless `context_shortcut` is used."
                },
                "context_id": {
                    "type": "string",
                    "description": "Case ID. Required when `mode=context` and `context_scope=case`."
                },
                "context_scope": {
                    "type": "string",
                    "description": "Context retrieval scope. Used only when `mode=context`.",
                    "enum": ["case", "repo"]
                },
                "context_shortcut": {
                    "type": "string",
                    "description": "Shortcut for common `mode=context` retrieval patterns. When set to `recent_work`, `query` is optional.",
                    "enum": ["recent_work"]
                },
                "find_status": {
                    "type": "string",
                    "description": "Optional case status filter. Used only when `mode=find`.",
                    "enum": ["open", "closed", "abandoned"]
                },
                "find_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Limit result count for `mode=find`. Must be at least 1 when provided."
                },
                "find_recent_days": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Only include cases updated within the last N days for `mode=find`. Must be at least 1 when provided."
                },
                "context_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Limit result count for `mode=context`. Must be at least 1 when provided."
                },
                "context_token_limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional token budget for returned context when `mode=context`."
                }
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CaseContextScopeInput {
    Case,
    Repo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaseContextShortcutInput {
    RecentWork,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaseRecallModeInput {
    #[default]
    Find,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct CaseListRequest {
    /// Optional case status filter.
    pub status: Option<CaseStatusInput>,
    /// Limit result count. Must be at least 1 when provided.
    #[schemars(range(min = 1))]
    pub limit: Option<usize>,
    /// Only include cases updated within the last N days. Must be at least 1 when provided.
    #[schemars(range(min = 1))]
    pub recent_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CaseStatusInput {
    Open,
    Closed,
    Abandoned,
}

impl From<CaseStatusInput> for CaseStatusArg {
    fn from(value: CaseStatusInput) -> Self {
        match value {
            CaseStatusInput::Open => CaseStatusArg::Open,
            CaseStatusInput::Closed => CaseStatusArg::Closed,
            CaseStatusInput::Abandoned => CaseStatusArg::Abandoned,
        }
    }
}

impl From<CaseContextScopeInput> for ContextScopeArg {
    fn from(value: CaseContextScopeInput) -> Self {
        match value {
            CaseContextScopeInput::Case => ContextScopeArg::Case,
            CaseContextScopeInput::Repo => ContextScopeArg::Repo,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepsAddRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// Steps to add. Must be non-empty. Accepts either plain strings like `"审阅报表"` or objects like `{"title":"审阅报表","reason":"补证","start":true}`.
    #[schemars(length(min = 1))]
    pub steps: Vec<StepInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum StepInput {
    Text(String),
    Detailed(StepObjectInput),
}

impl StepInput {
    fn title(&self) -> &str {
        match self {
            Self::Text(title) => title.as_str(),
            Self::Detailed(step) => step.title.as_str(),
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Text(_) => None,
            Self::Detailed(step) => step.reason.as_deref(),
        }
    }

    fn start(&self) -> bool {
        match self {
            Self::Text(_) => false,
            Self::Detailed(step) => step.start,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StepObjectInput {
    /// Step title.
    #[schemars(length(min = 1))]
    pub title: String,
    /// Why this step is needed.
    pub reason: Option<String>,
    /// Start the step immediately after creating it.
    #[serde(default)]
    pub start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StepStatusInput {
    Started,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(transform = describe_case_step_mark_as_request_schema)]
pub struct CaseStepMarkAsRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// Step ID from the case step list, such as `steps.current.id` or one entry in `steps.ordered`.
    pub step_id: String,
    /// Target status for the step. Use `case_step_advance` instead of `done`.
    pub status: StepStatusInput,
    /// Required and non-empty when `status` is `blocked`. Explains why the step cannot proceed.
    #[schemars(length(min = 1))]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepMoveRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// Step ID to move.
    pub step_id: String,
    /// Insert the moved step immediately before this step ID.
    pub before: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepAdvanceRecordInput {
    /// Fact summary recorded while advancing.
    pub summary: String,
    /// Record kind. Allowed: `note`, `finding`, `evidence`, `blocker`.
    #[serde(default, deserialize_with = "deserialize_optional_record_kind")]
    #[schemars(transform = describe_case_record_kind_schema)]
    pub kind: Option<RecordKind>,
    /// Related file paths.
    #[serde(default)]
    pub files: Vec<String>,
    /// Extra context.
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepAdvanceRequest {
    /// Case ID. Omit to use the open case in this repository.
    pub id: Option<String>,
    /// Step ID. Omit to use the current active step.
    pub step_id: Option<String>,
    /// Optional record appended while advancing.
    pub record: Option<CaseStepAdvanceRecordInput>,
    /// Explicit next step to start after completion.
    pub next_step_id: Option<String>,
    /// Start the next pending step automatically by order.
    #[serde(default)]
    pub next_step_auto: bool,
}

fn structured_tool_result<T>(
    payload: T,
    text: String,
    is_error: bool,
) -> Result<CallToolResult, ErrorData>
where
    T: Serialize,
{
    let value = serde_json::to_value(&payload).map_err(|err| {
        ErrorData::internal_error(format!("Failed to serialize MCP tool result: {err}"), None)
    })?;
    let mut result = if is_error {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    };
    result.content = vec![Content::text(text)];

    Ok(result)
}

fn validate_list_request(limit: Option<usize>, recent_days: Option<u32>) -> Result<(), ErrorData> {
    if matches!(limit, Some(0)) {
        return Err(ErrorData::invalid_params("limit must be at least 1", None));
    }

    if matches!(recent_days, Some(0)) {
        return Err(ErrorData::invalid_params(
            "recent_days must be at least 1",
            None,
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_router_exposes_case_tools() {
        let server = AgpodMcpServer::new();
        let tools = server.tool_router.list_all();
        let tool_names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();

        assert!(tool_names.contains(&"case_current"));
        assert!(tool_names.contains(&"hive"));
        assert!(tool_names.contains(&"case_open"));
        assert!(tool_names.contains(&"case_step_advance"));
        assert!(tool_names.contains(&"case_steps_add"));
        let current_tool = tools
            .iter()
            .find(|tool| tool.name == "case_current")
            .expect("case_current tool should exist");
        assert!(current_tool.output_schema.is_some());
        let schema = serde_json::to_string(current_tool.output_schema.as_ref().unwrap())
            .expect("schema should serialize");
        assert!(schema.contains("\"kind\""));
        assert!(schema.contains("\"raw\""));

        let info = server.get_info();
        assert!(info.instructions.is_some());
        let instructions = info.instructions.expect("instructions should exist");
        assert!(instructions.contains("case_current"));
        assert!(instructions.contains("case_resume"));
        assert!(instructions.contains("mode=context"));
        assert!(instructions.contains("find_status"));
        assert!(instructions.contains("context_scope"));
        assert!(instructions.contains("context_shortcut"));
        assert!(instructions.contains("needed_context_query"));
        assert!(instructions.contains("startup_context"));
        assert!(instructions.contains("omit `query` only when `context_shortcut=recent_work`"));
        assert!(instructions.contains("When `context_scope=case`, `context_id` is required"));
        assert!(instructions.contains("`hive` manages tmux-backed worker sessions"));
    }

    #[test]
    fn readonly_tool_router_exposes_only_read_tools() {
        let server = AgpodMcpServer::readonly();
        let tools = server.tool_router.list_all();
        let tool_names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();

        assert_eq!(tool_names.len(), 4);
        assert!(tool_names.contains(&"case_current"));
        assert!(tool_names.contains(&"case_show"));
        assert!(tool_names.contains(&"case_list"));
        assert!(tool_names.contains(&"case_recall"));
        assert!(!tool_names.contains(&"case_open"));
        assert!(!tool_names.contains(&"case_record"));
        assert!(!tool_names.contains(&"case_decide"));
        assert!(!tool_names.contains(&"case_redirect"));
        assert!(!tool_names.contains(&"case_finish"));
        assert!(!tool_names.contains(&"case_resume"));
        assert!(!tool_names.contains(&"case_steps_add"));
        assert!(!tool_names.contains(&"case_step_mark_as"));
        assert!(!tool_names.contains(&"hive"));

        let info = server.get_info();
        let instructions = info.instructions.expect("instructions should exist");
        assert!(instructions
            .contains("`case_current`, `case_show`, `case_list`, and `case_recall` are available"));
        assert!(instructions.contains("No tool in this server can open a case"));
    }

    #[test]
    fn tool_input_schema_omits_data_dir() {
        let server = AgpodMcpServer::with_data_dir(Some("/tmp/agpod-case.db".to_string()));
        let tools = server.tool_router.list_all();
        let current_tool = tools
            .iter()
            .find(|tool| tool.name == "case_current")
            .expect("case_current tool should exist");
        let open_tool = tools
            .iter()
            .find(|tool| tool.name == "case_open")
            .expect("case_open tool should exist");

        let current_schema =
            serde_json::to_value(&current_tool.input_schema).expect("schema should serialize");
        let hive_tool = tools
            .iter()
            .find(|tool| tool.name == "hive")
            .expect("hive tool should exist");
        let hive_schema =
            serde_json::to_value(&hive_tool.input_schema).expect("schema should serialize");
        let open_schema =
            serde_json::to_value(&open_tool.input_schema).expect("schema should serialize");
        let redirect_tool = tools
            .iter()
            .find(|tool| tool.name == "case_redirect")
            .expect("case_redirect tool should exist");
        let redirect_schema =
            serde_json::to_value(&redirect_tool.input_schema).expect("schema should serialize");
        let recall_tool = tools
            .iter()
            .find(|tool| tool.name == "case_recall")
            .expect("case_recall tool should exist");
        let recall_schema =
            serde_json::to_value(&recall_tool.input_schema).expect("schema should serialize");
        let list_tool = tools
            .iter()
            .find(|tool| tool.name == "case_list")
            .expect("case_list tool should exist");
        let list_schema =
            serde_json::to_value(&list_tool.input_schema).expect("schema should serialize");
        let finish_tool = tools
            .iter()
            .find(|tool| tool.name == "case_finish")
            .expect("case_finish tool should exist");
        let finish_schema =
            serde_json::to_value(&finish_tool.input_schema).expect("schema should serialize");
        let step_mark_tool = tools
            .iter()
            .find(|tool| tool.name == "case_step_mark_as")
            .expect("case_step_mark_as tool should exist");
        let step_mark_schema =
            serde_json::to_value(&step_mark_tool.input_schema).expect("schema should serialize");
        let steps_add_tool = tools
            .iter()
            .find(|tool| tool.name == "case_steps_add")
            .expect("case_steps_add tool should exist");
        let steps_add_schema =
            serde_json::to_value(&steps_add_tool.input_schema).expect("schema should serialize");

        assert!(!current_schema.to_string().contains("data_dir"));
        assert!(hive_schema.to_string().contains("\"ensure_session\""));
        assert!(hive_schema.to_string().contains("\"spawn_agent\""));
        assert!(hive_schema.to_string().contains("\"send_prompt\""));
        assert!(hive_schema.to_string().contains("\"reset_agent\""));
        assert!(hive_schema.to_string().contains("\"codex\""));
        assert!(hive_schema.to_string().contains("\"claude\""));
        assert!(!open_schema.to_string().contains("data_dir"));
        assert!(open_schema.to_string().contains("\"reopen\""));
        assert!(open_schema.to_string().contains("\"case_id\""));
        // Conditional allOf removed; verify fields still present in schema
        assert!(open_schema.to_string().contains("\"goal\""));
        assert!(open_schema.to_string().contains("\"direction\""));
        assert!(open_schema.to_string().contains("\"success_condition\""));
        assert!(open_schema.to_string().contains("\"abort_condition\""));
        assert!(open_schema.to_string().contains("needed_context_query"));
        assert!(open_schema.to_string().contains("how_to"));
        assert!(open_schema.to_string().contains("doc_about"));
        assert!(open_schema.to_string().contains("pitfalls_about"));
        assert!(open_schema.to_string().contains("known_patterns_for"));
        assert!(redirect_schema.to_string().contains("is_drift_from_goal"));
        assert!(recall_schema.to_string().contains("find_recent_days"));
        assert!(recall_schema.to_string().contains("find_status"));
        assert!(recall_schema.to_string().contains("\"find\""));
        assert!(recall_schema.to_string().contains("\"context\""));
        assert!(recall_schema.to_string().contains("context_id"));
        assert!(recall_schema.to_string().contains("context_scope"));
        assert!(recall_schema.to_string().contains("context_shortcut"));
        assert!(recall_schema.to_string().contains("recent_work"));
        assert!(recall_schema.to_string().contains("context_token_limit"));
        assert!(recall_schema.to_string().contains("context_limit"));
        assert!(recall_schema.to_string().contains("state retrieval focus"));
        assert!(list_schema.to_string().contains("limit"));
        assert!(list_schema.to_string().contains("\"minimum\":1"));
        assert!(recall_schema.to_string().contains("\"minimum\":1"));
        assert!(finish_schema.to_string().contains("\"completed\""));
        assert!(finish_schema.to_string().contains("\"abandoned\""));
        assert!(step_mark_schema.to_string().contains("\"started\""));
        assert!(!step_mark_schema.to_string().contains("\"done\""));
        assert!(step_mark_schema.to_string().contains("\"blocked\""));
        // Conditional allOf removed; verify reason field still present
        assert!(step_mark_schema.to_string().contains("\"reason\""));
        assert!(steps_add_schema.to_string().contains("\"minItems\":1"));

        let record_tool = tools
            .iter()
            .find(|tool| tool.name == "case_record")
            .expect("case_record tool should exist");
        let record_schema =
            serde_json::to_value(&record_tool.input_schema).expect("schema should serialize");
        let record_schema_text = record_schema.to_string();
        assert!(record_schema_text.contains("Kind of record to append. Supported values:"));
        assert!(record_schema_text.contains("Omit this field to default to `note`"));
        assert!(record_schema_text.contains("use `case_decide` instead"));
        assert!(record_schema_text.contains("`note`"));
        assert!(record_schema_text.contains("`finding`"));
        assert!(record_schema_text.contains("`evidence`"));
        assert!(record_schema_text.contains("`blocker`"));
        assert!(record_schema_text.contains("`goal_constraint_update`"));
        assert!(!record_schema_text.contains("\"decision\""));
        // Conditional allOf removed; verify goal_constraints field still present
        assert!(record_schema_text.contains("\"goal_constraints\""));
    }

    #[test]
    fn record_kind_deserialize_points_decision_to_case_decide() {
        let error = serde_json::from_value::<CaseRecordRequest>(serde_json::json!({
            "id": "C-1",
            "summary": "bad call",
            "kind": "decision"
        }))
        .expect_err("decision should not deserialize as record kind");

        assert!(error.to_string().contains("use `case_decide`"));
    }

    #[test]
    fn record_kind_deserialize_accepts_goal_constraint_update() {
        let request = serde_json::from_value::<CaseRecordRequest>(serde_json::json!({
            "id": "C-1",
            "summary": "update constraints",
            "kind": "goal_constraint_update",
            "goal_constraints": [
                {"rule": "先证据后推断", "reason": "避免臆断"}
            ]
        }))
        .expect("goal_constraint_update should deserialize");

        assert!(matches!(
            request.kind,
            Some(RecordKind::GoalConstraintUpdate)
        ));
        assert_eq!(request.goal_constraints.len(), 1);
    }

    #[test]
    fn list_request_validation_rejects_zero_values() {
        let limit_error =
            validate_list_request(Some(0), None).expect_err("zero limit should be rejected");
        assert!(limit_error.message.contains("limit"));

        let recent_error =
            validate_list_request(None, Some(0)).expect_err("zero recent_days should be rejected");
        assert!(recent_error.message.contains("recent_days"));
    }

    #[test]
    fn tool_envelope_extracts_stable_fields() {
        let raw = serde_json::json!({
            "ok": true,
            "case": {
                "id": "C-550e8400-e29b-41d4-a716-446655440000",
                "status": "open"
            },
            "context": {
                "active_case_id": "C-550e8400-e29b-41d4-a716-446655440000"
            }
        })
        .as_object()
        .cloned()
        .expect("raw payload should be an object");

        let envelope = ToolEnvelope::from_raw("case_current", None, raw);

        assert!(!envelope.is_error());
        assert_eq!(envelope.kind, "case_current");
        assert_eq!(
            envelope.case_id.as_deref(),
            Some("C-550e8400-e29b-41d4-a716-446655440000")
        );
        assert_eq!(envelope.state.as_deref(), Some("open"));
        assert!(envelope.message.is_none());
        assert!(envelope.raw.contains_key("case"));
    }

    #[test]
    fn tool_envelope_marks_no_open_case_as_none() {
        let raw = serde_json::json!({
            "ok": false,
            "error": "error",
            "message": "no open case in this repository"
        })
        .as_object()
        .cloned()
        .expect("raw payload should be an object");

        let envelope = ToolEnvelope::from_raw("case_current", None, raw);

        assert!(envelope.is_error());
        assert_eq!(envelope.state.as_deref(), Some("none"));
        assert_eq!(
            envelope.message.as_deref(),
            Some("no open case in this repository")
        );
    }

    #[test]
    fn tool_envelope_repo_context_has_no_case_id() {
        let raw = serde_json::json!({
            "ok": true,
            "repo": {
                "id": "repo-1"
            },
            "case_context": {
                "scope": "repo",
                "repo_id": "repo-1"
            }
        })
        .as_object()
        .cloned()
        .expect("raw payload should be an object");

        let envelope = ToolEnvelope::from_raw("case_recall", None, raw);

        assert!(!envelope.is_error());
        assert_eq!(envelope.case_id, None);
        assert!(envelope.message.is_none());
    }

    #[test]
    fn case_recall_mode_defaults_to_find() {
        let request: CaseRecallRequest =
            serde_json::from_value(serde_json::json!({"query":"vector digest"}))
                .expect("request should deserialize");
        assert!(request.mode.is_none());
        assert!(matches!(
            request.mode.unwrap_or_default(),
            CaseRecallModeInput::Find
        ));
    }

    #[test]
    fn case_recall_context_scope_defaults_to_repo() {
        let request: CaseRecallRequest = serde_json::from_value(serde_json::json!({
            "query":"vector digest",
            "mode":"context"
        }))
        .expect("request should deserialize");
        assert!(request.context_scope.is_none());
        assert!(matches!(
            request.context_scope.unwrap_or(CaseContextScopeInput::Repo),
            CaseContextScopeInput::Repo
        ));
    }

    #[test]
    fn case_recall_context_shortcut_deserializes() {
        let request: CaseRecallRequest = serde_json::from_value(serde_json::json!({
            "mode":"context",
            "context_shortcut":"recent_work"
        }))
        .expect("request should deserialize");

        assert!(matches!(
            request.context_shortcut,
            Some(CaseContextShortcutInput::RecentWork)
        ));
        assert!(request.query.is_none());
    }

    #[test]
    fn case_recall_schema_marks_query_optional_for_recent_work_shortcut() {
        let server = AgpodMcpServer::new();
        let tools = server.tool_router.list_all();
        let recall_tool = tools
            .iter()
            .find(|tool| tool.name == "case_recall")
            .expect("case_recall tool should exist");
        let recall_schema =
            serde_json::to_string(&recall_tool.input_schema).expect("schema should serialize");

        assert!(recall_schema.contains("\"context_shortcut\""));
        assert!(recall_schema.contains("\"recent_work\""));
        assert!(recall_schema.contains("\"context_id\""));
        assert!(recall_schema.contains("\"query\""));
        assert!(recall_schema.contains("state retrieval focus"));
    }

    #[test]
    fn case_recall_request_allows_missing_query_for_recent_work() {
        let request: CaseRecallRequest = serde_json::from_value(serde_json::json!({
            "mode":"context",
            "context_shortcut":"recent_work"
        }))
        .expect("recent_work should not require query");

        assert!(matches!(request.mode, Some(CaseRecallModeInput::Context)));
        assert!(request.query.is_none());
    }

    #[test]
    fn case_steps_add_success_aggregates_created_steps() {
        let last_result = serde_json::json!({
            "ok": true,
            "step": {
                "id": "case/S-002",
                "title": "second"
            },
            "steps": {
                "ordered": [
                    {"id": "case/S-001"},
                    {"id": "case/S-002"}
                ]
            },
            "context": {
                "active_case_id": "case"
            },
            "next": {
                "suggested_command": "record"
            }
        })
        .as_object()
        .cloned()
        .expect("raw payload should be an object");

        let raw = build_case_steps_add_success(
            vec![
                serde_json::json!({"id": "case/S-001", "title": "first"}),
                serde_json::json!({"id": "case/S-002", "title": "second"}),
            ],
            last_result,
        );

        assert_eq!(raw.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(raw.get("created_count").and_then(Value::as_u64), Some(2));
        assert_eq!(
            raw.get("created_steps")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            raw.get("steps")
                .and_then(|value| value.get("ordered"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            raw.get("next")
                .and_then(|value| value.get("suggested_command"))
                .and_then(Value::as_str),
            Some("record")
        );
    }

    #[test]
    fn case_steps_add_partial_error_preserves_successes() {
        let failed_result = serde_json::json!({
            "ok": false,
            "message": "step not found",
            "steps": {
                "ordered": [
                    {"id": "case/S-001"}
                ]
            },
            "context": {
                "active_case_id": "case"
            }
        })
        .as_object()
        .cloned()
        .expect("raw payload should be an object");

        let raw = build_case_steps_add_partial_error(
            2,
            StepInput::Detailed(StepObjectInput {
                title: "second".to_string(),
                reason: Some("because".to_string()),
                start: false,
            }),
            vec![serde_json::json!({"id": "case/S-001", "title": "first"})],
            failed_result,
        );

        assert_eq!(raw.get("ok").and_then(Value::as_bool), Some(false));
        assert_eq!(raw.get("created_count").and_then(Value::as_u64), Some(1));
        assert_eq!(raw.get("failed_index").and_then(Value::as_u64), Some(2));
        assert!(raw
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("failed at step 2")));
        assert_eq!(
            raw.get("failed_input")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str),
            Some("second")
        );
        assert!(raw.get("failure").is_some());
    }

    #[test]
    fn case_steps_add_request_accepts_string_steps() {
        let request: CaseStepsAddRequest = serde_json::from_value(serde_json::json!({
            "id": "case",
            "steps": ["first step", "second step"]
        }))
        .expect("string shorthand should deserialize");

        assert_eq!(request.steps.len(), 2);
        assert_eq!(request.steps[0].title(), "first step");
        assert_eq!(request.steps[0].reason(), None);
        assert!(!request.steps[0].start());
    }

    #[test]
    fn case_steps_add_request_keeps_object_steps() {
        let request: CaseStepsAddRequest = serde_json::from_value(serde_json::json!({
            "id": "case",
            "steps": [
                {
                    "title": "first step",
                    "reason": "because",
                    "start": true
                }
            ]
        }))
        .expect("object form should deserialize");

        assert_eq!(request.steps.len(), 1);
        assert_eq!(request.steps[0].title(), "first step");
        assert_eq!(request.steps[0].reason(), Some("because"));
        assert!(request.steps[0].start());
    }

    #[test]
    fn case_finish_request_accepts_known_outcomes() {
        let request: CaseFinishRequest = serde_json::from_value(serde_json::json!({
            "id": "case",
            "outcome": "completed",
            "summary": "done",
            "confirm_token": "token-1"
        }))
        .expect("known finish outcome should deserialize");

        assert!(matches!(request.outcome, CaseFinishOutcomeInput::Completed));
        assert_eq!(request.confirm_token.as_deref(), Some("token-1"));
    }

    #[test]
    fn case_open_request_accepts_reopen_mode() {
        let request: CaseOpenRequest = serde_json::from_value(serde_json::json!({
            "mode": "reopen",
            "case_id": "C-1"
        }))
        .expect("reopen request should deserialize");

        assert!(matches!(request.mode, CaseOpenModeInput::Reopen));
        assert_eq!(request.case_id.as_deref(), Some("C-1"));
        assert!(request.goal.is_none());
    }

    #[test]
    fn case_open_request_accepts_needed_context_query() {
        let request: CaseOpenRequest = serde_json::from_value(serde_json::json!({
            "goal": "improve case open startup memory",
            "direction": "wire startup context into case_open",
            "needed_context_query": {
                "how_to": ["run hosted smoke"],
                "doc_about": ["honcho integration"],
                "pitfalls_about": ["empty recall result"],
                "known_patterns_for": ["smoke testing"]
            }
        }))
        .expect("needed_context_query should deserialize");

        let query = request
            .needed_context_query
            .expect("needed_context_query should exist");
        assert_eq!(query.how_to, vec!["run hosted smoke"]);
        assert_eq!(query.doc_about, vec!["honcho integration"]);
        assert_eq!(query.pitfalls_about, vec!["empty recall result"]);
        assert_eq!(query.known_patterns_for, vec!["smoke testing"]);
    }

    #[tokio::test]
    async fn case_open_reopen_rejects_needed_context_query() {
        let server = AgpodMcpServer::new();
        let err = server
            .case_open(Parameters(CaseOpenRequest {
                mode: CaseOpenModeInput::Reopen,
                case_id: Some("C-1".to_string()),
                goal: None,
                direction: None,
                goal_constraints: Vec::new(),
                constraints: Vec::new(),
                success_condition: None,
                abort_condition: None,
                needed_context_query: Some(NeededContextQueryInput::default()),
            }))
            .await
            .expect_err("reopen should reject startup context query");

        assert!(err.message.contains("needed_context_query"));
    }

    #[test]
    fn case_step_mark_as_request_accepts_known_statuses() {
        let request: CaseStepMarkAsRequest = serde_json::from_value(serde_json::json!({
            "id": "case",
            "step_id": "case/S-001",
            "status": "blocked",
            "reason": "waiting"
        }))
        .expect("known step status should deserialize");

        assert!(matches!(request.status, StepStatusInput::Blocked));
        assert_eq!(request.reason.as_deref(), Some("waiting"));
    }

    #[test]
    fn case_step_advance_request_accepts_record_payload() {
        let request: CaseStepAdvanceRequest = serde_json::from_value(serde_json::json!({
            "id": "case",
            "step_id": "case/S-001",
            "record": {
                "summary": "captured evidence",
                "kind": "evidence",
                "files": ["docs/runbook.md"],
                "context": "from smoke"
            },
            "next_step_auto": true
        }))
        .expect("advance request should deserialize");

        let record = request.record.expect("record should exist");
        assert_eq!(record.summary, "captured evidence");
        assert!(matches!(record.kind, Some(RecordKind::Evidence)));
        assert_eq!(record.files, vec!["docs/runbook.md"]);
        assert_eq!(record.context.as_deref(), Some("from smoke"));
        assert!(request.next_step_auto);
    }

    #[tokio::test]
    async fn case_step_advance_rejects_conflicting_next_step_inputs() {
        let server = AgpodMcpServer::new();
        let err = server
            .case_step_advance(Parameters(CaseStepAdvanceRequest {
                id: Some("case".to_string()),
                step_id: None,
                record: None,
                next_step_id: Some("case/S-002".to_string()),
                next_step_auto: true,
            }))
            .await
            .expect_err("conflicting next step inputs should be invalid");

        assert!(err.message.contains("next_step_id"));
    }

    #[tokio::test]
    async fn case_steps_add_rejects_empty_array() {
        let server = AgpodMcpServer::new();
        let err = server
            .case_steps_add(Parameters(CaseStepsAddRequest {
                id: "case".to_string(),
                steps: Vec::new(),
            }))
            .await
            .expect_err("empty steps should be invalid");

        assert!(err.message.contains("steps array must not be empty"));
    }

    #[test]
    fn tool_response_sets_mcp_is_error() {
        let result = ToolResponse {
            result: ToolEnvelope {
                is_error: true,
                kind: "case_current".to_string(),
                case_id: None,
                state: Some("none".to_string()),
                message: Some("no open case in this repository".to_string()),
                raw: Map::new(),
            },
        }
        .into_call_tool_result()
        .expect("tool response should serialize");

        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.content,
            vec![Content::text("no open case in this repository")]
        );
        assert_eq!(
            result.structured_content,
            Some(serde_json::json!({
                "result": {
                    "kind": "case_current",
                    "state": "none",
                    "message": "no open case in this repository",
                    "raw": {}
                }
            }))
        );
    }

    #[test]
    fn hive_tool_response_sets_mcp_is_error() {
        let result = HiveToolResponse {
            result: HiveToolEnvelope::from_raw(
                serde_json::json!({
                    "ok": false,
                    "state": "limit_reached",
                    "message": "limit reached",
                    "session": { "id": "hive-q9" },
                    "agent": { "agent_id": "agent-01" }
                })
                .as_object()
                .cloned()
                .expect("raw payload should be object"),
            ),
        }
        .into_call_tool_result()
        .expect("tool response should serialize");

        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.content, vec![Content::text("limit reached")]);
    }
}
