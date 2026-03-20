# agpod-mcp handoff

## 当前目标

为 `agpod` 增一独立 `agpod-mcp` crate，提供 case workflow 的 MCP server，使 agent 可直接见：

- tool 名
- input schema
- output schema
- 简明且偏 agent 的语义说明

重点不在“把 CLI 包一层文本”，而在“让 agent 少试错，先知工具如何用、何时用、与何者区分”。

---

## 已完成

### 1. 新 crate 与 workspace 接入

已新增：

- `crates/agpod-mcp/Cargo.toml`
- `crates/agpod-mcp/src/lib.rs`
- `crates/agpod-mcp/src/main.rs`

已更新：

- `Cargo.toml`
- `release-please-config.json`
- `.release-please-manifest.json`

目前 `agpod-mcp` 为独立 stdio MCP server，不混入 `agpod` 主 CLI。

---

### 2. `agpod-case` 已暴露 JSON 入口供 MCP 复用

已改：

- `crates/agpod-case/src/lib.rs`
- `crates/agpod-case/src/commands.rs`
- `crates/agpod-case/src/output.rs`

现有新增入口：

- `agpod_case::run_json(args) -> serde_json::Value`

此入口复用既有 case 业务逻辑，返回与 `agpod case --json` 对齐的 JSON payload。

实现细节：

- `commands::execute_json` 新增
- 内部暂以 `_meta.json_mode` 复用旧渲染路径
- `output.rs` 的 `render_json` 会去掉 `_meta`

故：

- CLI 用户看不到 `_meta`
- MCP 可直接拿结构化 JSON

---

### 3. 已暴露之 MCP tools

当前 `agpod-mcp` 已做 case 相关 tools：

- `case_current`
- `case_open`
- `case_record`
- `case_decide`
- `case_redirect`
- `case_show`
- `case_close`
- `case_abandon`
- `case_list`
- `case_recall`
- `case_resume`
- `case_step_add`
- `case_step_start`
- `case_step_done`
- `case_step_move`
- `case_step_block`

各 tool 当前皆返回：

```json
{
  "result": { ...agpod case --json payload... }
}
```

为配合 inspector，`outputSchema.result` 目前是：

- `object`
- `additionalProperties: true`

先不暴露更细 output schema。

---

### 4. 文案已转为偏 agent 说明

已改 server instructions 与 tool descriptions，重点是：

- 简短
- 明示前置关系
- 明示语义边界
- 避免泛泛解释

例如：

- `case_current`: safe first call
- `case_open`: call first
- `case_record`: not for decisions or redirects
- `case_show`: use after `case_current` when more history is needed
- `case_step_add`: use after `case_open` or `case_redirect`

此部分代码在：

- `crates/agpod-mcp/src/lib.rs`

---

## 已验证

### 1. Rust 侧

已核实并采用：

- `rmcp = 1.2.0`
- 官方 Rust MCP SDK
- stdio transport

并已验证：

- `cargo test -p agpod-mcp`
- `cargo build -p agpod-mcp`

曾遇到两类问题，已处理：

1. `ToolResponse.result` 若用 `serde_json::Value`，inspector 对 `outputSchema` 不接受
   - 已改为 `Map<String, Value>`

2. 测试里 `tool.name.as_str()` 命中稳定性问题
   - 已改

---

### 2. Inspector 侧

已成功运行：

```bash
npx -y @modelcontextprotocol/inspector --cli --transport stdio --method tools/list ./target/debug/agpod-mcp
```

结果：

- `tools/list` 成功
- 可见全部 tool schema
- 可见当前 description 文案
- `outputSchema` 已被 inspector 接受

注意：

- 直接运行 `./target/debug/agpod-mcp --help` 无意义
- 它是纯 stdio MCP server，不是常规 CLI

---

## 当前未完成

### 1. `data_dir` 不应继续出现在每个 tool input schema

这是当前最明确的后续项。

你的新要求是：

- `data_dir` 不应由每个 tool 传入
- 应改由 MCP server 进程环境配置

我认同此方向。

当前状态：

- 几乎每个 request struct 都有 `data_dir: Option<String>`
- tool 调用时会把它灌入 `CaseArgs`

建议改法：

#### 目标

从所有 MCP tool input schema 中移除 `data_dir`。

#### 建议实现

在 `agpod-mcp` 内统一解析一次环境变量，例如：

- `AGPOD_CASE_DATA_DIR`

然后在 `run_case_tool(...)` 内注入到 `CaseArgs.data_dir`。

即：

- tool 参数不再携带 `data_dir`
- case DB 路径由 server process 环境统一决定

#### 受影响文件

- `crates/agpod-mcp/src/lib.rs`

#### 具体改动

1. 删掉所有 request struct 里的 `data_dir`
2. 给 `AgpodMcpServer` 增一个字段，如：
   - `data_dir: Option<String>`
3. 在 `AgpodMcpServer::new()` 中从环境读取：
   - `std::env::var("AGPOD_CASE_DATA_DIR").ok()`
