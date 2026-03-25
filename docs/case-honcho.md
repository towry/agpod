# Case Honcho

本文面向项目开发者，概述 `agpod case` 与 Honcho 集成之现状、映射与配置边界。

## 当前状态

- `agpod case` 可将事件同步至 Honcho
- 文件配置支持 `[case.plugins.honcho]`
- 环境变量仍可覆盖文件配置
- `peer_id` 仍存在实现层，但不列入用户文档
- 关键 Honcho 路径现会写入日志文件，便于诊断“不工作”问题

## 配置映射

用户可见配置：

```toml
[case.plugins.honcho]
enabled = true
sync_enabled = true
base_url = "https://api.honcho.dev"
workspace_id = "ws_123"
api_key = "honcho_secret"
api_key_env = "HONCHO_API_KEY"
```

实现层内部尚有：

- `peer_id`

其今仅作 Honcho message 之发送方标识，默认由实现管理。

`api_key` 与 `api_key_env` 可并存；同层配置中 `api_key` 优先。若环境变量层设 `AGPOD_CASE_HONCHO_API_KEY` 或 `AGPOD_CASE_HONCHO_API_KEY_ENV`，则仍依总优先级覆盖文件配置。

## 实体映射

- `repo` → Honcho `workspace`
- `case` → Honcho `session`
- `case event` → Honcho `message`

## 代码位置

- `crates/agpod-case/src/honcho.rs`
- `crates/agpod-case/src/config.rs`
- `crates/agpod-case/src/events.rs`
- `~/Library/Application Support/agpod/logs/agpod.log`（macOS 常见位置）

## 参考

稳定参考：

- `crates/agpod-case/src/honcho.rs`
- `crates/agpod-case/src/config.rs`
- `crates/agpod-case/src/events.rs`
- `https://docs.honcho.dev/v2/api-reference/introduction`
