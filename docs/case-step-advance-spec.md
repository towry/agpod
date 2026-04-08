# Case Step Advance Spec

本文专定 `case_step_advance` 之严规格。

前文 `docs/case-interface-redesign.md` 已立方向；
本文则补其未决边界，使之可实现、可测试、可审查。

本文之地位高于示例性描述。凡后续实现，须以本文为准。

---

## 1. 目的

`case_step_advance` 所对应者，不是“改 step 状态”这一底层原语，
而是如下高频领域动作：

> 完成当前工作步；可选记下该步所得事实；可选进入下一步；并回传推进后之工作面板。

故此口须解决三事：

- 减一次推进所需之 tool call 数
- 保持步骤状态语义一致
- 使“此记录出自哪一步”可被追溯

---

## 2. 当前代码事实

本规格以下列仓内现实为前提：

- `StepStatus` 现有五态：`pending` / `active` / `done` / `blocked` / `skipped`，见 `crates/agpod-case/src/types.rs:53`
- `Step` 已有 `order_index`，并由 `get_steps(... ORDER BY order_index)` 取序，见 `crates/agpod-case/src/types.rs:219` 与 `crates/agpod-case/src/client.rs:534`
- `move` 已改写 `order_index`，故步骤“逻辑顺序”即 `order_index`，见 `crates/agpod-case/src/commands.rs:2003`
- `Entry` 今仅挂 `case_id`，未挂 `step_id`，见 `crates/agpod-case/src/types.rs:231`
- `activate_step(...)` 现非事务；会先降旧 active，再升新 active，再写 `case.current_step_id`，见 `crates/agpod-case/src/commands.rs:1924`

是故：

- `next_step_auto` 之排序依据，今可明定为 `order_index`
- `case_step_advance` 若要守原子性，不能沿袭现有“多次独立 update”之写法

---

## 3. 最终设计裁定

本节专断前文未决之处。

### 3.1 事务策略

**裁定**：`case_step_advance` 必以**存储层真事务**实现；不得以命令层补偿逻辑模拟。

### 理由

- 此口涉及最少三写：
  - 完成当前步
  - 可选写 entry
  - 可选启动下一步
- 若以补偿回滚模拟，则实现与测试复杂度急增，且易遗半成态
- 此口既以“原子推进”为卖点，则其根应立于持久化层，而非命令层临机补漏

### 实现前置要求

在实现 `case_step_advance` 之前，须先为 `CaseClient` 增一条事务执行通路，使同一 case 之多写可在单事务内提交。

若此前置未成，则不得实现本工具。

---

### 3.2 `step_id` 是否可省

**裁定**：`step_id` 可省。

规则如下：

- 若显式给 `step_id`，则完成该步
- 若省 `step_id`，则默认取当前 `active` 步
- 若省 `step_id` 且当前无 `active` 步，则失败

### 理由

此与“默认当前工作上下文”之总原则相合。

若仍强索 `step_id`，则调用者常须先调 `case_current` 再回填，与减负宗旨相反。

---

### 3.3 `pending -> done` 是否允许

**裁定**：不允许。

`case_step_advance` 只接受当前状态为 `active` 之步。

以下状态一律失败：

- `pending`
- `done`
- `blocked`
- `skipped`

### 理由

`advance` 之义，不是“随便把一步改成 done”，乃是“推进当前执行中的一步”。

若容 `pending -> done`，则会混淆“未执行而跳过”与“执行已完成”。

若将来需“跳步”，应另立明确语义之接口，例如 `step skip`，不可借 `advance` 行之。

---

### 3.4 `blocked` 如何恢复

**裁定**：`blocked` 之恢复，不归 `case_step_advance`。

恢复路径如下：

- 先经 `case_step_mark_as(started)` 或其后续替代口，将该步重置为 `active`
- 再由 `case_step_advance` 完成之

### 理由

`blocked` 表示“此步因阻碍而中止”，不等于“该步已推进完成”。

故“解阻并恢复执行”与“完成执行”是两件事，不应混于一口。

---

### 3.5 `skipped` 之地位

**裁定**：`skipped` 非本轮实现目标。

但状态表中须保留其不可入 `advance` 之规则。

若将来启用 `skipped`：

- `advance` 仍不得将步转为 `skipped`
- `advance` 亦不得作用于 `skipped` 步

---

### 3.6 `record` 与 `step` 之存储关联

**裁定**：凡由 `case_step_advance` 附记之 `record entry`，必须持久化 `step_id`。

### 所需 schema 变更

为 `Entry` 增字段：

- `step_id: Option<String>`

其语义为：

- 若该 entry 由步骤推进动作直接产出，则记其所属 step
- 若为独立 `case_record`、`case_decide`、`case_redirect` 等，则可为空

### 理由

若不持久化之，则：

- `case_show` 无法精确表“此 finding 出自哪一步”
- `resume/current` 无法可靠摘要“当前最后一个与步骤相关之发现”

---

### 3.7 `next_step_auto` 之择步规则

