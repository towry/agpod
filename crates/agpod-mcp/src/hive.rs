//! Hive tool support for tmux-backed Claude exec workers.
//!
//! Keywords: hive, tmux, claude, exec, output file, worker status

use agpod_core::{Config, McpHiveClaudeConfig, McpHiveClaudeModeConfig};
use anyhow::{anyhow, Context, Result};
use rmcp::{
    model::{CallToolResult, Content, JsonObject},
    schemars, ErrorData,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{hash_map::DefaultHasher, BTreeMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tracing::warn;
use uuid::Uuid;

const HIVE_VERSION: u32 = 2;
const HIVE_AGENT_LIMIT: usize = 5;
const HIVE_BOOTSTRAP_WINDOW: &str = "__HIVE_HOME_EMPTY__";
const HIVE_LOCK_STALE_MS: u64 = 30_000;
const OUTPUT_EXCERPT_LIMIT: usize = 1200;
const SUPPORTED_MODE_NAMES: [&str; 2] = ["readonly", "full"];

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HiveActionInput {
    EnsureSession,
    ModeInfo,
    ListAgents,
    SpawnAgent,
    SendPrompt,
    CloseAgent,
    CloseSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HiveRequest {
    /// Hive action to perform.
    pub action: HiveActionInput,
    /// Derived session ID from the caller tmux pane. Optional validation guard for chained calls.
    pub session_id: Option<String>,
    /// Existing worker agent ID for `send_prompt` and `close_agent`.
    pub agent_id: Option<String>,
    /// Named Claude mode from agpod config. Supported public modes are `readonly` and `full`.
    /// Reads `[mcp.hive.claude.modes.<name>]`; `~` in configured paths is expanded.
    pub mode: Option<String>,
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
    mode: String,
    workdir: String,
    status: HiveAgentStatus,
    current_run: Option<HiveRunState>,
    last_run: Option<HiveRunState>,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HiveRunState {
    run_id: String,
    prompt_preview: String,
    output_path: String,
    prompt_path: String,
    result_path: String,
    window_id: Option<String>,
    pane_id: Option<String>,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
    exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum HiveAgentStatus {
    Idle,
    Running,
    Closed,
}

impl HiveAgentStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Closed => "closed",
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
    config: Config,
}

#[derive(Debug, Clone)]
struct SessionPaneInfo {
    window_id: String,
    pane_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HiveRunResultFile {
    exit_code: i32,
    started_at_ms: u64,
    finished_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyHiveSessionState {
    version: u32,
    session_id: String,
    session_name: String,
    queen_pane_id: String,
    tmux_socket: Option<String>,
    repo_root: String,
    agent_limit: usize,
    updated_at_ms: u64,
    agents: Vec<LegacyHiveAgentState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyHiveAgentState {
    agent_id: String,
    worker_name: String,
    workdir: String,
    window_id: String,
    pane_id: String,
    status: LegacyHiveAgentStatus,
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyHiveAgentStatus {
    Spawning,
    Idle,
    Busy,
    Resetting,
    Dead,
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
            config: Config::load(),
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

    fn gc_lock_file(&self) -> PathBuf {
        self.state_dir.join(".gc.lock")
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
        let mut state = parse_hive_session_state(&raw)
            .with_context(|| format!("failed to parse hive session file `{}`", path.display()))?;
        if state.session_id != self.session_id {
            return Err(anyhow!(
                "hive session file `{}` contains mismatched session_id `{}`",
                path.display(),
                state.session_id
            ));
        }
        state.version = HIVE_VERSION;
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
        acquire_lock_file(self.session_lock_file())
    }

    fn acquire_gc_lock(&self) -> Result<HiveStateGuard> {
        self.ensure_state_dirs()?;
        acquire_lock_file(self.gc_lock_file())
    }

    fn acquire_session_lock(&self, session_id: &str) -> Result<HiveStateGuard> {
        self.ensure_state_dirs()?;
        ensure_valid_session_id(session_id)?;
        acquire_lock_file(self.state_dir.join(format!("{session_id}.lock")))
    }

    fn claude_config(&self) -> Option<&McpHiveClaudeConfig> {
        self.config
            .mcp
            .as_ref()
            .and_then(|mcp| mcp.hive.as_ref())
            .and_then(|hive| hive.claude.as_ref())
    }

    fn resolve_mode_name(&self, requested: Option<&str>) -> String {
        requested
            .map(str::trim)
            .filter(|mode| !mode.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "readonly".to_string())
    }

    fn resolve_mode_config(&self, mode_name: &str) -> Result<McpHiveClaudeModeConfig, ErrorData> {
        if !SUPPORTED_MODE_NAMES.contains(&mode_name) {
            return Err(ErrorData::invalid_params(
                format!("unsupported hive mode `{mode_name}`; use `readonly` or `full`"),
                None,
            ));
        }
        let config = self
            .claude_config()
            .and_then(|cfg| cfg.modes.get(mode_name))
            .cloned()
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    format!(
                        "missing hive Claude mode config `{mode_name}` in `[mcp.hive.claude.modes.{mode_name}]`; call `hive` with action=`mode_info` for the expected shape"
                    ),
                    None,
                )
            })?;
        validate_mode_config(mode_name, &config)?;
        Ok(config)
    }
}

fn parse_hive_session_state(raw: &str) -> Result<HiveSessionState> {
    match serde_json::from_str::<HiveSessionState>(raw) {
        Ok(state) => Ok(state),
        Err(current_err) => match serde_json::from_str::<LegacyHiveSessionState>(raw) {
            Ok(legacy) => Ok(migrate_legacy_hive_session_state(legacy)),
            Err(_) => Err(current_err).context("failed to parse current or legacy hive session state"),
        },
    }
}

fn migrate_legacy_hive_session_state(legacy: LegacyHiveSessionState) -> HiveSessionState {
    HiveSessionState {
        version: HIVE_VERSION,
        session_id: legacy.session_id,
        session_name: legacy.session_name,
        queen_pane_id: legacy.queen_pane_id,
        tmux_socket: legacy.tmux_socket,
        repo_root: legacy.repo_root,
        agent_limit: legacy.agent_limit,
        updated_at_ms: legacy.updated_at_ms,
        agents: legacy
            .agents
            .into_iter()
            .map(migrate_legacy_hive_agent_state)
            .collect(),
    }
}

fn migrate_legacy_hive_agent_state(legacy: LegacyHiveAgentState) -> HiveAgentState {
    let (status, current_run) = match legacy.status {
        LegacyHiveAgentStatus::Idle => (HiveAgentStatus::Idle, None),
        LegacyHiveAgentStatus::Dead => (HiveAgentStatus::Closed, None),
        LegacyHiveAgentStatus::Busy | LegacyHiveAgentStatus::Resetting | LegacyHiveAgentStatus::Spawning => (
            HiveAgentStatus::Running,
            Some(HiveRunState {
                run_id: format!("legacy-{}", legacy.agent_id),
                prompt_preview: "legacy interactive hive state".to_string(),
                output_path: String::new(),
                prompt_path: String::new(),
                result_path: String::new(),
                window_id: Some(legacy.window_id.clone()),
                pane_id: Some(legacy.pane_id.clone()),
                started_at_ms: legacy.last_used_at_ms.unwrap_or(0),
                finished_at_ms: None,
                exit_code: None,
            }),
        ),
    };

    HiveAgentState {
        agent_id: legacy.agent_id,
        worker_name: legacy.worker_name,
        mode: "readonly".to_string(),
        workdir: legacy.workdir,
        status,
        current_run,
        last_run: None,
        last_used_at_ms: legacy.last_used_at_ms,
    }
}

fn acquire_lock_file(lock_path: PathBuf) -> Result<HiveStateGuard> {
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
        HiveActionInput::ModeInfo => mode_info(&runtime, req).await,
        HiveActionInput::ListAgents => list_agents(&runtime).await,
        HiveActionInput::SpawnAgent => spawn_agent(&runtime, req).await,
        HiveActionInput::SendPrompt => send_prompt(&runtime, req).await,
        HiveActionInput::CloseAgent => close_agent(&runtime, req).await,
        HiveActionInput::CloseSession => close_session(&runtime).await,
    }
}

async fn ensure_session(runtime: &HiveRuntime) -> Result<Map<String, Value>, ErrorData> {
    cleanup_orphaned_hive_sessions(runtime)
        .await
        .map_err(internal_error)?;
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

async fn mode_info(runtime: &HiveRuntime, req: HiveRequest) -> Result<Map<String, Value>, ErrorData> {
    let requested = req.mode.as_deref().map(str::trim).filter(|value| !value.is_empty());
    let selected = requested.unwrap_or("readonly");
    if !SUPPORTED_MODE_NAMES.contains(&selected) {
        return Err(ErrorData::invalid_params(
            format!("unsupported hive mode `{selected}`; use `readonly` or `full`"),
            None,
        ));
    }

    let config = runtime
        .claude_config()
        .and_then(|cfg| cfg.modes.get(selected))
        .cloned();
    let configured = config.is_some();
    let mode_config = config.as_ref();
    let mut raw = Map::new();
    raw.insert("ok".to_string(), Value::Bool(true));
    raw.insert("state".to_string(), Value::String("mode_info".to_string()));
    raw.insert(
        "message".to_string(),
        Value::String(if configured {
            format!("hive mode `{selected}` is configured")
        } else {
            format!(
                "hive mode `{selected}` is not configured; add `[mcp.hive.claude.modes.{selected}]`"
            )
        }),
    );
    raw.insert(
        "mode_info".to_string(),
        serde_json::json!({
            "selected_mode": selected,
            "supported_modes": SUPPORTED_MODE_NAMES,
            "configured": configured,
            "config_path": format!("[mcp.hive.claude.modes.{selected}]"),
            "default_mode_behavior": "hive defaults to `readonly` when `mode` is omitted",
            "fields": ["description", "command", "args", "settings", "mcp_config", "env"],
            "notes": [
                "Only `readonly` and `full` are valid public mode names.",
                "Configured `settings` and `mcp_config` paths may begin with `~`; hive expands them to the current user home directory before launch.",
                "If a mode is missing, `spawn_agent` and `send_prompt` fail fast instead of guessing defaults."
            ],
            "configured_values": mode_config.map(|cfg| serde_json::json!({
                "description": cfg.description.clone(),
                "command": cfg.command.clone(),
                "args": cfg.args.clone(),
                "settings": cfg.settings.clone(),
                "mcp_config": cfg.mcp_config.clone(),
                "env_keys": cfg.env.keys().cloned().collect::<Vec<_>>(),
            })),
            "example": {
                "readonly": {
                    "description": "Read-only Claude worker for inspection, summarization, and analysis.",
                    "command": "claw",
                    "args": ["--dangerously-skip-permissions"],
                    "settings": "~/.claude/settings.json",
                    "mcp_config": "~/.claude/generated/mcp-readonly.json",
                    "env": { "MAX_MCP_OUTPUT_TOKENS": "12000" }
                },
                "full": {
                    "description": "Full-access Claude worker for implementation and editing tasks.",
                    "command": "claw",
                    "args": [],
                    "settings": "~/.claude/settings.json",
                    "mcp_config": "~/.mcp.json",
                    "env": {}
                }
            }
        }),
    );
    Ok(raw)
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
        .filter(|agent| agent.status != HiveAgentStatus::Closed)
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

    let mode = runtime.resolve_mode_name(req.mode.as_deref());
    let _ = runtime.resolve_mode_config(&mode)?;
    let agent_id = next_agent_id(&state.agents);
    let worker_name = req
        .worker_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| agent_id.clone());

    let agent = HiveAgentState {
        agent_id: agent_id.clone(),
        worker_name,
        mode,
        workdir: workdir.display().to_string(),
        status: HiveAgentStatus::Idle,
        current_run: None,
        last_run: None,
        last_used_at_ms: None,
    };
    state.agents.push(agent.clone());
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    Ok(build_session_response(
        "spawned",
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

    let agent_index = state
        .agents
        .iter()
        .position(|agent| agent.agent_id == agent_id && agent.status != HiveAgentStatus::Closed)
        .ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "live hive agent `{agent_id}` was not found in session `{}`",
                    runtime.session_id
                ),
                None,
            )
        })?;
    if !prompt_accept_state(&state.agents[agent_index].status) {
        let agent = &state.agents[agent_index];
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

    let mode = state.agents[agent_index].mode.clone();
    let worker_name = state.agents[agent_index].worker_name.clone();
    let workdir = state.agents[agent_index].workdir.clone();
    let mode_config = runtime.resolve_mode_config(&mode)?;
    let run_id = format!("run-{}", Uuid::new_v4().simple());
    let run_dir = runtime
        .session_dir()
        .join(&agent_id)
        .join("runs")
        .join(&run_id);
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create run dir `{}`", run_dir.display()))
        .map_err(internal_error)?;

    let prompt_path = run_dir.join("prompt.txt");
    let output_path = run_dir.join("output.log");
    let result_path = run_dir.join("result.json");
    fs::write(&prompt_path, &prompt)
        .with_context(|| format!("failed to write prompt file `{}`", prompt_path.display()))
        .map_err(internal_error)?;

    let window_name = sanitize_window_name(&worker_name);
    let launch_command = build_claude_exec_command(
        runtime,
        &mode_config,
        Path::new(&workdir),
        &prompt_path,
        &output_path,
        &result_path,
    )
    .map_err(internal_error)?;
    let (window_id, pane_id) = tmux_new_window(
        &runtime.session_name,
        &window_name,
        Path::new(&workdir),
        Some(&launch_command),
    )
    .await
    .map_err(internal_error)?;

    let now = now_ms();
    let agent = &mut state.agents[agent_index];
    agent.status = HiveAgentStatus::Running;
    agent.last_used_at_ms = Some(now);
    agent.current_run = Some(HiveRunState {
        run_id,
        prompt_preview: prompt_preview(&prompt),
        output_path: output_path.display().to_string(),
        prompt_path: prompt_path.display().to_string(),
        result_path: result_path.display().to_string(),
        window_id: Some(window_id),
        pane_id: Some(pane_id),
        started_at_ms: now,
        finished_at_ms: None,
        exit_code: None,
    });

    state.updated_at_ms = now;
    runtime.save_state(&state).map_err(internal_error)?;
    let agent = state
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id)
        .expect("updated agent should exist");
    Ok(build_session_response(
        "running",
        format!("prompt started for hive agent `{agent_id}`"),
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

    let agent_index = state
        .agents
        .iter()
        .position(|agent| agent.agent_id == agent_id && agent.status != HiveAgentStatus::Closed)
        .ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "live hive agent `{agent_id}` was not found in session `{}`",
                    runtime.session_id
                ),
                None,
            )
        })?;

    let window_id = state.agents[agent_index]
        .current_run
        .as_ref()
        .and_then(|run| run.window_id.clone());
    if let Some(window_id) = window_id.as_deref() {
        tmux_kill_window(window_id).await.map_err(internal_error)?;
    }

    let agent = &mut state.agents[agent_index];
    if let Some(run) = agent.current_run.as_mut() {
        run.finished_at_ms = Some(now_ms());
    }
    if agent.current_run.is_some() {
        agent.last_run = agent.current_run.take();
    }
    agent.status = HiveAgentStatus::Closed;
    agent.last_used_at_ms = Some(now_ms());
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;

    let agent = state
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id)
        .expect("closed agent should exist");
    Ok(build_session_response(
        "closed_agent",
        format!("hive agent `{agent_id}` closed"),
        &state,
        Some(agent),
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
        agent.status = HiveAgentStatus::Closed;
        if let Some(run) = agent.current_run.as_mut() {
            run.finished_at_ms = Some(now_ms());
        }
        if agent.current_run.is_some() {
            agent.last_run = agent.current_run.take();
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrphanCleanupAction {
    Skip,
    DeleteState,
    KillSessionThenDeleteState,
}

async fn cleanup_orphaned_hive_sessions(runtime: &HiveRuntime) -> Result<()> {
    let _gc_lock = runtime.acquire_gc_lock()?;
    let session_names = tmux_list_sessions().await?;
    let pane_ids = tmux_list_all_panes().await?;

    for session_path in hive_session_state_files(&runtime.state_dir)? {
        let state = match load_hive_session_state(&session_path) {
            Ok(state) => state,
            Err(_) => continue,
        };
        let action = orphan_cleanup_action(
            &state,
            &runtime.session_id,
            runtime.tmux_socket.as_deref(),
            session_names.contains(&state.session_name),
            pane_ids.contains(&state.queen_pane_id),
        );
        if action == OrphanCleanupAction::Skip {
            continue;
        }

        let Ok(_session_lock) = runtime.acquire_session_lock(&state.session_id) else {
            continue;
        };

        if action == OrphanCleanupAction::KillSessionThenDeleteState {
            tmux_kill_session_if_exists(&state.session_name).await?;
        }
        remove_hive_session_state(&runtime.state_dir, &session_path, &state.session_id)?;
    }

    Ok(())
}

fn hive_session_state_files(state_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(state_dir)
        .with_context(|| format!("failed to read hive state dir `{}`", state_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        sessions.push(path);
    }
    Ok(sessions)
}

fn load_hive_session_state(path: &Path) -> Result<HiveSessionState> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read hive session file `{}`", path.display()))?;
    parse_hive_session_state(&raw)
        .with_context(|| format!("failed to parse hive session file `{}`", path.display()))
}

fn remove_hive_session_state(
    state_dir: &Path,
    session_path: &Path,
    session_id: &str,
) -> Result<()> {
    ensure_valid_session_id(session_id)?;
    let expected_file_name = format!("{session_id}.json");
    if session_path.file_name().and_then(|name| name.to_str()) != Some(expected_file_name.as_str())
    {
        return Err(anyhow!(
            "session file `{}` does not match session_id `{session_id}`",
            session_path.display()
        ));
    }
    let session_dir = state_dir.join(session_id);

    match fs::remove_file(session_path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed to remove hive session file `{}`",
                    session_path.display()
                )
            });
        }
    }
    match fs::remove_dir_all(&session_dir) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed to remove hive session directory `{}`",
                    session_dir.display()
                )
            });
        }
    }

    Ok(())
}

