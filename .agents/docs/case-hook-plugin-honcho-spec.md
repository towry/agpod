# Case Hook / Plugin / Honcho 扩展规范

本文承接既有 `case` 架构与 Honcho v2 集成实现，补足后续继续演进所需之实现规范。其意不在重述“可做什么”，而在明言“当如何做、何处做、何时做”。

## 1. 目标

此规范欲定五事：

1. `case` 领域事件何者为 canonical sync input
2. hook / plugin 之运行边界与生命周期
3. `case` ↔ Honcho 之实体映射与消息形制
4. 同步失败、补偿、重放、回退之语义
5. 后续实现顺序，使架构可持续扩而不坏真相边界

## 2. 已有基础

今仓内已具：

- typed domain events：`crates/agpod-case/src/events.rs`
- hook dispatcher：`crates/agpod-case/src/hooks.rs`
- context provider abstraction：`crates/agpod-case/src/context.rs`
- search backend abstraction：`crates/agpod-case/src/search.rs`
- Honcho adapter：`crates/agpod-case/src/honcho.rs`
- command glue：`crates/agpod-case/src/commands.rs`

故后续不应另起一套旁路插件体系；当沿此 seams 继续扩。

并当再守一律：

- `Honcho` 只是首个 adapter
- 不得成为 `case` core 之强制依赖
- 用户可全然不用 `Honcho`
- 用户亦可接自家 semantic / vector backend

故实现层宜分为：

- `Case Core`：事件、hooks、search/context traits、plugin runtime
- `Provider Adapter`：`honcho` 或其他第三方 / 自定义后端

其中 `Honcho` 最宜由 cargo feature 控之，如 `honcho` 或 `semantic-honcho`。feature 关闭时：

- `case` 基本写读流程仍可编译、测试、运行
- 本地 `case_recall` 与 local `case_context` 仍可用
- 仅 `Honcho` adapter 与其远端依赖退出编译面

## 3. Canonical 边界

### 3.1 真相所在

`case` 之 canonical truth 仅在本地数据层：

- `case`
- `direction`
- `step`
- `entry`
- typed `CaseDomainEvent`

Honcho 非业务真相，仅为：

- semantic index
- session context producer
- cross-case / cross-entry retrieval aid

### 3.2 Sync input

对外同步之唯一合法输入，当为 `CaseEventEnvelope`，不可直接以“当前 case 全快照”作主写模型。理由有四：

- 可重放
- 可幂等
- 可审计
- 易做 selective sync

故新增插件亦应订阅 event，而非私读数据库状态后自行猜测变更。

## 4. Hook / Plugin 模型

## 4.1 分层

建议分三层：

1. **Sink layer**：进程内 trait object，即今之 `CaseEventSink`
2. **Plugin runtime layer**：按配置组装多个 sink，并处理策略、过滤、失败语义
3. **Adapter layer**：如 Honcho、webhook、stdout audit、future external command

今仓仅有第一层与部分 adapter；后续宜补第二层，而非直接把复杂逻辑塞回 `commands.rs`。

## 4.2 生命周期

每一 sink / plugin 当有如下生命周期：

1. `register`：由配置决定是否装入 dispatcher
2. `filter`：按事件类型 / repo / case 状态决定是否处理
3. `handle`：消费 `CaseEventEnvelope`
4. `report`：产出成功 / 失败状态，汇入 `CaseDispatchReport`
5. `replay`：可选；供后续补偿重放

今 trait 仅显式含 `is_enabled` 与 `handle`；后续可在不破坏既有接口前提下，于 runtime 层补：

- event filter config
- retry policy
- dead letter capture
- replay cursor

## 4.3 Provider independence

hook/plugin runtime 不应知 `Honcho` 细节，只应知抽象契约。故：

- runtime 只组装已启用之 sinks / search backends / context providers
- feature 关闭之 adapter 不应在 command path 留编译时硬引用
- provider 选择应由配置与编译能力共同决定

建议语义如下：

- 编译无 `honcho` feature：一切 `honcho_*` 配置仅视为未启用，不报错
- 编译有 `honcho` feature 而配置关闭：不装配 `HonchoBackend`
- 编译有 `honcho` feature 且配置开启：方可装配并校验配置

此可保留“用户不装 `Honcho` 亦可完整使用 case”之性质。

## 4.4 推荐新增结构

建议后续新增：

- `crates/agpod-case/src/plugin.rs`
- `crates/agpod-case/src/plugin_config.rs`
- `crates/agpod-case/src/replay.rs`

其责如下：

### `plugin.rs`

定义稳定插件描述：

- plugin name
- enabled flag
- interested event types
- failure mode
- delivery mode

建议模型：

- `CasePluginMode::Inline`
- `CasePluginMode::Async`
- `CasePluginFailureMode::Warn`
- `CasePluginFailureMode::FailCommand`

