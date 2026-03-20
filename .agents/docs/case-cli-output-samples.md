# Case CLI 输出样例

本文按最终口径示例：

- `goal`：一案不变
- `goal_constraints`：全案级护栏，附 `reason`
- `direction`：当前主打法
- `direction.constraints`：当前打法护栏，附 `reason`
- `steps`：隶于当前 `direction` 之执行步列
- `success_condition / abort_condition`：此方向之出口

## 输出总原则

CLI 输出分二类：

- **状态面板**：`open/current/show/resume/recall/list`
- **事件回执**：`record/decide/redirect/step/close/abandon`

统一结构建议：

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
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
    "success_condition": "find a beta point that improves target gain without hurting toxic/p10/recent6",
    "abort_condition": "archive the signal if no beta point preserves all three guardrails"
  },
  "steps": {
    "current": null,
    "pending": []
  },
  "next": {
    "suggested_command": "step add",
    "why": "the direction is set but no execution step has been added yet"
  }
}
```

规则：

- 不返回伪完整命令串
- `goal` 与两层 `constraints` 尽量恒显
- 若场景在推进中，`current_step` 须可见
- `next.suggested_command` 只提示动作类别，不替 agent 伪造未知参数

---

## `case open`

### 成功

```
$ case open \
    --goal "determine whether q80_fail_tsum05 deserves entry into the locked production stack" \
    --goal-constraint '{"rule":"judge by terminal test metrics","reason":"validation-only improvement is insufficient"}' \
    --direction "test lightweight additive fusion on top of the locked base model" \
    --success-condition "find a beta point that improves target gain without hurting toxic/p10/recent6" \
    --abort-condition "archive the signal if no beta point preserves all three guardrails"

Case opened.

  id:         C-20260320-01
  goal:       determine whether q80_fail_tsum05 deserves entry into the locked production stack
  direction:  test lightweight additive fusion on top of the locked base model

Next:
  step add  — add the first execution step under the current direction
```

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
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
    "constraints": [],
    "success_condition": "find a beta point that improves target gain without hurting toxic/p10/recent6",
    "abort_condition": "archive the signal if no beta point preserves all three guardrails"
  },
  "steps": {
    "current": null,
    "pending": []
  },
  "context": {
    "active_case_id": "C-20260320-01",
    "current_direction_seq": 1
  },
  "next": {
    "suggested_command": "step add",
    "why": "the case is open but the execution queue is still empty"
  }
}
```

---

## `case current`

### 正常推进