fn ensure_valid_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(anyhow!("invalid hive session_id `{session_id}`"));
    }
    Ok(())
}

fn orphan_cleanup_action(
    state: &HiveSessionState,
    current_session_id: &str,
    current_tmux_socket: Option<&str>,
    session_exists: bool,
    queen_pane_exists: bool,
) -> OrphanCleanupAction {
    if !state.session_name.starts_with("agpod-") {
        return OrphanCleanupAction::Skip;
    }
    if state.session_id == current_session_id {
        return OrphanCleanupAction::Skip;
    }
    if state.tmux_socket.as_deref() != current_tmux_socket {
        return OrphanCleanupAction::Skip;
    }
    if !session_exists {
        return OrphanCleanupAction::DeleteState;
    }
    if !queen_pane_exists {
        return OrphanCleanupAction::KillSessionThenDeleteState;
    }
    OrphanCleanupAction::Skip
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
    let panes = if tmux_has_session(&runtime.session_name).await? {
        tmux_list_session_panes(&runtime.session_name).await?
    } else {
        Vec::new()
    };
    let pane_map: BTreeMap<_, _> = panes
        .into_iter()
        .map(|pane| (pane.pane_id.clone(), pane))
        .collect();

    for agent in &mut state.agents {
        if agent.status == HiveAgentStatus::Closed {
            continue;
        }
        let mut completed = false;
        if let Some(run) = agent.current_run.as_mut() {
            if let Some(pane_id) = run.pane_id.as_deref() {
                if let Some(pane) = pane_map.get(pane_id) {
                    run.window_id = Some(pane.window_id.clone());
                    run.pane_id = Some(pane.pane_id.clone());
                    agent.status = HiveAgentStatus::Running;
                } else {
                    hydrate_run_result(run);
                    completed = true;
                }
            } else {
                hydrate_run_result(run);
                completed = true;
            }
        }
        if completed {
            agent.last_run = agent.current_run.take();
            agent.status = HiveAgentStatus::Idle;
        }
    }
    Ok(())
}