4. `run_case_tool()` 改为使用 `self.data_dir.clone()`
5. 更新 tool schema tests
6. 再跑 inspector `tools/list`

这会显著减轻 agent 误用。

---

### 2. 需完成真实 `tools/call` 验证

已完成 `tools/list`。

但 `tools/call` 仍未完成一次最终通路验证。

我曾试：

```bash
npx -y @modelcontextprotocol/inspector --cli --transport stdio --method tools/call --params '{"name":"case_current","arguments":{}}' ./target/debug/agpod-mcp
```

此法不对；inspector CLI 不是用这套参数格式。

后续应改用 inspector 正确的 CLI 参数形式，例如：

- `--tool-name`
- 可能还有 `--tool-arg`

建议先执行：

```bash
npx -y @modelcontextprotocol/inspector --help
```

再按其 `tools/call` 子参数格式做一次：

- `case_current`
- `case_list`

至少验证一条不带复杂入参的工具调用。

---

### 3. 输出 schema 仍偏宽

当前 output schema 为：

- `result: object`
- `additionalProperties: true`

此可用，但不算精细。

若后续要让 agent 更强地“预知结果结构”，可考虑：

1. 先为 `case_current` / `case_show` / `case_resume` 单独定义 typed output
2. 再逐步把其他 tool 迁入 typed output

但这是第二阶段之事，非首版阻断项。

---

## 推荐下一步

续作时建议顺序：

1. 移除 MCP tool `data_dir` 入参
2. 改为 server env 注入 `AGPOD_CASE_DATA_DIR`
3. 重新 `cargo test -p agpod-mcp`
4. 重新 `cargo build -p agpod-mcp`
5. 用 inspector 跑：
   - `tools/list`
   - 至少一条 `tools/call`
6. 若稳定，再决定是否：
   - 增 typed output schema
   - 增 `diff` / `vcs-path` 类工具

---

## 测试方法

### 1. Rust 构建与测试

在仓根执行：

```bash
cargo test -p agpod-mcp
cargo build -p agpod-mcp
```

若只想先看 `agpod-case` 复用入口是否仍通：

```bash
cargo check -p agpod-case
```

---

### 2. 用 inspector 看 schema

最先做此步，确认 MCP server 可起，且 tool schema 能被客户端接受：

```bash
npx -y @modelcontextprotocol/inspector --cli --transport stdio --method tools/list ./target/debug/agpod-mcp
```

通过判据：

- 退出码为 0
- 能列出 `case_*` 与 `case_step_*` tools
- 能看到简短 description
- `inputSchema` 与 `outputSchema` 无校验报错

若此步失败，先不要试 `tools/call`。

---

### 3. 真实 tool call 测试

此步要在确认 inspector CLI 的当前参数格式后再跑。

建议先看帮助：

```bash
npx -y @modelcontextprotocol/inspector --help
```

然后优先试最安全之工具：

- `case_current`
- `case_list`

目标是验证：

- server 能处理一次真实 `tools/call`
- 返回 payload 为：

```json
{
  "result": { ... }
}
```

若当前仓已有 open case，优先测 `case_current`。
若无 open case，优先测 `case_list`。

---

### 4. 使用非默认 DB 路径测试

此项是“改完 `data_dir` 方案后”当补之测试。

目标：

- 不再通过 tool 参数传 `data_dir`
- 改由 MCP 进程环境控制 DB 路径

建议测试法：

```bash
export AGPOD_CASE_DATA_DIR=/tmp/agpod-case-test.db
npx -y @modelcontextprotocol/inspector --cli --transport stdio --method tools/list ./target/debug/agpod-mcp
```

然后用 inspector 调：

- `case_open`
- `case_current`

判据：

- tool schema 中不再出现 `data_dir`
- server 仍能正常读写 case 数据

---

### 5. 最小 smoke test 流程

待 `tools/call` 通后，建议走一遍最小闭环：

1. `case_open`
2. `case_step_add`
3. `case_step_start`
4. `case_record`
5. `case_current`
6. `case_show`
7. `case_resume`

通过判据：

- 每步调用成功
- 返回 JSON 结构与 `agpod case --json` 对齐
- `case_current` / `case_show` / `case_resume` 语义一致

---

## 关键文件

- `Cargo.toml`
- `release-please-config.json`
- `.release-please-manifest.json`
- `crates/agpod-case/src/lib.rs`
- `crates/agpod-case/src/commands.rs`
- `crates/agpod-case/src/output.rs`
- `crates/agpod-mcp/Cargo.toml`
- `crates/agpod-mcp/src/lib.rs`
- `crates/agpod-mcp/src/main.rs`

---

## 备注

- 仓内本就有未提交变更，尤其 `agpod-case` 相关文件；续作时勿误回退。
- `agpod-mcp` 当前为纯 stdio server，故直接执行二进制不会给正常 help。
- inspector 已证明 schema 可见；下一关键门槛是把一次真实 tool call 跑通。
