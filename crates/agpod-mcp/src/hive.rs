//! Hive tool support for tmux-backed worker orchestration.
//!
//! Keywords: hive, tmux, worker session, idle state, hook integration

use anyhow::{anyhow, Context, Result};
use rmcp::{
    model::{CallToolResult, Content, JsonObject},
    schemars, ErrorData,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use tracing::warn;

const HIVE_VERSION: u32 = 1;
const HIVE_AGENT_LIMIT: usize = 5;
const HIVE_BOOTSTRAP_WINDOW: &str = "__HIVE_HOME_EMPTY__";
const HIVE_LOCK_STALE_MS: u64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HiveActionInput {
    EnsureSession,
    ListAgents,
    SpawnAgent,
    SendPrompt,
    ResetAgent,
    CloseAgent,
    CloseSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HiveAgentKindInput {
    #[default]
    Codex,
    Claude,
}

impl HiveAgentKindInput {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HiveRequest {
    /// Hive action to perform.
    pub action: HiveActionInput,
    /// Derived session ID from the caller tmux pane. Optional validation guard for chained calls.
    pub session_id: Option<String>,
    /// Existing worker agent ID for `send_prompt` and `reset_agent`.
    pub agent_id: Option<String>,
    /// Worker runtime kind for `spawn_agent`.
    #[serde(default)]
    pub agent_kind: HiveAgentKindInput,
    /// Optional worker model hint passed through to the spawned agent command.
    pub model: Option<String>,
    /// Optional worker display name / tmux window label.
    pub worker_name: Option<String>,
    /// Optional working directory. Relative paths are resolved from the repo root.
    pub workdir: Option<String>,
    /// Prompt to send for `send_prompt`.
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HiveToolResponse {
    pub result: HiveToolEnvelope,
}

impl HiveToolResponse {
    pub fn into_call_tool_result(self) -> Result<CallToolResult, ErrorData> {
        let is_error = self.result.is_error();
        let text = self
            .result
            .message
            .clone()
            .unwrap_or_else(|| self.result.kind.clone());
        let value = serde_json::to_value(&self).map_err(|err| {
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
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HiveToolEnvelope {
    #[serde(skip)]
    is_error: bool,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub raw: Map<String, Value>,
}

impl HiveToolEnvelope {
    pub fn from_raw(raw: Map<String, Value>) -> Self {
        let ok = raw.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let message = raw
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let state = raw
            .get("state")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| {
                raw.get("agent")
                    .and_then(|agent| agent.get("status"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .or_else(|| {
                raw.get("session")
                    .and_then(|session| session.get("state"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let session_id = raw
            .get("session")
            .and_then(|session| session.get("id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let agent_id = raw
            .get("agent")
            .and_then(|agent| agent.get("agent_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        Self {
            is_error: !ok,
            kind: "hive".to_string(),
            session_id,
            agent_id,
            state,
            message,
            raw,
        }
    }

    pub fn is_error(&self) -> bool {
        self.is_error
    }
}

pub fn hive_tool_output_schema() -> Arc<JsonObject> {
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
                                "kind": { "type": "string" },
                                "session_id": { "type": ["string", "null"] },
                                "agent_id": { "type": ["string", "null"] },
                                "state": { "type": ["string", "null"] },
                                "message": { "type": ["string", "null"] },
                                "raw": {
                                    "type": "object",
                                    "additionalProperties": true
                                }
                            },
                            "required": ["kind", "raw"]
                        }
                    },
                    "required": ["result"],
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "title": "HiveToolResponse"
                })
                .as_object()
                .expect("output schema should be an object")
                .clone(),
            )
        })
        .clone()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HiveSessionState {
    version: u32,
    session_id: String,
    session_name: String,
    queen_pane_id: String,
    tmux_socket: Option<String>,
    repo_root: String,
    agent_limit: usize,
    updated_at_ms: u64,
    agents: Vec<HiveAgentState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HiveAgentState {
    agent_id: String,
    worker_name: String,
    agent_kind: HiveAgentKindInput,
    model: Option<String>,
    workdir: String,
    window_id: String,
    window_name: String,
    pane_id: String,
    status: HiveAgentStatus,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum HiveAgentStatus {
    Spawning,
    Idle,
    Busy,
    Resetting,
    Dead,
}

impl HiveAgentStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Spawning => "spawning",
            Self::Idle => "idle",
            Self::Busy => "busy",
            Self::Resetting => "resetting",
            Self::Dead => "dead",
        }
    }
}

fn prompt_accept_state(status: &HiveAgentStatus) -> bool {
    matches!(status, HiveAgentStatus::Idle)
}

#[derive(Debug, Clone)]
struct HiveRuntime {
    repo_root: PathBuf,
    state_dir: PathBuf,
    session_id: String,
    session_name: String,
    queen_pane_id: String,
    tmux_socket: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionPaneInfo {
    window_id: String,
    window_name: String,
    pane_id: String,
}

impl HiveRuntime {
    fn from_env(session_id_hint: Option<&str>) -> Result<Self, ErrorData> {
        let tmux_env = std::env::var("TMUX").ok();
        let term = std::env::var("TERM").ok();
        let queen_pane_id = std::env::var("TMUX_PANE").map_err(|_| {
            let cwd = std::env::current_dir()
                .ok()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unavailable>".to_string());
            warn!(
                has_tmux = tmux_env.is_some(),
                term = term.as_deref().unwrap_or("<missing>"),
                cwd = cwd.as_str(),
                "hive runtime missing TMUX_PANE"
            );
            ErrorData::invalid_params(
                "`hive` requires tmux, but `TMUX_PANE` is missing. Check your MCP configuration and ensure it forwards required environment variables such as `TMUX_PANE`.",
                None,
            )
        })?;
        let tmux_socket = tmux_env
            .as_deref()
            .and_then(|value| value.split(',').next())
            .map(ToOwned::to_owned);
        let session_id = derive_session_id(&queen_pane_id, tmux_socket.as_deref());
        if let Some(expected) = session_id_hint {
            if expected != session_id {
                return Err(ErrorData::invalid_params(
                    format!(
                        "session_id mismatch: expected `{expected}`, derived `{session_id}` from current tmux pane"
                    ),
                    None,
                ));
            }
        }

        let repo_root = std::env::current_dir().map_err(|err| {
            ErrorData::internal_error(format!("failed to resolve current directory: {err}"), None)
        })?;
        let state_dir = std::env::var("AGPOD_HIVE_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("agpod")
                    .join("hive")
            });

        Ok(Self {
            repo_root,
            state_dir,
            session_name: format!("agpod-{session_id}"),
            session_id,
            queen_pane_id,
            tmux_socket,
        })
    }

    fn session_file(&self) -> PathBuf {
        self.state_dir.join(format!("{}.json", self.session_id))
    }

    fn session_dir(&self) -> PathBuf {
        self.state_dir.join(&self.session_id)
    }

    fn session_lock_file(&self) -> PathBuf {
        self.state_dir.join(format!("{}.lock", self.session_id))
    }

    fn ensure_state_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.state_dir).with_context(|| {
            format!(
                "failed to create hive state dir `{}`",
                self.state_dir.display()
            )
        })?;
        fs::create_dir_all(self.session_dir()).with_context(|| {
            format!(
                "failed to create hive session dir `{}`",
                self.session_dir().display()
            )
        })?;
        Ok(())
    }

    fn empty_state(&self) -> HiveSessionState {
        HiveSessionState {
            version: HIVE_VERSION,
            session_id: self.session_id.clone(),
            session_name: self.session_name.clone(),
            queen_pane_id: self.queen_pane_id.clone(),
            tmux_socket: self.tmux_socket.clone(),
            repo_root: self.repo_root.display().to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: now_ms(),
            agents: Vec::new(),
        }
    }

    fn load_state(&self) -> Result<HiveSessionState> {
        let path = self.session_file();
        if !path.exists() {
            return Ok(self.empty_state());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read hive session file `{}`", path.display()))?;
        let mut state: HiveSessionState = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse hive session file `{}`", path.display()))?;
        if state.session_id != self.session_id {
            return Err(anyhow!(
                "hive session file `{}` contains mismatched session_id `{}`",
                path.display(),
                state.session_id
            ));
        }
        state.session_name = self.session_name.clone();
        state.queen_pane_id = self.queen_pane_id.clone();
        state.tmux_socket = self.tmux_socket.clone();
        state.repo_root = self.repo_root.display().to_string();
        Ok(state)
    }

    fn save_state(&self, state: &HiveSessionState) -> Result<()> {
        self.ensure_state_dirs()?;
        let path = self.session_file();
        let tmp = path.with_extension("json.tmp");
        let content = serde_json::to_vec_pretty(state)?;
        fs::write(&tmp, content).with_context(|| {
            format!("failed to write hive session tmp file `{}`", tmp.display())
        })?;
        fs::rename(&tmp, &path).with_context(|| {
            format!(
                "failed to move hive session file into place `{}`",
                path.display()
            )
        })?;
        Ok(())
    }

    fn acquire_lock(&self) -> Result<HiveStateGuard> {
        self.ensure_state_dirs()?;
        let lock_path = self.session_lock_file();
        for _ in 0..200 {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => {
                    return Ok(HiveStateGuard { lock_path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&lock_path, HIVE_LOCK_STALE_MS) {
                        let _ = fs::remove_file(&lock_path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to create hive lock file `{}`", lock_path.display())
                    });
                }
            }
        }
        Err(anyhow!(
            "timed out waiting for hive lock `{}`",
            lock_path.display()
        ))
    }
}

fn lock_is_stale(path: &Path, stale_after_ms: u64) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    age.as_millis() as u64 >= stale_after_ms
}

struct HiveStateGuard {
    lock_path: PathBuf,
}

impl Drop for HiveStateGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

pub async fn run_hive_request(req: HiveRequest) -> Result<Map<String, Value>, ErrorData> {
    let runtime = HiveRuntime::from_env(req.session_id.as_deref())?;
    runtime
        .ensure_state_dirs()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;

    match req.action {
        HiveActionInput::EnsureSession => ensure_session(&runtime).await,
        HiveActionInput::ListAgents => list_agents(&runtime).await,
        HiveActionInput::SpawnAgent => spawn_agent(&runtime, req).await,
        HiveActionInput::SendPrompt => send_prompt(&runtime, req).await,
        HiveActionInput::ResetAgent => reset_agent(&runtime, req).await,
        HiveActionInput::CloseAgent => close_agent(&runtime, req).await,
        HiveActionInput::CloseSession => close_session(&runtime).await,
    }
}

async fn ensure_session(runtime: &HiveRuntime) -> Result<Map<String, Value>, ErrorData> {
    let _lock = runtime.acquire_lock().map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    if !tmux_has_session(&runtime.session_name)
        .await
        .map_err(internal_error)?
    {
        tmux_new_session(&runtime.session_name, &runtime.repo_root)
            .await
            .map_err(internal_error)?;
    }
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    Ok(build_session_response(
        "ready",
        "hive session ready",
        &state,
        None,
    ))
}

async fn list_agents(runtime: &HiveRuntime) -> Result<Map<String, Value>, ErrorData> {
    let _lock = runtime.acquire_lock().map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    if !tmux_has_session(&runtime.session_name)
        .await
        .map_err(internal_error)?
    {
        tmux_new_session(&runtime.session_name, &runtime.repo_root)
            .await
            .map_err(internal_error)?;
    }
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    Ok(build_session_response(
        "listed",
        "hive agents listed",
        &state,
        None,
    ))
}

async fn spawn_agent(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let _lock = runtime.acquire_lock().map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    if !tmux_has_session(&runtime.session_name)
        .await
        .map_err(internal_error)?
    {
        tmux_new_session(&runtime.session_name, &runtime.repo_root)
            .await
            .map_err(internal_error)?;
    }
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;

    let live_count = state
        .agents
        .iter()
        .filter(|agent| agent.status != HiveAgentStatus::Dead)
        .count();
    if live_count >= HIVE_AGENT_LIMIT {
        return Ok(build_error_response(
            "limit_reached",
            format!("hive agent limit reached: maximum {HIVE_AGENT_LIMIT} live agents"),
            &state,
            None,
        ));
    }

    let workdir = resolve_workdir(req.workdir.as_deref(), runtime);
    if !workdir.is_dir() {
        return Err(ErrorData::invalid_params(
            format!(
                "workdir does not exist or is not a directory: `{}`",
                workdir.display()
            ),
            None,
        ));
    }

    let agent_id = next_agent_id(&state.agents);
    let worker_name = req
        .worker_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| agent_id.clone());
    let window_name = sanitize_window_name(&worker_name);
    let launch_command = build_worker_shell_command(
        runtime,
        &agent_id,
        &worker_name,
        &req.agent_kind,
        req.model.as_deref(),
        &workdir,
    )
    .map_err(internal_error)?;
    let (window_id, pane_id) = tmux_new_window(
        &runtime.session_name,
        &window_name,
        &workdir,
        Some(&launch_command),
    )
    .await
    .map_err(internal_error)?;

    let agent = HiveAgentState {
        agent_id: agent_id.clone(),
        worker_name,
        agent_kind: req.agent_kind,
        model: req.model,
        workdir: workdir.display().to_string(),
        window_id,
        window_name,
        pane_id,
        status: HiveAgentStatus::Spawning,
        last_used_at_ms: None,
    };
    state.agents.push(agent.clone());
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    Ok(build_session_response(
        "spawning",
        "hive agent spawned",
        &state,
        Some(&agent),
    ))
}

async fn send_prompt(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let agent_id = req.agent_id.ok_or_else(|| {
        ErrorData::invalid_params("`agent_id` is required for action=`send_prompt`", None)
    })?;
    let prompt = req.prompt.ok_or_else(|| {
        ErrorData::invalid_params("`prompt` is required for action=`send_prompt`", None)
    })?;

    let _lock = runtime.acquire_lock().map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;
    let existing_status = state
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id && agent.status != HiveAgentStatus::Dead)
        .map(|agent| agent.status.clone())
        .ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "live hive agent `{agent_id}` was not found in session `{}`",
                    runtime.session_id
                ),
                None,
            )
        })?;
    if !prompt_accept_state(&existing_status) {
        let agent = state
            .agents
            .iter()
            .find(|agent| agent.agent_id == agent_id && agent.status != HiveAgentStatus::Dead)
            .expect("checked above");
        return Ok(build_error_response(
            agent.status.as_str(),
            format!(
                "hive agent `{agent_id}` is currently `{}` and cannot accept a new prompt",
                agent.status.as_str()
            ),
            &state,
            Some(agent),
        ));
    }

    let pane_id = state
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id && agent.status != HiveAgentStatus::Dead)
        .map(|agent| agent.pane_id.clone())
        .expect("checked above");
    tmux_send_text(&pane_id, &prompt)
        .await
        .map_err(internal_error)?;
    let now = now_ms();
    let mut response_agent_id = None;
    for agent in &mut state.agents {
        if agent.agent_id == agent_id && agent.status != HiveAgentStatus::Dead {
            agent.status = HiveAgentStatus::Busy;
            agent.last_used_at_ms = Some(now);
            response_agent_id = Some(agent.agent_id.clone());
            break;
        }
    }
    state.updated_at_ms = now;
    runtime.save_state(&state).map_err(internal_error)?;
    let agent = state
        .agents
        .iter()
        .find(|agent| response_agent_id.as_deref() == Some(agent.agent_id.as_str()))
        .expect("updated agent should exist");
    Ok(build_session_response(
        "busy",
        format!("prompt sent to hive agent `{agent_id}`"),
        &state,
        Some(agent),
    ))
}

async fn reset_agent(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let agent_id = req.agent_id.ok_or_else(|| {
        ErrorData::invalid_params("`agent_id` is required for action=`reset_agent`", None)
    })?;

    let _lock = runtime.acquire_lock().map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;
    let pane_id = state
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id && agent.status != HiveAgentStatus::Dead)
        .map(|agent| agent.pane_id.clone())
        .ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "live hive agent `{agent_id}` was not found in session `{}`",
                    runtime.session_id
                ),
                None,
            )
        })?;
    tmux_send_text(&pane_id, "/new")
        .await
        .map_err(internal_error)?;
    let now = now_ms();
    let mut response_agent_id = None;
    for agent in &mut state.agents {
        if agent.agent_id == agent_id && agent.status != HiveAgentStatus::Dead {
            agent.status = HiveAgentStatus::Resetting;
            agent.last_used_at_ms = Some(now);
            response_agent_id = Some(agent.agent_id.clone());
            break;
        }
    }
    state.updated_at_ms = now;
    runtime.save_state(&state).map_err(internal_error)?;
    let agent = state
        .agents
        .iter()
        .find(|agent| response_agent_id.as_deref() == Some(agent.agent_id.as_str()))
        .expect("updated agent should exist");
    Ok(build_session_response(
        "resetting",
        format!("reset requested for hive agent `{agent_id}`; waiting for session-start hook to mark idle"),
        &state,
        Some(agent),
    ))
}

