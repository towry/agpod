# Case Tool 实现方案

## 目标

此器用于夜间自动调研与长会话探索，所求不在“写一卷工作史”，而在：

- 令 agent 始终知其 `goal`
- 令 agent 始终知其当下 `direction`
- 令 agent 半路插做他事，归来仍能续上原 `step`
- 令 agent 知其不可逾越之约束，及何时可收、何时当止

此器非通用项目管理板；其本体乃“单案主线 + 当前打法 + 可重排之执行步列”。

## 核心心智模型

### `goal`

案之终局目标。

例：

- 提升性能 10%
- 锁定 production-ready 的 money veto 主档方案
- 查明分享图来源并确认 `cartier` 是否有定制

规则：

- 一案之内，`goal` 不变
- 若目标实质已变，当 `close` 或 `abandon` 旧案，再开新案

### `goal_constraints`

全案级护栏。

其作用是：即便 `direction` 变更，亦仍约束全案。

结构：

```json
[
  {
    "rule": "必须以终局 test 指标为准",
    "reason": "validation gain alone does not justify production choice"
  }
]
```

规则：

- 绑定于 `goal`
- 不单独管理 active/expired 状态
- 若 `goal` 未变，则仍有效

### `direction`

达成 `goal` 之当前主打法。

例：

- 先按旧脚本口径复现，再做跨模型投影审计
- 不换主模，只做小权重补分验证
- 先查 share config layer，再查 backend metadata API

结构：

```json
{
  "summary": "...",
  "constraints": [
    {
      "rule": "...",
      "reason": "..."
    }
  ],
  "success_condition": "...",
  "abort_condition": "..."
}
```

规则：

- 任一时刻仅有一条 `current_direction`
- `redirect` 可改 `direction`
- 旧 `direction` 一灭，其上之 `constraints` 自灭；不另设失效状态机

### `steps`

隶于当前 `direction` 之执行步列。

`step` 不是浮动心情，而是可排、可插、可重排之具体动作项。

结构：

```json
[
  {
    "id": "S-002",
    "order": 2,
    "title": "rerun daily selection using Top50% -> Top10% buckets",
    "status": "active",
    "reason": "new reading suggests prior absolute-count assumption was wrong"
  }
]
```

规则：

- `id` 为稳定身份
- `order` 仅为显示次序，可变
- `status` 取 `pending | active | done | blocked | skipped`
- 任一时刻仅一条 `active step`
- 临时插修阻断事项时，通常新增或重排 `step`，未必触发 `redirect`

### `decision`

同一路线内之关键取舍。

例：

- 锁 `q90` 为主档
- `q92` 仅留作备档
- 停止扩格，不再主推全量替代

规则：

- `decision` 不等于 `redirect`
- 参数收敛、主备切换、锁档，皆属 `decision`
- 仅当“方法路径”改变，方用 `redirect`

### `redirect`

方向切换事件。

必须说明：

- `from_direction`
- `to_direction`
- `reason`
- `context`

且应同时给出：

- 新 `direction.constraints`
- 新 `success_condition`
- 新 `abort_condition`

## 生命周期

状态仅三：

- `open`
- `closed`
- `abandoned`

合法流转：

```text
open -> record* -> decide* -> step* -> redirect* -> close
open -> record* -> decide* -> step* -> redirect* -> abandon
```

约束：

- 同仓库同一时刻仅容一宗 `open case`
- `open` 时必须给 `goal` 与初始 `direction`
- `redirect` 不得改变 `goal`
- `close` 与 `abandon` 必须附摘要

## CLI 设计

### 核心命令

```bash
case open --goal "..." --direction "..."
case current
case record --id C-... --summary "..."
case decide --id C-... --summary "..." --reason "..."
case redirect --id C-... --direction "..." --reason "..." --context "..."
case show --id C-...
case close --id C-... --summary "..."
case abandon --id C-... --summary "..."
```

### 步列命令

```bash
case step add --id C-... --title "..."
case step start --id C-... --step-id S-...
case step done --id C-... --step-id S-...
case step move --id C-... --step-id S-... --before S-...
case step block --id C-... --step-id S-... --reason "..."
```

### 辅读命令

```bash
case recall "..."
case recall --file path/to/file
case list
case resume --id C-...
```

## 命令语义

### `case open`

输入：

- `--goal` 必填
- `--direction` 必填
- `--goal-constraint` 可选，多值
- `--goal-constraint-reason` 配对输入或结构化 JSON
- `--success-condition` 可选
- `--abort-condition` 可选

说明：

