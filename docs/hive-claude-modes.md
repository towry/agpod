# Hive Claude Modes

`agpod-mcp` 之 `hive`，今为本地子进程式 Claude exec worker。

## 概要

- 对外仅二 mode：`readonly`、`full`
- agent 若不知本机配置，可先调 `hive(action="mode_info")`
- `run_hive_agent` 一步建或复用 worker，并起本地 child process
- 缺省 `async=true`（推荐）；即返后可用 `wait_agent(agent_id=..., timeout_ms=...)` 阻塞等候，或以 `list_agents(agent_id=...)` 取快照
- `wait_agent` 缺省等待上限 `timeout_ms=30000`；超时则返“仍运行”，便于 caller 继续轮询
- 达 live limit 且未指 `agent_id` 时，不自动复用；直接报可执行建议（显式复用某 `agent_id`，或 `close_agent` / `close_session`）
- 若只想腾挪名额，不伤运行中 worker，可用 `clear_idle_agents`；此动作仅清空 idle worker
- 任务既毕宜及时 `close_agent`，免 live agent 积累后触发 limit
- 复用不等于续聊：`resume=false`（缺省）即新上下文；仅确需沿用既有对话时设 `resume=true`
- `resume` 由 caller 明定；缺已存 Claude session id 而强求 `resume=true`，径失败
- `settings`、`mcp_config` 若以 `~` 起首，运行时自动展为家目录

## 配置位置

全局：

```toml
# ~/.config/agpod/config.toml
[mcp.hive.claude.env_set]
ANTHROPIC_BASE_URL = "https://example.invalid"
ANTHROPIC_AUTH_TOKEN = "token"

[mcp.hive.claude.modes.readonly]
description = "只读 Claude worker；适合查阅、总结、分析。"
command = "claude"
args = ["--dangerously-skip-permissions"]
settings = "~/.claude/settings.json"
mcp_config = "~/.claude/generated/mcp-readonly.json"
system_prompt_file = "~/.config/agpod/prompts/readonly.md"
env = { MAX_MCP_OUTPUT_TOKENS = "12000" }

[mcp.hive.claude.modes.full]
description = "全权限 Claude worker；适合实现与改码。"
command = "claude"
args = []
settings = "~/.claude/settings.json"
mcp_config = "~/.mcp.json"
system_prompt = "You are a full-access coding assistant."
env = {}
```

仓库局部：

```toml
# .agpod.toml
[mcp.hive.claude.modes.readonly]
description = "只读 Claude worker；适合查阅、总结、分析。"
command = "claude"
args = ["--dangerously-skip-permissions"]
settings = "~/.claude/settings.json"
mcp_config = "~/.claude/generated/mcp-readonly.json"
system_prompt_file = "~/.config/agpod/prompts/readonly.md"
```

## 字段

- `description`：mode 之用途；供 `mode_info` 直出
- `command`：Claude 启动命令；必填
- `args`：固定参数数组
- `settings`：Claude settings 文件
- `mcp_config`：该 mode 所用 MCP 配置
- `system_prompt`：内联 system prompt 文本；与 `system_prompt_file` 互斥
- `system_prompt_file`：system prompt 文件路径，支持 `~` 展开；与 `system_prompt` 互斥
- `env_set`：顶层共享环境变量；对子进程统一注入
- `env`：附加环境变量

环境合并次序：

- 先继承 `agpod-mcp` 父进程环境（含 `PATH`）
- 起 worker 时，先经用户登录 shell 启动，再 `exec bash launcher.sh`
- 再应用 `[mcp.hive.claude.env_set]`
- 末了应用 mode 内 `env`

## System Prompt 交付

system prompt 之交付由 provider 能力层抽象，非硬编码于 Claude：

- 若 provider 支持文本方式（如 Claude 之 `--system-prompt`），配置给内联文本则直传；配置给文件路径则读取文件内容后转文本参数
- 若 provider 仅支持文件方式，配置给文件路径则直传；配置给内联文本则落临时文件于 run dir 再传路径
- 若 provider 兼支持文本与文件，内联文本走文本参数，文件路径走文件参数
- 若 provider 不支持 system prompt，则忽略该配置，不报错
- `system_prompt` 与 `system_prompt_file` 同时配置属配置错误，验证阶段即 fail fast

