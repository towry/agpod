//! Hive tool support for process-backed Claude exec workers.
//!
//! Keywords: hive, process, claude, exec, output file, worker status

use crate::hive_provider::{
    default_claude_provider, parse_provider_output as parse_provider_output_impl, provider_caps,
    resolve_system_prompt_args, HiveProviderOutput,
};
use agpod_core::{Config, McpHiveClaudeConfig, McpHiveClaudeModeConfig};
use anyhow::{anyhow, Context, Result};
use rmcp::{
    model::{CallToolResult, Content, JsonObject},
    schemars, ErrorData,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use tracing::warn;
use uuid::Uuid;

const HIVE_VERSION: u32 = 2;
const HIVE_AGENT_LIMIT: usize = 5;
const HIVE_LOCK_STALE_MS: u64 = 30_000;
const HIVE_BLOCKING_POLL_MS: u64 = 100;
const HIVE_WAIT_DEFAULT_TIMEOUT_MS: u64 = 30_000;
const HIVE_RUN_MARKER_PREFIX: &str = "--agpod-hive-run=";
const FNV1A_OFFSET_BASIS: u32 = 0x811c_9dc5;
const FNV1A_PRIME: u32 = 0x0100_0193;

const fn default_run_hive_agent_async() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HiveActionInput {
    ModeInfo,
    ListAgents,
    RunHiveAgent,
    WaitAgent,
    CloseAgent,
    CloseSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HiveRequest {
    /// Hive action to perform.
    pub action: HiveActionInput,
    /// Optional explicit session ID. When omitted, hive derives a repo-scoped default session.
    pub session_id: Option<String>,
    /// Existing worker agent ID for `run_hive_agent`, `wait_agent`, `list_agents`, and `close_agent`.
    pub agent_id: Option<String>,
    /// Mode name for `run_hive_agent`. Call `mode_info` to inspect available names.
    pub mode: Option<String>,
    /// Optional worker display name when `run_hive_agent` creates a new worker.
    pub worker_name: Option<String>,
    /// Optional working directory when `run_hive_agent` creates a new worker. Relative paths resolve from the repo root.
    pub workdir: Option<String>,
    /// Prompt to run for `run_hive_agent`.
    pub prompt: Option<String>,
    /// Whether `run_hive_agent` should resume the agent's last Claude conversation session.
    pub resume: Option<bool>,
    /// Whether `run_hive_agent` should return immediately after launch instead of blocking for the final response. Defaults to true and is the recommended mode.
    #[serde(default = "default_run_hive_agent_async", rename = "async")]
    pub async_: bool,
    /// Maximum blocking wait for `wait_agent` in milliseconds. Defaults to 30000 when omitted.
    pub timeout_ms: Option<u64>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HiveSessionState {
    version: u32,
    session_id: String,
    session_name: String,
    repo_root: String,
    agent_limit: usize,
    updated_at_ms: u64,
    agents: Vec<HiveAgentState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HiveAgentState {
    agent_id: String,
    worker_name: String,
    #[serde(default = "default_readonly_mode")]
    mode: String,
    workdir: String,
    #[serde(default)]
    conversation_session_id: Option<String>,
    status: HiveAgentStatus,
    #[serde(default)]
    current_run: Option<HiveRunState>,
    #[serde(default)]
    last_run: Option<HiveRunState>,
    #[serde(default)]
    last_used_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HiveRunState {
    run_id: String,
    prompt_preview: String,
    #[serde(default = "default_claude_provider")]
    provider: String,
    output_path: String,
    prompt_path: String,
    result_path: String,
    #[serde(default)]
    launcher_path: String,
    #[serde(default)]
    process_pid: Option<u32>,
    #[serde(default)]
    resume_requested: bool,
    #[serde(default)]
    #[serde(alias = "claude_session_id")]
    provider_session_id: Option<String>,
    started_at_ms: u64,
    #[serde(default)]
    finished_at_ms: Option<u64>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    termination_reason: Option<String>,
    #[serde(default)]
    provider_output: Option<HiveProviderOutput>,
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
    config: Config,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HiveRunResultFile {
    #[serde(default = "default_claude_provider")]
    provider: String,
    exit_code: i32,
    started_at_ms: u64,
    finished_at_ms: u64,
    #[serde(default, alias = "claude_session_id")]
    provider_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyHiveSessionState {
    version: u32,
    session_id: String,
    session_name: String,
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

fn default_readonly_mode() -> String {
    "readonly".to_string()
}

impl HiveRuntime {
    fn from_env(session_id_hint: Option<&str>) -> Result<Self, ErrorData> {
        let repo_root = std::env::current_dir().map_err(|err| {
            ErrorData::internal_error(format!("failed to resolve current directory: {err}"), None)
        })?;
        let state_dir = std::env::var("AGPOD_HIVE_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_hive_state_dir());
        let session_id = match session_id_hint {
            Some(value) => {
                ensure_valid_session_id(value).map_err(internal_error)?;
                value.to_string()
            }
            None => resolve_default_session_id(&repo_root, &state_dir),
        };

        Ok(Self {
            repo_root,
            state_dir,
            session_name: format!("agpod-{session_id}"),
            session_id,
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

    async fn acquire_lock(&self) -> Result<HiveStateGuard> {
        self.ensure_state_dirs()?;
        acquire_lock_file(self.session_lock_file()).await
    }

    fn claude_config(&self) -> Option<&McpHiveClaudeConfig> {
        self.config
            .mcp
            .as_ref()
            .and_then(|mcp| mcp.hive.as_ref())
            .and_then(|hive| hive.claude.as_ref())
    }

    fn shared_env_set(&self) -> BTreeMap<String, String> {
        self.claude_config()
            .map(|cfg| cfg.env_set.clone())
            .unwrap_or_default()
    }

    fn configured_mode_names(&self) -> Vec<String> {
        let mut mode_names = self
            .claude_config()
            .map(|cfg| cfg.modes.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        mode_names.sort();
        mode_names
    }

    fn resolve_mode_name(&self, requested: Option<&str>) -> String {
        requested
            .map(str::trim)
            .filter(|mode| !mode.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "readonly".to_string())
    }

    fn resolve_mode_config(&self, mode_name: &str) -> Result<McpHiveClaudeModeConfig, ErrorData> {
        let config = self
            .claude_config()
            .and_then(|cfg| cfg.modes.get(mode_name))
            .cloned()
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    format!(
                        "hive mode `{mode_name}` is not available; call `hive` with action=`mode_info` to inspect available mode names"
                    ),
                    None,
                )
            })?;
        validate_env_keys("[mcp.hive.claude.env_set]", &self.shared_env_set())?;
        validate_mode_config(mode_name, &config)?;
        Ok(config)
    }
}

fn default_hive_state_dir() -> PathBuf {
    std::env::temp_dir().join("agpod").join("hive")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HiveResponseShape {
    Summary,
    List,
}

fn parse_hive_session_state(raw: &str) -> Result<HiveSessionState> {
    match serde_json::from_str::<HiveSessionState>(raw) {
        Ok(state) => Ok(state),
        Err(current_err) => match serde_json::from_str::<LegacyHiveSessionState>(raw) {
            Ok(legacy) => Ok(migrate_legacy_hive_session_state(legacy)),
            Err(_) => {
                Err(current_err).context("failed to parse current or legacy hive session state")
            }
        },
    }
}

fn migrate_legacy_hive_session_state(legacy: LegacyHiveSessionState) -> HiveSessionState {
    HiveSessionState {
        version: HIVE_VERSION,
        session_id: legacy.session_id,
        session_name: legacy.session_name,
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
        LegacyHiveAgentStatus::Busy
        | LegacyHiveAgentStatus::Resetting
        | LegacyHiveAgentStatus::Spawning => (
            HiveAgentStatus::Running,
            Some(HiveRunState {
                run_id: format!("legacy-{}", legacy.agent_id),
                prompt_preview: "legacy interactive hive state".to_string(),
                provider: default_claude_provider(),
                output_path: String::new(),
                prompt_path: String::new(),
                result_path: String::new(),
                launcher_path: String::new(),
                process_pid: None,
                resume_requested: false,
                provider_session_id: None,
                started_at_ms: legacy.last_used_at_ms.unwrap_or(0),
                finished_at_ms: None,
                exit_code: None,
                termination_reason: Some("legacy_unmanaged_run".to_string()),
                provider_output: None,
            }),
        ),
    };

    HiveAgentState {
        agent_id: legacy.agent_id,
        worker_name: legacy.worker_name,
        mode: "readonly".to_string(),
        workdir: legacy.workdir,
        conversation_session_id: None,
        status,
        current_run,
        last_run: None,
        last_used_at_ms: legacy.last_used_at_ms,
    }
}

async fn acquire_lock_file(lock_path: PathBuf) -> Result<HiveStateGuard> {
    for _ in 0..200 {
        let path = lock_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => Ok(true),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path, HIVE_LOCK_STALE_MS) {
                        let _ = fs::remove_file(&path);
                        return Ok(false); // retry
                    }
                    Ok(false) // contended
                }
                Err(err) => Err(err),
            }
        })
        .await
        .map_err(|err| anyhow!("failed to join hive lock task: {err}"))?;

        match result {
            Ok(true) => return Ok(HiveStateGuard { lock_path }),
            Ok(false) => {
                sleep(Duration::from_millis(25)).await;
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
        HiveActionInput::ModeInfo => mode_info(&runtime, req).await,
        HiveActionInput::ListAgents => list_agents(&runtime, req).await,
        HiveActionInput::RunHiveAgent => run_hive_agent(&runtime, req).await,
        HiveActionInput::WaitAgent => wait_agent(&runtime, req).await,
        HiveActionInput::CloseAgent => close_agent(&runtime, req).await,
        HiveActionInput::CloseSession => close_session(&runtime).await,
    }
}

async fn mode_info(
    runtime: &HiveRuntime,
    _req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let mode_names = runtime.configured_mode_names();
    let modes = mode_names
        .iter()
        .map(|mode_name| {
            let config = runtime
                .claude_config()
                .and_then(|cfg| cfg.modes.get(mode_name));
            serde_json::json!({
                "name": mode_name,
                "available": config.is_some(),
                "description": config.and_then(|mode| mode.description.clone()),
                "has_system_prompt": config
                    .map(|mode| mode.system_prompt.is_some() || mode.system_prompt_file.is_some())
                    .unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    let mut raw = Map::new();
    raw.insert("ok".to_string(), Value::Bool(true));
    raw.insert("state".to_string(), Value::String("mode_info".to_string()));
    raw.insert(
        "message".to_string(),
        Value::String("hive modes listed".to_string()),
    );
    raw.insert(
        "mode_info".to_string(),
        serde_json::json!({
            "available_modes": mode_names,
            "default_mode_behavior": "hive defaults to `readonly` when `mode` is omitted",
            "modes": modes,
            "notes": [
                "此回包用于查看可用 mode（含自定义 mode 名）。",
                "mode 不可用时，`run_hive_agent` 快速失败，不猜默认。",
                "`run_hive_agent` 默认 `async=true`（荐）；可用 `agent_id` 轮询 `list_agents`。",
                "需阻塞等待时，用 `wait_agent` 并设 `timeout_ms`（默认 30000）。",
                "`run_hive_agent` 传 `agent_id` 时，不得再传 `mode`、`worker_name`、`workdir`。",
                "仅当调用方明需单次阻塞，方设 `async=false`。",
                "达 live limit 时不自动复用；欲复用须显式给 `agent_id`。",
                "`resume` 仅调用者显式控制；默认 `resume=false` 以避上下文膨胀。",
                "`resume=true` 仅在该 agent 已存会话 id 时可用。",
                "`hive` tool description 为契约正本；此 notes 仅释义。",
                "worker 继承父进程环境（含 PATH）。",
                "此回包故意隐藏配置路径、环境变量与其他敏感细节。"
            ]
        }),
    );
    Ok(raw)
}

async fn list_agents(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let _lock = runtime.acquire_lock().await.map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_processes(&mut state)
        .await
        .map_err(internal_error)?;
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    let selected_agent = match req.agent_id.as_deref() {
        Some(agent_id) => Some(
            state
                .agents
                .iter()
                .find(|agent| agent.agent_id == agent_id)
                .ok_or_else(|| {
                    ErrorData::invalid_params(
                        format!(
                            "hive agent `{agent_id}` was not found in session `{}`",
                            runtime.session_id
                        ),
                        None,
                    )
                })?,
        ),
        None => None,
    };
    let message = selected_agent.map_or_else(
        || "hive agents listed".to_string(),
        |agent| {
            let base = format!("hive agent `{}` status fetched", agent.agent_id);
            if agent.status == HiveAgentStatus::Running {
                return format!("{base}. {}", wait_agent_hint(&agent.agent_id));
            }
            base
        },
    );
    Ok(build_session_response(
        "listed",
        message,
        &state,
        selected_agent,
        HiveResponseShape::List,
    ))
}

async fn run_hive_agent(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let prompt = req.prompt.clone().ok_or_else(|| {
        ErrorData::invalid_params("`prompt` is required for action=`run_hive_agent`", None)
    })?;
    let run_async = req.async_;
    let (agent_id, run_id, immediate_response) = {
        let _lock = runtime.acquire_lock().await.map_err(internal_error)?;
        let mut state = runtime
            .load_state()
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
        sync_state_with_processes(&mut state)
            .await
            .map_err(internal_error)?;

        let agent_index = ensure_run_hive_agent_target(runtime, &mut state, &req)?;
        let agent_id = state.agents[agent_index].agent_id.clone();
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
                HiveResponseShape::Summary,
            ));
        }

        let run_id = start_agent_run(
            runtime,
            &mut state,
            agent_index,
            &prompt,
            req.resume.unwrap_or(false),
        )
        .await?;
        let response = build_session_response(
            "running",
            format!(
                "hive agent `{agent_id}` run `{run_id}` started. {}",
                wait_agent_hint(&agent_id)
            ),
            &state,
            Some(&state.agents[agent_index]),
            HiveResponseShape::Summary,
        );
        (agent_id, run_id, response)
    };

    if run_async {
        return Ok(immediate_response);
    }

    wait_for_run_completion(runtime, &agent_id, &run_id, None).await
}

async fn wait_agent(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let agent_id = req.agent_id.ok_or_else(|| {
        ErrorData::invalid_params("`agent_id` is required for action=`wait_agent`", None)
    })?;
    let timeout_ms = req.timeout_ms.unwrap_or(HIVE_WAIT_DEFAULT_TIMEOUT_MS);
    let timeout = Duration::from_millis(timeout_ms);
    let maybe_run_id = {
        let _lock = runtime.acquire_lock().await.map_err(internal_error)?;
        let mut state = runtime
            .load_state()
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
        sync_state_with_processes(&mut state)
            .await
            .map_err(internal_error)?;
        state.updated_at_ms = now_ms();
        runtime.save_state(&state).map_err(internal_error)?;

        let agent = state
            .agents
            .iter()
            .find(|candidate| candidate.agent_id == agent_id)
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    format!(
                        "hive agent `{agent_id}` was not found in session `{}`",
                        runtime.session_id
                    ),
                    None,
                )
            })?;
        if let Some(run) = agent.current_run.as_ref() {
            Some(run.run_id.clone())
        } else if let Some(run) = agent.last_run.as_ref() {
            return Ok(build_session_response(
                "completed",
                format!(
                    "{} No active run remains; returning the latest completed run.",
                    completed_run_message(&agent_id, run)
                ),
                &state,
                Some(agent),
                HiveResponseShape::Summary,
            ));
        } else {
            return Ok(build_error_response(
                "idle",
                format!(
                    "hive agent `{agent_id}` has no active run to wait for. Start one with action=`run_hive_agent`."
                ),
                &state,
                Some(agent),
                HiveResponseShape::Summary,
            ));
        }
    };

    if let Some(run_id) = maybe_run_id {
        return wait_for_run_completion(runtime, &agent_id, &run_id, Some(timeout)).await;
    }
    Err(ErrorData::internal_error(
        "wait_agent reached an impossible state without a run id",
        None,
    ))
}

async fn close_agent(
    runtime: &HiveRuntime,
    req: HiveRequest,
) -> Result<Map<String, Value>, ErrorData> {
    let agent_id = req.agent_id.ok_or_else(|| {
        ErrorData::invalid_params("`agent_id` is required for action=`close_agent`", None)
    })?;

    let _lock = runtime.acquire_lock().await.map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_processes(&mut state)
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

    let terminate_result = match state.agents[agent_index].current_run.as_ref() {
        Some(run) => terminate_run_process_if_owned(run)
            .await
            .map_err(internal_error)?,
        None => TerminateRunResult::NotRunning,
    };
    if terminate_result == TerminateRunResult::IdentityMismatch {
        warn!(
            agent_id = %agent_id,
            "close_agent refused because process identity no longer matches recorded launcher"
        );
        state.updated_at_ms = now_ms();
        runtime.save_state(&state).map_err(internal_error)?;
        let agent = &state.agents[agent_index];
        return Ok(build_error_response(
            "identity_mismatch",
            format!(
                "hive agent `{agent_id}` may still be running, but its pid no longer matches the recorded launcher; refusing automatic close"
            ),
            &state,
            Some(agent),
            HiveResponseShape::Summary,
        ));
    }

    let agent = &mut state.agents[agent_index];
    if let Some(run) = agent.current_run.as_mut() {
        let reason = match terminate_result {
            TerminateRunResult::Terminated | TerminateRunResult::NotRunning => "killed_by_hive",
            TerminateRunResult::IdentityMismatch => unreachable!("handled above"),
        };
        finalize_run(run, Some(reason));
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
        HiveResponseShape::Summary,
    ))
}

async fn close_session(runtime: &HiveRuntime) -> Result<Map<String, Value>, ErrorData> {
    let _lock = runtime.acquire_lock().await.map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_processes(&mut state)
        .await
        .map_err(internal_error)?;

    let mut mismatched_agents = Vec::new();
    for agent in &mut state.agents {
        let terminate_result = match agent.current_run.as_ref() {
            Some(run) => terminate_run_process_if_owned(run)
                .await
                .map_err(internal_error)?,
            None => TerminateRunResult::NotRunning,
        };
        if terminate_result == TerminateRunResult::IdentityMismatch {
            warn!(
                agent_id = %agent.agent_id,
                "close_session skipped agent because process identity no longer matches recorded launcher"
            );
            mismatched_agents.push(agent.agent_id.clone());
            continue;
        }
        if let Some(run) = agent.current_run.as_mut() {
            let reason = match terminate_result {
                TerminateRunResult::Terminated | TerminateRunResult::NotRunning => "killed_by_hive",
                TerminateRunResult::IdentityMismatch => unreachable!("handled above"),
            };
            finalize_run(run, Some(reason));
        }
        if agent.current_run.is_some() {
            agent.last_run = agent.current_run.take();
        }
        agent.status = HiveAgentStatus::Closed;
        agent.last_used_at_ms = Some(now_ms());
    }
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    if !mismatched_agents.is_empty() {
        return Ok(build_error_response(
            "identity_mismatch",
            format!(
                "hive session not fully closed; agents still need manual inspection: {}",
                mismatched_agents.join(", ")
            ),
            &state,
            None,
            HiveResponseShape::Summary,
        ));
    }
    Ok(build_session_response(
        "closed_session",
        "hive session closed",
        &state,
        None,
        HiveResponseShape::Summary,
    ))
}

fn ensure_run_hive_agent_target(
    runtime: &HiveRuntime,
    state: &mut HiveSessionState,
    req: &HiveRequest,
) -> Result<usize, ErrorData> {
    if let Some(agent_id) = req.agent_id.as_deref() {
        if req.mode.is_some() || req.worker_name.is_some() || req.workdir.is_some() {
            return Err(ErrorData::invalid_params(
                "`mode`, `worker_name`, and `workdir` are only allowed when `agent_id` is omitted for action=`run_hive_agent`",
                None,
            ));
        }
        return state
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
            });
    }

    let live_count = state
        .agents
        .iter()
        .filter(|agent| agent.status != HiveAgentStatus::Closed)
        .count();
    if live_count >= HIVE_AGENT_LIMIT {
        return Err(ErrorData::invalid_params(
            build_hive_agent_limit_message(state),
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
    state.agents.push(HiveAgentState {
        agent_id,
        worker_name,
        mode,
        workdir: workdir.display().to_string(),
        conversation_session_id: None,
        status: HiveAgentStatus::Idle,
        current_run: None,
        last_run: None,
        last_used_at_ms: None,
    });
    Ok(state.agents.len() - 1)
}

fn suggest_agents_to_close(state: &HiveSessionState, max_count: usize) -> Vec<String> {
    let mut candidates = state
        .agents
        .iter()
        .filter(|agent| agent.status != HiveAgentStatus::Closed)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let left_rank = if left.status == HiveAgentStatus::Idle {
            0_u8
        } else {
            1_u8
        };
        let right_rank = if right.status == HiveAgentStatus::Idle {
            0_u8
        } else {
            1_u8
        };
        left_rank
            .cmp(&right_rank)
            .then_with(|| {
                left.last_used_at_ms
                    .unwrap_or(0)
                    .cmp(&right.last_used_at_ms.unwrap_or(0))
            })
            .then_with(|| left.agent_id.cmp(&right.agent_id))
    });
    candidates
        .into_iter()
        .take(max_count)
        .map(|agent| agent.agent_id.clone())
        .collect()
}

fn build_hive_agent_limit_message(state: &HiveSessionState) -> String {
    let reusable_ids = state
        .agents
        .iter()
        .filter(|agent| agent.status == HiveAgentStatus::Idle)
        .map(|agent| agent.agent_id.as_str())
        .collect::<Vec<_>>();
    let live_status = state
        .agents
        .iter()
        .filter(|agent| agent.status != HiveAgentStatus::Closed)
        .map(|agent| format!("{}({})", agent.agent_id, agent.status.as_str()))
        .collect::<Vec<_>>();

    let mut message = format!("hive agent limit reached: maximum {HIVE_AGENT_LIMIT} live agents.");
    if reusable_ids.is_empty() {
        message.push_str(" No idle agents are currently reusable.");
    } else {
        message.push_str(&format!(
            " Idle agents are reusable only with explicit `agent_id` (reusable: {}).",
            reusable_ids.join(", ")
        ));
        message.push_str(" `resume` is caller-controlled only; keep `resume=false` (default) to avoid context bloat.");
    }
    let close_candidates = suggest_agents_to_close(state, 3);
    if close_candidates.is_empty() {
        message.push_str(" Suggested next step: call action=`close_session` to reset the session.");
    } else {
        message.push_str(&format!(
            " Suggested next steps: call action=`close_agent` for one of [{}], or action=`close_session` to reset all workers.",
            close_candidates.join(", ")
        ));
    }
    message.push_str(&format!(
        " Call action=`list_agents` to inspect live statuses ({}).",
        live_status.join(", ")
    ));
    message
}

async fn start_agent_run(
    runtime: &HiveRuntime,
    state: &mut HiveSessionState,
    agent_index: usize,
    prompt: &str,
    resume: bool,
) -> Result<String, ErrorData> {
    let agent_id = state.agents[agent_index].agent_id.clone();
    let mode = state.agents[agent_index].mode.clone();
    let workdir = state.agents[agent_index].workdir.clone();
    let conversation_session_id = state.agents[agent_index].conversation_session_id.clone();
    if resume && conversation_session_id.is_none() {
        return Err(ErrorData::invalid_params(
            format!(
                "resume requested for hive agent `{agent_id}`, but no saved Claude conversation session id is available"
            ),
            None,
        ));
    }

    let mode_config = runtime.resolve_mode_config(&mode)?;
    let shared_env_set = runtime.shared_env_set();
    let provider_command = mode_config
        .command
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("claude");
    let login_shell = default_login_shell();
    ensure_binary_available(&login_shell).map_err(internal_error)?;
    ensure_binary_available("bash").map_err(internal_error)?;
    ensure_binary_available("python3").map_err(internal_error)?;
    ensure_binary_available(provider_command).map_err(internal_error)?;

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
    let launcher_path = run_dir.join("launcher.sh");
    fs::write(&prompt_path, prompt)
        .with_context(|| format!("failed to write prompt file `{}`", prompt_path.display()))
        .map_err(internal_error)?;

    let now = now_ms();
    {
        let agent = &mut state.agents[agent_index];
        agent.status = HiveAgentStatus::Running;
        agent.last_used_at_ms = Some(now);
        agent.current_run = Some(HiveRunState {
            run_id: run_id.clone(),
            prompt_preview: prompt_preview(prompt),
            provider: default_claude_provider(),
            output_path: output_path.display().to_string(),
            prompt_path: prompt_path.display().to_string(),
            result_path: result_path.display().to_string(),
            launcher_path: launcher_path.display().to_string(),
            process_pid: None,
            resume_requested: resume,
            provider_session_id: conversation_session_id.clone(),
            started_at_ms: now,
            finished_at_ms: None,
            exit_code: None,
            termination_reason: None,
            provider_output: None,
        });
    }
    state.updated_at_ms = now;
    runtime.save_state(state).map_err(internal_error)?;

    let launch_command = build_claude_exec_command(
        runtime,
        &shared_env_set,
        &mode_config,
        Path::new(&workdir),
        &prompt_path,
        &output_path,
        &result_path,
        &run_dir,
        conversation_session_id.as_deref(),
        resume,
    )
    .map_err(|err| {
        rollback_launch_failure(runtime, state, agent_index, "launch_prepare_failed");
        internal_error(err)
    })?;
    fs::write(&launcher_path, &launch_command)
        .with_context(|| {
            format!(
                "failed to write launcher file `{}`",
                launcher_path.display()
            )
        })
        .map_err(|err| {
            rollback_launch_failure(runtime, state, agent_index, "launch_prepare_failed");
            internal_error(err)
        })?;

    let process_pid =
        match spawn_hive_run_process(Path::new(&workdir), &launcher_path, &run_id, &login_shell)
            .await
        {
            Ok(pid) => pid,
            Err(err) => {
                rollback_launch_failure(runtime, state, agent_index, "launch_failed");
                return Err(internal_error(err));
            }
        };

    if let Some(run) = state.agents[agent_index].current_run.as_mut() {
        run.process_pid = Some(process_pid);
    }
    state.updated_at_ms = now_ms();
    if let Err(err) = runtime.save_state(state) {
        terminate_process_group_if_owned(process_pid, &run_id, &launcher_path)
            .await
            .map_err(internal_error)?;
        rollback_launch_failure(runtime, state, agent_index, "state_persist_failed");
        return Err(internal_error(err));
    }

    Ok(run_id)
}

async fn wait_for_run_completion(
    runtime: &HiveRuntime,
    agent_id: &str,
    run_id: &str,
    timeout: Option<Duration>,
) -> Result<Map<String, Value>, ErrorData> {
    let mut identity_mismatch_since: Option<std::time::Instant> = None;
    let wait_started_at = std::time::Instant::now();
    loop {
        let maybe_response = {
            let _lock = runtime.acquire_lock().await.map_err(internal_error)?;
            let mut state = runtime
                .load_state()
                .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
            let state_before_sync = state.clone();
            sync_state_with_processes(&mut state)
                .await
                .map_err(internal_error)?;
            if state != state_before_sync {
                state.updated_at_ms = now_ms();
                runtime.save_state(&state).map_err(internal_error)?;
            }

            let agent = state
                .agents
                .iter()
                .find(|agent| agent.agent_id == agent_id)
                .ok_or_else(|| {
                    ErrorData::invalid_params(
                        format!(
                            "hive agent `{agent_id}` was not found in session `{}`",
                            runtime.session_id
                        ),
                        None,
                    )
                })?;

            if let Some(last_run) = agent.last_run.as_ref().filter(|run| run.run_id == run_id) {
                Some(Ok(build_session_response(
                    "completed",
                    completed_run_message(agent_id, last_run),
                    &state,
                    Some(agent),
                    HiveResponseShape::Summary,
                )))
            } else if let Some(current_run) = agent
                .current_run
                .as_ref()
                .filter(|run| run.run_id == run_id)
            {
                match run_process_state(current_run)
                    .await
                    .map_err(internal_error)?
                {
                    RunProcessState::LiveOwned | RunProcessState::FinishedOrMissing => {
                        identity_mismatch_since = None;
                        None
                    }
                    RunProcessState::IdentityMismatch => {
                        let mismatch_started_at =
                            identity_mismatch_since.get_or_insert_with(std::time::Instant::now);
                        if mismatch_started_at.elapsed() < Duration::from_secs(2) {
                            None
                        } else {
                            Some(Ok(build_error_response(
                                "identity_mismatch",
                                format!(
                                    "hive agent `{agent_id}` run `{run_id}` may still be running, but its pid no longer matches the recorded launcher; manual inspection required"
                                ),
                                &state,
                                Some(agent),
                                HiveResponseShape::Summary,
                            )))
                        }
                    }
                }
            } else {
                Some(Err(ErrorData::internal_error(
                    format!(
                        "hive agent `{agent_id}` no longer tracks run `{run_id}`; inspect `list_agents` for the latest state"
                    ),
                    None,
                )))
            }
        };

        if let Some(response) = maybe_response {
            return response;
        }
        if timeout.is_some_and(|max_wait| wait_started_at.elapsed() >= max_wait) {
            return wait_timeout_response(runtime, agent_id, run_id, wait_started_at.elapsed())
                .await;
        }
        sleep(Duration::from_millis(HIVE_BLOCKING_POLL_MS)).await;
    }
}

async fn wait_timeout_response(
    runtime: &HiveRuntime,
    agent_id: &str,
    run_id: &str,
    waited: Duration,
) -> Result<Map<String, Value>, ErrorData> {
    let _lock = runtime.acquire_lock().await.map_err(internal_error)?;
    let mut state = runtime
        .load_state()
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
    sync_state_with_processes(&mut state)
        .await
        .map_err(internal_error)?;
    state.updated_at_ms = now_ms();
    runtime.save_state(&state).map_err(internal_error)?;
    let agent = state
        .agents
        .iter()
        .find(|candidate| candidate.agent_id == agent_id)
        .ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "hive agent `{agent_id}` was not found in session `{}`",
                    runtime.session_id
                ),
                None,
            )
        })?;

    if let Some(last_run) = agent.last_run.as_ref().filter(|run| run.run_id == run_id) {
        return Ok(build_session_response(
            "completed",
            completed_run_message(agent_id, last_run),
            &state,
            Some(agent),
            HiveResponseShape::Summary,
        ));
    }
    if agent
        .current_run
        .as_ref()
        .is_some_and(|run| run.run_id == run_id)
    {
        return Ok(build_session_response(
            "running",
            format!(
                "hive agent `{agent_id}` run `{run_id}` is still running after {} ms. {}",
                waited.as_millis(),
                wait_agent_hint(agent_id)
            ),
            &state,
            Some(agent),
            HiveResponseShape::Summary,
        ));
    }
    Ok(build_error_response(
        "idle",
        format!(
            "hive agent `{agent_id}` no longer tracks run `{run_id}`; inspect `list_agents` for the latest state"
        ),
        &state,
        Some(agent),
        HiveResponseShape::Summary,
    ))
}

