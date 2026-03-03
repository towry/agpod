# Flow 文档驱动任务图实现规范（原型 v0）

## 1. 目标与边界

### 1.1 目标
- 用文档本身作为任务关系的事实来源（source of truth）。
- 将持久化索引放在用户目录，避免多人协作冲突。
- 支持从文档结构和元数据反推任务图（可重建）。
- 支持显式 session 上下文（`-s`），避免隐式 active task。

### 1.2 非目标（v0 不做）
- 不做远程同步。
- 不做分布式锁。
- 不做复杂权限系统。
- 不强依赖已有 `kiro` 工作流（`kiro` 可被 deprecated）。

## 2. 术语
- `Root Task`: 主任务节点。
- `Leaf Task`: 当前执行链路上的最深子任务。
- `Fork Task`: 从当前任务分叉出来的子任务。
- `Doc Node`: 文档节点（需求、设计、实现记录、问题分析等）。
- `Graph Cache`: 本地派生索引，可删除后重建。
- `Repo Identity`: 仓库唯一标识（`repo-id`）。

## 3. 本地存储布局（用户目录）

持久化数据放在用户目录：

```text
$XDG_DATA_HOME/agpod/flow/repos/<repo-id>/
  repo-meta.json
  graph.json
  diagnostics.json
```

若 `XDG_DATA_HOME` 未设置，则使用：

```text
~/.local/share/agpod/flow/repos/<repo-id>/
```

### 3.1 文件职责
- `repo-meta.json`: 记录 `repo-id` 来源、标准化输入、生成时间。
- `graph.json`: 从文档推理出的任务图缓存。
- `diagnostics.json`: 推理冲突、字段缺失、非法关系等诊断信息。

### 3.2 会话运行态（不持久化）

`active task` 不写入 `$XDG_DATA_HOME`。会话运行态仅保存在运行时目录：

```text
$XDG_RUNTIME_DIR/agpod/flow/sessions/<session-id>.json
```

若 `XDG_RUNTIME_DIR` 未设置，使用系统临时目录：

```text
/tmp/agpod/flow/sessions/<session-id>.json
```

约束：
- 会话文件是临时态，不做长期持久化保证。
- 进程重启、系统重启或清理临时目录后，会话可能失效。
- 失效后必须重新 `session new` 并 `focus`。

## 4. repo-id 规范

`repo-id` 必须稳定、可复现、与仓库重命名/移动尽量解耦。

### 4.1 输入来源（唯一）
`repo-id` 仅允许由 Git remote URL 推导，默认使用：
- `git remote get-url origin`

若不存在 `origin`，可按固定顺序尝试：
1. 远程名为 `upstream`
2. 其余 remotes 按字典序取第一个

### 4.1.1 Fail-fast 规则（无 fallback）
- 禁止使用仓库路径（realpath）作为 `repo-id` 来源。
- 禁止使用 CLI/env/git config 手工指定 `repo-id`。
- 若无法获取任何 Git remote URL，工具必须直接报错并退出。
- 报错信息应明确提示：先配置 remote（建议 `origin`）。

### 4.2 标准化规则
对来源字符串做以下标准化：
- 去掉前后空白。
- 若是 git URL：统一为 `host/full_path` 形式（支持 subgroup）。
- 去掉 `.git` 后缀。
- host 转小写（`GitHub.com` -> `github.com`）。
- ssh/https 统一归一：
  - `git@github.com:Org/Repo.git`
  - `https://github.com/Org/Repo.git`
  归一后都视为 `github.com/org/repo`。
  - `git@gitlab.company.com:group/subgroup/repo.git`
  - `https://gitlab.company.com/group/subgroup/repo.git`
  归一后都视为 `gitlab.company.com/group/subgroup/repo`。

### 4.3 生成算法（建议）
- `source = "v1:" + <normalized_source>`
- `repo_id = hex(sha256(source))[0..16]`

