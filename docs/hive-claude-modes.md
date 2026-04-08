# Hive Claude Modes

`agpod-mcp` 之 `hive`，今为本地子进程式 Claude exec worker。

## 概要

- 对外仅二 mode：`readonly`、`full`
- agent 若不知本机配置，可先调 `hive(action="mode_info")`
- `spawn_agent` 仅注册 worker，不预起进程
- `send_prompt` 始起一条本地 child process
- `resume` 由 caller 明定；缺已存 Claude session id 而强求 `resume=true`，径失败
- `settings`、`mcp_config` 若以 `~` 起首，运行时自动展为家目录

## 配置位置

全局：

```toml
# ~/.config/agpod/config.toml
[mcp.hive.claude.modes.readonly]
description = "只读 Claude worker；适合查阅、总结、分析。"
command = "claude"
args = ["--dangerously-skip-permissions"]
settings = "~/.claude/settings.json"
mcp_config = "~/.claude/generated/mcp-readonly.json"
env = { MAX_MCP_OUTPUT_TOKENS = "12000" }

[mcp.hive.claude.modes.full]
description = "全权限 Claude worker；适合实现与改码。"
command = "claude"
args = []
settings = "~/.claude/settings.json"
mcp_config = "~/.mcp.json"
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
```

## 字段

- `description`：mode 之用途；供 `mode_info` 直出
- `command`：Claude 启动命令；必填
- `args`：固定参数数组
- `settings`：Claude settings 文件
- `mcp_config`：该 mode 所用 MCP 配置
- `env`：附加环境变量

## `mode_info`

`hive(action="mode_info")` 返回：

- 支持之 mode 名
- 所选 mode 是否已配
- 所需配置节路径
- 已配置字段摘要
- mode 描述
- 最小示例

## `probe_mode`

`hive(action="probe_mode")` 不起长任务，只回：

- 所选 mode 将使用之命令、参数、配置路径、环境键
- prompt 预览
- 期望之 `result.json` 字段
- 期望之 `provider_output` 字段
- 缺失输出时之解析探针样例

## 生命周期

- `spawn_agent`：建 worker profile，尚无进程
- `send_prompt`：写 `prompt.txt`，生成 `launcher.sh`，再起 child process
- Claude 运行时，流式输出入 `output.log`
- 运行止后，`result.json` 记 `provider`、`exit_code`、起止时刻；会话 id 自 `output.log` 解析入统一封装
- `list_agents` 会依 pid 与 `result.json` 同步状态，并将所得 `provider_session_id` 回写为 agent 之 `conversation_session_id`
- `close_agent` / `close_session` 先发 `TERM`，短待后仍存者再 `KILL`

## 输出文件

每次 `send_prompt` 皆写：

- `prompt.txt`
- `output.log`
- `result.json`
- `launcher.sh`

母 agent 可借 `list_agents` 读其路径与输出摘要，以察“正在做何事”“已运行至何处”。

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
- 若 `resume=true`，`hive` 必取该 agent 先前保存之 `conversation_session_id`
- 若无已存会话 id，直接报错，不暗中新开
- 若 `resume=false`，即起新 Claude 会话

## 边缘风险

- 若 Claude 自行再 fork 并脱离原 pid，`hive` 仅能管理已记录之进程；故 `close_*` 先 `TERM` 后 `KILL`，尽量收束遗留
- 若 Claude 未产出合法 JSON stdout，`result.json` 仍会写出退出码与时刻，但 `provider_session_id` 或为空
- 若外部手动杀死子进程，`list_agents` 会将该 run 以 `process_missing_without_result` 收尾
- 运行机须有 `python3`，因 `launcher.sh` 借之写毫秒时间；provider 输出之解析则在 Rust 侧收束