## `mode_info`

`hive(action="mode_info")` 返回：

- 诸 public mode 之总览，不按入参筛滤
- 每一 mode 之 `configured`、`description`、`has_system_prompt`
- 支持之 mode 名与缺省行为
- 不回配置路径、环境变量名或其他敏感细节

## 生命周期

- `run_hive_agent`：建或复用 worker，写 `prompt.txt`，生成 `launcher.sh`，再起 child process
- `run_hive_agent` 传 `agent_id` 时即复用既存 worker；`mode`、`worker_name`、`workdir` 可省，若传则须与既存值一致
- `wait_agent`：对指定 `agent_id` 阻塞等待至完成或超时；适合异步后之有界等待
- `clear_idle_agents`：仅关闭 idle worker；不碰 running worker
- Claude 运行时，流式输出入 `output.log`
- 运行止后，`result.json` 记 `provider`、`exit_code`、起止时刻；会话 id 自 `output.log` 解析入统一封装
- `list_agents` 会依 pid 与 `result.json` 同步状态，并将所得 `provider_session_id` 回写为 agent 之 `conversation_session_id`
- `close_agent` / `close_session` 先发 `TERM`，短待后仍存者再 `KILL`
- 若 pid 尚存而进程指纹已不符，`hive` 不自动判死、不自动收尾，改报 `identity_mismatch`，待人工处置

## 输出文件

每次 `run_hive_agent` 皆写：

- `prompt.txt`
- `output.log`
- `result.json`
- `launcher.sh`

母 agent 可借 `list_agents(agent_id=...)` 读其路径与输出摘要，以察“正在做何事”“已运行至何处”。

`current_run` / `last_run` 现含：

- `provider`
- `process_pid`
- `resume_requested`
- `provider_session_id`
- `termination_reason`
- `provider_output`

`provider_output` 为内部统一封装，供后续接他家 agent/provider：

- `provider`：来源方，如 `claude`
- `format`：`json` / `text` / `unknown`
- `session_id`：自 provider 输出中抽得之会话 id
- `summary`：供母 agent 快读之摘要
- `json_keys`：若输出为 JSON object，则记其顶层键
- `parse_error`：解析失败缘由；不阻主状态机

## Resume 契约

- 默认 `resume=false`
- 若 `run_hive_agent` 指定 `resume=true`，`hive` 必取该 agent 先前保存之 `conversation_session_id`
- 若无已存会话 id，直接报错，不暗中新开
- 若 `resume=false`，即起新 Claude 会话
- 若传 `agent_id` 且又给 `mode`、`worker_name`、`workdir`，则仅当诸值与既存 worker 一致时方放行；不一致径报错，免 caller 误判已切换 worker 设定

## Wait 契约

- `wait_agent` 必填 `agent_id`
- `timeout_ms` 可省；省则取 `30000`
- 若等待期内完成，返 `state=completed`
- 若超时仍运行，返 `state=running` 并提示继续调用 `wait_agent`

## 边缘风险

- 若 Claude 自行再 fork 并脱离原 pid，`hive` 仅能管理已记录之进程；故 `close_*` 先 `TERM` 后 `KILL`，尽量收束遗留
- 若 Claude 未产出合法 JSON stdout，`result.json` 仍会写出退出码与时刻，但 `provider_session_id` 或为空
- 若外部手动杀死子进程，`list_agents` 会将该 run 以 `process_missing_without_result` 收尾
- 默认 repo session id 现取稳定哈希；若仅存唯一旧默认 state，`hive` 会续用旧 session id；若旧默认态多于一，则弃旧取新，免误接他会话
- 运行机须有 `python3`，因 `launcher.sh` 借之写毫秒时间；provider 输出之解析则在 Rust 侧收束
- 达 limit 时，宜先 `list_agents` 察 idle worker；能复用则复用，欲一键腾位则 `clear_idle_agents`，`close_session` 仅作重置全会话之末手
