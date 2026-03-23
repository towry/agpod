# agpod case 工具使用情况反馈

> 基于 2026-03-23 对全局 case 数据库的快照分析。

## 一、数据概况

| 仓库 | case 总数 | closed | abandoned | open | 时间跨度 |
|------|----------|--------|-----------|------|---------|
| agpod | 5 | 3 | 0 | 1 | 3/20–3/21 |
| snowball | 66 | 62 | 3 | 1 | 3/21–3/23（约 2 日） |

snowball 为高频使用场景，agent 约每 30 分钟 open/close 一个 case。

## 二、问题一：是否存在随意创建 goal 而不使用 redirect

**存在，且较显著。**

66 个 case 中至少 15 个 goal 含「继续」「推近」「进一步」「逼近」等延续措辞，本质服务于同一大目标（"使主候选逼近真实决策门槛"），例如：

- `C-936a1510` "使主候选……继续逼近……择定下一步最高杠杆工作"
- `C-b813c923` "继续将主候选……推近真实决策门槛"
- `C-1ad375ac` "继续将主候选……推近真实决策门槛，先修……"
- `C-97a889a4` "使已锁主候选……继续逼近……"

此类本可以一个 case + 多次 redirect 表达方向切换，而非每次 open 新 case。

### 可能成因

1. **跨 session 丢上下文**——agent 新 session 未先 `resume`，倾向重新 open。
2. **goal immutable 约束**——agent 觉得 goal 措辞不完全匹配新阶段，宁可新开。
3. **`is_drift_from_goal` 护栏过严**——agent 面对模糊边界时回避 redirect 以免被拒。

### 正面信号

agpod 仓 `C-20260321-01` 正确使用了 redirect（从 macOS 签名问题转向 case 时间语义修复），说明机制可用，但在高频场景下未被充分利用。

## 三、问题二：工具能否轻易被用来 recall

**部分达到，但有明显缺口。**

### 现有能力评估

| 命令 | 作用 | 评价 |
|------|------|------|
| `recall <query>` | 子串匹配搜索 | 能命中具体术语如 "shadow replay"，但不支持语义搜索，问"接下来做什么"返回空 |
| `resume` | 输出 open case 的 goal + direction + steps + next action | **最有价值的入口**，新 session 首调即知当前进度 |
| `current` | 紧凑导航面板 | 适合频繁调用 |
| `show --id <id>` | 完整 entries / direction_tree | 适合深度回顾 |
| `list` | 所有 case 标题与状态 | 66 个 case 一次输出，缺分页/过滤 |

### 关键不足

1. **`recall` 仅子串匹配**——无法回答"上次关于 financial coverage 的结论是什么"，agent 须精确猜到关键词。
2. **`list` 无过滤/分页**——66 case 一次输出，context window 压力大。缺 `--status open`、`--recent 7d` 等过滤。
3. **跨 session recall 路径未闭环**——理想流程：`resume → redirect`；实际：`list → 没匹配 → 新 open`，致碎片化。
4. **MCP schema 描述未强制引导**——agent 不知道应先 `resume` 再决策。

### 结论

`resume` 是高价值命令，但 agent 是否真正用它避免迷失，取决于 prompt/instruction 层是否强制新 session 先 `resume` 再决定 redirect 或 open。工具提供了机制，使用习惯引导还需强化。

## 四、本次新增：`--repo-root` 参数

为 `agpod case` CLI 新增全局 `--repo-root` 参数，可在非目标仓目录下查询任意仓的 case 数据。改动文件：

- `crates/agpod-case/src/cli.rs`
- `crates/agpod-case/src/commands.rs`
- `crates/agpod-case/src/lib.rs`
- `crates/agpod-mcp/src/lib.rs`

## 附录：研究过程所用命令

### 复制数据库（避锁）

```bash
# 数据库实际位置（macOS）
ls ~/Library/Application\ Support/agpod/case.db

# 复制到 /tmp 避免与运行中 agent 抢锁
cp -r ~/Library/Application\ Support/agpod/case.db /tmp/agpod-case-db-copy
```

### 查看全局 case 列表

```bash
# 列出当前仓（agpod）的 cases
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case list

# 列出指定仓（snowball）的 cases（需 --repo-root 参数）
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball list

# JSON 格式输出（便于脚本分析）
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball list --json
```

### 查看单个 case 详情

```bash
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball show --id C-936a1510-c5f2-4575-ab75-f894f564bb99
```

### 测试 recall 搜索

```bash
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball recall "shadow replay"
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball recall "主候选上线"
```

### 测试 resume 接续

```bash
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball resume
```

### 统计分析（Python 单行）

```bash
# 统计状态分布与时间线
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball list --json | python3 -c "
import sys, json
data = json.load(sys.stdin)
cases = data['cases']
print(f'总 case 数: {len(cases)}')
status_count = {}
for c in cases:
    s = c['status']
    status_count[s] = status_count.get(s, 0) + 1
print(f'状态分布: {status_count}')
for c in sorted(cases, key=lambda x: x['timestamps']['opened_at_utc']):
    opened = c['timestamps']['opened_at_utc'][:16]
    goal_short = c['goal'][:60]
    print(f'{c[\"id\"][:12]}  {c[\"status\"]:10}  {opened}  {goal_short}')
"

# 检查含延续性措辞的 goal
AGPOD_CASE_DATA_DIR=/tmp/agpod-case-db-copy agpod case --repo-root ~/workspace/snowball list --json | python3 -c "
import sys, json
data = json.load(sys.stdin)
keywords = ['继续', '进一步', '推近', '推进', '逼近', '复测', '复核']
cases = data['cases']
hits = [c for c in cases if any(k in c['goal'] for k in keywords)]
print(f'含延续性措辞的 goal: {len(hits)}/{len(cases)}')
for c in hits:
    print(f'  {c[\"id\"][:12]}  {c[\"goal\"][:80]}')
"
```
