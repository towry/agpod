# Case 外部 Hooks / Stdio 插件协议

本文定义 `case` 体系之后续扩展协议：允许用户**不修改本仓代码**，仅以配置声明外部命令，由 `stdin/stdout` 交互 JSON，以消费 case events，或提供 search/context 能力。

**本期只定协议，不实现代码。**

## 1. 目标

此协议欲解三事：

1. 用户不想用 `Honcho`，亦不想 fork 本仓代码
2. 用户想把合格事件同步到自家 semantic / vector / audit 系统
3. 用户想以外部进程方式实现 `search` / `context` provider

故本文之中心不是某个厂商 API，而是：

- 配置如何写
- 事件如何送
- 外部进程如何回
- 失败如何处置
- 安全边界如何守

## 2. 适用范围

本文拟定两类扩展：

### 2.1 Event Hook

于 case 写操作成功后触发，面向：

- semantic sync
- webhook relay
- 审计日志
- 通知
- 外部索引

### 2.2 External Provider

于读取路径触发，面向：

- `search`
- `context`

此类外部 provider 可完全替代 `Honcho`、或与本地 provider 并存。

## 3. 设计原则

### 3.1 Canonical truth 不外移

外部命令不得决定：

- case 状态
- direction 真相
- step 真相
- entry 真相

其只能：

- 消费事件
- 产出索引
- 回答查询
- 返回辅助信息

### 3.2 协议重于语言

扩展协议应与语言无关。用户可用：

- Python
- Node.js
- Go
- Rust
- Shell wrapper（不推荐为主）

只要能：

- 读 `stdin`
- 写 `stdout`
- 返回单个 JSON

### 3.3 勿绑死 `bash -c`

配置层宜以 **argv 数组** 表示命令：

- 善：`["my-case-plugin", "event-sync"]`
- 劣：`["bash", "-lc", "..."]`

后者仅作兼容通道，不作主路径。

### 3.4 Fail-fast + bounded execution

外部进程必须受控：

- 有超时
- 有最大输出尺寸
- 有退出码判定
- 有明确失败语义

## 4. 配置模型

建议于 `case` 配置下新增：

```toml
[case.hooks.semantic_sync]
enabled = true
events = ["record_appended", "decision_appended", "redirect_committed"]
command = ["my-case-plugin", "event-sync"]
timeout_ms = 5000
failure_mode = "warn"

[case.providers.context]
kind = "external_command"
command = ["my-case-plugin", "context"]
timeout_ms = 8000

[case.providers.search]
kind = "external_command"
command = ["my-case-plugin", "search"]
timeout_ms = 8000
```

## 5. Event Hook 配置语义

### `enabled`

- 是否启用此 hook

### `events`

- 感兴趣之事件类型列表
- 空则视为订阅全部，或于实现中禁止为空；此点可择一，但须文档定死
- 推荐首版：**必须非空**，免误订阅噪声

### `command`

- argv 数组
- 第一项为可执行文件
- 后续项为固定参数

### `timeout_ms`

- 单次调用最长时限
- 超时即失败

### `failure_mode`

推荐仅两值：

- `warn`
- `fail`

首版建议：

- event hooks 默认只允 `warn`
- 未来若真有强一致外部系统，再开放 `fail`

## 6. Event Hook 调用时机

调用顺序当为：

1. 本地命令写 DB 成功
2. 生成 `CaseEventEnvelope`
3. dispatcher 逐个调用 hooks
4. 收集结果入 `CaseDispatchReport`
5. 若 `warn` 模式失败，仅回 warnings

此与今 `Honcho` sync 路径之设计一致。

## 7. Event Hook Stdio 输入协议

每次调用，只向子进程 `stdin` 写入**一个 JSON 对象**。

推荐载荷如下：

```json
{
  "version": 1,
  "kind": "case_event",
  "event_id": "C-1:record_appended:3:2026-03-25T00:00:00Z",
  "event_type": "record_appended",
  "occurred_at": "2026-03-25T00:00:00Z",
  "case_id": "C-1",
  "repo_id": "repo-1",
  "repo_label": "github.com/example/repo",
  "worktree_id": "wt-1",
  "worktree_root": "/repo",
  "direction_seq": 1,
  "payload": {
    "case_id": "C-1",
    "direction_seq": 1,
    "entry_seq": 3,
    "entry_type": "record",
    "kind": "note",
    "summary": "captured a note"
  }
}
```

### 字段说明

- `version`
  - 协议版本
  - 首版固定 `1`
- `kind`
  - 固定为 `case_event`
- `event_id`
  - 幂等键候选
- `event_type`
  - 如 `record_appended`、`decision_appended`
- `payload`
  - 直接来自 `CaseDomainEvent::metadata()`，或其稳定投影

## 8. Event 类型与 payload 约定

推荐首版暴露：

- `case_opened`
- `case_reopened`
- `record_appended`
- `decision_appended`
- `redirect_committed`
- `step_started`
- `step_done`
- `step_blocked`
- `case_closed`
- `case_abandoned`

### `record_appended`

其 `payload` 应至少含：

- `case_id`
- `direction_seq`
- `entry_seq`
- `entry_type = record`
- `kind = note | finding | evidence | blocker | goal_constraint_update`