- 初始 `direction` 可暂无 `steps`
- 若场景明确，宜一并给出初始 `success_condition / abort_condition`

### `case current`

用途：返回当前导航面板。

输出应固定含：

- `goal`
- `goal_constraints`
- `current_direction`
- `direction_constraints`
- `current_step`
- `remaining_steps`
- `last_fact`
- `success_condition`
- `abort_condition`

若异常，再加：

- `health`: `on_track | looping | blocked`
- `warning`

### `case record`

用途：记录事实、发现、证据、阻断。

允许之 `kind`：

- `note`
- `finding`
- `evidence`
- `blocker`

规则：

- 不得偷带 `decision` 或 `redirect` 语义
- 若内容实为“锁定取舍”，应改用 `decide`
- 若内容实为“切换打法”，应改用 `redirect`

### `case decide`

用途：记录关键取舍。

输入：

- `--summary` 必填
- `--reason` 必填

适用：

- 锁参数
- 主备切换
- 收敛后之明确判断

### `case redirect`

用途：切换当前 `direction`。

输入：

- `--direction` 必填
- `--reason` 必填
- `--context` 必填
- `--constraint` 可选，多值
- `--constraint-reason` 配对输入或结构化 JSON
- `--success-condition` 必填
- `--abort-condition` 必填

规则：

- 自动记录 `from_direction`
- 自动建立新 `direction`
- 自动将旧 `direction` 之步列封存
- 新 `direction` 应从空步列或新步列开始

### `case step *`

用途：管理当前 `direction` 下之步列。

说明：

- `add`：新增一步
- `start`：置为当前 `active`
- `done`：标记完成
- `move`：重排序
- `block`：标记受阻，并附原因

规则：

- `step` 只绑定当前 `direction`
- `redirect` 后，旧方向之 `step` 仅可回看，不再激活

### `case show`

应整合：

- `goal`
- `goal_constraints`
- `status`
- `current_direction`
- `direction_history`
- 各方向之 `constraints`
- 各方向之 `steps`
- `records`
- `decisions`
- 结案或弃案摘要

### `case resume`

用途：供新 agent 于中断后快速接手。

输出应高浓缩，直给：

- `goal`
- `goal_constraints`
- `current_direction`
- `direction_constraints`
- `current_step`
- `next_pending_steps`
- `last_decision`
- `last_evidence`
- `success_condition`
- `abort_condition`

## 夜间自动调研设计要点

### 1. 先显锚，再显路

凡 `current`、`resume`、`show --brief`，皆先显：

1. `goal`
2. `goal_constraints`
3. `current_direction`
4. `direction_constraints`

### 2. 让护栏可见且可解

`constraints` 不只说“不许做什么”，还须说“为何不许”。

好例：

```json
{
  "rule": "only test small-weight additive fusion",
  "reason": "full replacement has already failed and further expansion would confound the trial"
}
```

### 3. 把“走开又回来”落在 `steps`

半路插修阻断事项时：

- 通常新增 `step`
- 或将阻断修复步提到前面
- 修毕后恢复原 `step`

而非轻率 `redirect`

### 4. 用出口条件收束方向

不再单设 `kill_criteria`。

每条 `direction` 只保二式出口：

- `success_condition`
- `abort_condition`

例：

- `success_condition`: 找到 beta 点，可提升收益且不伤 `test toxic / p10 / recent6`
- `abort_condition`: 若无 beta 点可同时守住上述三项，则归库存并停止此线

## 数据模型

### `cases`

```sql
CREATE TABLE cases (
  id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  goal TEXT NOT NULL,
  goal_constraints TEXT NOT NULL,   -- JSON array[{rule, reason}]
  status TEXT NOT NULL CHECK (status IN ('open', 'closed', 'abandoned')),
  current_direction_seq INTEGER NOT NULL,
  current_step_id TEXT,
  opened_at DATETIME NOT NULL,
  updated_at DATETIME NOT NULL,
  closed_at DATETIME,
  close_summary TEXT,
  abandoned_at DATETIME,
  abandon_summary TEXT
);
```

约束：

```sql
CREATE UNIQUE INDEX idx_cases_one_open_per_repo
ON cases(repo_id)
WHERE status = 'open';
```

### `directions`

```sql
CREATE TABLE directions (
  case_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  summary TEXT NOT NULL,
  constraints TEXT NOT NULL,        -- JSON array[{rule, reason}]
  success_condition TEXT NOT NULL,
  abort_condition TEXT NOT NULL,
  reason TEXT,
  context TEXT,
  created_at DATETIME NOT NULL,
  PRIMARY KEY (case_id, seq),
  FOREIGN KEY(case_id) REFERENCES cases(id)
);
```

