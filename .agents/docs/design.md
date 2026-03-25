# `case_redirect` 设计分层

本文改定一条原则：

- **内部存完整 transition**
- **写命令返回最小回执**
- **完整 transition 由专门读接口获取**

本文只论未来口径，不计现有兼容负担。

## 设计结论

`case_redirect` 的设计，须分三层：

- **存储模型**：系统内部持久化何种 canonical fact
- **写回执模型**：`case_redirect` 成功后返回什么
- **读模型**：UI、audit、replay、agent 如何读取完整 transition

本设计之最终结论如下：

1. `redirect` 在领域上是一条 **transition**
2. 该 `transition` 应被独立持久化，作为不可变事实
3. `case_redirect` 不应直接返回完整 `transition`
4. `case_redirect` 应返回 **command receipt + transition_ref + 少量摘要 + 当前状态 + next_action**
5. 完整 `transition` 必须通过专门读接口查询

换言之：

- **write model** 负责确认“写成了什么”
- **read model** 负责提供“完整历史事实”

## 核心原则

### 1. `transition` 是 canonical fact

`redirect` 不是简单地“切换当前 direction”，而是“发生一次从旧方向到新方向的跃迁”。

故系统内部必须存一条独立的 `transition` 记录，供：

- audit
- replay
- UI graph
- analytics
- 幂等恢复与冲突诊断

### 2. 写接口不承担完整读取职责

`case_redirect` 是写命令。

写命令之首责是返回：

- 此次写入是否成功
- 成功写入了哪条 transition
- 当前状态已推进到何处
- 下一步建议为何

其不应承担“顺便返回整条历史对象”之职责。

### 3. 读接口承担完整事实读取

若调用方需要完整 transition，则应调用专门读接口：

- 单条读取
- 列表读取
- 图视图聚合读取

如此可保 write/read 边界清晰。

### 4. `transition` 中的 direction 必须是快照

`transition` 中保存的 `from_direction` / `to_direction`，必须被定义为：

- **事件发生时的不可变快照**
- 而非当前 direction 表的可变投影

否则 audit 与 replay 会失真。

## 一、存储模型

### 目标

定义 `redirect` 的 canonical record。

### 结构建议

```json
{
  "transition_id": "tr_xxx",
  "case_id": "C-20260324-01",
  "type": "redirect",
  "status": "created",
  "entry_seq": 17,
  "from_direction_snapshot": {
    "seq": 2,
    "summary": "investigate stale server shadowing local binary",
    "constraints": [
      {
        "rule": "reproduce with isolated temp data dir",
        "reason": "avoid cross-test contamination"
      }
    ],
    "success_condition": "identify whether stale server process explains the mismatch",
    "abort_condition": "stop if the issue reproduces even with a fresh address",
    "reason": "the previous probe path no longer explains the observed behavior",
    "context": "the active server may be shadowing newly built binaries"
  },
  "to_direction_snapshot": {
    "seq": 3,
    "summary": "verify redirect behavior through explicit transition replay",
    "constraints": [
      {
        "rule": "capture each redirect response as structured JSON",
        "reason": "the result shape must be inspectable by tooling"
      }
    ],
    "success_condition": "confirm the transition model is sufficient for audit and UI rendering",
    "abort_condition": "stop if the replay cannot distinguish new redirect from recovered redirect",
    "reason": "the old direction was not sufficient for reconstructing history",
    "context": "future UI needs a first-class transition model"
  },
  "reason": "the old direction was not sufficient for reconstructing history",
  "context": "future UI needs a first-class transition model",
  "created_at": "2026-03-24T10:00:00Z",
  "actor": "agent",
  "source": "mcp"
}
```

### 存储字段

- `transition_id`
  - transition 稳定标识
  - 供引用、读接口、日志归并、幂等重试使用

- `case_id`
  - 所属 case

- `type = "redirect"`
  - 事件类型

- `status = "created" | "recovered"`
  - 执行结果状态
  - 不是业务类型，而是写入路径结果

- `entry_seq`
  - 若本次新建 redirect entry，则有值
  - 恢复支路可为 `null`

- `from_direction_snapshot`
  - redirect 前方向之不可变快照

- `to_direction_snapshot`
  - redirect 后方向之不可变快照

- `reason`
  - redirect 之缘由

- `context`
  - redirect 之上下文

- `created_at`
  - transition 记录时间

- `actor`
  - 谁发起了该操作

- `source`
  - 从何接口而来，如 CLI / MCP / API

## 二、写回执模型

### 目标