此处 `kind` 即是区分 `note`、`evidence`、`blocker` 之主键。

### `decision_appended`

其 `payload` 应至少含：

- `case_id`
- `direction_seq`
- `entry_seq`
- `entry_type = decision`

### `redirect_committed`

其 `payload` 应至少含：

- `case_id`
- `entry_seq`
- `from_direction_seq`
- `to_direction_seq`

### step state 事件

其 `payload` 应至少含：

- `case_id`
- `direction_seq`
- `step_id`
- `step_status`

## 9. Event Hook 输出协议

外部命令当只向 `stdout` 输出**一个 JSON 对象**。

### 成功

```json
{
  "ok": true,
  "message": "synced",
  "artifacts": {
    "external_id": "abc123"
  }
}
```

### 失败

```json
{
  "ok": false,
  "error": "upstream timeout"
}
```

### 输出语义

- `ok=true`
  - 视为成功
- `ok=false`
  - 视为业务失败
- 非 JSON / 空输出 / 超时 / 非零退出码
  - 视为协议失败

### `stderr`

- 仅作调试日志
- 不作协议载荷真相

## 10. Error mapping

建议本仓内部将失败分四类：

1. spawn failed
2. timeout
3. protocol invalid
4. plugin reported error

对用户可渲染为：

- `hook 'semantic_sync' failed: timeout after 5000ms`
- `hook 'semantic_sync' failed: invalid JSON response`
- `hook 'semantic_sync' failed: upstream timeout`

## 11. Context Provider 协议

若用户要以外部命令提供 `.context(query)`，则可定义第二类 `kind`：

```json
{
  "version": 1,
  "kind": "case_context_request",
  "case_id": "C-1",
  "query": "token limit",
  "limit": 5,
  "token_limit": 512,
  "repo_id": "repo-1",
  "repo_label": "github.com/example/repo"
}
```

### 成功输出

```json
{
  "ok": true,
  "backend": "external_command",
  "context": "...",
  "hits": [
    {
      "source": "plugin",
      "field": "content",
      "excerpt": "...",
      "score": 42,
      "direction_seq": 1,
      "entry_seq": 3,
      "step_id": null,
      "kind": "note"
    }
  ],
  "truncated": false,
  "generated_at": "2026-03-25T00:00:00Z"
}
```

### 失败输出

```json
{
  "ok": false,
  "error": "case not indexed yet"
}
```

## 12. Search Provider 协议

若用户要提供 `search`，则可定义：

```json
{
  "version": 1,
  "kind": "case_search_request",
  "case_id": "C-1",
  "query": "semantic memory",
  "limit": 10,
  "repo_id": "repo-1"
}
```

### 成功输出

```json
{
  "ok": true,
  "backend": "external_command",
  "hits": [
    {
      "source": "plugin",
      "field": "content",
      "excerpt": "...",
      "score": 91,
      "direction_seq": 1,
      "entry_seq": 2,
      "step_id": null,
      "kind": "decision"
    }
  ]
}
```

## 13. 运行约束

实现时建议强制：

- 一次调用只收一个 JSON 请求
- 一次调用只认一个 JSON 响应
- 设最大 `stdout` 尺寸，防插件刷爆内存
- 默认清洁环境，不把敏感 env 全量传下去
- 工作目录固定为 repo root，或显式配置

## 14. 安全边界

### 14.1 环境变量

不得默认把所有父进程环境透传给插件。

建议：

- 默认只透传白名单 env
- 由配置显式加 `env_allowlist`

### 14.2 Shell injection

不得要求用户写单字符串 shell 命令作为主配置格式。

必须优先：

- `command = ["bin", "arg1", "arg2"]`

### 14.3 资源控制

建议后续实现：

- timeout
- output size cap
- 并发数上限
- 每 hook 独立统计

## 15. 幂等与重放

外部 hook 若用于索引或同步，必须假设事件可重放。

故插件应以：

- `event_id`

作为幂等键，或自行 dedupe。

## 16. 推荐落地顺序

本期不实现，后续若做，建议三步：

### 第一步：Event Hook

只做：

- `command` hook
- `stdin` 单 JSON 输入
- `stdout` 单 JSON 输出
- `failure_mode = warn`

此已足够让用户把事件同步到自家系统。

### 第二步：Context Provider

再做：

- `external_command` context provider
- 统一 `CaseContextResult` 映射

### 第三步：Search Provider

最后做：

- `external_command` search provider
- 与本地 / Honcho provider 并列路由

## 17. 非目标

本期明确不做：

- 代码实现
- 动态加载 `.so/.dylib/.wasm`
- 跨进程流式双向会话协议
- 让外部插件改写 case canonical state

## 18. 与既有文档之关系

本文补足：

- `.agents/docs/case-hook-plugin-honcho-spec.md`
- `.agents/docs/case-honcho-integration-plan.md`

前两文定“为何要抽象、如何分层”；本文定“若走 external command / stdio，应如何实施”。

## 19. 结语

若要真支撑“默认可带 `Honcho`，而用户亦可无侵入地接自家实现”，则仅有 feature 尚不足；必须再有：

- 稳定 hooks 配置
- 稳定 stdio 协议
- 稳定 provider contract

此三者具，则 `Honcho` 只是默认 adapter，不再是唯一通路。