首版仅 Honcho sync 用 `Warn`，因 semantic backend 不可阻主写。

### `plugin_config.rs`

将零散配置收束为插件级配置，避免 `CaseConfig` 继续膨胀。可含：

- `plugins.honcho.sync.enabled`
- `plugins.honcho.sync.events`
- `plugins.honcho.context.enabled`
- `plugins.honcho.context.default_token_limit`
- `plugins.honcho.workspace.strategy`
- `plugins.honcho.peer.strategy`

### `replay.rs`

定义 event replay source。后续可由：

- `entry`
- `direction` snapshot
- `step` 状态

重建 `CaseDomainEvent` 序列，再喂各 sinks。

## 5. Event 分类与插件关注点

并非一切事件都应同等同步至 Honcho。建议分级：

### 5.1 必同步

- `CaseOpened`
- `CaseReopened`
- `RecordAppended`
- `DecisionAppended`
- `RedirectCommitted`
- `StepStarted`
- `StepDone`
- `StepBlocked`
- `CaseClosed`
- `CaseAbandoned`

其共同特点：语义价值高，足以改善检索与 context。

### 5.2 可同步

- `StepAdded`
- `StepsReordered`
- `RedirectRecovered`

此类更偏流程管理。默认可不同步，以免污染向量语料。

### 5.3 不建议同步原始噪声

若将来增更细粒度事件，如：

- heartbeat
- transient UI state
- internal retry note

则默认不可入 Honcho。

## 6. Honcho 映射规范

须先记：本节只定义 **`Honcho` adapter 契约**，非定义 `case` core 对外唯一形态。任一自定义 provider 只要遵同一抽象，即可替换之。

## 6.1 Workspace

推荐策略：

- **默认**：一 `repo_id` 对一 `workspace_id`
- **可配置**：多 repo 共用单 workspace，仅用于组织级检索场景

约束：

- workspace 负责隔离语义记忆域
- workspace 一旦选定，不宜对已存在 case 中途迁移

建议配置值：

- `repo_scoped`
- `explicit`

## 6.2 Session

推荐：一 `case_id` 对一 Honcho `session_id`。

理由：

- 与当前 `case_context` 直观对应
- 重放与补偿粒度清晰
- context budget 天然围绕当前 case

### Session metadata

每次 ensure session，metadata 至少含：

- `repo_id`
- `repo_label`
- `worktree_id`
- `worktree_root`
- `case_status`
- `goal`

其中 `goal` 值得补入，因其对 summary/context 质量有直接助益。

## 6.3 Peer

建议勿将 `peer_id` 固定为单值久之不变；应引入 peer strategy：

- `system`：case runtime / hook system message
- `agent:<name>`：agent 决策、记录
- `user`：人工输入

今 `HonchoBackend` 仅取单一 `honcho_peer_id`；此可作为首版默认，但后续应改为按事件来源派生。建议在 `CaseEventEnvelope` 增：

- `actor_kind`
- `actor_id`

若暂未能精确取得 actor，则默认：

- case lifecycle / step state → `system`
- record / decision / redirect → `agent:agpod`

## 6.4 Message

每个可同步事件映为一条 message。message 由两部构成：

1. `content`：面向语义检索之自然语言摘要
2. `metadata`：精确过滤、定位、重放所需结构化字段

### content 规范

摘要当：

- 明示 case id
- 明示动作
- 尽量带 goal / direction / step 语义
- 避免纯模板噪声
- 避免把大段原始 JSON 塞入 content

建议由 event 自己负责基础 `summary_text()`，另在 adapter 层按需要 enrich。勿在 `commands.rs` 拼 Honcho 文本。

### metadata 最低集合

- `event_id`
- `event_type`
- `repo_id`
- `repo_label`
- `case_id`
- `direction_seq`
- `occurred_at`
- `worktree_id`
- `worktree_root`
- `entry_seq`（若有）
- `step_id`（若有）
- `kind`（若有）
- `case_status`

### metadata 建议增补

- `goal`
- `direction_summary`
- `step_title`
- `entry_type`
- `sync_version`

其中 `sync_version` 甚要，可供未来 schema 演进与重放迁移。

## 7. 同步策略

## 7.1 Inline warn 为当前默认

今架构正确选择是：

- 命令先写本地 DB
- 成功后派发 event
- Honcho sync 若败，只写 `warnings/hooks`
- 主命令仍成功

此策略须保持，除非未来另加显式“强一致插件”类别。

## 7.2 Async 路线

待基础稳后，可加异步模式：

1. 本地写入 event
2. event 入本地 outbox
3. 背景 worker 送 Honcho
4. webhook 或轮询回写 sync status

此时应新增本地状态表或 entry kind：

- `sync_pending`
- `sync_succeeded`
- `sync_failed`