示例（仅示意）：
- normalized source: `github.com/towry/agpod`
- repo-id: `8f31a9c2b0d4e7f1`

### 4.4 可读别名（可选）
可同时存一个 `repo_label`，如 `github.com/towry/agpod`，用于日志展示；索引目录仍以 `repo-id` 为准。

## 5. 文档元数据规范（事实来源）

每个被纳入 Flow 的文档必须使用 YAML frontmatter：

```yaml
---
doc_id: D-20260303-001
doc_type: design
task_id: T-001.2
root_task_id: T-001
parent_task_id: T-001.1
created_at: 2026-03-03T11:20:00Z
updated_at: 2026-03-03T12:05:00Z
status: in_progress
branch: feature/t-001-2-auth-fix
agent_id: impl-a
---
```

### 5.1 字段定义
- `doc_id` 必填：文档唯一 ID，由 `agpod flow` 自动生成并写回 frontmatter。
- `doc_type` 必填：`requirement | design | task | impl | bug | decision | note | summary`。
- `task_id` 必填：该文档归属任务。
- `root_task_id` 建议必填：主任务 ID。
- `parent_task_id` 可选：当前任务的父任务。
- `created_at` / `updated_at` 必填：RFC3339 UTC 时间。
- `status` 必填：`todo | in_progress | blocked | done | archived`。
- `branch` 可选：文档产出时关联分支。
- `agent_id` 可选：产出文档的 agent 标识。

### 5.2 时间字段规则
- 一律 UTC（`Z` 后缀）。
- `updated_at` 必须 >= `created_at`。
- 工具更新文档时自动刷新 `updated_at`。

### 5.3 `doc_id` 生成与写回规则
- 人工不需要手动填写 `doc_id`。
- `agpod flow doc add --path <file> ...` 或 `agpod flow doc init --path <file> ...` 必须负责生成并写回 `doc_id`。
- 若文档未通过上述命令初始化，`rebuild` 发现缺失 `doc_id` 必须 fail fast。
- 报错信息应给出修复建议命令，例如：
  - `agpod flow -s <id> doc add --path <file> --task <task-id> --type <doc-type>`
  - `agpod flow doc init --path <file> --task <task-id> --type <doc-type>`

## 6. graph.json 结构（本地缓存）

```json
{
  "version": 1,
  "repo_id": "8f31a9c2b0d4e7f1",
  "generated_at": "2026-03-03T12:30:00Z",
  "tasks": {
    "T-001": {
      "task_id": "T-001",
      "root_task_id": "T-001",
      "parent_task_id": null,
      "children": ["T-001.1"],
      "status": "in_progress"
    }
  },
  "docs": {
    "D-20260303-001": {
      "doc_id": "D-20260303-001",
      "doc_type": "design",
      "task_id": "T-001.2",
      "path": "docs/flow/design-auth.md",
      "created_at": "2026-03-03T11:20:00Z",
      "updated_at": "2026-03-03T12:05:00Z"
    }
  },
  "edges": [
    { "type": "parent_child", "from": "T-001", "to": "T-001.1" },
    { "type": "doc_task", "from": "D-20260303-001", "to": "T-001.2" }
  ]
}
```

说明：`graph.json` 可以删除并重建，不是唯一事实源。

## 7. Session 模型（显式上下文，无隐式 active）

```json
{
  "version": 1,
  "session_id": "S-8b4c2f",
  "repo_id": "8f31a9c2b0d4e7f1",
  "active_task_id": "T-001.2",
  "created_at": "2026-03-03T12:35:00Z",
  "updated_at": "2026-03-03T12:40:00Z"
}
```

### 7.1 强制显式规则
- 所有依赖任务上下文的命令必须显式传 `-s <session-id>`。
- 不允许使用“当前默认 active task”。
- 不允许自动恢复“上次任务”。

### 7.2 约束
- 会话创建后默认 `active_task_id = null`。
- 当 `active_task_id = null` 时，`fork/parent/doc add` 等命令必须直接报错。
- `focus --task <id>` 是设置会话上下文的唯一入口。

