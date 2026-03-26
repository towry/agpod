# `case show` 大输出优化实现案

## 背景

现 `agpod case show` 直出全案卷宗，含：

- `case`
- `direction_history`
- `steps_by_direction`
- `entries`

实现见 `crates/agpod-case/src/commands.rs:1222`。  
对人尚可，对 agent 则常致：

- 单次输出过巨
- token 成本暴涨
- 上下文污染
- 明知只需一角，仍被迫吞全文

`case_resume` 已有较佳 handoff 摘要，见 `crates/agpod-case/src/commands.rs:1901`；故 `case_show` 宜转为“可寻址、可分窗之案卷读取器”，而非默认 full dump。

## 样本现状

样本文件：`/tmp/snowball-case-show.txt`

- 行数：`905`
- 词数：`19546`
- 字节：`283728`
- 文件特征：UTF-8，且有极长行

按样本观之，结构近于：

1. case header
2. `direction_tree`
3. `entries`

其中：

- `direction_tree` 自约第 `17` 行起
- `entries` 自约第 `149` 行起
- 主要膨胀源为 `entries`

此与代码实现相符：`entries` 由全量 `client.get_entries(&case.id)` 直映射输出。见 `crates/agpod-case/src/commands.rs:1225`

## 设计目标

从 agent 角度，`case_show` 之理想用途非“读完整案”，乃：

- 先知此案多大
- 先得总览与目录
- 再定位具体 section / entry / direction
- 再按窗口取局部
- 每轮输出恒受 token 预算约束

故新设计须满足：

1. **默认不全量输出**
2. **先回总 token 大小**
3. **支持分窗读取**
4. **支持搜索后再取邻近块**
5. **返回稳定 locator/cursor，便于 agent 续读**

## 非目标

- 不做 `less` 式交互分页
- 不依赖 TTY/终端态
- 不以字符数/行数粗切正文
- 不让 agent 自行拼 `sed/head/tail` 模式处理巨型单响应

## 用户与 agent 视角之读取模型

大文件常用法为：

- 先 `head`
- 再 `tail`
- 再找锚点
- 再按窗口展开周边

`case_show` 亦当同理：

- `outline` = 目录/总览
- `search` = 找锚点
- `window` = 取局部正文

换言之，`case_show` 应从“整本书导出”改为“案卷阅读协议”。

## 建议接口模型

### 一、默认视图：`outline`

默认 `case show` 仅返回：

- case header
- current direction summary
- current / pending step 摘要
- recent entry 摘要若干
- 总 token 大小
- section 目录
- 下一窗口指针

建议返回形：

```json
{
  "ok": true,
  "view": "outline",
  "case_id": "C-xxx",
  "total_tokens": 102059,
  "window_token_budget": 4000,
  "sections": [
    { "id": "header", "tokens": 320 },
    { "id": "direction_tree", "tokens": 8150 },
    { "id": "entries", "tokens": 93200 }
  ],
  "content": {
    "case": {},
    "current_direction": {},
    "current_step": {},
    "next_pending_steps": [],
    "recent_entries": []
  },
  "has_next": true,
  "next_cursor": "section:entries@0"
}
```

### 二、窗口视图：`window`

显式取局部卷宗。

建议参数：

- `--cursor <cursor>`
- `--token-budget <n>`
- `--section <id>`
- `--around <locator>`

建议返回：

```json
{
  "ok": true,
  "view": "window",
  "case_id": "C-xxx",
  "total_tokens": 102059,
  "cursor": "section:entries@0",
  "window_index": 0,
  "window_tokens": 3812,
  "has_prev": false,
  "has_next": true,
  "next_cursor": "section:entries@1",
  "included_sections": ["entries"],
  "content": {
    "entries": []
  }
}
```

### 三、搜索视图：`search`

先定位，再展开。

建议参数：

- `--search <query>`
- `--limit <n>`

建议返回：

```json
{
  "ok": true,
  "view": "search",
  "case_id": "C-xxx",
  "query": "snapshot_fail",
  "matches": [
    {
      "locator": "entry:469",
      "kind": "record/evidence",
      "excerpt": "Oracle rule-slice audit 已成...",
      "token_estimate": 120
    }
  ]
}
```

而后 agent 可再以：

- `--around entry:469`
- 或 `--cursor section:entries@12`

取邻近块。

## 分页单位：以语义块为先

不宜按行切，宜按语义块切。

建议一级 section：

- `header`
- `direction_tree`
- `entries`

于 `entries` 内，再按 entry 粒度组成窗口。

建议 locator：

- `section:header`
- `section:direction_tree`
- `entry:471`
- `entry:520`
- `entries:460-479`

此可支持：

- 搜索命中
- 邻近展开
- 精准续读

## token 预算策略

### 原则

- 先按语义块估 token
- 再组窗
- 每窗不超预算

### 建议预算

- MCP / agent 默认：`3000~5000`
- CLI 人类读：可更高，如 `8000~12000`

