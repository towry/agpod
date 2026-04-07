# Case Interface Redesign

本文定 `agpod case` 之后续接口口径。

宗旨有三：

- 减 agent 心智负担
- 减高频 tool call 次数
- 令接口与领域模型相称，不为旧口所拘

本文重在**目标设计**，非兼容说明书。

---

## 1. 问题总览

今 `case` 工具之重病，不在字段多少，乃在调用拓扑失当。

高频序列常如此：

1. `case_current`
2. `case_resume`
3. 取 `case_id`
4. `case_record`
5. `case_step_mark_as`
6. `case_step_mark_as`

此中数步，并非业务必需，乃接口裂缝所迫。

归根言之，有四病：

- **读口分裂**：`current` 与 `resume` 分掌同一“恢复上下文”之事
- **身份外泄**：单仓仅容一 open case，而变更口仍强索 `case_id`
- **建案裂步**：`open` 与初始 `steps_add` 人为分离
- **推进裂事**：记录、结步、启下一步，本属一事，却裂为多调

故本文不作枝叶修饰，直改接口职责。

---

## 2. 设计门禁

凡新接口，须同时合乎下列准绳：

### 2.1 先领域，后接口

接口须映领域动作，不得映调用者之机械操作。

例如：

- “推进一步”是领域动作
- “先记一条 note，再把某 step 设 done，再把另一 step 设 started”不是领域动作，只是旧接口拼装之结果

### 2.2 默认取当前工作上下文

系统既有“每仓至多一 open case”之约束，则接口应以“当前 open case”为默认作用域。

若调用者仍需处置特定 case，可再显式指名；但常态不应强索之。

### 2.3 一次调用，应足以完成一项常见意图

若常见意图恒需 2~3 次串行调用，接口即未建好。

### 2.4 读口当分层，不当分裂

“看当前状态”“恢复继续工作”“查看全案卷宗”三者可分层，不可乱裂。

### 2.5 不以人类 CLI 习惯束 agent tool

凡“防手滑二次确认”“先拿 id 再回填”等，若只服务人类 shell 交互，而不服务 agent 任务推进，则不应强加于 MCP。

---

## 3. 目标接口总图

目标工具组如下：

### 3.1 读取类

- `case_current`：当前工作面板；亦为恢复入口
- `case_show`：全案卷宗与可寻址细览
- `case_list`：列案
- `case_recall`：检旧案与取上下文

### 3.2 变更类

- `case_open`：开案；可并注初始 steps
- `case_record`：记事实
- `case_decide`：记决断
- `case_redirect`：换方向
- `case_steps_add`：添步骤
- `case_step_move`：重排步骤
- `case_step_advance`：推进一步
- `case_finish`：结案

### 3.3 删除之口

- `case_resume`

其职当并入 `case_current`，不再独存。

---

## 4. 各接口之目标职责

### 来自真实反馈之附加原则

据真实长案反馈，当前最痛者主要有四：

- 高频查看太重
- 高频记录太碎
- step 推进不顺
- redirect / direction 语义太重

故本轮设计优先次序，当以后两原则约束之：

1. 先治高频当前推进，再治中低频历史回溯
2. 先减机械操作，再增语义化入口

## 4.1 `case_current`

### 目标

一调用而得“当前可继续工作之最小充分上下文”。

### 应答须含

- `case`
- `direction`
- `steps.current`
- `steps.pending`
- `last_fact`
- `last_decision`
- `health`
- `next`
- 必要时之 `direction_history` 摘要

### 不再另调

- 不再先 `case_current` 后 `case_resume`

### 与 `case_show` 之分界

- `case_current`：给“现在该做什么”
- `case_show`：给“全案细账与历史全貌”

### 由此引出之未决项：是否另立 `case_brief`

真实反馈指出，高频时常只需：

- `goal`
- `active_step`
- `latest_fact`
- `current_blocker`
- `next_action`
- `goal_drift_risk`

故 `case_brief` 确属合理候选。

但本轮先不纳入终形，理由有二：

- 若 `case_current` 能被收敛到“最小充分上下文”，则未必还需再分一口
- 若仓促立 `brief`，而其生成规则又与 `current` 分叉，则将再造一层读口漂移

故本轮先定：

- `case_current` 为唯一恢复入口
- 是否再立 `case_brief`，待后续更多真实反馈再裁

---

## 4.2 `case_open`

### 目标

一调用而完成：

1. 开案
2. 立初始方向
3. 可选地注入初始步骤

### 新输入

`steps` 与 `needed_context_query` 皆为可选字段，且**可同次并用**，形如：