fn hydrate_run_result(run: &mut HiveRunState) {
    let result_path = Path::new(&run.result_path);
    if let Ok(raw) = fs::read_to_string(result_path) {
        if let Ok(result) = serde_json::from_str::<HiveRunResultFile>(&raw) {
            run.exit_code = Some(result.exit_code);
            run.started_at_ms = result.started_at_ms;
            run.finished_at_ms = Some(result.finished_at_ms);
        }
    }
    if run.finished_at_ms.is_none() {
        run.finished_at_ms = Some(now_ms());
    }
    run.window_id = None;
    run.pane_id = None;
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
        "mode": agent.mode,
        "workdir": agent.workdir,
        "status": agent.status.as_str(),
        "last_used_at_ms": agent.last_used_at_ms,
        "current_run": agent.current_run.as_ref().map(run_json),
        "last_run": agent.last_run.as_ref().map(run_json),
    })
}

fn run_json(run: &HiveRunState) -> Value {
    serde_json::json!({
        "run_id": run.run_id,
        "prompt_preview": run.prompt_preview,
        "output_path": run.output_path,
        "prompt_path": run.prompt_path,
        "result_path": run.result_path,
        "window_id": run.window_id,
        "pane_id": run.pane_id,
        "started_at_ms": run.started_at_ms,
        "finished_at_ms": run.finished_at_ms,
        "exit_code": run.exit_code,
        "output_excerpt": read_output_excerpt(&run.output_path),
    })
}