```
$ case current

C-20260320-01  [open]

  goal:  determine whether q80_fail_tsum05 deserves entry into the locked production stack

  goal_constraints:
    - judge by terminal test metrics
      because: validation-only improvement is insufficient

  current_direction:
    test lightweight additive fusion on top of the locked base model

  direction_constraints:
    - keep Top50->Top10 + alpha=0.245 + money_q90 unchanged
      because: the base pipeline is already locked
    - only test small-weight additive fusion
      because: full replacement has already been ruled out

  current_step:
    S-002  scan beta * f_q80_fail_tsum05

  pending_steps:
    - S-003  summarize keep-or-archive decision

  last_fact:
    oracle and zeus both reject the full replacement path; only the minimal additive trial remains

  success_condition:
    find a beta point that improves target gain without hurting toxic/p10/recent6

  abort_condition:
    archive the signal if no beta point preserves all three guardrails

  health: on_track

Next:
  record  — capture the beta scan findings under the active step
```

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
    "goal_constraints": [
      {
        "rule": "judge by terminal test metrics",
        "reason": "validation-only improvement is insufficient"
      }
    ],
    "status": "open"
  },
  "direction": {
    "seq": 2,
    "summary": "test lightweight additive fusion on top of the locked base model",
    "constraints": [
      {
        "rule": "keep Top50->Top10 + alpha=0.245 + money_q90 unchanged",
        "reason": "the base pipeline is already locked"
      },
      {
        "rule": "only test small-weight additive fusion",
        "reason": "full replacement has already been ruled out"
      }
    ],
    "success_condition": "find a beta point that improves target gain without hurting toxic/p10/recent6",
    "abort_condition": "archive the signal if no beta point preserves all three guardrails"
  },
  "steps": {
    "current": {
      "id": "S-002",
      "order": 2,
      "title": "scan beta * f_q80_fail_tsum05",
      "status": "active"
    },
    "pending": [
      {
        "id": "S-003",
        "order": 3,
        "title": "summarize keep-or-archive decision",
        "status": "pending"
      }
    ]
  },
  "last_fact": "oracle and zeus both reject the full replacement path; only the minimal additive trial remains",
  "health": "on_track",
  "context": {
    "active_case_id": "C-20260320-01",
    "current_direction_seq": 2
  },
  "next": {
    "suggested_command": "record",
    "why": "the active step is collecting evidence"
  }
}
```

### 疑似打墙

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
    "goal_constraints": [
      {
        "rule": "judge by terminal test metrics",
        "reason": "validation-only improvement is insufficient"
      }
    ],
    "status": "open"
  },
  "direction": {
    "seq": 2,
    "summary": "expand beta grid around the additive fusion candidate",
    "constraints": [
      {
        "rule": "do not touch the locked base pipeline",
        "reason": "this trial is only about the additive signal"
      }
    ],
    "success_condition": "find a stable beta point worth keeping",
    "abort_condition": "abort the additive path if repeated grid expansion yields no viable point"
  },
  "steps": {
    "current": {
      "id": "S-004",
      "order": 4,
      "title": "expand the beta grid",
      "status": "active"
    },
    "pending": []
  },
  "last_fact": "six more beta runs changed nothing material",
  "health": "looping",
  "warning": "the current step has repeated without producing a decision or redirect",
  "next": {
    "suggested_command": "redirect",
    "why": "the current direction appears to have plateaued"
  }
}
```

---

## `case redirect`

### 改道并带新护栏

```
$ case redirect --id C-20260320-01 \
    --direction "test lightweight additive fusion on top of the locked base model" \
    --reason "full replacement failed in terminal metrics" \
    --context "oracle and zeus both reject continuing the replacement path; only the minimal additive trial remains" \
    --constraint '{"rule":"keep Top50->Top10 + alpha=0.245 + money_q90 unchanged","reason":"the base pipeline is already locked"}' \
    --constraint '{"rule":"only test small-weight additive fusion","reason":"full replacement has already been ruled out"}' \
    --success-condition "find a beta point that improves target gain without hurting toxic/p10/recent6" \
    --abort-condition "archive the signal if no beta point preserves all three guardrails"

Direction updated.

  from:  evaluate full replacement of the base model
  to:    test lightweight additive fusion on top of the locked base model

Next:
  step add  — build the execution queue for the new direction
```

```json
{
  "ok": true,
  "event": {
    "seq": 5,
    "entry_type": "redirect",
    "from_direction": "evaluate full replacement of the base model",
    "to_direction": "test lightweight additive fusion on top of the locked base model",
    "reason": "full replacement failed in terminal metrics",
    "context": "oracle and zeus both reject continuing the replacement path; only the minimal additive trial remains"
  },
  "direction": {
    "seq": 2,
    "summary": "test lightweight additive fusion on top of the locked base model",
    "constraints": [
      {
        "rule": "keep Top50->Top10 + alpha=0.245 + money_q90 unchanged",
        "reason": "the base pipeline is already locked"
      },
      {
        "rule": "only test small-weight additive fusion",
        "reason": "full replacement has already been ruled out"
      }
    ],
    "success_condition": "find a beta point that improves target gain without hurting toxic/p10/recent6",
    "abort_condition": "archive the signal if no beta point preserves all three guardrails"
  },
  "steps": {
    "current": null,
    "pending": []
  },
  "context": {
    "active_case_id": "C-20260320-01",
    "current_direction_seq": 2
  },
  "next": {
    "suggested_command": "step add",
    "why": "the new direction needs a fresh execution queue"
  }
}
```

### 缺出口条件，应拒绝

