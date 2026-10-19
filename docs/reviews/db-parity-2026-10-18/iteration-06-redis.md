# Redis Parity — Iteration 06: Transactions (`MULTI` / `EXEC` / `DISCARD` / `WATCH` / `UNWATCH`)

Focused review pass on transaction semantics, error modes, and read-only classification across `crates/otto-dbviewer/src/drivers/redis.rs` and `crates/otto-dbviewer/tests/redis_e2e.rs`, verified against live Redis `7.4.11` (`otto-redis-parity-26379` on `127.0.0.1:26379`).

## Concrete Cases Traced & Verified

1. **Same-Connection Transaction Execution (`MULTI` → `+QUEUED` → `EXEC`)**:
   - `RedisDriver::run` (`redis.rs:330`) opens one operation-isolated connection via `self.connect_isolated(cfg, db, timeout)` and executes every command in `req.statement` sequentially on that connection.
   - Queued commands return `RedisValue::SimpleString("QUEUED")`, rendered as a single-cell row `["QUEUED"]` labeled with `statement_preview`.
   - `EXEC` returns `RedisValue::Array(items)`, rendered one row per queued command result (`bounded_reply_to_result_with_budget`).
2. **Runtime Errors Inside `EXEC` (`WRONGTYPE`) vs Syntax Errors During Queueing (`EXECABORT`)**:
   - **Live E2E Bug Found & Fixed (`redis.rs:491–520`)**: `redis::Cmd::query_async::<RedisValue>(conn)` internally invokes `Value::extract_error()`, which recursively scans `RedisValue::Array` and turns the **entire** `EXEC` array into `Err(WRONGTYPE)` whenever any single queued command produces a runtime `ServerError` — discarding all sibling results!
   - **Fix**: Switched `exec_command` to `redis::aio::ConnectionLike::req_packed_command(&cmd)` and matched only a **top-level** `RedisValue::ServerError(e)` as `Err`. Now, when `EXEC` returns `*3\r\n:1\r\n-WRONGTYPE ...\r\n$1\r\n1\r\n` on live Redis `7.4.11` (`127.0.0.1:26379`), all 3 rows (`[1]`, `["(error) WRONGTYPE: Operation against a key holding the wrong kind of value"]`, `["1"]`) are preserved in the `EXEC` result grid, matching `redis-cli`.
   - When a command fails with a syntax/arity error during queueing (`MULTI` → `SET only_one_arg` → `EXEC`), the bad command returns a top-level `ServerError` (`Err` at line 365), recording `errored_batch_entry("SET only_one_arg", ...)` and stopping the batch; dropping `conn` on return closes the socket and discards the aborted transaction on the server.
3. **`WATCH` / `UNWATCH` Contention (`EXEC` Returning `Nil`)**:
   - When a `WATCH`ed key is modified before `EXEC`, Redis replies to `EXEC` with `*-1\r\n` (`RedisValue::Nil`), which `reply_to_result` renders as `QueryResult::message("(nil)")` (`rows: [["(nil)"]], message: Some("(nil)")`), matching `redis-cli`'s `(nil)` output.
4. **Uncommitted `MULTI` Isolation Across `run()` Calls**:
   - Because `conn` is local to `RedisDriver::run` and never inserted into `self.clients`, a batch that ends after `MULTI\nINCR k` (without `EXEC` or `DISCARD`) drops `conn` on return. A subsequent `run()` with `EXEC` opens a fresh socket and receives `ERR EXEC without MULTI`, proving zero transaction state bleed.
5. **Read-Only Gate Interaction (`statement_is_read_only`)**:
   - `MULTI`, `EXEC`, `DISCARD`, `WATCH`, and `UNWATCH` are in `REDIS_READ_ONLY_VERBS` (`redis.rs:1633`), and `statement_is_read_only` requires `.all()` commands in the batch to be read-only. Thus `MULTI\nGET a\nTTL a\nEXEC` is allowed on a read-only connection, whereas `MULTI\nINCR a\nEXEC` is rejected before connecting.

## Scores (Iteration 06)

| Lens | Score | Evidence Boundary |
|------|------:|-------------------|
| **L13** | 9.8 | Transaction verbs (`MULTI`, `EXEC`, `DISCARD`, `WATCH`, `UNWATCH`) in completions and read-only classifier; write inside `MULTI` still blocked in read-only mode. |
| **L14** | 9.8 | Multi-line transaction batches preserve per-step `QUEUED` and `EXEC` result ordering. |
| **L15** | 9.8 | Nested `ServerError` inside `EXEC` array replies and `Nil` from contended `WATCH`/`EXEC` render cleanly without failing the whole batch. |
| **L16** | 9.8 | Operation-isolated connection guarantees `MULTI`/`EXEC` share one socket within `run()` and never leak an open transaction to subsequent calls. |
