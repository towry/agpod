//! MCP server for agpod case workflows.
//!
//! Keywords: mcp, model context protocol, case tools, schema, stdio

use agpod_case::{CaseArgs, CaseCommand, StepCommand};
use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{JsonObject, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData, Json, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone)]
pub struct AgpodMcpServer {
    data_dir: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl AgpodMcpServer {
    pub fn new() -> Self {
        Self::with_data_dir(std::env::var("AGPOD_CASE_DATA_DIR").ok())
    }

    fn with_data_dir(data_dir: Option<String>) -> Self {
        Self {
            data_dir,
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
    ) -> Result<Json<ToolResponse>, ErrorData> {
        let args = CaseArgs {
            data_dir: self.data_dir.clone(),
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
        Ok(Json(ToolResponse {
            result: ToolEnvelope::from_raw(kind, case_id_hint, result),
        }))
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
                                "ok": {
                                    "type": "boolean",
                                    "description": "Whether the agpod case command succeeded."
                                },
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
                            "required": ["ok", "kind", "raw"]
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
            "agpod case MCP. One open case per repo. Flow: `case_open` -> step tools -> `case_record`/`case_decide`/`case_redirect` -> `case_close` or `case_abandon`. Tools return structured JSON aligned with `agpod case --json`.",
        )
    }
}

#[tool_router]
impl AgpodMcpServer {
    #[tool(
        name = "case_current",
        description = "Read active case state. Safe first call.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_current(
        &self,
        Parameters(_req): Parameters<CaseCurrentRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool("case_current", CaseCommand::Current, None)
            .await
    }

    #[tool(
        name = "case_open",
        description = "Open the repo's only active case. Call first.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_open(
        &self,
        Parameters(req): Parameters<CaseOpenRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
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
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_record",
            CaseCommand::Record {
                id: req.id.clone(),
                summary: req.summary,
                kind: req.kind.unwrap_or_else(|| "note".to_string()),
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
    ) -> Result<Json<ToolResponse>, ErrorData> {
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
        description = "Change direction on an open case when the path changes.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_redirect(
        &self,
        Parameters(req): Parameters<CaseRedirectRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_redirect",
            CaseCommand::Redirect {
                id: req.id.clone(),
                direction: req.direction,
                reason: req.reason,
                context: req.context,
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
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool("case_show", CaseCommand::Show { id: req.id }, None)
            .await
    }

    #[tool(
        name = "case_close",
        description = "Close an open case once the goal is met.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_close(
        &self,
        Parameters(req): Parameters<CaseCloseRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_close",
            CaseCommand::Close {
                id: req.id.clone(),
                summary: req.summary,
            },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_abandon",
        description = "Abandon an open case when the goal is no longer worth pursuing.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_abandon(
        &self,
        Parameters(req): Parameters<CaseCloseRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_abandon",
            CaseCommand::Abandon {
                id: req.id.clone(),
                summary: req.summary,
            },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_list",
        description = "List repo cases. Safe discovery call.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_list(
        &self,
        Parameters(_req): Parameters<CaseCurrentRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool("case_list", CaseCommand::List, None)
            .await
    }

    #[tool(
        name = "case_recall",
        description = "Search past cases by text.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_recall(
        &self,
        Parameters(req): Parameters<CaseRecallRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_recall",
            CaseCommand::Recall { query: req.query },
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
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool("case_resume", CaseCommand::Resume { id: req.id }, None)
            .await
    }

