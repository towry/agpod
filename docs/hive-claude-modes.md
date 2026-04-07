# Hive Claude Modes

`agpod-mcp` 之 `hive` 工具，今仅支 Claude exec worker。

## 概要

- 对外仅二 mode：`readonly`、`full`
- agent 若不知本机配置，可先调 `hive(action="mode_info")`
- `spawn_agent` 与 `send_prompt` 若所需 mode 未配，径失败，不猜默认值
- `spawn_agent` 只注册 worker，不预建 tmux session/window
- `send_prompt` 方懒创建或复用 pane 派生之 tmux session，并起实窗运行
- mode 中 `settings`、`mcp_config` 若以 `~` 起首，运行时自动展为家目录

## 配置位置

全局：

```toml
# ~/.config/agpod/config.toml
[mcp.hive.claude.modes.readonly]
description = "只读 Claude worker；适合查阅、总结、分析。"
command = "claw"
args = ["--dangerously-skip-permissions"]
settings = "~/.claude/settings.json"
mcp_config = "~/.claude/generated/mcp-readonly.json"
env = { MAX_MCP_OUTPUT_TOKENS = "12000" }

[mcp.hive.claude.modes.full]
description = "全权限 Claude worker；适合实现与改码。"
command = "claw"
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
command = "claw"
args = ["--dangerously-skip-permissions"]
settings = "~/.claude/settings.json"
mcp_config = "~/.claude/generated/mcp-readonly.json"
```

## 字段

- `command`：Claude 启动命令；必填
- `description`：此 mode 之用途说明；建议填写，供 `mode_info` 直出
- `args`：固定参数数组
- `settings`：Claude settings 文件
- `mcp_config`：该 mode 所用 MCP 配置
- `env`：附加环境变量

## `mode_info`

`hive(action="mode_info")` 返回：

- 支持之 mode 名
- 所选 mode 当前是否已配置
- 所需配置节路径
- 最小示例
- 已配置字段摘要
- mode 描述

若传 `mode="full"`，则仅查看 `full`。

## 输出文件

每次 `send_prompt` 皆写：

- `prompt.txt`
- `output.log`
- `result.json`

母 agent 可借 `list_agents` 读其路径与输出摘要，以察“正在做何事”“已运行至何处”。

`list_agents` 中之 `current_run` / `last_run`，今另含：

- `window_name`：本次 run 之 tmux window 名
- `termination_reason`：若 run 非正常写出 `result.json` 而结束，则记终止缘由；如 `killed_by_hive`、`launch_failed`

`send_prompt` 今先落盘 run 状态，再起 tmux window；若起窗失败，run 会转入 `last_run`，并写 `termination_reason="launch_failed"`。
