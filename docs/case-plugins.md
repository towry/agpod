# Case Plugins

本文面向项目开发者，整理 `case` 之 hooks / plugins / external provider 方向。

## 目标

- 不强制用户使用 Honcho
- 允许未来接入自家 semantic / vector / audit 系统
- 保持 `case` 本地数据库为 canonical truth

## 当前原则

- `case` 真相仅在本地
- 外部系统只消费事件或回答查询
- `Honcho` 只是首个 adapter
- 用户向配置以 `[case.plugins.<name>]` 为宜

## 推荐分层

- `Case Core`
  - events
  - hooks
  - search/context traits
- `Provider Adapter`
  - honcho
  - external command
  - future custom providers

## 当前已落地

- `CaseEventSink`
- `CaseContextProvider`
- `CaseSearchBackend`
- `HonchoBackend`

## 后续方向

- 外部 stdio hook / provider
- 更清晰之 plugin runtime
- 按事件来源派生内部 `peer_id`

## 参考

稳定参考：

- `crates/agpod-case/src/hooks.rs`
- `crates/agpod-case/src/search.rs`
- `crates/agpod-case/src/context.rs`
- `crates/agpod-case/src/events.rs`