```json
{
  "goal": "string",
  "direction": "string",
  "success_condition": "string",
  "abort_condition": "string",
  "steps": [
    "inspect current MCP contract",
    {
      "title": "draft target tool set",
      "reason": "freeze interface before implementation",
      "start": true
    }
  ],
  "needed_context_query": {
    "how_to": [
      "run hosted smoke",
      "use case recall effectively"
    ],
    "doc_about": [
      "honcho integration",
      "case recall startup memory"
    ],
    "pitfalls_about": [
      "empty recall result",
      "tool usage gotchas"
    ],
    "known_patterns_for": [
      "smoke testing",
      "debug playbooks"
    ]
  }
}
```

若只需上下文查询，亦可只给：

```json
{
  "goal": "string",
  "direction": "string",
  "needed_context_query": {
    "how_to": [
      "run hosted smoke"
    ]
  }
}
```

### 设计义

“开案而未立执行步列”虽可存在，然多属瞬时态，非常态。接口应容常态一路成之。

### `needed_context_query` 之设计义

新案开局时，最清楚“此刻缺何上下文”者，正是调用 `case_open` 之 LLM。

故与其另立一口补脑，不如让开案者显式声明：

- 我想学何种 `how_to`
- 我想先看何类文档 `doc_about`
- 我想避何种坑 `pitfalls_about`
- 我想继承何种已验证套路 `known_patterns_for`

实现时，此输入应转成**聚焦的 context query**，而非泛用 `recent_work`。

真实反馈表明：

- `context_shortcut=recent_work` 适合速览近况
- `mode=find` 适合发现候选 case
- `mode=context` + 聚焦 `query` 才适合抽取特定证据

故当目标是开局补足“正确用法 / 调试套路 / 复跑输入 / 阻塞描述”时，
`needed_context_query` 应驱动底层 memory 走聚焦查询，
并在 query 中要求“若无具体证据，也须明言缺失”。

### 返回形

若给 `needed_context_query`，则 `case_open` 可附带返回：

```json
{
  "startup_context": {
    "recommended_docs": [],
    "recommended_external_refs": [],
    "known_working_patterns": [],
    "known_pitfalls": [],
    "relevant_past_cases": [],
    "why_these_are_relevant": []
  },
  "startup_context_status": "ok | empty | degraded"
}
```

### `startup_context_status` 判据

- `ok`
  - memory / recall 后端成功执行
  - 且返回了至少一条 startup context 项

- `empty`
  - memory / recall 后端成功执行
  - 但未找到足够相关之上下文

- `degraded`
  - memory / recall 后端发生错误、超时、部分失败，或返回结构不完整到无法安全消费
  - 但 `case_open` 本身仍成功

在三者之下，`case_open.ok` 皆仍可为 `true`；
`startup_context_status` 仅表达“补充上下文之健康度”，不表达开案成败。

其最小判定规则为：

- 后端成功且命中数 `> 0`：`ok`
- 后端成功且命中数 `= 0`：`empty`
- 后端报错 / 超时 / 结构化结果缺关键字段：`degraded`

### 三条硬规

- `case_open` 不可因 startup context 空回而失败
- `startup_context` 应以**引用优先**，少灌长文
- `needed_context_query` 全可省；省则只做普通开案

---

## 4.3 诸变更口之默认作用域

下列工具默认作用于**当前 open case**：

- `case_record`
- `case_decide`
- `case_redirect`
- `case_steps_add`
- `case_step_move`
- `case_step_advance`
- `case_finish`

### 设计义

调用者之真实意图，通常是“对当前这案做事”，非“再手填一遍 case id”。

---

## 4.4 `case_finish`

### 目标

一调用结案。

### 设计义

tool 级二次确认，本为人类终端防误触之设，不宜上升为 agent 常态负担。

### 输入

```json
{
  "outcome": "completed",
  "summary": "string"
}
```

或：

```json
{
  "outcome": "abandoned",
  "summary": "string"
}
```

### 输出

- `case.status`
- `summary`
- `next`

---

## 5. `case_step_advance` 详解

此为本文最重要之新增接口。

## 5.1 为何须有此口

今步骤推进常裂为数调：

1. `case_record`：记发现
2. `case_step_mark_as(status=done)`：结当前步
3. `case_step_mark_as(status=started)`：启下一步

然调用者之真实意图，只是一句：

> “此步已做完；记下结果；继续下一步。”

故应有一口，直映此意。

---

## 5.2 定义

`case_step_advance` = **以当前活动步为中心之原子推进动作**

其可一次完成四事：

1. 完成指定 step
2. 可选地附记一条事实记录
3. 可选地启动下一步
4. 返回推进后之新工作面板

