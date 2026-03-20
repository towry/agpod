//! MCP server for agpod case workflows.
//!
//! Keywords: mcp, model context protocol, case tools, schema, stdio

use agpod_case::{CaseArgs, CaseCommand, StepCommand};
use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData, Json, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct AgpodMcpServer {
    tool_router: ToolRouter<Self>,
}

impl AgpodMcpServer {
    pub fn new() -> Self {
        Self {
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
        command: CaseCommand,
        data_dir: Option<String>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        let args = CaseArgs {
            data_dir,
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
        Ok(Json(ToolResponse { result }))
    }
}

impl Default for AgpodMcpServer {
    fn default() -> Self {
        Self::new()
    }
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
        description = "Read active case state. Safe first call."
    )]
    async fn case_current(
        &self,
        Parameters(req): Parameters<CaseCurrentRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(CaseCommand::Current, req.data_dir).await
    }

    #[tool(
        name = "case_open",
        description = "Open the repo's only active case. Call first."
    )]
    async fn case_open(
        &self,
        Parameters(req): Parameters<CaseOpenRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Open {
                goal: req.goal,
                direction: req.direction,
                goal_constraints: encode_constraints(req.goal_constraints),
                constraints: encode_constraints(req.constraints),
                success_condition: req.success_condition,
                abort_condition: req.abort_condition,
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_record",
        description = "Append a fact to an open case. Not for decisions or redirects."
    )]
    async fn case_record(
        &self,
        Parameters(req): Parameters<CaseRecordRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Record {
                id: req.id,
                summary: req.summary,
                kind: req.kind.unwrap_or_else(|| "note".to_string()),
                files: req.files.map(|files| files.join(",")),
                context: req.context,
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_decide",
        description = "Record an in-direction decision on an open case."
    )]
    async fn case_decide(
        &self,
        Parameters(req): Parameters<CaseDecideRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Decide {
                id: req.id,
                summary: req.summary,
                reason: req.reason,
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_redirect",
        description = "Change direction on an open case when the path changes."
    )]
    async fn case_redirect(
        &self,
        Parameters(req): Parameters<CaseRedirectRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Redirect {
                id: req.id,
                direction: req.direction,
                reason: req.reason,
                context: req.context,
                constraints: encode_constraints(req.constraints),
                success_condition: req.success_condition,
                abort_condition: req.abort_condition,
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_show",
        description = "Show case tree and step history. Use after `case_current` when needed."
    )]
    async fn case_show(
        &self,
        Parameters(req): Parameters<CaseShowRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(CaseCommand::Show { id: req.id }, req.data_dir)
            .await
    }

    #[tool(
        name = "case_close",
        description = "Close an open case once the goal is met."
    )]
    async fn case_close(
        &self,
        Parameters(req): Parameters<CaseCloseRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Close {
                id: req.id,
                summary: req.summary,
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_abandon",
        description = "Abandon an open case when the goal is no longer worth pursuing."
    )]
    async fn case_abandon(
        &self,
        Parameters(req): Parameters<CaseCloseRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Abandon {
                id: req.id,
                summary: req.summary,
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_list",
        description = "List repo cases. Safe discovery call."
    )]
    async fn case_list(
        &self,
        Parameters(req): Parameters<CaseCurrentRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(CaseCommand::List, req.data_dir).await
    }

    #[tool(name = "case_recall", description = "Search past cases by text.")]
    async fn case_recall(
        &self,
        Parameters(req): Parameters<CaseRecallRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(CaseCommand::Recall { query: req.query }, req.data_dir)
            .await
    }

    #[tool(
        name = "case_resume",
        description = "Get a handoff summary for an open case or a chosen case."
    )]
    async fn case_resume(
        &self,
        Parameters(req): Parameters<CaseShowRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(CaseCommand::Resume { id: req.id }, req.data_dir)
            .await
    }

    #[tool(
        name = "case_step_add",
        description = "Add a step to the current direction. Use after `case_open` or `case_redirect`."
    )]
    async fn case_step_add(
        &self,
        Parameters(req): Parameters<CaseStepAddRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Step {
                command: StepCommand::Add {
                    id: req.id,
                    title: req.title,
                    reason: req.reason,
                },
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_step_start",
        description = "Start a pending step on an open case."
    )]
    async fn case_step_start(
        &self,
        Parameters(req): Parameters<CaseStepIdRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Step {
                command: StepCommand::Start {
                    id: req.id,
                    step_id: req.step_id,
                },
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_step_done",
        description = "Mark an active or pending step done."
    )]
    async fn case_step_done(
        &self,
        Parameters(req): Parameters<CaseStepIdRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Step {
                command: StepCommand::Done {
                    id: req.id,
                    step_id: req.step_id,
                },
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_step_move",
        description = "Reorder steps within the current direction."
    )]
    async fn case_step_move(
        &self,
        Parameters(req): Parameters<CaseStepMoveRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Step {
                command: StepCommand::Move {
                    id: req.id,
                    step_id: req.step_id,
                    before: req.before,
                },
            },
            req.data_dir,
        )
        .await
    }

    #[tool(
        name = "case_step_block",
        description = "Mark a step blocked when execution cannot proceed."
    )]
    async fn case_step_block(
        &self,
        Parameters(req): Parameters<CaseStepBlockRequest>,
    ) -> Result<Json<ToolResponse>, ErrorData> {
        self.run_case_tool(
            CaseCommand::Step {
                command: StepCommand::Block {
                    id: req.id,
                    step_id: req.step_id,
                    reason: req.reason,
                },
            },
            req.data_dir,
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
    pub result: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct CaseCurrentRequest {
    /// Override case data directory.
    pub data_dir: Option<String>,
}

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
    /// Override case data directory.
    pub data_dir: Option<String>,
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
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseDecideRequest {
    /// Case ID.
    pub id: String,
    /// Decision summary.
    pub summary: String,
    /// Why this decision was made.
    pub reason: String,
    /// Override case data directory.
    pub data_dir: Option<String>,
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
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseShowRequest {
    /// Case ID. Omit to use the open case.
    pub id: Option<String>,
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseCloseRequest {
    /// Case ID.
    pub id: String,
    /// Closing or abandonment summary.
    pub summary: String,
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseRecallRequest {
    /// Search query.
    pub query: String,
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepAddRequest {
    /// Case ID.
    pub id: String,
    /// Step title.
    pub title: String,
    /// Why this step is needed.
    pub reason: Option<String>,
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepIdRequest {
    /// Case ID.
    pub id: String,
    /// Step ID.
    pub step_id: String,
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepMoveRequest {
    /// Case ID.
    pub id: String,
    /// Step to move.
    pub step_id: String,
    /// Place before this step ID.
    pub before: String,
    /// Override case data directory.
    pub data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaseStepBlockRequest {
    /// Case ID.
    pub id: String,
    /// Step ID.
    pub step_id: String,
    /// Why the step is blocked.
    pub reason: String,
    /// Override case data directory.
    pub data_dir: Option<String>,
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
    }
}