### 7.3 友好报错（Fail-fast）
示例：
- `No active task in session S-8b4c2f`
- `Run: agpod flow -s S-8b4c2f focus --task T-001`
- `Or pass --task explicitly if the command supports it`

## 8. 从文档重建图（Rebuild）

### 8.1 仓库级文档目录配置

为最大程度遵循仓库既有文档结构，扫描目录必须支持仓库级配置。

配置文件位置（仓库根目录）：
- `.agpod.flow.toml`

配置结构（v0）：

```toml
[flow.docs]
roots = ["docs", "llm", "notes", "specs/architecture"]
include_globs = ["**/*.md", "**/*.mdx"]
exclude_globs = ["**/node_modules/**", "**/.git/**", "**/dist/**"]
frontmatter_required = true
follow_symlinks = false
```

字段说明：
- `roots`: 文档扫描根目录列表，路径相对仓库根目录。
- `include_globs`: 文件匹配白名单（默认 `**/*.md`, `**/*.mdx`）。
- `exclude_globs`: 排除规则（用于跳过构建产物和第三方目录）。
- `frontmatter_required`: 是否要求 frontmatter；默认 `true`。在 v0 中缺失 frontmatter 必须 fail fast。
- `follow_symlinks`: 是否跟随软链接；默认 `false`，避免循环扫描。

### 8.2 配置加载优先级
1. 仓库根目录 `.agpod.flow.toml`。
2. 若缺失该文件，则使用内置默认值。

说明：`flow` 的文档扫描目录由仓库声明，不依赖用户本地目录结构。

### 8.3 输入
- 先读取仓库配置中的 `[flow.docs]`。
- 再基于 `roots + include_globs - exclude_globs` 计算候选文档集合。

### 8.4 推理优先级（高 -> 低）
1. 文档 frontmatter 显式字段。
2. Git commit trailers（可选增强）：`Task-Id`、`Root-Task-Id`、`Parent-Task-Id`。
3. 目录层级推断（例如 `T-001/T-001.2/`）。
4. 文件名约定推断（最后兜底）。

### 8.5 Rebuild 步骤
1. 扫描候选文档并解析 frontmatter。
2. 若文档缺失 frontmatter 且 `frontmatter_required=true`，立即报错退出（fail fast）。
3. 若 frontmatter 缺失 `doc_id`，立即报错退出（fail fast），并提示使用 `doc add` 或 `doc init` 修复。
4. 校验字段合法性（时间、status、ID 格式）。
5. 构建 task/doc 节点。
6. 根据 `task_id/parent_task_id/root_task_id` 建边。
7. 推断缺失字段并写入诊断。
8. 输出 `graph.json` + `diagnostics.json`。

### 8.6 冲突处理策略
- 同一 `doc_id` 对应多个文件：标记冲突，按 `updated_at` 最新者为主。
- `task_id` 与路径推断冲突：以 frontmatter 为准，记录 warning。
- 父任务不存在：标记 orphan，不自动猜测父节点。

## 9. 与 Git Stack Commits 对齐

建议每个 commit 附加 trailer（非强制）：

```text
Task-Id: T-001.2
Root-Task-Id: T-001
Parent-Task-Id: T-001.1
Doc-Refs: D-20260303-001,D-20260303-007
```

规则建议：
- 一个 commit 只对应一个 `task_id`。
- 一个 stack layer 对应一个任务节点。
- merge/rebase 后可通过 trailer 反查任务关系。

## 10. CLI 语义（原型）

### 10.1 无状态查询命令（不需要 `-s`）
- `agpod flow rebuild`：从文档重建 `graph.json`。
- `agpod flow recent [-n 10] [--days 14] [--json]`：列出最近任务候选，供快速聚焦。
- `agpod flow tree [--root <task-id>] [--json]`：打印任务树和文档挂载（ASCII 树形图，使用 `termtree` 渲染）。