---

## 5.3 适用场景

### 场景一：纯结步

“这一步做完了，但暂不记额外事实，也不立刻启下一步。”

### 场景二：结步并记发现

“这一步做完了，并要把所得结果记入案中。”

### 场景三：结步并启指定下一步

“这一步做完了，接着启动 `S-003`。”

### 场景四：结步、记发现、自动启下一待办步

“这一步做完了；把结论记下；若有下一个 pending step，便自动启之。”

此即 agent 最常见之高频动作。

---

## 5.4 输入形

MCP 目标输入如下：

```json
{
  "id": "optional string",
  "step_id": "optional string",
  "record": {
    "summary": "required string",
    "kind": "optional string",
    "files": ["optional", "path"],
    "context": "optional string"
  },
  "next_step_id": "optional string",
  "next_step_auto": false
}
```

### 字段释义

- `id`
  - 可省
  - 省则取当前 open case

- `step_id`
  - 欲完成之 step
  - 可省
  - 省则默认取当前 `active` step

- `record`
  - 可省
  - 省则本次不附记事实

- `next_step_id`
  - 可省
  - 若给之，则推进后显式启动此步

- `next_step_auto`
  - 可省，默认 `false`
  - 为 `true` 时，若存在顺序上之下一 pending step，则自动启动之

### 互斥规约

`next_step_id` 与 `next_step_auto` 不得并用。

盖一者为显式指名，一者为隐式择下一步；并用则语义冲突。

---

## 5.5 语义规约

以下诸条，皆属此口之硬约束。

### 5.5.1 完成对象须唯一

若给 `step_id`，其须指向当前方向中之一真实 step。

若省 `step_id`，则默认取当前 `active` step。

若不存在，则失败。

### 5.5.2 被完成之步须可完成

仅容 `active` 状态之步入此口。

若为：

- `pending`
- `done`
- `blocked`
- `skipped`

则皆失败，不作静默幂等。

### 5.5.3 若附记 `record`，其性质仍为“事实”

`record.kind` 仅容：

- `note`
- `finding`
- `evidence`
- `blocker`

不容：

- `decision`
- `goal_constraint_update`

盖：

- “决断须附理由”，仍当走 `case_decide`
- “goal constraint update” 乃全案级规则变更，不应附着于某一步之完成

### 5.5.4 启下一步属可选副动作

若不设 `next_step_id`，亦不设 `next_step_auto=true`，则此口只负责结当前步。

### 5.5.5 `next_step_auto=true` 之择步规则

默认取：

- 同一方向下
- 顺序紧随其后
- 状态为 `pending`

之首步。

若无此步，则不报错，仅返回“未启动新步”之结果，并由 `next` 提示后续动作。

### 5.5.6 此口为单一事务语义

调用者视角下，此口应被视为一个原子动作：

- 要么整体成功
- 要么整体失败

不应出现：

- record 已写入，而 step 未结
- 当前步已结，而指定下一步未启

之半成态。

此处之原子性，最终应由存储层真事务保证，而非命令层补偿性回滚。

---

## 5.6 输出形

目标输出如下：

```json
{
  "ok": true,
  "completed_step": {
    "id": "S-002",
    "title": "scan beta * f_q80_fail_tsum05",
    "status": "done"
  },
  "record_entry": {
    "seq": 8,
    "entry_type": "record",
    "kind": "finding",
    "summary": "beta 0.04 improves target gain without hurting toxic or p10"
  },
  "started_step": {
    "id": "S-003",
    "title": "summarize keep-or-archive decision",
    "status": "active"
  },
  "steps": {
    "current": {
      "id": "S-003",
      "status": "active",
      "title": "summarize keep-or-archive decision"
    },
    "pending": []
  },
  "case": {
    "id": "C-20260320-01",
    "status": "open"
  },
  "direction": {
    "seq": 2,
    "summary": "test lightweight additive fusion on top of the locked base model"
  },
  "next": {
    "suggested_command": "case_decide",
    "why": "the last execution step has finished and the case now needs a keep-or-archive decision"
  }
}
```

### 关键字段

- `completed_step`
- `record_entry`
- `started_step`
- `steps`
- `next`

此数者使调用者**不必再立即补调 `case_current`**。

---

## 5.7 四个典型例子

## 例一：仅结当前步

输入：

```json
{
  "step_id": "S-002"
}
```

语义：

- `S-002` 设为 `done`
- 不记额外事实
- 不启新步

适于：

- 此步只是中间机械动作
- 结果已在先前记录中

---

## 例二：结步并记发现

输入：

