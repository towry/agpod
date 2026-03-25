# Honcho v2 与 agpod case 集成摘记

本文据 Honcho 官方 v2 文档与索引页整理，供本仓后续索引与实现参照。

## 1. 研究范围

本轮重点查证：

- Honcho v2 顶层实体模型
- 与 case 语义检索最相关之 API 端点
- webhook / hook 接入点
- SDK 环境变量
- v2 与 v3 口径差异中，对本仓设计有影响者

## 2. Honcho v2 之核心模型

据官方概览与 API 文档，v2 之主干可概括为：

- `Workspace`：顶层隔离边界
- `Session`：某一会话 / 线程 / 任务之记忆容器
- `Peer`：会话参与方或被建模之实体
- `Message`：写入 Session 之原始消息
- `Webhook Endpoint`：事件回调出口

对 agpod case 而言，最自然之映射为：

- 一个仓库或一个 agpod 部署，对应一个 Honcho `Workspace`
- 一个 `case`，对应一个 Honcho `Session`
- agent / system / human，可映射为 `Peer`
- `entry` / `decision` / `redirect` 等事件，可映射为 `Message`

## 3. 已核之 v2 端点

### 3.1 Workspace

- `POST /v2/workspaces`
  - 页：`https://docs.honcho.dev/v2/api-reference/endpoint/workspaces/get-or-create-workspace`
  - 语义：Get or create workspace
  - 备注：文档明确言，若传 `workspace_id` query parameter，则须与 JWT 中之 `workspace_id` 相符；否则取 JWT 内之 `workspace_id`

- `POST /v2/workspaces/list`
  - 页：`https://docs.honcho.dev/v2/api-reference/endpoint/workspaces/get-all-workspaces`
  - 语义：列出 workspaces

- `POST /v2/workspaces/{workspace_id}/search`
  - 页：`https://docs.honcho.dev/v2/api-reference/endpoint/workspaces/search-workspace`
  - 语义：Search a Workspace
  - 用途：跨 session / 更广范围搜相关记忆

### 3.2 Session

- `POST /v2/workspaces/{workspace_id}/sessions`
  - 页：`https://docs.honcho.dev/v2/api-reference/endpoint/sessions/get-or-create-session`
  - 语义：Get or create session
  - 备注：若传 `session_id` query parameter，则校验该 session 属于该 workspace；否则由 JWT 做校验

- `POST /v2/workspaces/{workspace_id}/sessions/{session_id}/search`
  - 页：`https://docs.honcho.dev/v2/api-reference/endpoint/sessions/search-session`
  - 语义：Search a Session
  - 备注：文档摘要明确提及可用 `limit` 控返回数量

- `GET /v2/workspaces/{workspace_id}/sessions/{session_id}/context`
  - 页：`https://docs.honcho.dev/v2/api-reference/endpoint/sessions/get-session-context`
  - 语义：Produce a context object from the session
  - 备注：官方写明可给 `token limit`；若未给，则回尽量完整之上下文；其预算分配为约 `40% summary + 60% recent messages`

### 3.3 Message 写入

- `POST /v2/workspaces/{workspace_id}/sessions/{session_id}/messages`
  - 证据：`https://docs.honcho.dev/v2/api-reference/introduction` 索引页包含 `Create Messages For Session`，且含该 OpenAPI 路径
  - 语义：向 session 添加消息
  - 对本仓意义：此端点最适合作为 case event / entry 同步入口

### 3.4 Webhook

- `POST /v2/workspaces/{workspace_id}/webhooks`
  - 页：`https://docs.honcho.dev/v2/api-reference/endpoint/webhooks/get-or-create-webhook-endpoint`
  - 语义：Get or create a webhook endpoint URL
  - 对本仓意义：可反向接 Honcho 事件，作 digest 完成、索引就绪、异步处理回执

## 4. SDK 与配置

据 SDK 文档页 `https://docs.honcho.dev/v2/documentation/reference/sdk`，可确认环境变量：

- `HONCHO_API_KEY`
- `HONCHO_BASE_URL`
- `HONCHO_WORKSPACE_ID`

文档亦明确其提供 Python 与 TypeScript SDK。

## 5. 对 agpod case 最有用之 Honcho 能力

若以“case 语义搜索”而论，最有价值者有三：

1. `messages` 写入
   - 把 case entry / redirect / decision 同步进去
2. `session search`
   - 面向单一 case 作自然语言检索
3. `session context`
   - 直接产出可给 LLM / agent 使用之上下文对象

其次为：

4. `workspace search`
   - 跨 case 找相似案例
5. `webhooks`
   - 做异步 digest / 状态回调

## 6. 与本仓现状之对照

本仓现已具备：

- `CaseConfig.semantic_recall_enabled`
- `CaseConfig.vector_digest_job_enabled`
- `commands.rs` 中 `case_recall` 之注记：后续以 `CaseSearchIndex` 接向量搜索

然截至当前代码：

- 尚无 `CaseSearchIndex` 实现
- 尚无 Honcho client / adapter
- 尚无 entry 写后 hook / plugin dispatch
- `case_recall` 仍只是本地加权文本匹配

## 7. 推荐映射

### 7.1 Identity 映射

建议默认：

- `workspace_id = <repo_id>` 或配置显式指定
- `session_id = <case_id>`
- `peer_id = agpod-system`、`agent-<name>`、`user-<id>` 等

### 7.2 Message 映射

建议将以下事件序列化为 Honcho messages：

- `record`
- `decision`
- `redirect`
- `step started / done / blocked`
- `case reopened / closed / abandoned`

每条消息宜带 metadata：

- `case_id`
- `repo_id`
- `direction_seq`
- `entry_seq` 或 `step_id`
- `entry_type`
- `kind`
- `created_at`
- `files`
- `artifacts`

## 8. v2 / v3 迁移注意

今轮目标是接 v2；但官方索引亦可见 v3 已有更丰富端点，如：

- `Search Workspace`
- `Search Session`
- `Get Session Context`
- `Query Conclusions`

可得两点：

- v2 足够支撑本仓第一阶段“写入消息 + 搜 session + 拉 context”
- 若将来要更强之结论层 / peer reasoning，可再评是否升 v3

故建议：

- 第一版适配层以 `HonchoBackend` trait 抽象
- v2 作为首个实现
- API path、请求体、返回体勿散落业务层，皆封于 adapter 内

## 9. 结论

对 agpod case 而言，最短闭环不是直接造自有向量库，而是：

- 保持 case 为 canonical truth
- 把合格 event 同步到 Honcho `Session`
- 用 Honcho `session search` 与 `session context` 回补 agent 取上下文之能力
- 用 webhook / 异步任务完善 digest 与状态回流

此路与本仓现有 `semantic_recall_enabled` / `vector_digest_job_enabled` 旗标方向一致，但当前尚未实现。