### 裁剪优先级

若预算不足，保留顺序：

1. header
2. current direction / current step
3. next action / recent entries summary
4. direction tree
5. older entries

## 建议引入之技术能力

### token 计数

若以 OpenAI / tiktoken 兼容为主，建议优先调研并接入：

- `tiktoken`
- 或 `tiktoken-rs`

其职责仅为：

- 估整案 `total_tokens`
- 估各 section token
- 估每窗 token

分页本身不应委给第三方 crate，宜由 `agpod-case` 自行实现。

### 稳定 cursor

建议 cursor 不直接暴露 token 偏移，而以语义位置编码：

- `section:entries@page=0`
- 或 `entries:seq:460`

如此较稳，亦便日志与调试。

## CLI 建议

### 默认行为

`agpod case show`

返回 `outline`，不再 full dump。

### 显式全量

若仍需旧行为，可临时保留：

- `agpod case show --view full`

但：

- 不建议暴露给 MCP
- 或需显式 `--unsafe-large-output`

### 推荐新参数

- `--view outline|window|search|full`
- `--cursor <cursor>`
- `--section <section>`
- `--search <query>`
- `--around <locator>`
- `--token-budget <n>`

## MCP 建议

### 不宜继续以单一 `case_show` 混合诸义

较佳有二路：

#### 路一：扩展 `case_show`

为 `CaseShowRequest` 增：

- `view`
- `cursor`
- `section`
- `search`
- `around`
- `token_budget`

优点：

- 兼容旧名

缺点：

- 语义渐重

#### 路二：拆为三工具

- `case_outline`
- `case_show_window`
- `case_search`

优点：

- agent 更不易误用
- tool description 更清

缺点：

- MCP surface 稍增

### 建议裁断

若以 agent 可用性优先，**更推荐拆工具**。  
理由：

- `case_show` 之“给我整案”心智过强
- agent 易再次误调用 full dump
- 分拆后可强引导：
  - 先 `case_resume`
  - 再 `case_outline`
  - 再 `case_show_window`

## 与现有工具之角色重排

建议之后之读取阶梯为：

1. `case_current`
   - 只问：有无 open case、状态为何
2. `case_resume`
   - 只问：我现在接手，需要知道什么
3. `case_outline`
   - 只问：此案卷有多大、分几段
4. `case_search`
   - 只问：某信息在哪里
5. `case_show_window`
   - 只取局部正文
6. `case_show --view full`
   - 仅人工或调试场景

## 最小实现步骤

### Step 1：先加 token 与 outline

目标文件：

- `crates/agpod-case/src/commands.rs`
- `crates/agpod-case/src/output.rs`
- `crates/agpod-case/src/types.rs`
- `crates/agpod-case/src/cli.rs`

改动：

- 为 `case show` 新增 `outline` 视图
- 计算 `total_tokens`
- 计算 section token 摘要
- 默认改回 `outline`

完成判据：

- `case show` 不再输出全量 `entries`
- 返回 `total_tokens`
- 返回 `sections`

### Step 2：加入 entries window

目标文件同上。

改动：

- 为 `entries` 建窗口读取
- 支持 `cursor` / `token_budget`
- 返回 `has_next/prev`

完成判据：

- 可稳定取 `entries` 任一窗口
- 同案多次读取边界稳定

### Step 3：加入 search / around

目标文件同上。

改动：

- 最小文本搜索
- 命中回 `locator + excerpt`
- 支持 `around locator`

完成判据：

- agent 可先 search 后读 window

### Step 4：MCP 收口

目标文件：

- `crates/agpod-mcp/src/lib.rs`

改动：

- MCP tool 描述重写
- 默认指引改为先 `case_resume`
- 若拆工具，则新增 schema 与文案

完成判据：

- agent 不再被鼓励直接 full dump

## 风险与取舍

### 风险一：旧脚本依赖 full output

对策：

- 短期保留 `--view full`
- 并于 MCP 层不默认暴露

### 风险二：token 估计与真实模型计费不完全等同

对策：

- 文案明示为 estimate
- 先保证相对稳定与上界控制

### 风险三：cursor 于内容变化后失效

对策：

- cursor 编码带 `case_id + updated_at + section + seq range`
- 不匹配则快失败并提示重取 outline

## 建议之第一优先级

若只做一轮最小止血，我建议：

1. 默认 `case show` 改为 `outline`
2. 返回 `total_tokens`
3. `entries` 改为窗口输出
4. 保留 `--view full` 仅作人工调试

此四项即可显著压低 agent 单轮成本，并保留诊断能力。

## 结论

`case_show` 当前之主病，不在“显示方式”，而在“协议语义”：  
它仍把大案卷当一次性消息，而非可导航文档。

故正解非接入 `less`，亦非单做终端分页，乃是将其改造成：

- 先知大小
- 再得目录
- 可搜索
- 可取窗
- 可续读

此方真合 agent 使用之法，亦能实减 token 支出。