```json
{
  "step_id": "S-002",
  "record": {
    "summary": "beta 0.04 is the first point that clears all guardrails",
    "kind": "finding"
  }
}
```

语义：

- `S-002` 完成
- 追加一条 `finding`
- 不启新步

适于：

- 需把此步结论挂入案卷
- 但下一步尚待人工或上层 agent 选择

---

## 例三：结步并启指定下一步

输入：

```json
{
  "step_id": "S-002",
  "next_step_id": "S-003"
}
```

语义：

- `S-002` 完成
- `S-003` 启动

适于：

- 步列虽多，但下一步已明定

---

## 例四：结步、记发现、自动启下一待办步

输入：

```json
{
  "step_id": "S-002",
  "record": {
    "summary": "beta 0.04 improves target gain and keeps toxic/p10/recent6 intact",
    "kind": "evidence",
    "files": ["reports/beta_scan.csv"],
    "context": "Top50->Top10 + alpha=0.245 + money_q90 remained unchanged"
  },
  "next_step_auto": true
}
```

语义：

- `S-002` 完成
- 追加 `evidence`
- 自动启下一 pending step
- 返回推进后之工作面板

此乃本接口最常见、亦最有价值之路径。

---

## 5.8 CLI 口径

CLI 宜纳于 `step` 之下：

```bash
agpod case step advance --step-id S-002
```

或：

```bash
agpod case step advance \
  --step-id S-002 \
  --record-kind finding \
  --record-summary "beta 0.04 is the first point that clears all guardrails" \
  --next-step-auto
```

如此与既有 `step add / move / ...` 之族谱相顺。

---

## 5.9 与现有工具之边界

### `case_step_advance` 与 `case_record`

- 前者：为“推进步骤”而设，记录只是可选附属
- 后者：为独立记事实而设，不涉步骤状态
- 若某一步完成后，尚需连续补记二条以上 evidence / finding，则仍应多次调用 `case_record`
- 不应因“完成一步后常需记数条事实”而把批量记录、决断、结案一并塞入 `advance`

### `case_step_advance` 与语义化记录入口

真实反馈曾提：

- `mark_test_result`
- `mark_smoke_result`
- `mark_patch_applied`
- `mark_handoff`

此类入口，确可能降低记录分类负担。

然本轮不将其纳入终形，理由在于：

- 其收益主要在“记录更顺手”
- 本轮主目标则是“减少高频 tool call 与根治职责裂缝”

故当前判断为：

- 先以 `record` / `decide` / `advance` 理顺骨架
- 待真实反馈累积，再择最高频工程动作增少数语义化记录口

### `case_step_advance` 与 `case_decide`

- 前者：不记决断
- 后者：专记决断，且须附理由

### `case_step_advance` 与 `case_step_mark_as`

- MCP 层：`case_step_mark_as` 仅用于 `started` / `blocked`
- CLI 层：`step done` 可留作底层调试口
- 高频完成步骤：统一走 `case_step_advance`

故 agent 不应再以 `case_step_mark_as(done)` 完成步骤。

### 设计戒律：勿令 `advance` 膨胀为神口

真实闭环常见如下：

1. 推进当前步
2. 连续记录一至数条 evidence / finding
3. 记录 decision
4. 结案

此时，宜由：

- `case_step_advance`
- `case_record`
- `case_decide`
- `case_finish`

分司其职。

不得因其常相邻出现，便并为一巨型聚合口。否则：

- `advance` 将吞并事实记录
- `advance` 将吞并决断语义
- `advance` 将吞并结案语义

终致接口表意混浊，且难以审查。

---

## 6. 推荐之常见调用序列

## 6.1 恢复工作

旧：

1. `case_current`
2. `case_resume`

新：

1. `case_current`

## 6.2 开新案

旧：

1. `case_open`
2. `case_steps_add`

新：

1. `case_open(steps=..., needed_context_query=...)`

## 6.3 推进一步

旧：

1. `case_record`
2. `case_step_mark_as(done)`
3. `case_step_mark_as(started)`

新：

1. `case_step_advance`

今后 MCP 工具层不再暴露 `done` 给 `case_step_mark_as`；
该旧序列只作历史对照，不作新 agent 建议。

### 真实反馈校验：完整收案序列

以下序列出自真实使用反馈：

1. `step done`
2. `step started`
3. `record evidence`
4. `record evidence`
5. `decide`
6. `step done`

其说明两事：

- 纯机械之 step 状态维护，确可由 `advance` 吸收
- 而多条 `evidence`、`decision` 本身，仍是独立语义动作，不宜硬并

故本设计对该类真实序列之改写为：

