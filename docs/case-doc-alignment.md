# Case / Document Alignment

本文专定 `case` 与设计文档、实现文档之分工与对齐规则。

总旨如下：

> `case` 管推进现场；文档管稳定知识；二者以引用相连，不以全文互抄相替。

---

## 1. 总原则

### 1.1 `case` 不作长文容器

`case` 之职责，在于维持当前案之推进现场：

- 当前 goal
- 当前 direction
- 当前 step
- 关键事实 / 证据 / 决断
- 当前 blocker
- 下一动作

`case` 不应用作：

- 设计文全文存放处
- 实现方案全文存放处
- 长篇 runbook 复制处
- 多页外部讨论摘要之替代品

### 1.2 稳定知识应升格为文档

凡满足下列任一者，应自 `case` 提炼为文档，而非继续只存于 case：

- 将长期影响接口或架构
- 将长期影响实现顺序
- 将长期影响测试或运维方式
- 后续多人 / 多 agent 皆需反复引用

### 1.3 `case` 中优先存“引用”，不存“重抄”

当相关设计、实现、runbook、外部工单已存在时，`case` 中应优先写：

- 简短结论
- 文档引用

而非再把整段内容抄入 `case_record` / `case_decide`。

---

## 2. 三类载体之职责边界

## 2.1 `case`

适合放：

- “现在哪一步”
- “刚刚证实了什么”
- “当前阻塞是什么”
- “接下来做什么”
- “哪份文档是当前依据”

不适合放：

- 完整接口规格
- 完整状态机
- 完整迁移计划
- 大段 runbook

## 2.2 设计文档

适合放：

- 目标接口
- 状态机
- 事务语义
- 数据模型
- 非目标
- 设计取舍

不适合放：

- 每轮最新 smoke 结果
- 某次临时 blocker 的聊天式现场

## 2.3 实现文档

适合放：

- 改动切片
- 文件落点
- 实施顺序
- 测试矩阵
- 前置依赖
- 风险与验收

不适合放：

- 每次运行结果全文
- 高频现场 checkpoint

---

## 3. `case` 中如何引用文档

## 3.1 引用优先级

当已有正式文档时，`case` 中应按如下形态记录：

1. 一句结论
2. 一到数个引用

例如：

```json
{
  "summary": "已冻结 case 工具终形与 step_advance 规格，后续实现以文档为准。",
  "files": [
    "docs/case-interface-redesign.md",
    "docs/case-step-advance-spec.md"
  ]
}
```

## 3.2 可引用之目标

`case` 所引，不限于仓内文件，亦可引外部文档，例如：

- `Linear` issue / project / comment 链接
- 线上设计文链接
- 外部 runbook 链接
- 代码评审链接
- PR / commit / CI run 链接

故引用目标分两类：

- **本地引用**
  - 例如：`docs/case-interface-redesign.md`
- **外部引用**
  - 例如：`https://linear.app/...`

## 3.3 推荐字段

对 case 工具而言，长期宜支持统一之“引用”负载，例如：

```json
{
  "references": [
    {
      "kind": "doc",
      "title": "Case Interface Redesign",
      "target": "docs/case-interface-redesign.md"
    },
    {
      "kind": "linear",
      "title": "Linear issue",
      "target": "https://linear.app/example/issue/ABC-123"
    }
  ]
}
```

若当前工具尚无 `references` 字段，则可先退以：

- 本地文档放 `files`
- 外部链接临时写入 `context`

但此仅为过渡表达，不应视为终形。

---

## 4. 何时从 `case` 提炼为文档

遇下列信号，即应把 case 中之结论升格入文：

### 4.1 设计已冻结

例如：

- 工具最终形态已定
- 状态机已定
- 事务策略已定

此时不应只留于 `case_decide`，当写入设计文。

### 4.2 实施顺序已冻结

例如：

- 先做事务 spike
- 再做 `Entry.step_id`
- 再做 `case_step_advance`

此应写入实现文，而非只散于数条 case note。

### 4.3 外部系统结论已成长期约束

例如：

- Linear 某 issue 已确立为单一事实源
- 外部 runbook 已成为后续 smoke 之依据

此时 `case` 应只保留引用与当前推进关系。

---

## 5. 何时从文档回写 `case`

并非文档写完，`case` 便无须更新。

应回写 `case` 之情形如下：

- 文档已冻结某设计结论，需通知当前案之后续步骤以此为准
- 文档新增前置依赖，影响当前推进顺序
- 文档改写某原有假设，使当前 blocker 或 next action 改变

回写时，宜简写为：

- 变了什么
- 影响当前案何处
- 引用哪份文档

而不应全文复述文档内容。

---

## 6. 推荐写法

## 6.1 好例

```json
{
  "summary": "已确认 `case_step_advance` 必须走真事务；后续实现以前置 spike 结果为准。",
  "files": [
    "docs/case-step-advance-spec.md"
  ],
  "context": "See also Linear: https://linear.app/example/issue/ABC-123"
}
```

好处：

- `case` 保留当前推进意义
- 细节仍回到正式文档
- 外部来源亦可追

## 6.2 坏例

```json
{
  "summary": "这里粘贴三百行设计稿全文……"
}
```

坏处：

- 漂移
- 重复
- 难维护
- 难判断何者为最新口径

---

## 7. 对工具设计之反推

若以上原则成立，则 `case` 工具后续宜支持：

1. **记录引用**
   - 可同时容本地文件与外部 URL

2. **在 `current` / `show` 中显式展示关键引用**
   - 使 agent 不必再从自由文本中抽链接

3. **让引用成为一等字段**
   - 不再把外链硬塞 `context`

此三项中，第三项最值后续纳入正式接口演进。

### 对 `case_open.needed_context_query` 之直接影响

若后续在 `case_open` 中新增“开案主动补记忆”字段，例如 `needed_context_query`，
则其返回之 `startup_context` 亦应遵守同一原则：

- 优先返回文档引用
- 优先返回 runbook / design / Linear / PR / CI 等目标链接
- 少回大段旧 findings 全文

盖 startup context 之义，在于“指出开局先看何物”，
而不在“把旧案再抄一遍”。

---

## 8. 一言蔽之

`case` 中应写：

> “此刻为何、何处、何据、下一步。”

文档中应写：

> “系统最终应如何、为何如此、如何落地。”

二者之桥，不是互抄全文，而是稳定引用。