async fn close_agent(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let agent_id = req.agent_id.ok_or_else(|| {
        ErrorData::invalid_params("`agent_id` is required for action=`close_agent`", None)
    })?;

    let _lock = runtime.acquire_lock().map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;

    let target = state
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id && agent.status != HiveAgentStatus::Dead)
        .cloned()
        .ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "live hive agent `{agent_id}` was not found in session `{}`",
                    runtime.session_id
                ),
                None,
            )
        })?;

    tmux_kill_window(&target.window_id)
        .await
        .map_err(internal_error)?;
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;

    let agent = state
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id)
        .or(Some(&target));
    Ok(build_session_response(
        "closed_agent",
        format!("hive agent `{agent_id}` closed"),
        &state,
        agent,
    ))
}

async fn close_session(runtime: &HiveRuntime) -> Result<Map<String, Value>, ErrorData> {
    let _lock = runtime.acquire_lock().map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_tmux(runtime, &mut state)
        .await
        .map_err(internal_error)?;

    if tmux_has_session(&runtime.session_name)
        .await
        .map_err(internal_error)?
    {
        tmux_kill_session(&runtime.session_name)
            .await
            .map_err(internal_error)?;
    }

    for agent in &mut state.agents {
        agent.status = HiveAgentStatus::Dead;
    }
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    Ok(build_session_response(
        "closed_session",
        "hive session closed",
        &state,
        None,
    ))
}

