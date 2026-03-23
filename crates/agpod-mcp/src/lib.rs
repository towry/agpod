//! MCP server for agpod case workflows.
//!
//! Keywords: mcp, model context protocol, case tools, schema, stdio

use agpod_case::{CaseArgs, CaseCommand, CaseStatusArg, GoalDriftFlag, RecordKind, StepCommand};
use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, JsonObject, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt,
};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone)]
pub struct AgpodMcpServer {
    data_dir: Option<String>,
    server_addr: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl AgpodMcpServer {
    pub fn new() -> Self {
        Self::with_case_config(
            std::env::var("AGPOD_CASE_DATA_DIR").ok(),
            std::env::var("AGPOD_CASE_SERVER_ADDR").ok(),
        )
    }

    #[cfg(test)]
    fn with_data_dir(data_dir: Option<String>) -> Self {
        Self::with_case_config(data_dir, None)
    }

    fn with_case_config(data_dir: Option<String>, server_addr: Option<String>) -> Self {
        Self {
            data_dir,
            server_addr,
            tool_router: Self::tool_router(),
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
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "agpod case MCP. One open case per repo. Start with `case_current`; if a case is open, call `case_resume` before deciding whether to use step tools, `case_record`, `case_decide`, or `case_redirect`. Call `case_open` only when `case_current` reports no open case. Tools return structured JSON aligned with `agpod case --json`.",
        )
    }
}

#[tool_router]
impl AgpodMcpServer {
    #[tool(
        name = "case_current",
        description = "Read active case state. Preferred first call for a new session.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_current(
        &self,
        Parameters(_req): Parameters<CaseCurrentRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool("case_current", CaseCommand::Current, None)
            .await
    }

    #[tool(
        name = "case_open",
        description = "Open the repo's only active case, but only when `case_current` shows there is no open case.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_open(
        &self,
        Parameters(req): Parameters<CaseOpenRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run_case_tool(
            "case_open",
            CaseCommand::Open {
                goal: req.goal,
                direction: req.direction,
                goal_constraints: encode_constraints(req.goal_constraints),
                constraints: encode_constraints(req.constraints),
                success_condition: req.success_condition,
                abort_condition: req.abort_condition,
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

    #[tool(
        name = "case_recall",
        description = "Search past cases by weighted text match, with optional status, recency, and limit filters.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_recall(
        &self,
        Parameters(req): Parameters<CaseRecallRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_list_request(req.limit, req.recent_days)?;
        if req.query.trim().is_empty() {
            return Err(ErrorData::invalid_params("query must not be empty", None));
        }

        self.run_case_tool(
            "case_recall",
            CaseCommand::Recall {
                query: req.query,
                status: req.status.map(Into::into),
                limit: req.limit,
                recent_days: req.recent_days,
            },
            None,
        )
        .await
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
        description = "Transition a step's status: started, done, or blocked.",
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
            StepStatusInput::Done => StepCommand::Done {
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

fn describe_case_record_request_schema(schema: &mut schemars::Schema) {
    let object = schema.ensure_object();
    object.insert(
        "allOf".to_string(),
        serde_json::json!([
            {
                "if": {
                    "properties": {
                        "kind": { "const": "goal_constraint_update" }
                    },
                    "required": ["kind"]
                },
                "then": {
                    "properties": {
                        "goal_constraints": { "minItems": 1 }
                    }
                },
                "else": {
                    "properties": {
                        "goal_constraints": { "maxItems": 0 }
                    }
                }
            }
        ]),
    );
}

fn describe_case_step_mark_as_request_schema(schema: &mut schemars::Schema) {
    let object = schema.ensure_object();
    object.insert(
        "allOf".to_string(),
        serde_json::json!([
            {
                "if": {
                    "properties": {
                        "status": { "const": "blocked" }
                    }
                },
                "then": {
                    "required": ["reason"],
                    "properties": {
                        "reason": { "minLength": 1 }
                    }
                }
            }
        ]),
    );
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
pub struct CaseOpenRequest {
    /// Immutable case goal.
    pub goal: String,
    /// Initial direction summary.
    pub direction: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseRecallRequest {
    /// Search query. Must not be empty or whitespace-only.
    #[schemars(length(min = 1))]
    pub query: String,
    /// Optional case status filter.
    pub status: Option<CaseStatusInput>,
    /// Limit result count. Must be at least 1 when provided.
    #[schemars(range(min = 1))]
    pub limit: Option<usize>,
    /// Only include cases updated within the last N days. Must be at least 1 when provided.
    #[schemars(range(min = 1))]
    pub recent_days: Option<u32>,
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
#[schemars(transform = describe_case_step_mark_as_request_schema)]
#[serde(rename_all = "lowercase")]
pub enum StepStatusInput {
    Started,
    Done,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(transform = describe_case_step_mark_as_request_schema)]
pub struct CaseStepMarkAsRequest {
    /// Case ID, usually from `case_current`, `case_open`, or a previous tool result's `result.case_id`.
    pub id: String,
    /// Step ID from the case step list, such as `steps.current.id` or one entry in `steps.ordered`.
    pub step_id: String,
    /// Target status for the step.
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
        assert!(tool_names.contains(&"case_open"));
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
        assert!(!open_schema.to_string().contains("data_dir"));
        assert!(redirect_schema.to_string().contains("is_drift_from_goal"));
        assert!(recall_schema.to_string().contains("recent_days"));
        assert!(recall_schema.to_string().contains("status"));
        assert!(list_schema.to_string().contains("limit"));
        assert!(list_schema.to_string().contains("\"minimum\":1"));
        assert!(recall_schema.to_string().contains("\"minimum\":1"));
        assert!(finish_schema.to_string().contains("\"completed\""));
        assert!(finish_schema.to_string().contains("\"abandoned\""));
        assert!(step_mark_schema.to_string().contains("\"started\""));
        assert!(step_mark_schema.to_string().contains("\"done\""));
        assert!(step_mark_schema.to_string().contains("\"blocked\""));
        assert!(step_mark_schema.to_string().contains("\"required\":[\"reason\"]"));
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
        assert!(record_schema_text.contains("\"minItems\":1"));
        assert!(record_schema_text.contains("\"maxItems\":0"));
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
}