    #[tool(
        name = "case_step_add",
        description = "Add a step to the current direction. Use after `case_open` or `case_redirect`.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_step_add(
        &self,
        Parameters(req): Parameters<CaseStepAddRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_step_add",
            CaseCommand::Step {
                command: StepCommand::Add {
                    id: req.id.clone(),
                    title: req.title,
                    reason: req.reason,
                },
            },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_step_start",
        description = "Start a pending step on an open case.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_step_start(
        &self,
        Parameters(req): Parameters<CaseStepIdRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_step_start",
            CaseCommand::Step {
                command: StepCommand::Start {
                    id: req.id.clone(),
                    step_id: req.step_id,
                },
            },
            Some(req.id),
        )
        .await
    }

    #[tool(
        name = "case_step_done",
        description = "Mark an active or pending step done.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_step_done(
        &self,
        Parameters(req): Parameters<CaseStepIdRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_step_done",
            CaseCommand::Step {
                command: StepCommand::Done {
                    id: req.id.clone(),
                    step_id: req.step_id,
                },
            },
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
    ) -> Result<Json<ToolResponse>, ErrorData> {
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

    #[tool(
        name = "case_step_block",
        description = "Mark a step blocked when execution cannot proceed.",
        output_schema = case_tool_output_schema()
    )]
    async fn case_step_block(
        &self,
        Parameters(req): Parameters<CaseStepBlockRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            "case_step_block",
            CaseCommand::Step {
                command: StepCommand::Block {
                    id: req.id.clone(),
                    step_id: req.step_id,
                    reason: req.reason,
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
        .map(|constraint| serde_json::to_string(&constraint).expect("constraint should serialize"))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConstraintInput {
    /// Constraint text.
    pub rule: String,
    /// Why the constraint exists.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ToolResponse {
    pub result: ToolEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ToolEnvelope {
    pub ok: bool,
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
            ok,
            kind: kind.to_string(),
            case_id,
            state,
            message,
            raw,
        }
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
    raw.get("case")
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            if raw.get("resume").is_some() {
                Some("resume".to_string())
            } else if raw.get("cases").is_some() {
                Some("list".to_string())
            } else if raw.get("step").is_some() || raw.get("steps").is_some() {
                Some("step".to_string())
            } else if !ok
                && raw
                    .get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message == "no open case in this repository")
            {
                Some("none".to_string())
            } else if ok {
                Some("ok".to_string())
            } else {
                Some("error".to_string())
            }
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct CaseCurrentRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseOpenRequest {
    /// Immutable case goal.
    pub goal: String,
    /// Initial direction summary.
    pub direction: String,
    /// Case-wide constraints.
    #[serde(default)]
    pub goal_constraints: Vec<ConstraintInput>,
    /// Direction-local constraints.
    #[serde(default)]
    pub constraints: Vec<ConstraintInput>,
    /// Condition for success on this direction.
    pub success_condition: Option<String>,
    /// Condition for aborting this direction.
    pub abort_condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseRecordRequest {
    /// Case ID.
    pub id: String,
    /// Fact summary.
    pub summary: String,
    /// note, finding, evidence, or blocker.
    pub kind: Option<String>,
    /// Related file paths.
    pub files: Option<Vec<String>>,
    /// Extra context.
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseDecideRequest {
    /// Case ID.
    pub id: String,
    /// Decision summary.
    pub summary: String,
    /// Why this decision was made.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseRedirectRequest {
    /// Case ID.
    pub id: String,
    /// New direction summary.
    pub direction: String,
    /// Why direction changed.
    pub reason: String,
    /// Context carried from prior work.
    pub context: String,
    /// New direction constraints.
    #[serde(default)]
    pub constraints: Vec<ConstraintInput>,
    /// Condition for success on the new direction.
    pub success_condition: String,
    /// Condition for aborting the new direction.
    pub abort_condition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseShowRequest {
    /// Case ID. Omit to use the open case.
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseCloseRequest {
    /// Case ID.
    pub id: String,
    /// Closing or abandonment summary.
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseRecallRequest {
    /// Search query.
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepAddRequest {
    /// Case ID.
    pub id: String,
    /// Step title.
    pub title: String,
    /// Why this step is needed.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepIdRequest {
    /// Case ID.
    pub id: String,
    /// Step ID.
    pub step_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepMoveRequest {
    /// Case ID.
    pub id: String,
    /// Step to move.
    pub step_id: String,
    /// Place before this step ID.
    pub before: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepBlockRequest {
    /// Case ID.
    pub id: String,
    /// Step ID.
    pub step_id: String,
    /// Why the step is blocked.
    pub reason: String,
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
        assert!(tool_names.contains(&"case_step_add"));

        let current_tool = tools
            .iter()
            .find(|tool| tool.name == "case_current")
            .expect("case_current tool should exist");
        assert!(current_tool.output_schema.is_some());
        let schema = serde_json::to_string(current_tool.output_schema.as_ref().unwrap())
            .expect("schema should serialize");
        assert!(schema.contains("\"kind\""));
        assert!(schema.contains("\"raw\""));
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

        assert!(!current_schema.to_string().contains("data_dir"));
        assert!(!open_schema.to_string().contains("data_dir"));
    }

    #[test]
    fn tool_envelope_extracts_stable_fields() {
        let raw = serde_json::json!({
            "ok": true,
            "case": {
                "id": "C-20260320-01",
                "status": "open"
            },
            "context": {
                "active_case_id": "C-20260320-01"
            }
        })
        .as_object()
        .cloned()
        .expect("raw payload should be an object");

        let envelope = ToolEnvelope::from_raw("case_current", None, raw);

        assert!(envelope.ok);
        assert_eq!(envelope.kind, "case_current");
        assert_eq!(envelope.case_id.as_deref(), Some("C-20260320-01"));
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

        assert!(!envelope.ok);
        assert_eq!(envelope.state.as_deref(), Some("none"));
        assert_eq!(
            envelope.message.as_deref(),
            Some("no open case in this repository")
        );
    }
}