fn completed_run_message(agent_id: &str, run: &HiveRunState) -> String {
    if let Some(code) = run.exit_code {
        if code == 0 {
            return format!("hive agent `{agent_id}` run `{}` completed", run.run_id);
        }
        return format!(
            "hive agent `{agent_id}` run `{}` finished with exit code {code}",
            run.run_id
        );
    }
    if let Some(reason) = run.termination_reason.as_deref() {
        return format!(
            "hive agent `{agent_id}` run `{}` finished with `{reason}`",
            run.run_id
        );
    }
    format!("hive agent `{agent_id}` run `{}` completed", run.run_id)
}

fn wait_agent_hint(agent_id: &str) -> String {
    format!(
        "Call action=`wait_agent` with `agent_id`=`{agent_id}` and optional `timeout_ms` to wait in one blocking call."
    )
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

fn rollback_launch_failure(
    runtime: &HiveRuntime,
    state: &mut HiveSessionState,
    agent_index: usize,
    reason: &str,
) {
    if let Some(agent) = state.agents.get_mut(agent_index) {
        if let Some(run) = agent.current_run.as_mut() {
            finalize_run(run, Some(reason));
        }
        agent.last_run = agent.current_run.take();
        agent.status = HiveAgentStatus::Idle;
        agent.last_used_at_ms = Some(now_ms());
    }
    state.updated_at_ms = now_ms();
    if let Err(err) = runtime.save_state(state) {
        warn!(reason = %reason, error = %err, "failed to persist hive rollback state");
    }
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

async fn sync_state_with_processes(state: &mut HiveSessionState) -> Result<()> {
    for agent in &mut state.agents {
        if agent.status == HiveAgentStatus::Closed {
            continue;
        }

        let Some(run) = agent.current_run.as_mut() else {
            continue;
        };

        let process_state = run_process_state(run).await?;
        if process_state == RunProcessState::LiveOwned {
            agent.status = HiveAgentStatus::Running;
            continue;
        }
        if process_state == RunProcessState::IdentityMismatch {
            warn!(
                agent_id = %agent.agent_id,
                pid = ?run.process_pid,
                launcher = %run.launcher_path,
                "sync detected process identity mismatch; preserving running state for manual inspection"
            );
            agent.status = HiveAgentStatus::Running;
            continue;
        }

        finalize_run(run, Some("process_missing_without_result"));
        let session_id = run.provider_session_id.clone();
        agent.last_run = agent.current_run.take();
        if session_id.is_some() {
            agent.conversation_session_id = session_id;
        }
        agent.status = HiveAgentStatus::Idle;
        agent.last_used_at_ms = Some(now_ms());
    }
    Ok(())
}

fn finalize_run(run: &mut HiveRunState, fallback_reason: Option<&str>) {
    let result_path = Path::new(&run.result_path);
    let mut loaded = false;
    if let Ok(raw) = fs::read_to_string(result_path) {
        if let Ok(result) = serde_json::from_str::<HiveRunResultFile>(&raw) {
            let provider_output = parse_provider_output(&result.provider, &run.output_path);
            run.exit_code = Some(result.exit_code);
            run.started_at_ms = result.started_at_ms;
            run.finished_at_ms = Some(result.finished_at_ms);
            run.provider = result.provider;
            run.provider_output = Some(provider_output.clone());
            run.provider_session_id = provider_output
                .session_id
                .clone()
                .or(result.provider_session_id);
            run.termination_reason = None;
            loaded = true;
        }
    }
    if run.finished_at_ms.is_none() {
        run.finished_at_ms = Some(now_ms());
    }
    if !loaded && run.termination_reason.is_none() {
        run.termination_reason = fallback_reason.map(ToOwned::to_owned);
    }
    if !loaded && run.provider_output.is_none() {
        let provider_output = parse_provider_output(&run.provider, &run.output_path);
        run.provider_session_id = provider_output.session_id.clone();
        run.provider_output = Some(provider_output);
    }
    run.process_pid = None;
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

fn derive_session_id(repo_root: &Path) -> String {
    format!("hive-repo-{:08x}", stable_repo_hash(repo_root))
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

fn stable_repo_hash(repo_root: &Path) -> u32 {
    repo_root
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(FNV1A_OFFSET_BASIS, |acc, byte| {
            (acc ^ u32::from(*byte)).wrapping_mul(FNV1A_PRIME)
        })
}

fn resolve_default_session_id(repo_root: &Path, state_dir: &Path) -> String {
    let stable = derive_session_id(repo_root);
    if state_dir.join(format!("{stable}.json")).exists() {
        return stable;
    }
    if let Some(existing) = find_existing_default_session_id_for_repo(repo_root, state_dir) {
        return existing;
    }
    stable
}

fn find_existing_default_session_id_for_repo(repo_root: &Path, state_dir: &Path) -> Option<String> {
    let repo_root_str = repo_root.display().to_string();
    let mut matches = fs::read_dir(state_dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|path| {
            let raw = fs::read_to_string(&path).ok()?;
            let state = parse_hive_session_state(&raw).ok()?;
            if state.repo_root != repo_root_str {
                return None;
            }
            if state.session_id.starts_with("hive-repo-") {
                return Some(state.session_id);
            }
            None
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        return matches.into_iter().next();
    }
    if matches.len() > 1 {
        warn!(
            repo_root = %repo_root.display(),
            "multiple historical default hive sessions found; using the new stable default session id"
        );
    }
    None
}

fn build_session_response(
    state_name: &str,
    message: impl Into<String>,
    state: &HiveSessionState,
    agent: Option<&HiveAgentState>,
    shape: HiveResponseShape,
) -> Map<String, Value> {
    let session_state = derive_session_state(state);

    let mut raw = Map::new();
    raw.insert("ok".to_string(), Value::Bool(true));
    raw.insert("state".to_string(), Value::String(state_name.to_string()));
    raw.insert("message".to_string(), Value::String(message.into()));
    raw.insert(
        "session".to_string(),
        serde_json::json!({
            "id": state.session_id,
            "name": state.session_name,
            "agent_limit": state.agent_limit,
            "state": session_state
        }),
    );
    if shape == HiveResponseShape::List {
        let agents: Vec<Value> = state.agents.iter().map(agent_json).collect();
        let reusable_agents: Vec<Value> = state
            .agents
            .iter()
            .filter(|agent| agent.status == HiveAgentStatus::Idle)
            .map(|agent| Value::String(agent.agent_id.clone()))
            .collect();
        raw.insert("agents".to_string(), Value::Array(agents));
        raw.insert("reusable_agents".to_string(), Value::Array(reusable_agents));
    }
    if let Some(agent) = agent {
        raw.insert("agent".to_string(), agent_json(agent));
    }
    raw
}

fn derive_session_state(state: &HiveSessionState) -> &'static str {
    if state
        .agents
        .iter()
        .any(|agent| agent.status == HiveAgentStatus::Running)
    {
        "running"
    } else if state
        .agents
        .iter()
        .any(|agent| agent.status != HiveAgentStatus::Closed)
    {
        "registered"
    } else {
        "empty"
    }
}

fn build_error_response(
    state_name: &str,
    message: impl Into<String>,
    state: &HiveSessionState,
    agent: Option<&HiveAgentState>,
    shape: HiveResponseShape,
) -> Map<String, Value> {
    let mut raw = build_session_response(state_name, message, state, agent, shape);
    raw.insert("ok".to_string(), Value::Bool(false));
    raw
}

fn agent_json(agent: &HiveAgentState) -> Value {
    serde_json::json!({
        "agent_id": agent.agent_id,
        "worker_name": agent.worker_name,
        "mode": agent.mode,
        "workdir": agent.workdir,
        "conversation_session_id": agent.conversation_session_id,
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
        "provider": run.provider,
        "output_path": run.output_path,
        "prompt_path": run.prompt_path,
        "result_path": run.result_path,
        "launcher_path": run.launcher_path,
        "process_pid": run.process_pid,
        "resume_requested": run.resume_requested,
        "provider_session_id": run.provider_session_id,
        "started_at_ms": run.started_at_ms,
        "finished_at_ms": run.finished_at_ms,
        "exit_code": run.exit_code,
        "termination_reason": run.termination_reason,
        "provider_output": run.provider_output.as_ref().map(provider_output_json),
    })
}

fn provider_output_json(output: &HiveProviderOutput) -> Value {
    serde_json::json!({
        "summary": output.summary,
    })
}

fn parse_provider_output(provider: &str, output_path: &str) -> HiveProviderOutput {
    parse_provider_output_impl(provider, output_path, prompt_preview)
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

fn validate_mode_config(
    mode_name: &str,
    config: &McpHiveClaudeModeConfig,
) -> Result<(), ErrorData> {
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
    if config.system_prompt.is_some() && config.system_prompt_file.is_some() {
        return Err(ErrorData::invalid_params(
            format!(
                "hive mode `{mode_name}` sets both `system_prompt` and `system_prompt_file`; only one is allowed"
            ),
            None,
        ));
    }
    validate_env_keys(&format!("hive mode `{mode_name}`"), &config.env)?;
    Ok(())
}

fn validate_env_keys(scope: &str, env_map: &BTreeMap<String, String>) -> Result<(), ErrorData> {
    for key in env_map.keys() {
        let first = key.chars().next();
        if first.is_none()
            || first.is_some_and(|ch| ch.is_ascii_digit())
            || !key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(ErrorData::invalid_params(
                format!(
                    "{scope} has invalid env key `{key}`; env keys must start with a letter or `_`"
                ),
                None,
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_claude_exec_command(
    runtime: &HiveRuntime,
    shared_env_set: &BTreeMap<String, String>,
    mode_config: &McpHiveClaudeModeConfig,
    workdir: &Path,
    prompt_path: &Path,
    output_path: &Path,
    result_path: &Path,
    run_dir: &Path,
    resume_session_id: Option<&str>,
    resume: bool,
) -> Result<String> {
    let command = mode_config
        .command
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("claude");
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

    let caps = provider_caps(command);
    let system_prompt_args = resolve_system_prompt_args(
        &caps,
        mode_config.system_prompt.as_deref(),
        mode_config.system_prompt_file.as_deref(),
        run_dir,
    )?;

    let mut command_parts = vec![shell_escape(command)];
    command_parts.extend(
        mode_config
            .args
            .iter()
            .map(|arg| shell_escape(arg))
            .collect::<Vec<_>>(),
    );
    if resume {
        let session_id = resume_session_id
            .ok_or_else(|| anyhow!("resume requested but agent has no saved Claude session id"))?;
        command_parts.push("--resume".to_string());
        command_parts.push(shell_escape(session_id));
    }
    if let Some(settings) = expanded_settings {
        command_parts.push("--settings".to_string());
        command_parts.push(shell_escape(&settings.display().to_string()));
    }
    if let Some(mcp_config) = expanded_mcp {
        command_parts.push("--mcp-config".to_string());
        command_parts.push(shell_escape(&mcp_config.display().to_string()));
    }
    for arg in &system_prompt_args {
        command_parts.push(shell_escape(arg));
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
    for (key, value) in shared_env_set {
        script.push_str(&format!(
            "export {}={}\n",
            shell_var_name(key)?,
            shell_escape(value)
        ));
    }
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
    script.push_str(
        "STARTED_AT_MS=$(python3 - <<'PY'\nimport time\nprint(int(time.time() * 1000))\nPY\n)\n",
    );
    script.push_str("RC=0\n");
    script.push_str("set +e\n");
    script.push_str(&format!(
        "{} -p --output-format json \"$PROMPT\" >{} 2>&1\n",
        command_parts.join(" "),
        shell_escape(&output_path.display().to_string()),
    ));
    script.push_str("RC=$?\n");
    script.push_str("set -e\n");
    script.push_str(
        "FINISHED_AT_MS=$(python3 - <<'PY'\nimport time\nprint(int(time.time() * 1000))\nPY\n)\n",
    );
    script.push_str(&format!(
        "python3 - <<'PY' {} \"$STARTED_AT_MS\" \"$FINISHED_AT_MS\" \"$RC\"\nimport json, pathlib, sys\nresult_path = pathlib.Path(sys.argv[1])\nstarted_at_ms = int(sys.argv[2])\nfinished_at_ms = int(sys.argv[3])\nexit_code = int(sys.argv[4])\nresult_path.write_text(json.dumps({{\n    'provider': 'claude',\n    'exit_code': exit_code,\n    'started_at_ms': started_at_ms,\n    'finished_at_ms': finished_at_ms,\n}}))\nPY\n",
        shell_escape(&result_path.display().to_string()),
    ));
    script.push_str("exit \"$RC\"\n");

    Ok(script)
}

async fn spawn_hive_run_process(
    workdir: &Path,
    launcher_path: &Path,
    run_id: &str,
    login_shell: &str,
) -> Result<u32> {
    let mut command = Command::new(login_shell);
    command
        .args(login_shell_command_args(login_shell, launcher_path, run_id))
        .current_dir(workdir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().with_context(|| {
        format!(
            "failed to spawn hive launcher `{}` via login shell `{login_shell}`",
            launcher_path.display()
        )
    })?;
    child
        .id()
        .ok_or_else(|| anyhow!("failed to read spawned process pid"))
}

fn default_login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "bash".to_string())
}

fn login_shell_command_args(login_shell: &str, launcher_path: &Path, run_id: &str) -> Vec<String> {
    let command = format!(
        "exec bash {} {}",
        shell_escape(&launcher_path.display().to_string()),
        shell_escape(&hive_run_marker(run_id))
    );
    match Path::new(login_shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(login_shell)
    {
        "fish" => vec!["-l".to_string(), "-c".to_string(), command],
        _ => vec!["-lc".to_string(), command],
    }
}

async fn process_is_alive(pid: u32) -> Result<bool> {
    let status = Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .await
        .with_context(|| format!("failed to probe process `{pid}`"))?;
    Ok(status.success())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunProcessState {
    LiveOwned,
    FinishedOrMissing,
    IdentityMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminateRunResult {
    Terminated,
    NotRunning,
    IdentityMismatch,
}

async fn process_matches_run(pid: u32, run_id: &str, launcher_path: &str) -> Result<bool> {
    let output = Command::new("ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .await
        .with_context(|| format!("failed to inspect process `{pid}` command line"))?;
    if !output.status.success() {
        return Ok(false);
    }
    let command_line = String::from_utf8_lossy(&output.stdout);
    let marker = hive_run_marker(run_id);
    if command_line.split_whitespace().any(|token| token == marker) {
        return Ok(true);
    }
    Ok(command_line_has_launcher_suffix(
        &command_line,
        launcher_path,
    ))
}

async fn run_process_state(run: &HiveRunState) -> Result<RunProcessState> {
    if persisted_run_result_exists(run) {
        return Ok(RunProcessState::FinishedOrMissing);
    }
    let Some(pid) = run.process_pid else {
        return Ok(RunProcessState::FinishedOrMissing);
    };
    if !process_is_alive(pid).await? {
        return Ok(RunProcessState::FinishedOrMissing);
    }
    if !process_matches_run(pid, &run.run_id, &run.launcher_path).await? {
        return Ok(RunProcessState::IdentityMismatch);
    }
    Ok(RunProcessState::LiveOwned)
}

fn persisted_run_result_exists(run: &HiveRunState) -> bool {
    let result_path = Path::new(&run.result_path);
    if !result_path.is_file() {
        return false;
    }
    fs::read_to_string(result_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<HiveRunResultFile>(&raw).ok())
        .is_some()
}

async fn kill_process_group(pid: u32, signal: &str) -> Result<()> {
    let status = Command::new("kill")
        .arg(signal)
        .arg(format!("-{pid}"))
        .status()
        .await
        .with_context(|| format!("failed to send signal {signal} to process group `{pid}`"))?;
    if status.success() {
        return Ok(());
    }
    if !process_is_alive(pid).await? {
        return Ok(());
    }
    Err(anyhow!(
        "failed to send signal {signal} to process group `{pid}`"
    ))
}

async fn terminate_run_process_if_owned(run: &HiveRunState) -> Result<TerminateRunResult> {
    let Some(pid) = run.process_pid else {
        return Ok(TerminateRunResult::NotRunning);
    };
    match run_process_state(run).await? {
        RunProcessState::FinishedOrMissing => Ok(TerminateRunResult::NotRunning),
        RunProcessState::IdentityMismatch => Ok(TerminateRunResult::IdentityMismatch),
        RunProcessState::LiveOwned => {
            kill_process_group(pid, "-TERM").await?;
            sleep(Duration::from_millis(100)).await;
            if run_process_state(run).await? == RunProcessState::LiveOwned {
                kill_process_group(pid, "-KILL").await?;
                sleep(Duration::from_millis(50)).await;
                if run_process_state(run).await? == RunProcessState::LiveOwned {
                    return Err(anyhow!("failed to force terminate process group `{pid}`"));
                }
            }
            Ok(TerminateRunResult::Terminated)
        }
    }
}

async fn terminate_process_group_if_owned(
    pid: u32,
    run_id: &str,
    launcher_path: &Path,
) -> Result<()> {
    if !process_is_alive(pid).await? {
        return Ok(());
    }
    let launcher = launcher_path.display().to_string();
    if !process_matches_run(pid, run_id, &launcher).await? {
        return Ok(());
    }
    kill_process_group(pid, "-TERM").await?;
    sleep(Duration::from_millis(100)).await;
    if process_is_alive(pid).await? && process_matches_run(pid, run_id, &launcher).await? {
        kill_process_group(pid, "-KILL").await?;
    }
    Ok(())
}

fn hive_run_marker(run_id: &str) -> String {
    format!("{HIVE_RUN_MARKER_PREFIX}{run_id}")
}

fn command_line_has_launcher_suffix(command_line: &str, launcher_path: &str) -> bool {
    if launcher_path.is_empty() {
        return false;
    }
    let trimmed = command_line.trim_end();
    let Some(prefix) = trimmed.strip_suffix(launcher_path) else {
        return false;
    };
    match prefix.chars().last() {
        None => true,
        Some(ch) => ch.is_whitespace() || ch == '\'' || ch == '"',
    }
}

fn ensure_binary_available(binary: &str) -> Result<()> {
    let path = Path::new(binary);
    if path.components().count() > 1 {
        if path.is_file() {
            return Ok(());
        }
        return Err(anyhow!("required binary `{binary}` does not exist"));
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return Err(anyhow!(
            "required binary `{binary}` is not available in PATH"
        ));
    };
    if std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file()) {
        return Ok(());
    }
    Err(anyhow!(
        "required binary `{binary}` is not available in PATH"
    ))
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
    use crate::hive_provider::HiveProviderOutputFormat;
    use std::collections::BTreeMap;
    use std::process::Command as StdCommand;
    use tempfile::tempdir;

    fn sample_runtime(temp: &tempfile::TempDir) -> HiveRuntime {
        HiveRuntime {
            repo_root: temp.path().to_path_buf(),
            state_dir: temp.path().join("state"),
            session_id: "hive-repo-1234abcd".to_string(),
            session_name: "agpod-hive-repo-1234abcd".to_string(),
            config: Config::default(),
        }
    }

    fn runtime_with_shell_mode(temp: &tempfile::TempDir, shell_body: &str) -> HiveRuntime {
        let mut runtime = sample_runtime(temp);
        runtime.config.mcp = Some(agpod_core::McpConfig {
            hive: Some(agpod_core::McpHiveConfig {
                claude: Some(McpHiveClaudeConfig {
                    env_set: BTreeMap::new(),
                    modes: BTreeMap::from([(
                        "readonly".to_string(),
                        McpHiveClaudeModeConfig {
                            description: Some("readonly".to_string()),
                            command: Some("sh".to_string()),
                            args: vec!["-c".to_string(), shell_body.to_string()],
                            ..Default::default()
                        },
                    )]),
                }),
            }),
        });
        runtime
    }

    #[test]
    fn derive_session_id_is_stable_for_same_repo() {
        let repo = Path::new("/tmp/project-a");
        let first = derive_session_id(repo);
        let second = derive_session_id(repo);
        assert_eq!(first, second);
        assert!(first.starts_with("hive-repo-"));
    }

    #[tokio::test]
    async fn mode_info_lists_custom_modes_and_hides_sensitive_configuration_details() {
        let temp = tempdir().expect("temp dir");
        let mut runtime = sample_runtime(&temp);
        let mut readonly = McpHiveClaudeModeConfig {
            description: Some("Read-only worker".to_string()),
            command: Some("claude".to_string()),
            settings: Some("~/.claude/settings.json".to_string()),
            mcp_config: Some("~/.claude/generated/mcp-readonly.json".to_string()),
            ..Default::default()
        };
        readonly
            .env
            .insert("MAX_MCP_OUTPUT_TOKENS".to_string(), "12000".to_string());
        runtime.config.mcp = Some(agpod_core::McpConfig {
            hive: Some(agpod_core::McpHiveConfig {
                claude: Some(McpHiveClaudeConfig {
                    env_set: BTreeMap::from([(
                        "ANTHROPIC_AUTH_TOKEN".to_string(),
                        "secret".to_string(),
                    )]),
                    modes: BTreeMap::from([
                        ("readonly".to_string(), readonly),
                        (
                            "analysis".to_string(),
                            McpHiveClaudeModeConfig {
                                description: Some("Analysis worker".to_string()),
                                command: Some("claude".to_string()),
                                ..Default::default()
                            },
                        ),
                    ]),
                }),
            }),
        });

        let raw = mode_info(
            &runtime,
            HiveRequest {
                action: HiveActionInput::ModeInfo,
                session_id: None,
                agent_id: None,
                mode: Some("not-a-real-mode".to_string()),
                worker_name: None,
                workdir: None,
                prompt: None,
                resume: None,
                async_: false,
                timeout_ms: None,
            },
        )
        .await
        .expect("mode_info should succeed");

        let text = serde_json::to_string(&raw).expect("serialize mode_info");
        assert!(text.contains("\"modes\""));
        assert!(text.contains("\"available_modes\""));
        assert!(text.contains("\"analysis\""));
        assert!(text.contains("\"readonly\""));
        assert!(!text.contains("selected_mode"));
        assert!(!text.contains("settings"));
        assert!(!text.contains("mcp_config"));
        assert!(!text.contains("env_keys"));
        assert!(!text.contains("config_path"));
        assert!(!text.contains("ANTHROPIC_AUTH_TOKEN"));
    }

    #[tokio::test]
    async fn run_hive_agent_accepts_custom_configured_mode() {
        let temp = tempdir().expect("temp dir");
        let mut runtime = sample_runtime(&temp);
        runtime.config.mcp = Some(agpod_core::McpConfig {
            hive: Some(agpod_core::McpHiveConfig {
                claude: Some(McpHiveClaudeConfig {
                    env_set: BTreeMap::new(),
                    modes: BTreeMap::from([(
                        "analysis".to_string(),
                        McpHiveClaudeModeConfig {
                            description: Some("analysis".to_string()),
                            command: Some("sh".to_string()),
                            args: vec![
                                "-c".to_string(),
                                "sleep 0.2; printf '{\"session_id\":\"sess-analysis\",\"summary\":\"ok\"}\\n'"
                                    .to_string(),
                            ],
                            ..Default::default()
                        },
                    )]),
                }),
            }),
        });

        let raw = run_hive_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::RunHiveAgent,
                session_id: None,
                agent_id: None,
                mode: Some("analysis".to_string()),
                worker_name: Some("worker".to_string()),
                workdir: Some(temp.path().display().to_string()),
                prompt: Some("hello".to_string()),
                resume: None,
                async_: false,
                timeout_ms: None,
            },
        )
        .await
        .expect("run_hive_agent should accept custom mode");

        assert_eq!(raw.get("state").and_then(Value::as_str), Some("completed"));
        assert_eq!(
            raw.get("agent")
                .and_then(|agent| agent.get("mode"))
                .and_then(Value::as_str),
            Some("analysis")
        );
    }

    #[test]
    fn next_agent_id_skips_existing_indexes() {
        let existing = vec![
            HiveAgentState {
                agent_id: "agent-01".to_string(),
                worker_name: "a".to_string(),
                mode: "default".to_string(),
                workdir: "/tmp".to_string(),
                conversation_session_id: None,
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
                conversation_session_id: None,
                status: HiveAgentStatus::Idle,
                current_run: None,
                last_run: None,
                last_used_at_ms: None,
            },
        ];
        assert_eq!(next_agent_id(&existing), "agent-04");
    }

    #[test]
    fn hive_request_async_defaults_to_true() {
        let req: HiveRequest = serde_json::from_value(serde_json::json!({
            "action": "run_hive_agent",
            "prompt": "hello"
        }))
        .expect("hive request should deserialize");
        assert!(req.async_);
    }

    #[test]
    fn build_session_response_marks_idle_agents_reusable() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-repo-1234abcd".to_string(),
            session_name: "agpod-hive-repo-1234abcd".to_string(),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: vec![
                HiveAgentState {
                    agent_id: "agent-01".to_string(),
                    worker_name: "idle".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: Some("sess-1".to_string()),
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
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: Some(HiveRunState {
                        run_id: "run-1".to_string(),
                        prompt_preview: "hello".to_string(),
                        provider: "claude".to_string(),
                        output_path: "/tmp/output".to_string(),
                        prompt_path: "/tmp/prompt".to_string(),
                        result_path: "/tmp/result".to_string(),
                        launcher_path: "/tmp/launcher".to_string(),
                        process_pid: Some(42),
                        resume_requested: false,
                        provider_session_id: None,
                        started_at_ms: 2,
                        finished_at_ms: None,
                        exit_code: None,
                        termination_reason: None,
                        provider_output: None,
                    }),
                    last_run: None,
                    last_used_at_ms: Some(2),
                },
            ],
        };

        let raw = build_session_response("listed", "ok", &state, None, HiveResponseShape::List);
        let reusable = raw
            .get("reusable_agents")
            .and_then(Value::as_array)
            .expect("reusable agents should be array");
        assert_eq!(reusable, &vec![Value::String("agent-01".to_string())]);
        assert_eq!(
            raw.get("session")
                .and_then(|session| session.get("state"))
                .and_then(Value::as_str),
            Some("running")
        );
    }

    #[test]
    fn build_session_response_summary_omits_agent_lists() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-repo-1234abcd".to_string(),
            session_name: "agpod-hive-repo-1234abcd".to_string(),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: vec![],
        };

        let raw =
            build_session_response("completed", "ok", &state, None, HiveResponseShape::Summary);
        assert!(raw.get("agents").is_none());
        assert!(raw.get("reusable_agents").is_none());
    }

    #[test]
    fn prompt_only_targets_idle_agents() {
        assert!(prompt_accept_state(&HiveAgentStatus::Idle));
        assert!(!prompt_accept_state(&HiveAgentStatus::Running));
        assert!(!prompt_accept_state(&HiveAgentStatus::Closed));
    }

    #[test]
    fn build_hive_agent_limit_message_suggests_reuse_when_idle_exists() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-repo-1234abcd".to_string(),
            session_name: "agpod-hive-repo-1234abcd".to_string(),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: vec![
                HiveAgentState {
                    agent_id: "agent-01".to_string(),
                    worker_name: "idle".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Idle,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
                HiveAgentState {
                    agent_id: "agent-02".to_string(),
                    worker_name: "running".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
            ],
        };

        let message = build_hive_agent_limit_message(&state);
        assert!(message.contains("maximum 5 live agents"));
        assert!(message.contains("explicit `agent_id`"));
        assert!(message.contains("reusable: agent-01"));
        assert!(message.contains("resume=false"));
        assert!(message.contains("action=`close_agent`"));
        assert!(message.contains("action=`close_session`"));
    }

    #[test]
    fn build_hive_agent_limit_message_mentions_no_idle_reuse_path() {
        let state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-repo-1234abcd".to_string(),
            session_name: "agpod-hive-repo-1234abcd".to_string(),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: vec![
                HiveAgentState {
                    agent_id: "agent-01".to_string(),
                    worker_name: "running-a".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
                HiveAgentState {
                    agent_id: "agent-02".to_string(),
                    worker_name: "running-b".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
            ],
        };

        let message = build_hive_agent_limit_message(&state);
        assert!(message.contains("No idle agents are currently reusable"));
        assert!(message.contains("action=`close_agent`"));
        assert!(message.contains("action=`list_agents`"));
    }

    #[test]
    fn ensure_run_hive_agent_target_requires_explicit_agent_id_when_limit_reached() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        let mut state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: runtime.session_id.clone(),
            session_name: runtime.session_name.clone(),
            repo_root: runtime.repo_root.display().to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: vec![
                HiveAgentState {
                    agent_id: "agent-01".to_string(),
                    worker_name: "idle".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Idle,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
                HiveAgentState {
                    agent_id: "agent-02".to_string(),
                    worker_name: "running-a".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
                HiveAgentState {
                    agent_id: "agent-03".to_string(),
                    worker_name: "running-b".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
                HiveAgentState {
                    agent_id: "agent-04".to_string(),
                    worker_name: "running-c".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
                HiveAgentState {
                    agent_id: "agent-05".to_string(),
                    worker_name: "running-d".to_string(),
                    mode: "readonly".to_string(),
                    workdir: "/repo".to_string(),
                    conversation_session_id: None,
                    status: HiveAgentStatus::Running,
                    current_run: None,
                    last_run: None,
                    last_used_at_ms: None,
                },
            ],
        };
        let req = HiveRequest {
            action: HiveActionInput::RunHiveAgent,
            session_id: None,
            agent_id: None,
            mode: None,
            worker_name: None,
            workdir: None,
            prompt: Some("hello".to_string()),
            resume: None,
            async_: true,
            timeout_ms: None,
        };

        let err = ensure_run_hive_agent_target(&runtime, &mut state, &req)
            .expect_err("limit should require explicit agent selection");
        assert!(err.message.contains("explicit `agent_id`"));
        assert!(err.message.contains("resume=false"));
        assert!(err.message.contains("action=`close_agent`"));
        assert!(err.message.contains("action=`close_session`"));
    }

    #[tokio::test]
    async fn run_hive_agent_blocks_until_completion_when_async_is_false() {
        let temp = tempdir().expect("temp dir");
        let runtime = runtime_with_shell_mode(
            &temp,
            "sleep 0.2; printf '{\"session_id\":\"sess-block\",\"summary\":\"done\"}\\n'",
        );
        let raw = run_hive_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::RunHiveAgent,
                session_id: None,
                agent_id: None,
                mode: Some("readonly".to_string()),
                worker_name: Some("worker".to_string()),
                workdir: Some(temp.path().display().to_string()),
                prompt: Some("hello".to_string()),
                resume: None,
                async_: false,
                timeout_ms: None,
            },
        )
        .await
        .expect("run_hive_agent should succeed");

        assert_eq!(raw.get("state").and_then(Value::as_str), Some("completed"));
        assert!(raw.get("agents").is_none());
        assert!(raw.get("reusable_agents").is_none());
        assert_eq!(
            raw.get("agent")
                .and_then(|agent| agent.get("status"))
                .and_then(Value::as_str),
            Some("idle")
        );
        assert_eq!(
            raw.get("agent")
                .and_then(|agent| agent.get("last_run"))
                .and_then(|run| run.get("exit_code"))
                .and_then(Value::as_i64),
            Some(0)
        );
        assert_eq!(
            raw.get("agent")
                .and_then(|agent| agent.get("last_run"))
                .and_then(|run| run.get("provider_output"))
                .and_then(|output| output.get("summary"))
                .and_then(Value::as_str),
            Some("done")
        );
        assert_eq!(
            raw.get("agent")
                .and_then(|agent| agent.get("last_run"))
                .and_then(|run| run.get("provider_output"))
                .and_then(Value::as_object)
                .map(|object| object.keys().cloned().collect::<Vec<_>>()),
            Some(vec!["summary".to_string()])
        );
    }

    #[tokio::test]
    async fn run_hive_agent_async_can_be_polled_via_list_agents() {
        let temp = tempdir().expect("temp dir");
        let runtime = runtime_with_shell_mode(
            &temp,
            "sleep 0.2; printf '{\"session_id\":\"sess-async\",\"summary\":\"later\"}\\n'",
        );
        let started = run_hive_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::RunHiveAgent,
                session_id: None,
                agent_id: None,
                mode: Some("readonly".to_string()),
                worker_name: Some("worker".to_string()),
                workdir: Some(temp.path().display().to_string()),
                prompt: Some("hello".to_string()),
                resume: None,
                async_: true,
                timeout_ms: None,
            },
        )
        .await
        .expect("async run_hive_agent should succeed");

        assert_eq!(
            started.get("state").and_then(Value::as_str),
            Some("running")
        );
        assert!(started.get("agents").is_none());
        assert!(started.get("reusable_agents").is_none());
        let agent_id = started
            .get("agent")
            .and_then(|agent| agent.get("agent_id"))
            .and_then(Value::as_str)
            .expect("agent id")
            .to_string();
        let expected_message = format!("hive agent `{agent_id}` status fetched");
        let mut listed = None;
        let mut last_status = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let response = list_agents(
                &runtime,
                HiveRequest {
                    action: HiveActionInput::ListAgents,
                    session_id: None,
                    agent_id: Some(agent_id.clone()),
                    mode: None,
                    worker_name: None,
                    workdir: None,
                    prompt: None,
                    resume: None,
                    async_: false,
                    timeout_ms: None,
                },
            )
            .await
            .expect("list_agents should succeed");
            let status = response
                .get("agent")
                .and_then(|agent| agent.get("status"))
                .and_then(Value::as_str);
            last_status = status.map(ToOwned::to_owned);
            if status == Some("idle") {
                listed = Some(response);
                break;
            }
            sleep(Duration::from_millis(150)).await;
        }
        let listed = listed.unwrap_or_else(|| {
            panic!(
                "async run should settle to idle; last observed status: {:?}",
                last_status
            )
        });

        assert_eq!(listed.get("state").and_then(Value::as_str), Some("listed"));
        assert_eq!(
            listed.get("message").and_then(Value::as_str),
            Some(expected_message.as_str())
        );
        assert_eq!(
            listed
                .get("agent")
                .and_then(|agent| agent.get("status"))
                .and_then(Value::as_str),
            Some("idle")
        );
        assert_eq!(
            listed
                .get("agent")
                .and_then(|agent| agent.get("last_run"))
                .and_then(|run| run.get("provider_output"))
                .and_then(|output| output.get("summary"))
                .and_then(Value::as_str),
            Some("later")
        );
    }

    #[tokio::test]
    async fn wait_agent_requires_agent_id() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        let err = wait_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::WaitAgent,
                session_id: None,
                agent_id: None,
                mode: None,
                worker_name: None,
                workdir: None,
                prompt: None,
                resume: None,
                async_: false,
                timeout_ms: None,
            },
        )
        .await
        .expect_err("wait_agent without agent_id should fail");
        assert!(err.message.contains("`agent_id` is required"));
    }

    #[tokio::test]
    async fn wait_agent_returns_running_when_timeout_is_hit() {
        let temp = tempdir().expect("temp dir");
        let runtime = runtime_with_shell_mode(
            &temp,
            "sleep 0.4; printf '{\"session_id\":\"sess-wait\",\"summary\":\"later\"}\\n'",
        );
        let started = run_hive_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::RunHiveAgent,
                session_id: None,
                agent_id: None,
                mode: Some("readonly".to_string()),
                worker_name: Some("worker".to_string()),
                workdir: Some(temp.path().display().to_string()),
                prompt: Some("hello".to_string()),
                resume: None,
                async_: true,
                timeout_ms: None,
            },
        )
        .await
        .expect("async run_hive_agent should succeed");
        let agent_id = started
            .get("agent")
            .and_then(|agent| agent.get("agent_id"))
            .and_then(Value::as_str)
            .expect("agent id")
            .to_string();

        let waiting = wait_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::WaitAgent,
                session_id: None,
                agent_id: Some(agent_id.clone()),
                mode: None,
                worker_name: None,
                workdir: None,
                prompt: None,
                resume: None,
                async_: false,
                timeout_ms: Some(20),
            },
        )
        .await
        .expect("wait_agent should return running response on timeout");

        assert_eq!(
            waiting.get("state").and_then(Value::as_str),
            Some("running")
        );
        assert!(waiting
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("action=`wait_agent`")));
        assert!(waiting
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("still running")));
    }

    #[tokio::test]
    async fn wait_agent_returns_latest_completed_run_when_idle() {
        let temp = tempdir().expect("temp dir");
        let runtime = runtime_with_shell_mode(
            &temp,
            "sleep 0.05; printf '{\"session_id\":\"sess-done\",\"summary\":\"done\"}\\n'",
        );
        let completed = run_hive_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::RunHiveAgent,
                session_id: None,
                agent_id: None,
                mode: Some("readonly".to_string()),
                worker_name: Some("worker".to_string()),
                workdir: Some(temp.path().display().to_string()),
                prompt: Some("hello".to_string()),
                resume: None,
                async_: false,
                timeout_ms: None,
            },
        )
        .await
        .expect("blocking run_hive_agent should complete");
        let agent_id = completed
            .get("agent")
            .and_then(|agent| agent.get("agent_id"))
            .and_then(Value::as_str)
            .expect("agent id")
            .to_string();

        let waited = wait_agent(
            &runtime,
            HiveRequest {
                action: HiveActionInput::WaitAgent,
                session_id: None,
                agent_id: Some(agent_id),
                mode: None,
                worker_name: None,
                workdir: None,
                prompt: None,
                resume: None,
                async_: false,
                timeout_ms: Some(10),
            },
        )
        .await
        .expect("wait_agent should return latest completed run");

        assert_eq!(
            waited.get("state").and_then(Value::as_str),
            Some("completed")
        );
        assert!(waited.get("agents").is_none());
        assert!(waited
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("No active run remains")));
    }

    #[test]
    fn default_hive_state_dir_uses_tmp_root() {
        let path = default_hive_state_dir();
        assert_eq!(path, std::env::temp_dir().join("agpod").join("hive"));
    }

    #[tokio::test]
    async fn hive_runtime_lock_blocks_reentry_until_drop() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);

        let first = runtime.acquire_lock().await.expect("first lock");
        // The second acquire spins up to 200×25 ms = 5 s before failing.
        // Use a timeout so the test doesn't hang if something goes wrong,
        // but still validates that reentry is blocked.
        let second = tokio::time::timeout(Duration::from_secs(10), runtime.acquire_lock())
            .await
            .expect("lock attempt should not hang");
        assert!(second.is_err());
        drop(first);
        let third = runtime.acquire_lock().await;
        assert!(third.is_ok());
    }

    #[tokio::test]
    async fn stale_lock_is_reclaimed() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
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
        assert!(runtime.acquire_lock().await.is_ok());
    }

    #[test]
    fn validate_mode_config_requires_command() {
        let cfg = McpHiveClaudeModeConfig::default();
        let err = validate_mode_config("readonly", &cfg).expect_err("missing command should fail");
        assert!(err.message.contains("requires non-empty `command`"));
    }

    #[test]
    fn validate_mode_config_rejects_env_key_with_dash() {
        let mut cfg = McpHiveClaudeModeConfig {
            command: Some("claude".to_string()),
            ..Default::default()
        };
        cfg.env.insert("BAD-KEY".to_string(), "x".to_string());
        let err = validate_mode_config("readonly", &cfg).expect_err("invalid env key should fail");
        assert!(err.message.contains("invalid env key"));
    }

    #[test]
    fn validate_mode_config_rejects_both_system_prompt_fields() {
        let cfg = McpHiveClaudeModeConfig {
            command: Some("claude".to_string()),
            system_prompt: Some("hello".to_string()),
            system_prompt_file: Some("~/.config/agpod/prompt.md".to_string()),
            ..Default::default()
        };
        let err = validate_mode_config("readonly", &cfg)
            .expect_err("both system_prompt fields should fail");
        assert!(err.message.contains("system_prompt"));
        assert!(err.message.contains("system_prompt_file"));
    }

    #[test]
    fn validate_mode_config_accepts_system_prompt_alone() {
        let cfg = McpHiveClaudeModeConfig {
            command: Some("claude".to_string()),
            system_prompt: Some("You are a helper.".to_string()),
            ..Default::default()
        };
        assert!(validate_mode_config("readonly", &cfg).is_ok());
    }

    #[test]
    fn validate_mode_config_accepts_system_prompt_file_alone() {
        let cfg = McpHiveClaudeModeConfig {
            command: Some("claude".to_string()),
            system_prompt_file: Some("~/.config/agpod/prompt.md".to_string()),
            ..Default::default()
        };
        assert!(validate_mode_config("readonly", &cfg).is_ok());
    }

    #[test]
    fn load_state_migrates_legacy_session_shape() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        runtime.ensure_state_dirs().expect("state dirs");
        fs::write(
            runtime.session_file(),
            serde_json::json!({
                "version": 1,
                "session_id": "hive-repo-1234abcd",
                "session_name": "agpod-hive-repo-1234abcd",
                "repo_root": "/repo",
                "agent_limit": 5,
                "updated_at_ms": 10,
                "agents": [{
                    "agent_id": "agent-01",
                    "worker_name": "legacy",
                    "workdir": "/repo",
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
        assert_eq!(
            agent
                .current_run
                .as_ref()
                .map(|run| run.termination_reason.as_deref()),
            Some(Some("legacy_unmanaged_run"))
        );
    }

    #[test]
    fn load_state_tolerates_session_file_missing_new_run_fields() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        runtime.ensure_state_dirs().expect("state dirs");
        fs::write(
            runtime.session_file(),
            serde_json::json!({
                "version": 2,
                "session_id": "hive-repo-1234abcd",
                "session_name": "agpod-hive-repo-1234abcd",
                "repo_root": "/repo",
                "agent_limit": 5,
                "updated_at_ms": 10,
                "agents": [{
                    "agent_id": "agent-01",
                    "worker_name": "worker",
                    "workdir": "/repo",
                    "status": "running",
                    "current_run": {
                        "run_id": "run-1",
                        "prompt_preview": "hello",
                        "output_path": "/tmp/output",
                        "prompt_path": "/tmp/prompt",
                        "result_path": "/tmp/result",
                        "started_at_ms": 1
                    }
                }]
            })
            .to_string(),
        )
        .expect("write state");

        let state = runtime.load_state().expect("load migrated current state");
        let run = state.agents[0]
            .current_run
            .as_ref()
            .expect("run should exist");
        assert_eq!(state.agents[0].mode, "readonly");
        assert_eq!(run.launcher_path, "");
        assert_eq!(run.process_pid, None);
        assert!(!run.resume_requested);
    }

    #[test]
    fn finalize_run_marks_fallback_reason_when_result_missing() {
        let mut run = HiveRunState {
            run_id: "run-1".to_string(),
            prompt_preview: "hello".to_string(),
            provider: "claude".to_string(),
            output_path: "/tmp/output".to_string(),
            prompt_path: "/tmp/prompt".to_string(),
            result_path: "/tmp/missing-result".to_string(),
            launcher_path: "/tmp/launcher".to_string(),
            process_pid: Some(10),
            resume_requested: false,
            provider_session_id: None,
            started_at_ms: 1,
            finished_at_ms: None,
            exit_code: None,
            termination_reason: None,
            provider_output: None,
        };

        finalize_run(&mut run, Some("killed_by_hive"));
        assert_eq!(run.exit_code, None);
        assert_eq!(run.termination_reason.as_deref(), Some("killed_by_hive"));
        assert!(run.finished_at_ms.is_some());
        assert_eq!(run.process_pid, None);
    }

    #[test]
    fn finalize_run_prefers_result_file_and_session_id() {
        let temp = tempdir().expect("temp dir");
        let result_path = temp.path().join("result.json");
        let output_path = temp.path().join("output.json");
        fs::write(
            &output_path,
            serde_json::json!({
                "session_id": "claude-session-1",
                "result": "done"
            })
            .to_string(),
        )
        .expect("write output");
        fs::write(
            &result_path,
            serde_json::json!({
                "provider": "claude",
                "exit_code": 7,
                "started_at_ms": 11,
                "finished_at_ms": 22
            })
            .to_string(),
        )
        .expect("write result");

        let mut run = HiveRunState {
            run_id: "run-1".to_string(),
            prompt_preview: "hello".to_string(),
            provider: "claude".to_string(),
            output_path: output_path.display().to_string(),
            prompt_path: "/tmp/prompt".to_string(),
            result_path: result_path.display().to_string(),
            launcher_path: "/tmp/launcher".to_string(),
            process_pid: Some(10),
            resume_requested: true,
            provider_session_id: None,
            started_at_ms: 1,
            finished_at_ms: None,
            exit_code: None,
            termination_reason: None,
            provider_output: None,
        };

        finalize_run(&mut run, Some("killed_by_hive"));
        assert_eq!(run.exit_code, Some(7));
        assert_eq!(run.started_at_ms, 11);
        assert_eq!(run.finished_at_ms, Some(22));
        assert_eq!(run.provider_session_id.as_deref(), Some("claude-session-1"));
        assert_eq!(run.termination_reason, None);
        assert_eq!(
            run.provider_output
                .as_ref()
                .and_then(|output| output.summary.as_deref()),
            Some("done")
        );
    }

    #[tokio::test]
    async fn sync_state_with_processes_moves_finished_run_to_last_run() {
        let temp = tempdir().expect("temp dir");
        let result_path = temp.path().join("result.json");
        fs::write(
            &result_path,
            serde_json::json!({
                "provider": "claude",
                "exit_code": 0,
                "started_at_ms": 1,
                "finished_at_ms": 2
            })
            .to_string(),
        )
        .expect("write result");
        let output_path = temp.path().join("output.json");
        fs::write(
            &output_path,
            serde_json::json!({
                "session_id": "sess-2",
                "summary": "completed"
            })
            .to_string(),
        )
        .expect("write output");

        let mut state = HiveSessionState {
            version: HIVE_VERSION,
            session_id: "hive-repo-1234abcd".to_string(),
            session_name: "agpod-hive-repo-1234abcd".to_string(),
            repo_root: "/repo".to_string(),
            agent_limit: HIVE_AGENT_LIMIT,
            updated_at_ms: 1,
            agents: vec![HiveAgentState {
                agent_id: "agent-01".to_string(),
                worker_name: "a".to_string(),
                mode: "readonly".to_string(),
                workdir: "/repo".to_string(),
                conversation_session_id: None,
                status: HiveAgentStatus::Running,
                current_run: Some(HiveRunState {
                    run_id: "run-1".to_string(),
                    prompt_preview: "hello".to_string(),
                    provider: "claude".to_string(),
                    output_path: output_path.display().to_string(),
                    prompt_path: "/tmp/prompt".to_string(),
                    result_path: result_path.display().to_string(),
                    launcher_path: "/tmp/launcher".to_string(),
                    process_pid: Some(999_999),
                    resume_requested: false,
                    provider_session_id: None,
                    started_at_ms: 1,
                    finished_at_ms: None,
                    exit_code: None,
                    termination_reason: None,
                    provider_output: None,
                }),
                last_run: None,
                last_used_at_ms: None,
            }],
        };

        sync_state_with_processes(&mut state)
            .await
            .expect("sync should succeed");
        let agent = &state.agents[0];
        assert_eq!(agent.status, HiveAgentStatus::Idle);
        assert!(agent.current_run.is_none());
        assert_eq!(agent.conversation_session_id.as_deref(), Some("sess-2"));
        assert_eq!(
            agent.last_run.as_ref().and_then(|run| run.exit_code),
            Some(0)
        );
    }

    #[test]
    fn build_claude_exec_command_adds_resume_flag() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        let run_dir = temp.path().join("run");
        fs::create_dir_all(&run_dir).expect("run dir");
        let cfg = McpHiveClaudeModeConfig {
            description: Some("readonly".to_string()),
            command: Some("claude".to_string()),
            args: vec!["--dangerously-skip-permissions".to_string()],
            settings: Some("~/.claude/settings.json".to_string()),
            mcp_config: Some("~/.mcp.json".to_string()),
            env: Default::default(),
            ..Default::default()
        };
        let script = build_claude_exec_command(
            &runtime,
            &BTreeMap::new(),
            &cfg,
            Path::new("/repo"),
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/output.log"),
            Path::new("/tmp/result.json"),
            &run_dir,
            Some("claude-session-1"),
            true,
        )
        .expect("script should build");

        assert!(script.contains("--resume"));
        assert!(script.contains("'claude-session-1'"));
        assert!(script.contains("--output-format json"));
    }

    #[test]
    fn build_claude_exec_command_includes_system_prompt_flag() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        let run_dir = temp.path().join("run");
        fs::create_dir_all(&run_dir).expect("run dir");
        let cfg = McpHiveClaudeModeConfig {
            command: Some("claude".to_string()),
            system_prompt: Some("You are a readonly assistant.".to_string()),
            ..Default::default()
        };
        let script = build_claude_exec_command(
            &runtime,
            &BTreeMap::new(),
            &cfg,
            Path::new("/repo"),
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/output.log"),
            Path::new("/tmp/result.json"),
            &run_dir,
            None,
            false,
        )
        .expect("script should build");

        assert!(script.contains("--system-prompt"));
        assert!(script.contains("You are a readonly assistant."));
        assert!(script.contains("--output-format json"));
    }

    #[test]
    fn build_claude_exec_command_passes_system_prompt_file_directly() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        let run_dir = temp.path().join("run");
        fs::create_dir_all(&run_dir).expect("run dir");
        let prompt_file = temp.path().join("sys_prompt.md");
        fs::write(&prompt_file, "File-based system prompt content.").expect("write prompt file");
        let cfg = McpHiveClaudeModeConfig {
            command: Some("claude".to_string()),
            system_prompt_file: Some(prompt_file.display().to_string()),
            ..Default::default()
        };
        let script = build_claude_exec_command(
            &runtime,
            &BTreeMap::new(),
            &cfg,
            Path::new("/repo"),
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/output.log"),
            Path::new("/tmp/result.json"),
            &run_dir,
            None,
            false,
        )
        .expect("script should build");

        assert!(script.contains("--system-prompt-file"));
        assert!(script.contains(&prompt_file.display().to_string()));
    }

    #[test]
    fn validate_env_keys_rejects_shared_env_key_with_dash() {
        let mut env_set = BTreeMap::new();
        env_set.insert("BAD-KEY".to_string(), "x".to_string());
        let err = validate_env_keys("[mcp.hive.claude.env_set]", &env_set)
            .expect_err("invalid env key should fail");
        assert!(err.message.contains("invalid env key"));
    }

    #[test]
    fn build_claude_exec_command_exports_shared_env_set_before_mode_env() {
        let temp = tempdir().expect("temp dir");
        let runtime = sample_runtime(&temp);
        let run_dir = temp.path().join("run");
        fs::create_dir_all(&run_dir).expect("run dir");
        let mut shared_env_set = BTreeMap::new();
        shared_env_set.insert(
            "ANTHROPIC_AUTH_TOKEN".to_string(),
            "shared-token".to_string(),
        );
        let mut mode_env = BTreeMap::new();
        mode_env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), "mode-token".to_string());
        let cfg = McpHiveClaudeModeConfig {
            command: Some("claude".to_string()),
            env: mode_env,
            ..Default::default()
        };
        let script = build_claude_exec_command(
            &runtime,
            &shared_env_set,
            &cfg,
            Path::new("/repo"),
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/output.log"),
            Path::new("/tmp/result.json"),
            &run_dir,
            None,
            false,
        )
        .expect("script should build");

        let shared_pos = script
            .find("export ANTHROPIC_AUTH_TOKEN='shared-token'")
            .expect("shared export should exist");
        let mode_pos = script
            .rfind("export ANTHROPIC_AUTH_TOKEN='mode-token'")
            .expect("mode export should exist");
        assert!(shared_pos < mode_pos);
    }

    #[test]
    fn expand_home_like_expands_tilde_prefix() {
        let expanded = expand_home_like("~/test-path").expect("expand home");
        assert!(expanded.is_absolute());
        assert!(expanded.to_string_lossy().contains("test-path"));
    }

    #[test]
    fn parse_provider_output_extracts_json_session_and_keys() {
        let temp = tempdir().expect("temp dir");
        let output_path = temp.path().join("output.json");
        fs::write(
            &output_path,
            serde_json::json!({
                "session_id": "sess-1",
                "summary": "ok",
                "other": 1
            })
            .to_string(),
        )
        .expect("write output");

        let output = parse_provider_output("claude", &output_path.display().to_string());
        assert_eq!(output.format, HiveProviderOutputFormat::Json);
        assert_eq!(output.session_id.as_deref(), Some("sess-1"));
        assert_eq!(output.summary.as_deref(), Some("ok"));
        assert!(output.json_keys.contains(&"session_id".to_string()));
        assert!(output.parse_error.is_none());
    }

    #[test]
    fn parse_provider_output_falls_back_to_text_summary() {
        let temp = tempdir().expect("temp dir");
        let output_path = temp.path().join("output.log");
        fs::write(&output_path, "working...\nstep 2\n").expect("write output");

        let output = parse_provider_output("claude", &output_path.display().to_string());
        assert_eq!(output.format, HiveProviderOutputFormat::Text);
        assert_eq!(output.session_id, None);
        assert_eq!(output.summary.as_deref(), Some("working... step 2"));
        assert!(output.parse_error.is_some());
    }

    #[tokio::test]
    async fn run_process_state_marks_identity_mismatch_when_pid_command_differs() {
        let run = HiveRunState {
            run_id: "run-1".to_string(),
            prompt_preview: "hello".to_string(),
            provider: "claude".to_string(),
            output_path: "/tmp/output".to_string(),
            prompt_path: "/tmp/prompt".to_string(),
            result_path: "/tmp/result".to_string(),
            launcher_path: "/definitely/not/the/current/process/launcher.sh".to_string(),
            process_pid: Some(std::process::id()),
            resume_requested: false,
            provider_session_id: None,
            started_at_ms: 1,
            finished_at_ms: None,
            exit_code: None,
            termination_reason: None,
            provider_output: None,
        };

        let state = run_process_state(&run)
            .await
            .expect("state probe should succeed");
        assert_eq!(state, RunProcessState::IdentityMismatch);
    }

    #[tokio::test]
    async fn run_process_state_prefers_persisted_result_over_pid_identity() {
        let temp = tempdir().expect("temp dir");
        let result_path = temp.path().join("result.json");
        fs::write(
            &result_path,
            r#"{"provider":"claude","exit_code":0,"started_at_ms":1,"finished_at_ms":2}"#,
        )
        .expect("write result");

        let run = HiveRunState {
            run_id: "run-1".to_string(),
            prompt_preview: "hello".to_string(),
            provider: "claude".to_string(),
            output_path: temp.path().join("output.log").display().to_string(),
            prompt_path: temp.path().join("prompt.txt").display().to_string(),
            result_path: result_path.display().to_string(),
            launcher_path: "/definitely/not/the/current/process/launcher.sh".to_string(),
            process_pid: Some(std::process::id()),
            resume_requested: false,
            provider_session_id: None,
            started_at_ms: 1,
            finished_at_ms: None,
            exit_code: None,
            termination_reason: None,
            provider_output: None,
        };

        let state = run_process_state(&run)
            .await
            .expect("state probe should succeed");
        assert_eq!(state, RunProcessState::FinishedOrMissing);
    }

    #[tokio::test]
    async fn run_process_state_ignores_malformed_persisted_result() {
        let temp = tempdir().expect("temp dir");
        let result_path = temp.path().join("result.json");
        fs::write(&result_path, "{not-json").expect("write malformed result");

        let run = HiveRunState {
            run_id: "run-1".to_string(),
            prompt_preview: "hello".to_string(),
            provider: "claude".to_string(),
            output_path: temp.path().join("output.log").display().to_string(),
            prompt_path: temp.path().join("prompt.txt").display().to_string(),
            result_path: result_path.display().to_string(),
            launcher_path: "/definitely/not/the/current/process/launcher.sh".to_string(),
            process_pid: Some(std::process::id()),
            resume_requested: false,
            provider_session_id: None,
            started_at_ms: 1,
            finished_at_ms: None,
            exit_code: None,
            termination_reason: None,
            provider_output: None,
        };

        let state = run_process_state(&run)
            .await
            .expect("state probe should succeed");
        assert_eq!(state, RunProcessState::IdentityMismatch);
    }

    #[test]
    fn command_line_has_launcher_suffix_handles_spaces_without_substring_match() {
        assert!(command_line_has_launcher_suffix(
            "bash /tmp/hive run/launcher.sh",
            "/tmp/hive run/launcher.sh"
        ));
        assert!(!command_line_has_launcher_suffix(
            "bash /tmp/hive run/launcher.sh.old",
            "/tmp/hive run/launcher.sh"
        ));
    }

    #[test]
    fn login_shell_command_args_use_login_mode_for_bash_like_shells() {
        let args = login_shell_command_args(
            "/bin/bash",
            Path::new("/tmp/hive run/launcher.sh"),
            "run-123",
        );
        assert_eq!(args[0], "-lc");
        assert!(args[1].contains("exec bash"));
        assert!(args[1].contains("/tmp/hive run/launcher.sh"));
        assert!(args[1].contains("--agpod-hive-run=run-123"));
    }

    #[test]
    fn login_shell_command_args_use_login_mode_for_fish() {
        let args = login_shell_command_args(
            "/opt/homebrew/bin/fish",
            Path::new("/tmp/hive run/launcher.sh"),
            "run-456",
        );
        assert_eq!(args[0], "-l");
        assert_eq!(args[1], "-c");
        assert!(args[2].contains("exec bash"));
        assert!(args[2].contains("/tmp/hive run/launcher.sh"));
        assert!(args[2].contains("--agpod-hive-run=run-456"));
    }

    #[test]
    fn derive_session_id_is_cross_toolchain_stable() {
        let id = derive_session_id(Path::new("/tmp/project-a"));
        assert_eq!(id, "hive-repo-c5e2c6af");
    }

    #[test]
    fn derive_session_id_differs_for_different_repos() {
        let a = derive_session_id(Path::new("/tmp/project-a"));
        let b = derive_session_id(Path::new("/tmp/project-b"));
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_default_session_id_prefers_existing_legacy_state() {
        let temp = tempdir().expect("temp dir");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).expect("state dir");
        let legacy_session = "hive-repo-legacy1234";
        fs::write(
            state_dir.join(format!("{legacy_session}.json")),
            serde_json::json!({
                "version": 2,
                "session_id": legacy_session,
                "session_name": "agpod-hive-repo-legacy1234",
                "repo_root": "/tmp/project-a",
                "agent_limit": 5,
                "updated_at_ms": 10,
                "agents": []
            })
            .to_string(),
        )
        .expect("write legacy session");

        let resolved = resolve_default_session_id(Path::new("/tmp/project-a"), &state_dir);
        assert_eq!(resolved, legacy_session);
    }

    #[test]
    fn resolve_default_session_id_ignores_ambiguous_legacy_defaults() {
        let temp = tempdir().expect("temp dir");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).expect("state dir");
        for session_id in ["hive-repo-legacy0001", "hive-repo-legacy0002"] {
            fs::write(
                state_dir.join(format!("{session_id}.json")),
                serde_json::json!({
                    "version": 2,
                    "session_id": session_id,
                    "session_name": session_id,
                    "repo_root": "/tmp/project-a",
                    "agent_limit": 5,
                    "updated_at_ms": 10,
                    "agents": []
                })
                .to_string(),
            )
            .expect("write ambiguous session");
        }

        let resolved = resolve_default_session_id(Path::new("/tmp/project-a"), &state_dir);
        assert_eq!(resolved, "hive-repo-c5e2c6af");
    }
}