1. `case_step_advance(next_step_id=...)`
2. `case_record`
3. `case_record`
4. `case_decide`
5. `case_step_advance()`
6. `case_finish`

此既减调用数，又不毁语义边界

## 6.3.1 完整收案闭环

适用于：

- 当前步完成
- 需补记数条证据
- 需形成一条决断
- 最后结案

旧：

1. `case_step_mark_as(done)`
2. `case_step_mark_as(started)`
3. `case_record`
4. `case_record`
5. `case_decide`
6. `case_step_mark_as(done)`
7. `case_finish`

新：

1. `case_step_advance(next_step_id=...)`
2. `case_record`
3. `case_record`
4. `case_decide`
5. `case_step_advance()`
6. `case_finish`

要义不在“把一切并成一调”，而在：

- 去除纯机械之步骤状态拼接
- 保留事实、决断、结案之独立语义边界

## 6.4 结案

旧：

1. `case_finish`
2. `case_finish(confirm_token=...)`

新：

1. `case_finish`

---

## 7. 实施次序

若依收益与风险权衡，宜如此次第行之：

1. 并 `case_resume` 入 `case_current`
2. 去诸变更口对 `case_id` 之常态强依赖
3. 扩 `case_open` 以纳 `steps`
4. 改 `case_finish` 为单次结案
5. 增 `case_step_advance`

末项最值期待，然亦最须先定语义，故本文特详之。

---

## 8. 非目标

本文暂不处理：

- `case_show` 分窗与分页细则
- `case_recall` 检索策略
- case storage schema 之再设计
- transition event read model 之全面重构

此数者虽重要，然非本轮“减心智负担、减调用次数”之主矢。

---

## 9. Recall 使用准则

`case_recall` 不应一律以 `recent_work` 为先。

建议准则如下：

- 需速览近况：用 `mode=context` + `context_shortcut=recent_work`
- 需发现候选旧案：用 `mode=find` + query
- 已知候选 case，需抽取具体证据：用 `mode=context` + `context_scope=case` + 聚焦 query
- 未知具体 case，但需 repo 范围抽取特定证据：用 `mode=context` + `context_scope=repo` + 聚焦 query

典型聚焦 query 应明示：

- 要抽取何种字段或证据
- 优先返回具体 ID / 文件 / 命令
- 若无具体记录，须明言缺失

例如：

```text
提取此案中与真实 topic relation smoke 输入有关的具体证据：
topic_id、topic_revision_id、已 settled revision 样本、成功或失败的 smoke 输入、
以及用于复跑的命令或阻塞描述。若无具体 UUID，也请明确指出缺失。
```

此准则亦约束 `case_open.needed_context_query`：

- 它不应退化成泛用 `recent_work`
- 它应生成以当前 goal / direction / 所列 topics 为核心之聚焦 context query

---

## 10. 与 `case` / 文档之对齐

本设计与 `case` 之关系，须循下列原则：

- `case` 记推进现场
- 设计文记稳定规则
- 实现文记落地步骤

故当设计已冻结后，`case` 中应优先记录：

- 一句当前结论
- 一到数个文档引用

而不应再于 `case_record` / `case_decide` 中重抄大段设计正文。

此处之“引用”，既可指仓内文档，亦可指外部链接，例如：

- 仓内设计文
- runbook
- Linear issue / comment
- PR / CI / 评审链接

换言之，长期目标是：

- `case` 以引用连接文档
- 文档以稳定规则承接 `case` 中已冻结之结论

详见 `docs/case-doc-alignment.md`。

---

## 11. 实现后 smoke 要求

凡本设计所涉功能完成后，须补一组“旧数据兼容 smoke”。

目的：

- 避免新功能对旧 case 数据不兼容报错
- 验证新增字段 / 新返回结构不会破坏既有读取路径

最低要求如下：

1. 在功能实现前，先备份一份旧 `AGPOD_CASE_DATA_DIR`
2. 功能完成后，以该旧数据目录副本运行新二进制
3. 在旧数据上执行最小 smoke：
   - `agpod case current --json`
   - `agpod case show --json`
   - 与本轮新功能相邻之一条最小命令
4. 若旧记录缺少新字段，则应以空值 / 缺省值兼容读取，不得直接报错

尤其当本轮改动涉及：

- `Entry.step_id`
- `case_open.needed_context_query`
- `startup_context`
- 其他新增持久化字段或新增结构化返回字段

则此 smoke 为阻断项，不可省。

---

## 12. 一言以蔽之

本轮改造之旨，不是“给旧工具再写说明”，而是：

> 令接口直接对应 agent 真实意图。

若意图是一件事，接口便只应让它调一次。