fn resolve_workdir(workdir: Option<&str>, runtime: &HiveRuntime) -> PathBuf {
    match workdir {
        Some(path) if !path.trim().is_empty() => {
            let candidate = PathBuf::from(path);
            if candidate.is_absolute() {
                candidate
            } else {
                runtime.repo_root.join(candidate)
            }
        }
        _ => runtime.repo_root.clone(),
    }
}

async fn sync_state_with_tmux(runtime: &HiveRuntime, state: &mut HiveSessionState) -> Result<()> {
    let panes = tmux_list_session_panes(&runtime.session_name).await?;
    let pane_map: HashMap<_, _> = panes
        .into_iter()
        .map(|pane| (pane.pane_id.clone(), pane))
        .collect();

    for agent in &mut state.agents {
        if let Some(pane) = pane_map.get(&agent.pane_id) {
            agent.window_id = pane.window_id.clone();
            agent.window_name = pane.window_name.clone();
        } else {
            agent.status = HiveAgentStatus::Dead;
        }
    }
    Ok(())
}

fn next_agent_id(existing: &[HiveAgentState]) -> String {
    let max_index = existing
        .iter()
        .filter_map(|agent| {
            agent
                .agent_id
                .strip_prefix("agent-")
                .and_then(|suffix| suffix.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    format!("agent-{:02}", max_index + 1)
}

fn derive_session_id(queen_pane_id: &str, tmux_socket: Option<&str>) -> String {
    let mut hasher = DefaultHasher::new();
    queen_pane_id.hash(&mut hasher);
    tmux_socket.unwrap_or_default().hash(&mut hasher);
    let pane = queen_pane_id.trim_start_matches('%');
    format!(
        "hive-q{pane}-{:08x}",
        (hasher.finish() & 0xffff_ffff) as u32
    )
}

fn sanitize_window_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed.to_string()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn internal_error(err: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(err.to_string(), None)
}

fn build_session_response(
    state_name: &str,
    message: impl Into<String>,
    state: &HiveSessionState,
    agent: Option<&HiveAgentState>,
) -> Map<String, Value> {
    let agents: Vec<Value> = state.agents.iter().map(agent_json).collect();
    let reusable_agents: Vec<Value> = state
        .agents
        .iter()
        .filter(|agent| agent.status == HiveAgentStatus::Idle)
        .map(|agent| Value::String(agent.agent_id.clone()))
        .collect();

    let mut raw = Map::new();
    raw.insert("ok".to_string(), Value::Bool(true));
    raw.insert("state".to_string(), Value::String(state_name.to_string()));
    raw.insert("message".to_string(), Value::String(message.into()));
    raw.insert(
        "session".to_string(),
        serde_json::json!({
            "id": state.session_id,
            "name": state.session_name,
            "queen_pane_id": state.queen_pane_id,
            "agent_limit": state.agent_limit,
            "state": "ready"
        }),
    );
    raw.insert("agents".to_string(), Value::Array(agents));
    raw.insert("reusable_agents".to_string(), Value::Array(reusable_agents));
    if let Some(agent) = agent {
        raw.insert("agent".to_string(), agent_json(agent));
    }
    raw
}

fn build_error_response(
    state_name: &str,
    message: impl Into<String>,
    state: &HiveSessionState,
    agent: Option<&HiveAgentState>,
) -> Map<String, Value> {
    let mut raw = build_session_response(state_name, message, state, agent);
    raw.insert("ok".to_string(), Value::Bool(false));
    raw
}

fn agent_json(agent: &HiveAgentState) -> Value {
    serde_json::json!({
        "agent_id": agent.agent_id,
        "worker_name": agent.worker_name,
        "agent_kind": agent.agent_kind.as_str(),
        "model": agent.model,
        "workdir": agent.workdir,
        "window_id": agent.window_id,
        "window_name": agent.window_name,
        "pane_id": agent.pane_id,
        "status": agent.status.as_str(),
        "last_used_at_ms": agent.last_used_at_ms
    })
}

async fn tmux_has_session(session_name: &str) -> Result<bool> {
    let status = Command::new("tmux")
        .arg("has-session")
        .arg("-t")
        .arg(session_name)
        .status()
        .await
        .context("failed to run `tmux has-session`")?;
    Ok(status.success())
}

async fn tmux_new_session(session_name: &str, workdir: &Path) -> Result<()> {
    let status = Command::new("tmux")
        .arg("new-session")
        .arg("-d")
        .arg("-s")
        .arg(session_name)
        .arg("-n")
        .arg(HIVE_BOOTSTRAP_WINDOW)
        .arg("-c")
        .arg(workdir)
        .status()
        .await
        .context("failed to run `tmux new-session`")?;
    if !status.success() {
        return Err(anyhow!("`tmux new-session` exited with status {status}"));
    }
    Ok(())
}

async fn tmux_new_window(
    session_name: &str,
    window_name: &str,
    workdir: &Path,
    shell_command: Option<&str>,
) -> Result<(String, String)> {
    let mut command = Command::new("tmux");
    command.args(tmux_new_window_args(
        session_name,
        window_name,
        workdir,
        shell_command,
    ));
    let output = command
        .output()
        .await
        .context("failed to run `tmux new-window`")?;
    if !output.status.success() {
        return Err(anyhow!(
            "`tmux new-window` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).context("tmux output was not utf-8")?;
    let mut parts = text.trim().split('\t');
    let window_id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing tmux window_id in new-window output"))?;
    let pane_id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing tmux pane_id in new-window output"))?;
    Ok((window_id.to_string(), pane_id.to_string()))
}

fn tmux_new_window_args(
    session_name: &str,
    window_name: &str,
    workdir: &Path,
    shell_command: Option<&str>,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("new-window"),
        OsString::from("-d"),
        OsString::from("-t"),
        OsString::from(session_name),
        OsString::from("-n"),
        OsString::from(window_name),
        OsString::from("-c"),
        workdir.as_os_str().to_os_string(),
        OsString::from("-P"),
        OsString::from("-F"),
        OsString::from("#{window_id}\t#{pane_id}"),
    ];
    if let Some(command) = shell_command {
        args.push(OsString::from(command));
    }
    args
}

async fn tmux_list_session_panes(session_name: &str) -> Result<Vec<SessionPaneInfo>> {
    let output = Command::new("tmux")
        .arg("list-panes")
        .arg("-a")
        .arg("-F")
        .arg("#{session_name}\t#{window_id}\t#{window_name}\t#{pane_id}")
        .output()
        .await
        .context("failed to run `tmux list-panes`")?;
    if !output.status.success() {
        return Err(anyhow!(
            "`tmux list-panes` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).context("tmux output was not utf-8")?;
    let mut panes = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('\t');
        let listed_session = parts.next().unwrap_or_default();
        if listed_session != session_name {
            continue;
        }
        let window_id = parts.next().unwrap_or_default();
        let window_name = parts.next().unwrap_or_default();
        let pane_id = parts.next().unwrap_or_default();
        if pane_id.is_empty() {
            continue;
        }
        panes.push(SessionPaneInfo {
            window_id: window_id.to_string(),
            window_name: window_name.to_string(),
            pane_id: pane_id.to_string(),
        });
    }
    Ok(panes)
}

async fn tmux_send_text(pane_id: &str, text: &str) -> Result<()> {
    run_tmux_send_keys(pane_id, &["C-u"]).await?;
    sleep(Duration::from_millis(300)).await;
    run_tmux_send_keys(pane_id, &["-l", text]).await?;
    sleep(Duration::from_millis(300)).await;
    run_tmux_send_keys(pane_id, &["C-m"]).await?;
    sleep(Duration::from_millis(300)).await;
    run_tmux_send_keys(pane_id, &["Enter"]).await?;
    Ok(())
}

async fn run_tmux_send_keys(pane_id: &str, keys: &[&str]) -> Result<()> {
    let mut command = Command::new("tmux");
    command.arg("send-keys").arg("-t").arg(pane_id);
    command.args(keys);
    let status = command
        .status()
        .await
        .with_context(|| format!("failed to run `tmux send-keys` for pane `{pane_id}`"))?;
    if !status.success() {
        return Err(anyhow!(
            "`tmux send-keys` exited with status {status} for pane `{pane_id}`"
        ));
    }
    Ok(())
}

async fn tmux_kill_window(window_id: &str) -> Result<()> {
    let status = Command::new("tmux")
        .arg("kill-window")
        .arg("-t")
        .arg(window_id)
        .status()
        .await
        .with_context(|| format!("failed to run `tmux kill-window` for `{window_id}`"))?;
    if !status.success() {
        return Err(anyhow!(
            "`tmux kill-window` exited with status {status} for `{window_id}`"
        ));
    }
    Ok(())
}

async fn tmux_kill_session(session_name: &str) -> Result<()> {
    let status = Command::new("tmux")
        .arg("kill-session")
        .arg("-t")
        .arg(session_name)
        .status()
        .await
        .with_context(|| format!("failed to run `tmux kill-session` for `{session_name}`"))?;
    if !status.success() {
        return Err(anyhow!(
            "`tmux kill-session` exited with status {status} for `{session_name}`"
        ));
    }
    Ok(())
}

fn build_worker_shell_command(
    runtime: &HiveRuntime,
    agent_id: &str,
    worker_name: &str,
    agent_kind: &HiveAgentKindInput,
    model: Option<&str>,
    workdir: &Path,
) -> Result<String> {
    let hook_session_start = runtime
        .repo_root
        .join("hooks")
        .join("hive-session-start.ts");
    let hook_agent_stop = runtime.repo_root.join("hooks").join("hive-agent-stop.ts");
    let cc_hooks_src = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".dotfiles")
        .join("packages")
        .join("cc-hooks")
        .join("src");
    let env_prefix = format!(
        "AGPOD_HIVE_MODE=1 AGPOD_HIVE_STATE_DIR={} AGPOD_HIVE_SESSION_ID={} AGPOD_HIVE_AGENT_ID={} TMUX_HIVE_QUEEN={} TMUX_HIVE_WORKER_NAME={} CC_HOOKS_SRC={}",
        shell_escape(&runtime.state_dir.display().to_string()),
        shell_escape(&runtime.session_id),
        shell_escape(agent_id),
        shell_escape(&runtime.queen_pane_id),
        shell_escape(worker_name),
        shell_escape(&cc_hooks_src.display().to_string()),
    );

    match agent_kind {
        HiveAgentKindInput::Claude => {
            let settings_path =
                build_claude_settings(runtime, agent_id, &hook_session_start, &hook_agent_stop)?;
            let mut command = format!(
                "{env_prefix} claude --settings {} -n {}",
                shell_escape(&settings_path.display().to_string()),
                shell_escape(worker_name),
            );
            if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
                command.push_str(" --model ");
                command.push_str(&shell_escape(model));
            }
            if workdir != runtime.repo_root {
                command.push_str(" --add-dir ");
                command.push_str(&shell_escape(&workdir.display().to_string()));
            }
            Ok(command)
        }
        HiveAgentKindInput::Codex => {
            let codex_home =
                build_codex_home(runtime, agent_id, &hook_session_start, &hook_agent_stop)?;
            let mut command = format!(
                "CODEX_HOME={} {env_prefix} codex --no-alt-screen -C {}",
                shell_escape(&codex_home.display().to_string()),
                shell_escape(&workdir.display().to_string()),
            );
            if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
                command.push_str(" -m ");
                command.push_str(&shell_escape(model));
            }
            Ok(command)
        }
    }
}

fn build_claude_settings(
    runtime: &HiveRuntime,
    agent_id: &str,
    hook_session_start: &Path,
    hook_agent_stop: &Path,
) -> Result<PathBuf> {
    let settings_path = runtime
        .session_dir()
        .join(agent_id)
        .join("claude-settings.json");
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let source = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".claude")
        .join("settings.json");
    let mut root: Value = if source.exists() {
        serde_json::from_str(&fs::read_to_string(&source)?)
            .context("failed to parse ~/.claude/settings.json")?
    } else {
        serde_json::json!({})
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("claude settings root must be a JSON object"))?;
    let hooks = object
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow!("claude settings `hooks` must be an object"))?;

    append_hook_command(
        hooks_obj,
        "SessionStart",
        format!(
            "bun run -i --silent {} --agent claude",
            hook_session_start.display()
        ),
    );
    append_hook_command(
        hooks_obj,
        "Stop",
        format!(
            "bun run -i --silent {} --agent claude",
            hook_agent_stop.display()
        ),
    );

    fs::write(&settings_path, serde_json::to_vec_pretty(&root)?)?;
    Ok(settings_path)
}

fn append_hook_command(hooks_obj: &mut Map<String, Value>, event: &str, command: String) {
    let entry = hooks_obj
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let events = entry.as_array_mut().expect("hook event should be array");
    events.push(serde_json::json!({
        "matcher": "",
        "hooks": [
            {
                "type": "command",
                "command": command
            }
        ]
    }));
}

fn build_codex_home(
    runtime: &HiveRuntime,
    agent_id: &str,
    hook_session_start: &Path,
    hook_agent_stop: &Path,
) -> Result<PathBuf> {
    let source_home = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".codex");
    let target_home = runtime.session_dir().join(agent_id).join("codex-home");
    fs::create_dir_all(&target_home)?;

    for entry in fs::read_dir(&source_home)
        .with_context(|| format!("failed to read `{}`", source_home.display()))?
    {
        let entry = entry?;
        let source = entry.path();
        let file_name = entry.file_name();
        if file_name == "hooks.json" {
            continue;
        }
        let target = target_home.join(&file_name);
        replace_with_symlink(&source, &target)?;
    }

    let source_hooks = source_home.join("hooks.json");
    let mut hooks_root: Value = if source_hooks.exists() {
        serde_json::from_str(&fs::read_to_string(&source_hooks)?)
            .context("failed to parse ~/.codex/hooks.json")?
    } else {
        serde_json::json!({ "hooks": {} })
    };
    let object = hooks_root
        .as_object_mut()
        .ok_or_else(|| anyhow!("codex hooks root must be a JSON object"))?;
    let hooks = object
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow!("codex hooks `hooks` must be an object"))?;

    append_hook_command(
        hooks_obj,
        "SessionStart",
        format!(
            "bun run -i --silent {} --agent codex",
            hook_session_start.display()
        ),
    );
    append_hook_command(
        hooks_obj,
        "Stop",
        format!(
            "bun run -i --silent {} --agent codex",
            hook_agent_stop.display()
        ),
    );
    fs::write(
        target_home.join("hooks.json"),
        serde_json::to_vec_pretty(&hooks_root)?,
    )?;
    Ok(target_home)
}

