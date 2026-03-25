# Honcho 冒烟测试

## 何时用

- 修 Honcho 写入、`case context`、semantic recall 后
- 怀疑本机配置、临时 `case-server` 环境、或 Honcho 限流所致失败时

## 前提

- 仓库根有 `.env.sh`，内含 `HONCHO_API_KEY`、`HONCHO_BASE_URL`、`HONCHO_WORKSPACE_ID`
- 用临时数据目录与独立端口，勿污染常用 case 数据
- 若要走 Honcho semantic recall，务加 `AGPOD_CASE_SEMANTIC_RECALL=true`

## 步骤

### 1. 构建

```bash
cd /Users/towry/workspace/agpod && source .env.sh && cargo build -p agpod -p agpod-case-server
```

### 2. 起临时 server

```bash
cd /Users/towry/workspace/agpod && source .env.sh && \
AGPOD_CASE_HONCHO_ENABLED=true \
AGPOD_CASE_HONCHO_SYNC_ENABLED=true \
AGPOD_CASE_SEMANTIC_RECALL=true \
AGPOD_CASE_DATA_DIR=/tmp/agpod-honcho-smoke.db \
target/debug/agpod-case-server \
  --data-dir /tmp/agpod-honcho-smoke.db \
  --server-addr 127.0.0.1:6252
```

- 若另开 shell，可把 stdout/stderr 重定向到 `/tmp/agpod-honcho-smoke.log`

### 3. 开临时 case

```bash
cd /Users/towry/workspace/agpod && source .env.sh && \
AGPOD_CASE_HONCHO_ENABLED=true \
AGPOD_CASE_HONCHO_SYNC_ENABLED=true \
AGPOD_CASE_SEMANTIC_RECALL=true \
AGPOD_CASE_DATA_DIR=/tmp/agpod-honcho-smoke.db \
AGPOD_CASE_SERVER_ADDR=127.0.0.1:6252 \
target/debug/agpod case open --json \
  --goal 'Honcho recall smoke' \
  --direction 'write several differentiated records then test context recall' \
  --success-condition 'case context and honcho search return relevant entries' \
  --abort-condition 'honcho sync or recall fails repeatedly'
```

- 预期：返回 `hooks.statuses`，且 `sink=honcho`, `ok=true`

### 4. 写几条差异消息

```bash
cd /Users/towry/workspace/agpod && source .env.sh && \
AGPOD_CASE_HONCHO_ENABLED=true \
AGPOD_CASE_HONCHO_SYNC_ENABLED=true \
AGPOD_CASE_SEMANTIC_RECALL=true \
AGPOD_CASE_DATA_DIR=/tmp/agpod-honcho-smoke.db \
AGPOD_CASE_SERVER_ADDR=127.0.0.1:6252 \
target/debug/agpod case record --json \
  --id <CASE_ID> \
  --kind evidence \
  --summary 'vector digest queue stalls when replaying patch streams' \
  --context 'keywords: vector digest queue replay patch stream backpressure'
```

```bash
cd /Users/towry/workspace/agpod && source .env.sh && \
AGPOD_CASE_HONCHO_ENABLED=true \
AGPOD_CASE_HONCHO_SYNC_ENABLED=true \
AGPOD_CASE_SEMANTIC_RECALL=true \
AGPOD_CASE_DATA_DIR=/tmp/agpod-honcho-smoke.db \
AGPOD_CASE_SERVER_ADDR=127.0.0.1:6252 \
target/debug/agpod case decide --json \
  --id <CASE_ID> \
  --summary 'prefer bounded replay batches for vector digest imports' \
  --reason 'smaller replay chunks reduce queue starvation and improve honcho recall freshness'
```

- 写太快或见 `429 Too Many Requests` 时，候 1–2 秒再续

### 5. 用自然语言做 case-scope recall

```bash
cd /Users/towry/workspace/agpod && source .env.sh && \
AGPOD_CASE_HONCHO_ENABLED=true \
AGPOD_CASE_HONCHO_SYNC_ENABLED=true \
AGPOD_CASE_SEMANTIC_RECALL=true \
AGPOD_CASE_DATA_DIR=/tmp/agpod-honcho-smoke.db \
AGPOD_CASE_SERVER_ADDR=127.0.0.1:6252 \
target/debug/agpod case context --json \
  --id <CASE_ID> \
  --scope case \
  --query 'Which notes in this case discuss replay queues, vector digest imports, or why smaller batches help keep recall fresh?' \
  --limit 5 \
  --token-limit 512
```

- 预期：
  - `case_context.backend = "honcho"`
  - `hits` 含 replay queue / bounded replay batches 相关消息

### 6. 用自然语言做 repo-scope recall

```bash
cd /Users/towry/workspace/agpod && source .env.sh && \
AGPOD_CASE_HONCHO_ENABLED=true \
AGPOD_CASE_HONCHO_SYNC_ENABLED=true \
AGPOD_CASE_SEMANTIC_RECALL=true \
AGPOD_CASE_DATA_DIR=/tmp/agpod-honcho-smoke.db \
AGPOD_CASE_SERVER_ADDR=127.0.0.1:6252 \
target/debug/agpod case context --json \
  --id <CASE_ID> \
  --scope repo \
  --query 'Across this repository, what recent case material mentions replay queues, vector digest stalls, or choosing smaller replay batches to reduce starvation?' \
  --limit 5 \
  --token-limit 512
```

- 预期：
  - `case_context.backend = "honcho"`
  - `hits` 可含当前 smoke case，亦可含同 repo 旧 smoke case

### 7. 远端直查 Honcho（可选）

```bash
cd /Users/towry/workspace/agpod && source .env.sh && python3 - <<'PY'
import json, os, urllib.request
workspace=os.environ['HONCHO_WORKSPACE_ID']
base=os.environ['HONCHO_BASE_URL']
key=os.environ['HONCHO_API_KEY']
case_id='<CASE_ID>'
query='Which notes in this case discuss replay queues, vector digest imports, or why smaller batches help keep recall fresh?'
url=f"{base.rstrip('/')}/v3/workspaces/{workspace}/sessions/{case_id}/search"
payload=json.dumps({'query': query, 'limit': 5}).encode()
req=urllib.request.Request(url, data=payload, headers={
    'Authorization': f'Bearer {key}',
    'Accept': 'application/json',
    'Content-Type': 'application/json',
})
with urllib.request.urlopen(req, timeout=20) as resp:
    print(resp.read().decode())
PY
```

## 判据

- 写入成功：CLI 回 `hooks.statuses[*].sink=honcho`, `ok=true`
- case recall 成功：`backend=honcho` 且 `hits` 含预期语义消息
- repo recall 成功：`backend=honcho` 且 `hits` 至少含当前仓相关 case 消息

## 常见故障

- **`backend=local_text`**
  - 未启 `AGPOD_CASE_SEMANTIC_RECALL=true`
  - 或起 server 时未带 Honcho / semantic recall 环境

- **`429 Too Many Requests`**
  - Honcho 限流；放慢写入或稍候再试

- **已有 open case**
  - 换临时数据目录，或先关闭旧临时 case

- **repo-scope 偶发网络错**
  - 先重试；若直连 Honcho `/search` 正常而 CLI 偶发失败，多半属瞬时传输问题