`tree` 输出示例（ASCII 树形图）：

```text
▶ T-001 [in_progress]
├── 📄 D-20260303-005 (docs/requirement.md) [requirement]
├── ✓ T-001.1 [done]
│   ├── 📄 D-20260303-002 (docs/impl-auth.md) [impl]
│   └── ○ T-001.1.1 [todo]
├── ▶ T-001.2 [in_progress]
│   ├── 📄 D-20260303-001 (docs/design-auth.md) [design]
│   └── 📄 D-20260303-003 (docs/impl-fix.md) [impl]
└── ⏸ T-001.3 [blocked]
    └── 📄 D-20260303-004 (docs/bug.md) [bug]
```

状态图标：`✓` done / `▶` in_progress / `⏸` blocked / `○` todo / `⊘` archived / `?` unknown

`recent` 评分证据来源（高 -> 低）：
1. Git commit trailers（`Task-Id` / `Root-Task-Id` / `Parent-Task-Id`）
2. 文档 frontmatter `updated_at`
3. 文档文件在 Git 历史中的最近修改时间

`recent` 输出字段建议：
- `task_id`
- `last_seen_at`
- `score`
- `evidence`
- `suggested_command`（例如 `agpod flow -s <id> focus --task T-001.2`）

### 10.1.1 `recent` 排序算法（伪代码）

```text
input:
  now
  days_window (default: 14)
  limit_n (default: 10)
  evidence list from:
    - git commit trailers
    - doc frontmatter updated_at
    - git file history mtime

constants:
  WEIGHT_COMMIT_TRAILER = 100
  WEIGHT_DOC_UPDATED_AT = 60
  WEIGHT_GIT_FILE_MTIME = 30
  DECAY_HALF_LIFE_DAYS = 7

function time_decay(ts):
  age_days = max(0, (now - ts).days)
  return 0.5 ^ (age_days / DECAY_HALF_LIFE_DAYS)

map task_scores = {}
map task_last_seen = {}
map task_evidence = {}

for each evidence_item:
  if evidence_item.timestamp older than days_window:
    continue

  base_weight = weight_by_type(evidence_item.type)
  delta = base_weight * time_decay(evidence_item.timestamp)

  task_id = evidence_item.task_id
  task_scores[task_id] += delta
  task_last_seen[task_id] = max(task_last_seen[task_id], evidence_item.timestamp)
  append evidence_item.summary to task_evidence[task_id]

result = tasks(task_scores) mapped to:
  {
    task_id,
    score = round(task_scores[task_id], 4),
    last_seen_at = task_last_seen[task_id],
    evidence = top_k(task_evidence[task_id], 5),
    suggested_command = "agpod flow -s <id> focus --task " + task_id
  }

sort result by:
  1) score desc
  2) last_seen_at desc
  3) task_id asc

return first limit_n items
```

实现约束：
- 同一 commit / 同一文档证据只计一次（去重）。
- 没有任何证据时返回空列表，不做隐式推断。
- `recent` 是只读查询，不创建 session，不修改 active task。

### 10.1.2 `recent` 实现性能要求

证据收集必须最小化子进程调用和文件 I/O：
- Git commit trailers：**单次** `git log` 调用，使用 `%x00` 分隔符避免歧义。
- Doc frontmatter `updated_at`：**单次**文档扫描，复用已读取内容。
- Git file mtime：**单次** `git log --name-only` 批量获取所有最近修改文件，与文档扫描结果交叉引用。
- 禁止每个文件单独调用 `git log`（O(N) 进程开销不可接受）。

### 10.2 会话生命周期命令
- `agpod flow session new`：创建会话，返回 `session-id`。
- `agpod flow session list`：列出当前 repo 的会话。
- `agpod flow session close -s <id>`：关闭会话。