```json
{
  "ok": false,
  "error": "missing_direction_exit_conditions",
  "message": "`redirect` requires both `success_condition` and `abort_condition`",
  "next": {
    "suggested_command": "redirect",
    "why": "a new direction must define both how to win and how to stop"
  }
}
```

---

## `case step add`

### 新增一步

```
$ case step add --id C-20260320-01 --title "scan beta * f_q80_fail_tsum05"

Step added.

  step_id:  S-002
  order:    2
  title:    scan beta * f_q80_fail_tsum05

Next:
  step start  — activate the step when ready to execute it
```

```json
{
  "ok": true,
  "step": {
    "id": "S-002",
    "order": 2,
    "title": "scan beta * f_q80_fail_tsum05",
    "status": "pending"
  },
  "context": {
    "active_case_id": "C-20260320-01",
    "current_direction_seq": 2
  },
  "next": {
    "suggested_command": "step start",
    "why": "the step exists but is not active yet"
  }
}
```

## `case step move`

### 半路插修阻断事项

```
$ case step move --id C-20260320-01 --step-id S-004 --before S-002

Step reordered.

  moved:   S-004  fix the export script blocking beta scan
  before:  S-002  scan beta * f_q80_fail_tsum05

Next:
  step start  — switch to the blocker-fix step first
```

```json
{
  "ok": true,
  "steps": {
    "current": {
      "id": "S-001",
      "order": 1,
      "title": "prepare locked-base score inputs",
      "status": "done"
    },
    "pending": [
      {
        "id": "S-004",
        "order": 2,
        "title": "fix the export script blocking beta scan",
        "status": "pending",
        "reason": "the scan cannot continue until the blocker is removed"
      },
      {
        "id": "S-002",
        "order": 3,
        "title": "scan beta * f_q80_fail_tsum05",
        "status": "pending"
      }
    ]
  },
  "next": {
    "suggested_command": "step start",
    "why": "the reordered blocker-fix step should now run first"
  }
}
```

此例要旨：

- 临时插修之事，通常只改 `steps`
- 未必构成 `redirect`

---

## `case record`

### 记录证据

```json
{
  "ok": true,
  "event": {
    "seq": 7,
    "entry_type": "record",
    "kind": "evidence",
    "summary": "beta=0.08 preserves toxic/p10/recent6 but does not improve target gain",
    "files": [
      "cache/ml_reports/q80_fail_tsum05_beta_scan_20260320.csv"
    ]
  },
  "steps": {
    "current": {
      "id": "S-002",
      "title": "scan beta * f_q80_fail_tsum05",
      "status": "active"
    }
  },
  "next": {
    "suggested_command": "record",
    "why": "the scan step is still gathering evidence"
  }
}
```

---

## `case decide`

### 锁定取舍

```json
{
  "ok": true,
  "event": {
    "seq": 8,
    "entry_type": "decision",
    "summary": "do not expand beyond small additive beta scan",
    "reason": "the locked baseline must remain interpretable and comparable"
  },
  "next": {
    "suggested_command": "step done",
    "why": "the current decision narrows the step queue rather than changing direction"
  }
}
```

---

## `case show`

### 完整视图

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
    "goal_constraints": [
      {
        "rule": "judge by terminal test metrics",
        "reason": "validation-only improvement is insufficient"
      }
    ],
    "status": "open"
  },
  "direction_history": [
    {
      "seq": 1,
      "summary": "evaluate full replacement of the base model",
      "constraints": [],
      "success_condition": "replacement outperforms the locked baseline on terminal metrics",
      "abort_condition": "drop the path if terminal test metrics regress"
    },
    {
      "seq": 2,
      "summary": "test lightweight additive fusion on top of the locked base model",
      "constraints": [
        {
          "rule": "keep Top50->Top10 + alpha=0.245 + money_q90 unchanged",
          "reason": "the base pipeline is already locked"
        },
        {
          "rule": "only test small-weight additive fusion",
          "reason": "full replacement has already been ruled out"
        }
      ],
      "success_condition": "find a beta point that improves target gain without hurting toxic/p10/recent6",
      "abort_condition": "archive the signal if no beta point preserves all three guardrails"
    }
  ],
  "steps_by_direction": {
    "2": [
      {
        "id": "S-001",
        "order": 1,
        "title": "prepare locked-base score inputs",
        "status": "done"
      },
      {
        "id": "S-002",
        "order": 2,
        "title": "scan beta * f_q80_fail_tsum05",
        "status": "active"
      },
      {
        "id": "S-003",
        "order": 3,
        "title": "summarize keep-or-archive decision",
        "status": "pending"
      }
    ]
  }
}
```

---

## `case resume`

### 中断后接管

```
$ case resume --id C-20260320-01