**裁定**：其唯一排序依据为**同方向内之 `order_index`**。

具体规则：

1. 取当前 case 之当前 direction
2. 取其中所有 `status = pending` 之步
3. 筛出 `order_index > completed_step.order_index`
4. 取其中 `order_index` 最小者

若无，则本次不自动启动新步。

### 额外裁定

`next_step_auto` **不会回绕**到较早之 pending 步。

若调用者欲启动较早之待办步，应显式给 `next_step_id`。

### 理由

“auto” 只应意谓“顺着现行步列自然推进”，不应替调用者做回溯重排。

---

## 4. 工具定义

## 4.1 MCP 名称

工具名定为：

- `case_step_advance`

## 4.2 CLI 名称

CLI 口定为：

- `agpod case step advance`

---

## 5. 输入规格

## 5.1 MCP 输入

```json
{
  "id": "optional string",
  "step_id": "optional string",
  "record": {
    "summary": "required string",
    "kind": "optional string",
    "files": ["optional string"],
    "context": "optional string"
  },
  "next_step_id": "optional string",
  "next_step_auto": false
}
```

## 5.2 字段规则

### `id`

- 可省
- 省则取当前 open case

### `step_id`

- 可省
- 省则取当前 active step

### `record`

- 可省
- 若给出，则 `summary` 必填
- `kind` 若省，默认 `note`

### `record.kind`

仅容：

- `note`
- `finding`
- `evidence`
- `blocker`

不容：

- `decision`
- `goal_constraint_update`

### 理由

- `decision` 当走 `case_decide`
- `goal_constraint_update` 当走 `case_record(kind=goal_constraint_update)`；此乃全案级规则变动，不应附属于“完成某一步”

### `next_step_id`

- 可省
- 若给出，则必须：
  - 属当前 direction
  - 当前状态为 `pending`

### `next_step_auto`

- 可省，默认 `false`

### 互斥规则

以下两者不得并用：

- `next_step_id`
- `next_step_auto = true`

---

## 5.3 CLI 参数口径

CLI 映射如下：

```bash
agpod case step advance \
  [--id <case-id>] \
  [--step-id <step-id>] \
  [--record-summary <text>] \
  [--record-kind <note|finding|evidence|blocker>] \
  [--record-file <path> ...] \
  [--record-context <text>] \
  [--next-step-id <step-id> | --next-step-auto]
```

### CLI 细则

- `--record-file` 可重复；每次追加一项
- `--record-context` 为单字符串；shell 中若含换行，由调用者自行引号包裹或文件重定向生成
- 若出现任一 `--record-*` 参数，则视为提供了 `record`
- 若用了 `--record-kind` / `--record-file` / `--record-context` 而无 `--record-summary`，则报错

---

## 6. 状态转移表

下表只论 `case_step_advance`。

| 初始状态 | 是否允许 | 结果 |
|---|---:|---|
| `pending` | 否 | 报错：step is not active |
| `active` | 是 | 转 `done` |
| `done` | 否 | 报错：step already done |
| `blocked` | 否 | 报错：step is blocked; resume it first |
| `skipped` | 否 | 报错：step is skipped |

### 额外不变式

- 同一 direction 同时至多一 `active` 步
- 成功后，`completed_step` 必为 `done`
- 若成功启动下一步，则新 `active` 步恰一
- 若未启动下一步，则 `case.current_step_id` 置空

---

## 7. 执行算法

设本次调用输入为 `req`。

成功路径必须依下列次序，且在**同一事务**中完成：

1. 解析 `case_id`
2. 读取 case，验证其 `status = open`
3. 解析目标 step：
   - 若 `req.step_id` 给出，则取之
   - 否则取当前 `active` step
4. 校验该步：
   - 属当前 direction
   - 状态为 `active`
5. 若给 `next_step_id`：
   - 读该步
   - 校验属当前 direction 且状态为 `pending`
6. 若 `next_step_auto = true`：
   - 依 `order_index` 规则选下一 pending 步
7. 将目标 step 改为 `done`
8. 若给 `record`：
   - 生成一条 `EntryType::Record`
   - 其 `step_id = completed_step.id`
9. 若要启动下一步：
   - 将所选步改为 `active`
   - 更新 `case.current_step_id = started_step.id`
10. 否则：
   - 更新 `case.current_step_id = ''`
11. 提交事务
12. 重新读取当前面板并组装返回值

---

## 8. `next` 字段规则表

`case_step_advance` 成功后，`next` 之生成依下表：

| 条件 | `suggested_command` | `why` |
|---|---|---|
| 已自动或显式启动下一步 | `record` | active step is now collecting evidence |
| 未启动下一步，且仍有后续 pending 步 | `step start` | there are pending steps waiting to be started |
| 无 pending 步，且方向已具充分执行证据，待作判断 | `case_decide` | execution steps are complete and the case now needs a decision |
| 无 pending 步，且最近已有明确 decision，待结案 | `case_finish` | all execution steps and decisions are in place |
| 无 pending 步，但是否决断不可由规则安全推出 | `case_show` | inspect the case history before deciding the next action |