fn read_output_excerpt(path: &str) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let seek_back = std::cmp::min(file_len, OUTPUT_EXCERPT_LIMIT as u64);
    if file.seek(SeekFrom::End(-(seek_back as i64))).is_err() {
        return None;
    }
    let mut buf = Vec::with_capacity(seek_back as usize);
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn prompt_preview(prompt: &str) -> String {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview: String = normalized.chars().take(120).collect();
    if normalized.chars().count() > 120 {
        format!("{preview}...")
    } else {
        preview
    }
}

fn validate_mode_config(mode_name: &str, config: &McpHiveClaudeModeConfig) -> Result<(), ErrorData> {
    if config
        .command
        .as_deref()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        return Err(ErrorData::invalid_params(
            format!("hive mode `{mode_name}` requires non-empty `command`"),
            None,
        ));
    }
    for key in config.env.keys() {
        let first = key.chars().next();
        if first.is_none()
            || first.is_some_and(|ch| ch.is_ascii_digit())
            || !key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(ErrorData::invalid_params(
                format!("hive mode `{mode_name}` has invalid env key `{key}`; env keys must start with a letter or `_`"),
                None,
            ));
        }
    }
    Ok(())
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
        let _window_name = parts.next().unwrap_or_default();
        let pane_id = parts.next().unwrap_or_default();
        if pane_id.is_empty() {
            continue;
        }
        panes.push(SessionPaneInfo {
            window_id: window_id.to_string(),
            pane_id: pane_id.to_string(),
        });
    }
    Ok(panes)
}

