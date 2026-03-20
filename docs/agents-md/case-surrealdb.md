# agpod-case: SurrealDB Embedded

## When to Use

When modifying `crates/agpod-case/` — especially `client.rs` queries or schema.

## Architecture

- **Engine**: SurrealDB 3.x embedded with RocksDB backend (`kv-rocksdb` feature)
- **No external service**: data stored at `$XDG_DATA_HOME/agpod/case.db` (configurable via `--data-dir` or `AGPOD_CASE_DATA_DIR`)
- **Namespace**: `agpod`, database: `case`

## Schema

Schema is defined inline in `client.rs::ensure_schema()`. Four tables:

| Table | Key Field | Purpose |
|-------|-----------|---------|
| `case` | `case_id` | Exploration cases |
| `direction` | `(case_id, seq)` | Direction within a case |
| `step` | `step_id` | Execution steps |
| `entry` | `(case_id, seq)` | Event log entries |

All fields are `TYPE string` or `TYPE int`. JSON arrays/objects are stored as serialized strings.

## Query Pattern

```rust
// All queries go through query_raw:
async fn query_raw(&self, sql: &str, bindings: Value) -> CaseResult<Vec<Value>>

// Results are Vec<serde_json::Value>, parsed by helper functions:
// parse_case, parse_single_direction, parse_single_step, parse_single_entry
```

## Adding New Queries

1. Write SurrealQL with `$param` bindings
2. Call `self.query_raw(sql, json!({...})).await?`
3. Parse results with existing `parse_*` helpers or add new ones
4. Field names in DB match struct fields (snake_case)

## Common Mistakes

- Forgetting `IF NOT EXISTS` on schema definitions (causes errors on restart)
- Using `id` instead of `case_id` / `step_id` (SurrealDB has a built-in `id` field for record IDs)
- Not serializing Vec/struct fields to JSON strings before storage