让 `case_redirect` 返回值专注于：

- 确认写成
- 指向 canonical transition
- 告知当前 state
- 指出下一动作

### 推荐返回结构

```json
{
  "ok": true,
  "transition_ref": {
    "id": "tr_xxx",
    "case_id": "C-20260324-01",
    "type": "redirect",
    "status": "created"
  },
  "summary": {
    "from_direction_seq": 2,
    "from_direction_summary": "investigate stale server shadowing local binary",
    "to_direction_seq": 3,
    "to_direction_summary": "verify redirect behavior through explicit transition replay"
  },
  "state": {
    "current_direction_seq": 3
  },
  "next_action": {
    "suggested_command": "step add",
    "why": "the new direction needs a fresh execution queue"
  }
}
```

### 为何不用完整 `transition`

若 `case_redirect` 直接返回完整 `transition`，则：

- 写接口与读接口边界混浊
- CLI/MCP 回执变肥
- 调用方倾向依赖“写后顺带全量读”
- 后续接口演进困难

故最优方式不是“写命令返回全部事实”，而是：

- **返回稳定引用**
- **返回少量摘要**
- **把完整事实交给读接口**

### 写回执字段

- `ok`
  - 是否成功

- `transition_ref`
  - 指向 canonical transition
  - 最低要求：`id`、`case_id`、`type`、`status`

- `summary`
  - 给 CLI / agent / shell 快速判断所用
  - 只放前后方向的最小可读摘要

- `state`
  - 当前状态推进结果
  - 最低要求：`current_direction_seq`

- `next_action`
  - 下一建议动作

## 三、读模型

### 目标

让 UI、audit、replay、agent automation 从稳定接口获取完整 transition。

### 最低必需接口

- `case_transition_get(id)`
  - 读取单条 transition

- `case_transition_list(case_id, cursor?, limit?)`
  - 按 case 列出 transitions

### 可选聚合接口

- `case_show`
  - 返回当前 case 状态
  - 可选内联 recent transitions

- `case_graph`
  - 专供 UI 图视图
  - 返回 directions + transitions

### 读模型职责

- 供 UI 画 graph
- 供 audit 查看单条 transition 详情
- 供 replay 按时间顺序重演
- 供 agent 在 receipt 之后按 ref 补取全量信息

## 幂等语义

### 建议规则

同一请求重试时：

- 若命中一致残留，则返回同一 `transition_ref`
- `transition_ref.status = "recovered"`
- `summary` 与 `state` 应与先前一致

若请求参数与残留不一致，则：

- 应明确报冲突
- 不应伪装为成功恢复

### 为何 `status` 比单纯 `recovered: true/false` 更好

`status` 可自然扩展为：

- `created`
- `recovered`
- `conflicted`

比单一布尔更利排障与审计。

## CLI / MCP / UI 分工

### CLI

CLI 调用写命令时，优先消费 receipt：

- 是否成功
- 从何方向转向何方向
- 当前方向序号
- 下一步做什么

若用户要求详情，再调用 transition 读接口。

### MCP / agent

agent 调用写命令时：

- 先用 `transition_ref.id` 判重
- 再按需调用读接口取完整 transition

如此可减少上下文噪音。

### UI

UI 不应依赖写命令回执构图。

UI 应基于：

- `case_transition_list`
- `case_graph`
- `case_transition_get`

来读正式图模型与历史模型。

## 与旧模型之区别

旧想法：

- 将完整 `transition` 直接作为 `case_redirect` 返回值

新结论：

- 将完整 `transition` 作为**内部 canonical record**
- 将 `case_redirect` 返回值定义为**receipt**
- 将完整读取交予**read API**

此一改动，解决之核心问题是：

- 领域事实与写回执分离
- 写/读职责边界清楚
- UI / audit / replay 不依赖写命令形状
- CLI / MCP 响应更稳、更清晰、更可演进

## 未决问题

以下留待后续迭代：

1. `transition_id` 应取派生键，抑或存储层生成 UUID
2. `actor` / `source` 是否首轮即纳入持久化
3. `case_show` 是否默认附 recent transitions
4. 是否需要专门的 `case_graph` 读接口
5. 其他写命令是否也采用同样 receipt / read-model 分层

## 本文结论

最稳之设计不是“让 `case_redirect` 返回完整 transition”，而是：

- **存：完整 transition**
- **写：最小 receipt**
- **读：专门 query 接口**

一句话概之：

> `case_redirect` 应确认“这次 redirect 写成了哪条 transition”，而完整 transition 本身，应由读模型负责提供。