fn replace_with_symlink(source: &Path, target: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(target) {
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(target)?;
        } else {
            fs::remove_file(target)?;
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)?;
    }
    #[cfg(not(unix))]
    {
        if source.is_dir() {
            fs::create_dir_all(target)?;
        } else {
            fs::copy(source, target)?;
        }
    }
    Ok(())
}

fn shell_escape(input: &str) -> String {
    if input.is_empty() {
        return "''".to_string();
    }
    let escaped = input.replace('\'', r#"'\''"#);
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;
    use tempfile::tempdir;

    #[test]
    fn derive_session_id_is_stable_for_same_pane() {
        let first = derive_session_id("%9", Some("/tmp/tmux-501/default"));
        let second = derive_session_id("%9", Some("/tmp/tmux-501/default"));
        assert_eq!(first, second);
        assert!(first.starts_with("hive-q9-"));
    }

    #[test]
    fn hive_runtime_from_env_reports_missing_tmux_pane_helpfully() {
        let previous = std::env::var_os("TMUX_PANE");
        std::env::remove_var("TMUX_PANE");

        let err = HiveRuntime::from_env(None).expect_err("missing TMUX_PANE should fail");
        assert!(err.message.contains("Check your MCP configuration"));
        assert!(err.message.contains("TMUX_PANE"));

        if let Some(value) = previous {
            std::env::set_var("TMUX_PANE", value);
        }
    }

    #[test]
    fn window_name_sanitizes_symbols() {
        assert_eq!(sanitize_window_name("worker:alpha"), "worker-alpha");
        assert_eq!(sanitize_window_name("  "), "agent");
    }

    #[test]
    fn tmux_new_window_args_append_shell_command_when_present() {
        let workdir = Path::new("/tmp/project");

        let without_command = tmux_new_window_args("session", "worker", workdir, None);
        assert_eq!(
            without_command.last().and_then(|s| s.to_str()),
            Some("#{window_id}\t#{pane_id}")
        );

        let with_command = tmux_new_window_args("session", "worker", workdir, Some("exec codex"));
        assert_eq!(
            with_command.last().and_then(|s| s.to_str()),
            Some("exec codex")
        );
        assert!(with_command.iter().any(|arg| arg == "worker"));
    }

    #[test]
    fn next_agent_id_skips_existing_indexes() {
        let existing = vec![
            HiveAgentState {
                agent_id: "agent-01".to_string(),
                worker_name: "a".to_string(),
                agent_kind: HiveAgentKindInput::Codex,
                model: None,
                workdir: "/tmp".to_string(),
                window_id: "@1".to_string(),
                window_name: "a".to_string(),
                pane_id: "%1".to_string(),
                status: HiveAgentStatus::Dead,
                last_used_at_ms: None,
            },
            HiveAgentState {
                agent_id: "agent-03".to_string(),
                worker_name: "b".to_string(),
                agent_kind: HiveAgentKindInput::Claude,
                model: None,
                workdir: "/tmp".to_string(),
                window_id: "@2".to_string(),
                window_name: "b".to_string(),
                pane_id: "%2".to_string(),
                status: HiveAgentStatus::Idle,
                last_used_at_ms: None,
            },
        ];
        assert_eq!(next_agent_id(&existing), "agent-04");
    }

    #[test]
    fn build_session_response_marks_idle_agents_reusable() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-q9".to_string(),
            session_name: "agpod-hive-q9".to_string(),
            queen_pane_id: "%9".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: vec![
                HiveAgentState {
                    agent_id: "agent-01".to_string(),
                    worker_name: "idle".to_string(),
                    agent_kind: HiveAgentKindInput::Codex,
                    model: None,
                    workdir: "/repo".to_string(),
                    window_id: "@1".to_string(),
                    window_name: "idle".to_string(),
                    pane_id: "%11".to_string(),
                    status: HiveAgentStatus::Idle,
                    last_used_at_ms: Some(1),
                },
                HiveAgentState {
                    agent_id: "agent-02".to_string(),
                    worker_name: "busy".to_string(),
                    agent_kind: HiveAgentKindInput::Claude,
                    model: None,
                    workdir: "/repo".to_string(),
                    window_id: "@2".to_string(),
                    window_name: "busy".to_string(),
                    pane_id: "%12".to_string(),
                    status: HiveAgentStatus::Busy,
                    last_used_at_ms: Some(2),
                },
            ],
        };

        let raw = build_session_response("listed", "ok", &state, None);
        let reusable = raw
            .get("reusable_agents")
            .and_then(Value::as_array)
            .expect("reusable agents should be array");
        assert_eq!(reusable, &vec![Value::String("agent-01".to_string())]);
    }

    #[test]
    fn prompt_only_targets_idle_agents() {
        assert!(prompt_accept_state(&HiveAgentStatus::Idle));
        assert!(!prompt_accept_state(&HiveAgentStatus::Spawning));
        assert!(!prompt_accept_state(&HiveAgentStatus::Busy));
        assert!(!prompt_accept_state(&HiveAgentStatus::Resetting));
        assert!(!prompt_accept_state(&HiveAgentStatus::Dead));
    }

    #[test]
    fn hive_runtime_lock_blocks_reentry_until_drop() {
        let temp = tempdir().expect("temp dir");
        let runtime = HiveRuntime {
            repo_root: temp.path().to_path_buf(),
            state_dir: temp.path().join("state"),
            session_id: "hive-q1".to_string(),
            session_name: "agpod-hive-q1".to_string(),
            queen_pane_id: "%1".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
        };

        let first = runtime.acquire_lock().expect("first lock");
        let second = runtime.acquire_lock();
        assert!(second.is_err());
        drop(first);
        let third = runtime.acquire_lock();
        assert!(third.is_ok());
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let temp = tempdir().expect("temp dir");
        let runtime = HiveRuntime {
            repo_root: temp.path().to_path_buf(),
            state_dir: temp.path().join("state"),
            session_id: "hive-q1".to_string(),
            session_name: "agpod-hive-q1".to_string(),
            queen_pane_id: "%1".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
        };
        runtime.ensure_state_dirs().expect("state dir");
        let lock = runtime.session_lock_file();
        fs::write(&lock, "").expect("write stale lock");
        let status = StdCommand::new("touch")
            .arg("-t")
            .arg("200001010000")
            .arg(&lock)
            .status()
            .expect("touch stale lock");
        assert!(status.success());
        assert!(runtime.acquire_lock().is_ok());
    }
}