说明：

- 初始方向亦为 `seq = 1`
- 每次 `redirect` 新增一条 direction 记录，并更新 `cases.current_direction_seq`

### `steps`

```sql
CREATE TABLE steps (
  id TEXT PRIMARY KEY,
  case_id TEXT NOT NULL,
  direction_seq INTEGER NOT NULL,
  order_index INTEGER NOT NULL,
  title TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'active', 'done', 'blocked', 'skipped')),
  reason TEXT,
  created_at DATETIME NOT NULL,
  updated_at DATETIME NOT NULL,
  FOREIGN KEY(case_id) REFERENCES cases(id),
  FOREIGN KEY(case_id, direction_seq) REFERENCES directions(case_id, seq)
);
```

### `entries`

```sql
CREATE TABLE entries (
  id TEXT PRIMARY KEY,
  case_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  entry_type TEXT NOT NULL CHECK (entry_type IN ('record', 'decision', 'redirect')),
  kind TEXT,
  summary TEXT NOT NULL,
  reason TEXT,
  context TEXT,
  files TEXT,      -- JSON array
  artifacts TEXT,  -- JSON array refs
  created_at DATETIME NOT NULL,
  FOREIGN KEY(case_id) REFERENCES cases(id)
);
```

## 输出结构建议

统一骨架如下：

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "lock the production-ready money veto setup",
    "goal_constraints": [
      {
        "rule": "judge by terminal test metrics",
        "reason": "validation-only improvement is insufficient"
      }
    ],
    "status": "open"
  },
  "direction": {
    "summary": "test lightweight additive fusion on top of the locked base model",
    "constraints": [
      {
        "rule": "keep Top50->Top10 + alpha=0.245 + money_q90 unchanged",
        "reason": "the base pipeline is already locked"
      }
    ],
    "success_condition": "find a beta point that improves the target without hurting toxic/p10/recent6",
    "abort_condition": "archive the signal if no beta point can preserve all three guardrails"
  },
  "steps": {
    "current": {
      "id": "S-003",
      "title": "scan small beta weights for q80_fail_tsum05",
      "status": "active"
    },
    "pending": [
      {
        "id": "S-004",
        "title": "summarize keep-or-archive decision"
      }
    ]
  },
  "next": {
    "suggested_command": "record",
    "why": "the active step is in evidence collection mode"
  }
}
```

## 典型场景：ML 会话

```bash
case open \
  --goal "determine whether q80_fail_tsum05 deserves entry into the locked production stack" \
  --goal-constraint '{"rule":"judge by terminal test metrics","reason":"offline validation alone is insufficient"}' \
  --direction "test lightweight additive fusion on top of the locked base model" \
  --success-condition "find a beta point that improves target gain without hurting toxic/p10/recent6" \
  --abort-condition "archive q80_fail_tsum05 if no beta preserves all three guardrails"

case redirect --id C-20260320-01 \
  --direction "test lightweight additive fusion on top of the locked base model" \
  --reason "full replacement failed in terminal metrics" \
  --context "the replacement path no longer has enough upside; only the minimal additive trial remains" \
  --constraint '{"rule":"keep Top50->Top10 + alpha=0.245 + money_q90 unchanged","reason":"the base pipeline is already locked"}' \
  --constraint '{"rule":"only test small-weight additive fusion","reason":"full replacement has been ruled out"}' \
  --success-condition "find a beta point that improves target gain without hurting toxic/p10/recent6" \
  --abort-condition "archive the signal if no beta point preserves all three guardrails"

case step add --id C-20260320-01 --title "prepare locked-base score inputs"
case step add --id C-20260320-01 --title "scan beta * f_q80_fail_tsum05"
case step add --id C-20260320-01 --title "decide keep-or-archive"
```

## 非目标

本工具当前不解决：

- 多主线并行协作
- 任意分支树
- 通用任务管理
- 替代 git 历史
- 替代代码搜索工具

其职责只是：

- 把一次探索组织成“目标恒定、打法清楚、步列可回、护栏可见、出口明确”之主案

## 交付检查单

- 是否 `goal` 在一案内不可变
- 是否 `goal_constraints` 与 `direction.constraints` 皆可带 `reason`
- 是否 `redirect` 强制新方向之 `constraints/success_condition/abort_condition`
- 是否 `steps` 绑定当前 `direction`，且可重排
- 是否 `current/resume` 恒显 `goal + constraints + current_direction + current_step`