### 判定细则

为免猜测，第三、四行所涉“是否已有明确 decision”，须以事实字段判定：

- 所谓“最近一条 entry”，指**本次 `advance` 提交后，排除本次新写入之 `record_entry`，按全案时间线倒序所见之第一条既有 entry**
- 若此条既有 entry 为 `decision`，则可落第四行
- 否则落第三或第五行

其中第三、第五之区分，初版可保守处理：**统一落第五行**。

故初版实现最小规则为：

- 若启动了下一步：`record`
- 否则若尚有 pending：`step start`
- 否则若最近 entry 为 `decision`：`case_finish`
- 否则：`case_show`

此规则可测、可审、且不臆断“该不该决策”。

---

## 9. 输出规格

成功返回须至少含下列字段：

```json
{
  "ok": true,
  "completed_step": {
    "id": "S-002",
    "order": 2,
    "title": "scan beta",
    "status": "done"
  },
  "record_entry": {
    "seq": 8,
    "entry_type": "record",
    "kind": "finding",
    "step_id": "S-002",
    "summary": "beta 0.04 clears all guardrails"
  },
  "started_step": {
    "id": "S-003",
    "order": 3,
    "title": "summarize decision",
    "status": "active"
  },
  "steps": {
    "current": {
      "id": "S-003",
      "order": 3,
      "title": "summarize decision",
      "status": "active"
    },
    "pending": []
  },
  "context": {
    "active_case_id": "C-20260320-01",
    "current_direction_seq": 2
  },
  "next": {
    "suggested_command": "record",
    "why": "active step is now collecting evidence"
  }
}
```

### 字段要求

- `completed_step` 必有
- `record_entry` 仅在本次附记时出现
- `started_step` 仅在本次启动下一步时出现
- `steps` 必为推进后新状态
- `next` 必有

### 返回目的

调用者在成功后，不必立即补调 `case_current`。

---

## 10. 错误规格

以下错误必须显式可辨：

- 无 open case
- 未找到目标 step
- `step_id` 省略但当前无 active step
- 目标 step 不属当前 direction
- 目标 step 非 `active`
- `next_step_id` 不属当前 direction
- `next_step_id` 当前非 `pending`
- `next_step_id` 与 `next_step_auto` 并用
- `record.kind` 非允许值
- 事务提交失败

### 错误信息原则

- 先说违反何规则
- 再说当前实际状态
- 必要时给下一动作建议

例如：

- `step S-002 is blocked; resume it before advancing`
- `no active step in current direction; pass --step-id explicitly or start a step first`

---

## 11. 非职责清单

`case_step_advance` 永不承载下列事项：

- 记录 `decision`
- 更新 goal constraint
- redirect direction
- reorder steps
- 跳过 step
- 解 blocked step
- 批量推进多步
- 批量记录多条 evidence / finding
- 直接结案

若后续有人欲将此数事塞入 `advance`，应视为设计越界。

### 真实反馈校验

已有真实使用序列表明，常见闭环可能是：

1. 完成一步
2. 连记两条以上 `evidence`
3. 记一条 `decision`
4. 再完成最后一步
5. 结案

此类场景**不构成**扩张 `advance` 之理由。

正确处理应为：

- `advance` 只吸收机械状态推进
- 多条 `record` 仍逐条调用
- `decision` 仍走 `case_decide`
- `finish` 仍走 `case_finish`

换言之，真实反馈支持：

- 增 `advance`

而不支持：

- 增“万能推进并记录并决断并结案”之巨型聚合口

---

## 12. 测试面

进入实现时，最少须有下列测试：

### 成功类

- 省 `step_id` 时，能推进当前 active step
- 推进一步并附记 finding
- 推进一步并显式启动 `next_step_id`
- 推进一步并 `next_step_auto=true`
- 无下一步时，`current_step_id` 被清空
- `record_entry.step_id` 被持久化

### 失败类

- `pending` 步不可 advance
- `blocked` 步不可 advance
- `done` 步不可 advance
- `next_step_id` 非 pending 时失败
- `next_step_id` 与 `next_step_auto` 并用失败
- `record.kind=decision` 失败

### 原子性类

- 若写 entry 失败，则 completed step 不得残留为 `done`
- 若启动下一步失败，则 entry 与 completed step 皆不得提交

## 13. 实施前置清单

在写 `case_step_advance` 前，必须先满足：

1. `CaseClient` 已具事务执行能力
2. `Entry` 已增 `step_id`
3. `output.rs` 已能渲染带 `step_id` 之 entry
4. `case_current` / `case_show` 已能消费该关联
5. `StepCommand` / MCP schema 已容 `advance`

若此前置未齐，不得声称“已实现 `case_step_advance`”。

---

## 14. 一言断之

`case_step_advance` 不是“把三个旧命令包一层壳”，而是：

> 以事务方式实现“推进当前工作步”这一独立领域动作。

若不能守此义，则宁缓做，不可草做。