然此属第二阶段，不宜今即大铺实现。

## 7.3 Replay / Backfill

欲使插件可靠，必须可重放。建议：

- 以 `case_id` 为范围重放
- 以 `event_id` 为幂等键
- plugin adapter 自负责去重或 upsert 语义

Honcho 若不保证天然幂等，则本仓应至少保证：

- 重放可接受重复而不坏语义
- 或 metadata 中带 `event_id` 供后端 dedupe

## 8. `case_context` 与未来 `.context()`

## 8.1 当前语义

今 `case_context` 实为 `.context(query)` 之服务端原型：

- 有 Honcho 且开 semantic recall → 走 Honcho
- 否则 → 走 LocalCaseContextProvider

此决策妥当，宜保留。

## 8.2 后续建议

若要让 agent 真用 `.context("自然语言")`，建议稳定 contract 为：

- `backend`
- `query`
- `generated_at`
- `context`
- `hits`
- `truncated`
- `token_limit`

今尚缺 `truncated` 明示位；本地 context 仅做字符截断。后续宜显式回：

- 是否截断
- 估计 token 数
- 命中数与实际返回数

## 8.3 Workspace 级语义检索

当前 `case_context` 仅查 session。后续可增：

- `case similar --query ...`
- `case search-workspace --query ...`

然其返回须与 `case_context` 分离，勿混为一命令。

## 9. 配置演进建议

今 `CaseConfig` 已有若干 Honcho 字段。后续建议分群：

### 核心开关

- `honcho_enabled`
- `honcho_sync_enabled`
- `semantic_recall_enabled`

### 连接参数

- `honcho_base_url`
- `honcho_workspace_id`
- `honcho_api_key_env`

### 策略参数

- `honcho_peer_strategy`
- `honcho_workspace_strategy`
- `honcho_sync_mode`
- `honcho_sync_event_types`
- `honcho_context_default_token_limit`

### 编译特性

- `honcho`：启用 `HonchoBackend`、远端 HTTP client、相关配置校验
- 无此 feature 时：`CaseConfig` 可保留字段以兼容配置文件，但 runtime 只走 local provider

## 10. 自定义 Provider 契约

若用户欲接自家服务，而非 `Honcho`，建议最少实现三 trait：

- `CaseEventSink`
- `CaseSearchBackend`
- `CaseContextProvider`

对应职责：

### `CaseEventSink`

- 消费成功写操作后之 `CaseEventEnvelope`
- 负责增量同步
- 不得反向改写 case canonical state

### `CaseSearchBackend`

- 负责自然语言查 case 内命中
- 返回统一 `CaseContextHit`
- 可映射任意向量库、全文库或混合检索

### `CaseContextProvider`

- 面向 agent 产出 token-bounded context
- 可内部调用自家 search / summary / memory API
- 但返回 contract 当与 `case_context` 一致

### 最小接入原则

用户自定义 provider 当只需两步即可接入：

1. 实现上述 trait
2. 于 plugin/runtime 注册并由配置选择

不得要求用户改 `commands.rs` 业务流向，亦不宜让其 fork `case` core。

## 11. 代码落点建议

后续实现宜依次落于：

1. `crates/agpod-case/src/events.rs`
   - 增 actor / schema version / richer metadata helper
2. `crates/agpod-case/src/hooks.rs`
   - 增 runtime policy / plugin descriptor
3. `crates/agpod-case/src/honcho.rs`
   - 增 peer strategy、session metadata enrich、event filter
4. `crates/agpod-case/src/context.rs`
   - 增 truncation metadata、token estimate
5. `crates/agpod-case/src/config.rs`
   - 收束插件配置
6. `crates/agpod-case/src/commands.rs`
   - 保持胶水薄，仅负责 route，不纳 adapter 细节

## 12. 实现次序

推荐三期：

### 第一期：稳接口

- 固化 plugin descriptor / policy
- 增 `sync_version`
- 丰富 session metadata
- 增 `case_context.truncated`

### 第二期：稳同步

- local outbox
- replay/backfill command
- selective event sync config

### 第三期：扩生态

- external webhook sink
- command sink
- workspace-level semantic search
- actor-aware peer mapping

## 13. 非目标

今阶段不宜：

- 令 Honcho 决定 case 状态
- 以 Honcho 取代本地 recall 全部路径
- 强制用户使用 `Honcho`
- 未经抽象即把某厂商 API 烙进 command layer
- 引入动态加载 `.so/.dylib/.wasm` 插件系统
- 为第三方 API 行为写空泛测试

## 14. 结语

此后若继续实现，当守一条总律：

> `case` 管真相，event 管同步，plugin 管扩展，Honcho 管语义。

四者不可越位。若守此界，则将来加 webhook、outbox、workspace search、agent native `.context()`，皆可顺流而下。