Resume brief:

  goal:
    determine whether q80_fail_tsum05 deserves entry into the locked production stack

  goal_constraints:
    - judge by terminal test metrics
      because: validation-only improvement is insufficient

  current_direction:
    test lightweight additive fusion on top of the locked base model

  direction_constraints:
    - keep Top50->Top10 + alpha=0.245 + money_q90 unchanged
      because: the base pipeline is already locked
    - only test small-weight additive fusion
      because: full replacement has already been ruled out

  current_step:
    S-002  scan beta * f_q80_fail_tsum05

  next_pending_steps:
    - S-003  summarize keep-or-archive decision

  last_decision:
    do not expand beyond small additive beta scan

  last_evidence:
    beta=0.08 preserves toxic/p10/recent6 but does not improve target gain

  success_condition:
    find a beta point that improves target gain without hurting toxic/p10/recent6

  abort_condition:
    archive the signal if no beta point preserves all three guardrails
```

```json
{
  "ok": true,
  "resume": {
    "case_id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
    "goal_constraints": [
      {
        "rule": "judge by terminal test metrics",
        "reason": "validation-only improvement is insufficient"
      }
    ],
    "current_direction": "test lightweight additive fusion on top of the locked base model",
    "direction_constraints": [
      {
        "rule": "keep Top50->Top10 + alpha=0.245 + money_q90 unchanged",
        "reason": "the base pipeline is already locked"
      },
      {
        "rule": "only test small-weight additive fusion",
        "reason": "full replacement has already been ruled out"
      }
    ],
    "current_step": {
      "id": "S-002",
      "title": "scan beta * f_q80_fail_tsum05"
    },
    "next_pending_steps": [
      {
        "id": "S-003",
        "title": "summarize keep-or-archive decision"
      }
    ],
    "last_decision": "do not expand beyond small additive beta scan",
    "last_evidence": "beta=0.08 preserves toxic/p10/recent6 but does not improve target gain",
    "success_condition": "find a beta point that improves target gain without hurting toxic/p10/recent6",
    "abort_condition": "archive the signal if no beta point preserves all three guardrails"
  },
  "next": {
    "suggested_command": "record",
    "why": "the resumed active step is still in evidence collection mode"
  }
}
```

---

## `case close`

### 达成目标

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
    "status": "closed",
    "close_summary": "a viable additive beta point was found and accepted without breaking the locked guardrails"
  },
  "next": {
    "suggested_command": "open",
    "why": "the repository now has no active case"
  }
}
```

## `case abandon`

### 止损归库

```json
{
  "ok": true,
  "case": {
    "id": "C-20260320-01",
    "goal": "determine whether q80_fail_tsum05 deserves entry into the locked production stack",
    "status": "abandoned",
    "abandon_summary": "no beta point preserved toxic/p10/recent6 simultaneously, so the signal was archived"
  },
  "next": {
    "suggested_command": "open",
    "why": "the previous goal has been explicitly abandoned"
  }
}
```

---

## 设计结论

此版取舍如下：

1. `goal` 恒定，作全案主锚
2. `goal_constraints` 与 `direction.constraints` 皆须可解释
3. `steps` 绑定当前 `direction`，用于承接“半路插修后归来”
4. `success_condition / abort_condition` 取代零散之“停/杀”字段
5. `current/resume` 专为夜间接力，不令 agent 自长文中猜主线