### 10.3 有状态执行命令（必须 `-s`）
- `agpod flow -s <id> status`：显示该 session 的 active task。
- `agpod flow -s <id> focus --task <id>`：设置会话焦点。
- `agpod flow -s <id> fork --to <new-task-id> [--from <parent-task-id>] [--no-switch]`：分叉子任务。
  - `--from`：指定父任务（默认为 session active task），支持从任意任务分叉。
  - `--no-switch`：创建后不切换 focus（placeholder/TODO 模式）。
  - 不带 `--no-switch` 时默认切换到新子任务。
- `agpod flow -s <id> parent`：回到父任务。
- `agpod flow -s <id> doc add --path <file> [--task <id>]`：挂载文档到任务（默认使用 session active task）。
- `agpod flow -s <id> doc init --path <file> --task <id> --type <doc-type>`：初始化 frontmatter（包含自动生成 `doc_id`）。

命令规则：
- `focus/status/fork/parent/doc add` 这类上下文命令必须带 `-s`。
- `recent/rebuild/tree` 为无状态查询命令，不依赖 session。

### 10.4 `fork` 使用场景

```bash
# 场景 1：从当前任务分叉并切换（默认行为）
agpod flow -s S-xxx fork --to T-001.3

# 场景 2：记录想法但不离开当前任务（placeholder/TODO）
agpod flow -s S-xxx fork --to T-001.3 --no-switch
# → Created T-001.3 (parent: T-001.2), staying on T-001.2

# 场景 3：在父任务下创建平级任务，不离开当前任务
agpod flow -s S-xxx fork --to T-001.4 --from T-001 --no-switch
# → Created T-001.4 (parent: T-001), staying on T-001.2

# 场景 4：指定任意父任务并切换过去
agpod flow -s S-xxx fork --to T-002.1 --from T-002
# → Created T-002.1 (parent: T-002), switched to T-002.1
```

### 10.5 Summary 文档（规划中）

每个任务可关联一个 `doc_type: summary` 的文档，作为 agent 理解任务的入口：
- 路径约定：`<first-doc-root>/flow/<task_id>/SUMMARY.md`，路径可预测，agent 无需搜索。
- 唯一性约束：一个 task_id 最多一份 summary，`rebuild` 时校验。
- `focus` 自动创建：当 `focus --task T-001` 时，若 SUMMARY.md 不存在，自动 init。
- `summary sync`：agent 编辑完 SUMMARY.md 后，刷新 `updated_at`。
- `summary path`：输出当前 active task 的 summary 文件路径（供 agent 读取）。

Agent 工作循环：

```text
focus T-001 → 读 SUMMARY.md → 干活 → 写回 SUMMARY.md → summary sync
```

## 11. 默认值（Default-first）

为减少交互阻塞，v0 默认：
- 若仓库未提供 `.agpod.flow.toml`，扫描目录默认 `llm/`, `docs/`, `notes/`。
- 默认 `include_globs=["**/*.md", "**/*.mdx"]`。
- 默认 `exclude_globs=["**/node_modules/**", "**/.git/**", "**/dist/**"]`。
- 默认 `frontmatter_required=true`。
- 默认 `follow_symlinks=false`。
- `recent` 默认 `-n 10 --days 14`。
- `session new` 后默认 `active_task_id=null`。
- `fork` 成功后默认切换到新子任务（`--no-switch` 可阻止切换）。
- `graph.json` 每次 `rebuild` 全量重算。

如需调整，再在后续版本引入配置项。

## 12. 实现里程碑

1. ✅ `repo-id` 计算与本地目录管理（`repo_id.rs`, `storage.rs`）。
2. ✅ frontmatter 解析（`gray_matter` crate）+ 文档扫描器（`globset` crate）。
3. ✅ graph rebuild + diagnostics（`graph.rs`）。
4. ✅ session 生命周期管理（new/list/close/focus）。
5. ✅ `status/tree/focus/parent/fork` 命令 + `recent` 评分 + ASCII tree 渲染（`termtree` crate）。
6. ⬜ Summary 文档管理（`summary path/sync`，`focus` 自动创建）。