async fn tmux_list_sessions() -> Result<HashSet<String>> {
    let output = Command::new("tmux")
        .arg("list-sessions")
        .arg("-F")
        .arg("#{session_name}")
        .output()
        .await
        .context("failed to run `tmux list-sessions`")?;
    if !output.status.success() {
        return Err(anyhow!(
            "`tmux list-sessions` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).context("tmux output was not utf-8")?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

async fn tmux_list_all_panes() -> Result<HashSet<String>> {
    let output = Command::new("tmux")
        .arg("list-panes")
        .arg("-a")
        .arg("-F")
        .arg("#{pane_id}")
        .output()
        .await
        .context("failed to run `tmux list-panes` for all panes")?;
    if !output.status.success() {
        return Err(anyhow!(
            "`tmux list-panes -a` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).context("tmux output was not utf-8")?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect())
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

async fn tmux_kill_session_if_exists(session_name: &str) -> Result<()> {
    let output = Command::new("tmux")
        .arg("kill-session")
        .arg("-t")
        .arg(session_name)
        .output()
        .await
        .with_context(|| format!("failed to run `tmux kill-session` for `{session_name}`"))?;
    if output.status.success() {
        return Ok(());
    }
    if !tmux_has_session(session_name).await? {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "`tmux kill-session` failed for `{session_name}`: {}",
        stderr.trim()
    ))
}

fn build_claude_exec_command(
    runtime: &HiveRuntime,
    mode_config: &McpHiveClaudeModeConfig,
    workdir: &Path,
    prompt_path: &Path,
    output_path: &Path,
    result_path: &Path,
) -> Result<String> {
    let command = mode_config
        .command
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("claw");
    let expanded_settings = mode_config
        .settings
        .as_deref()
        .map(expand_home_like)
        .transpose()?;
    let expanded_mcp = mode_config
        .mcp_config
        .as_deref()
        .map(expand_home_like)
        .transpose()?;

    let mut command_parts = vec![shell_escape(command)];
    command_parts.extend(
        mode_config
            .args
            .iter()
            .map(|arg| shell_escape(arg))
            .collect::<Vec<_>>(),
    );
    if let Some(settings) = expanded_settings {
        command_parts.push("--settings".to_string());
        command_parts.push(shell_escape(&settings.display().to_string()));
    }
    if let Some(mcp_config) = expanded_mcp {
        command_parts.push("--mcp-config".to_string());
        command_parts.push(shell_escape(&mcp_config.display().to_string()));
    }

    let mut script = String::from("set -euo pipefail\n");
    script.push_str(&format!(
        "cd {}\n",
        shell_escape(&workdir.display().to_string())
    ));
    script.push_str(&format!(
        "mkdir -p {}\n",
        shell_escape(&runtime.session_dir().display().to_string())
    ));
    for (key, value) in &mode_config.env {
        script.push_str(&format!(
            "export {}={}\n",
            shell_var_name(key)?,
            shell_escape(value)
        ));
    }
    script.push_str(&format!(
        "PROMPT=$(cat {})\n",
        shell_escape(&prompt_path.display().to_string())
    ));
    script.push_str("STARTED_AT_MS=$(date +%s000)\n");
    script.push_str("RC=0\n");
    script.push_str("set +e\n");
    script.push_str(&format!(
        "{} -p \"$PROMPT\" >{} 2>&1 || RC=$?\n",
        command_parts.join(" "),
        shell_escape(&output_path.display().to_string()),
    ));
    script.push_str("set -e\n");
    script.push_str("FINISHED_AT_MS=$(date +%s000)\n");
    script.push_str(&format!(
        "printf '{{\"exit_code\":%s,\"started_at_ms\":%s,\"finished_at_ms\":%s}}\\n' \"$RC\" \"$STARTED_AT_MS\" \"$FINISHED_AT_MS\" >{}\n",
        shell_escape(&result_path.display().to_string()),
    ));
    script.push_str("exit \"$RC\"\n");

    Ok(format!("bash -lc {}", shell_escape(&script)))
}

fn shell_var_name(name: &str) -> Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(anyhow!("invalid env key `{name}`"));
    }
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return Err(anyhow!("invalid env key `{name}`"));
    }
    Ok(name.to_string())
}

fn expand_home_like(path: &str) -> Result<PathBuf> {
    if path == "~" {
        return dirs::home_dir().ok_or_else(|| anyhow!("failed to resolve home directory"));
    }
    if let Some(stripped) = path.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("failed to resolve home directory"))?;
        return Ok(home.join(stripped));
    }
    Ok(PathBuf::from(path))
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

        let with_command = tmux_new_window_args("session", "worker", workdir, Some("exec claude"));
        assert_eq!(
            with_command.last().and_then(|s| s.to_str()),
            Some("exec claude")
        );
        assert!(with_command.iter().any(|arg| arg == "worker"));
    }

    #[test]
    fn next_agent_id_skips_existing_indexes() {
        let existing = vec![
            HiveAgentState {
                agent_id: "agent-01".to_string(),
                worker_name: "a".to_string(),
                mode: "default".to_string(),
                workdir: "/tmp".to_string(),
                status: HiveAgentStatus::Closed,
                current_run: None,
                last_run: None,
                last_used_at_ms: None,
            },
            HiveAgentState {
                agent_id: "agent-03".to_string(),
                worker_name: "b".to_string(),
                mode: "default".to_string(),
                workdir: "/tmp".to_string(),
                status: HiveAgentStatus::Idle,
                current_run: None,
                last_run: None,
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
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    status: HiveAgentStatus::Idle,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: Some(1),
                },
                HiveAgentState {
                    agent_id: "agent-02".to_string(),
                    worker_name: "busy".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    status: HiveAgentStatus::Running,
                    current_run: Some(HiveRunState {
                        run_id: "run-1".to_string(),
                        prompt_preview: "hello".to_string(),
                        output_path: "/tmp/output".to_string(),
                        prompt_path: "/tmp/prompt".to_string(),
                        result_path: "/tmp/result".to_string(),
                        window_id: Some("@1".to_string()),
                        pane_id: Some("%12".to_string()),
                        started_at_ms: 2,
                        finished_at_ms: None,
                        exit_code: None,
                    }),
                    last_run: None,
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
        assert!(!prompt_accept_state(&HiveAgentStatus::Running));
        assert!(!prompt_accept_state(&HiveAgentStatus::Closed));
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
            config: Config::default(),
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
            config: Config::default(),
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

    #[test]
    fn orphan_cleanup_skips_current_session() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-q1".to_string(),
            session_name: "agpod-hive-q1".to_string(),
            queen_pane_id: "%1".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: Vec::new(),
        };

        assert_eq!(
            orphan_cleanup_action(&state, "hive-q1", Some("/tmp/tmux"), true, false),
            OrphanCleanupAction::Skip
        );
    }

    #[test]
    fn orphan_cleanup_deletes_state_when_tmux_session_is_gone() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-q2".to_string(),
            session_name: "agpod-hive-q2".to_string(),
            queen_pane_id: "%2".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: Vec::new(),
        };

        assert_eq!(
            orphan_cleanup_action(&state, "hive-q1", Some("/tmp/tmux"), false, false),
            OrphanCleanupAction::DeleteState
        );
    }

    #[test]
    fn orphan_cleanup_kills_session_when_queen_pane_is_gone() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-q2".to_string(),
            session_name: "agpod-hive-q2".to_string(),
            queen_pane_id: "%2".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: Vec::new(),
        };

        assert_eq!(
            orphan_cleanup_action(&state, "hive-q1", Some("/tmp/tmux"), true, false),
            OrphanCleanupAction::KillSessionThenDeleteState
        );
    }

    #[test]
    fn remove_hive_session_state_rejects_mismatched_file_name() {
        let temp = tempdir().expect("temp dir");
        let state_dir = temp.path();
        let session_path = state_dir.join("wrong.json");
        fs::write(&session_path, "{}").expect("write session file");

        let err = remove_hive_session_state(state_dir, &session_path, "hive-q1")
            .expect_err("mismatched file name should fail");
        assert!(err.to_string().contains("does not match session_id"));
    }

    #[test]
    fn remove_hive_session_state_rejects_invalid_session_id() {
        let temp = tempdir().expect("temp dir");
        let state_dir = temp.path();
        let session_path = state_dir.join("../bad.json");

        let err = remove_hive_session_state(state_dir, &session_path, "../bad")
            .expect_err("invalid session_id should fail");
        assert!(err.to_string().contains("invalid hive session_id"));
    }

    #[test]
    fn acquire_session_lock_rejects_invalid_session_id() {
        let temp = tempdir().expect("temp dir");
        let runtime = HiveRuntime {
            repo_root: temp.path().to_path_buf(),
            state_dir: temp.path().join("state"),
            session_id: "hive-q1".to_string(),
            session_name: "agpod-hive-q1".to_string(),
            queen_pane_id: "%1".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
            config: Config::default(),
        };

        let err = match runtime.acquire_session_lock("../bad") {
            Ok(_) => panic!("invalid session_id should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("invalid hive session_id"));
    }

    #[test]
    fn validate_mode_config_requires_command() {
        let cfg = McpHiveClaudeModeConfig::default();
        let err = validate_mode_config("readonly", &cfg).expect_err("missing command should fail");
        assert!(err.message.contains("requires non-empty `command`"));
    }

    #[test]
    fn validate_mode_config_rejects_env_key_starting_with_digit() {
        let mut cfg = McpHiveClaudeModeConfig::default();
        cfg.command = Some("claw".to_string());
        cfg.env.insert("1BAD".to_string(), "x".to_string());
        let err = validate_mode_config("readonly", &cfg).expect_err("invalid env key should fail");
        assert!(err.message.contains("invalid env key"));
    }

    #[test]
    fn validate_mode_config_rejects_env_key_with_dash() {
        let mut cfg = McpHiveClaudeModeConfig::default();
        cfg.command = Some("claw".to_string());
        cfg.env.insert("BAD-KEY".to_string(), "x".to_string());
        let err = validate_mode_config("readonly", &cfg).expect_err("invalid env key should fail");
        assert!(err.message.contains("invalid env key"));
    }

    #[test]
    fn load_state_migrates_legacy_session_shape() {
        let temp = tempdir().expect("temp dir");
        let runtime = HiveRuntime {
            repo_root: temp.path().to_path_buf(),
            state_dir: temp.path().join("state"),
            session_id: "hive-q1".to_string(),
            session_name: "agpod-hive-q1".to_string(),
            queen_pane_id: "%1".to_string(),
            tmux_socket: Some("/tmp/tmux".to_string()),
            config: Config::default(),
        };
        runtime.ensure_state_dirs().expect("state dirs");
        fs::write(
            runtime.session_file(),
            serde_json::json!({
                "version": 1,
                "session_id": "hive-q1",
                "session_name": "agpod-hive-q1",
                "queen_pane_id": "%1",
                "tmux_socket": "/tmp/tmux",
                "repo_root": "/repo",
                "agent_limit": 5,
                "updated_at_ms": 10,
                "agents": [{
                    "agent_id": "agent-01",
                    "worker_name": "legacy",
                    "agent_kind": "claude",
                    "model": null,
                    "workdir": "/repo",
                    "window_id": "@1",
                    "window_name": "legacy",
                    "pane_id": "%11",
                    "status": "busy",
                    "last_used_at_ms": 9
                }]
            })
            .to_string(),
        )
        .expect("write state");

        let state = runtime.load_state().expect("load legacy state");
        assert_eq!(state.version, HIVE_VERSION);
        assert_eq!(state.agents.len(), 1);
        let agent = &state.agents[0];
        assert_eq!(agent.mode, "readonly");
        assert_eq!(agent.status, HiveAgentStatus::Running);
        assert!(agent.current_run.is_some());
    }
}
