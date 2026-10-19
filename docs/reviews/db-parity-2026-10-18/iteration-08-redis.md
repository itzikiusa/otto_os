# Redis Parity — Iteration 08: Blocking Commands, Timeouts, Cancellation & SSH/TLS Transport

Focused review pass on bounded blocking execution (`BLPOP`, `XREAD BLOCK`, `WAIT`), configurable transport response timeouts (`>30s`), cancellation/socket teardown, and SSH/TLS transport across `crates/otto-dbviewer/src/drivers/redis/transport.rs`, `tls.rs`, and `redis.rs`.

## Concrete Cases Traced & Verified

1. **Transport Response Timeout Scaling (`Connection::connect_with_timeout`)**:
   - Previously, `transport::from_stream` hardcoded `AsyncConnectionConfig::new().set_response_timeout(Some(Duration::from_secs(30)))`, which would abort a `BLPOP jobs 45` with `req.timeout_ms = Some(60_000)` at 30s.
   - `RedisDriver::connect_isolated` (`redis.rs:577–583`) now computes `response_timeout = timeout.unwrap_or(Duration::from_secs(30)).max(Duration::from_secs(30)).saturating_add(Duration::from_secs(5))` and passes it to `Connection::connect_with_timeout` (`transport.rs:103–155, 200–220`), so `tokio::time::timeout(timeout, ...)` in `exec_command` (`redis.rs:536–549`) is always the governing deadline.
   - **Controlled-Clock >30s Regression (`tests/unit/redis_transport.rs:blocking_command_beyond_30s_succeeds_only_when_response_timeout_is_extended`)**: Uses `#[tokio::test(start_paused = true)]` with a 35s delayed `BLPOP slow-jobs 35` in `peer` to prove both the red case (default 30s `Connection::from_stream` times out at 30s and retires the connection) and the green case (`Connection::from_stream_with_timeout(..., Some(Duration::from_secs(50)))` with a 45s request timeout succeeds at 35s and returns `["slow-jobs", "after-35"]`).
2. **Bounded Blocking Commands (`BLPOP`, `BRPOP`, `BLMOVE`, `BLMPOP`, `BZPOPMIN`, `BZPOPMAX`, `BZMPOP`, `WAIT`, `WAITAOF`, `XREAD BLOCK`, `XREADGROUP BLOCK`)**:
   - Verified against live Redis `7.4.11` (`127.0.0.1:26379`):
     - `RPUSH parity:list item-1` followed by `BLPOP parity:list 1` returns `[["parity:list"], ["item-1"]]`.
     - `XREAD COUNT 1 BLOCK 50 STREAMS parity:empty:stream 0-0` waits 50ms and returns `RedisValue::Nil` (`rows: [["(nil)"]], message: Some("(nil)")`).
   - Because `run()` uses an operation-isolated connection, a blocking command never stalls concurrent schema browsing, `object_detail`, or another tab's `run()`.
3. **Cancellation & Socket Closure (`RequestGuard` + `AbortOnDrop`)**:
   - In `transport.rs:42–92`, `Connection` wraps `Arc<Owner>` (which holds `AbortOnDrop` for the `redis-rs` background driver task).
   - For `run()`, the isolated `Connection` has no clone in `self.clients`. When `run()` finishes, times out, or its future is dropped/cancelled by the caller, both `RequestGuard` and the sole `Connection` drop immediately, aborting the driver task and closing the underlying TCP/TLS socket so the Redis server unblocks and cleans up the client connection immediately.
4. **Continuous Server-Push Streams Explicitly Refused**:
   - `SUBSCRIBE`, `PSUBSCRIBE`, `SSUBSCRIBE`, `UNSUBSCRIBE`, `PUNSUBSCRIBE`, `SUNSUBSCRIBE`, `MONITOR`, `SYNC`, and `PSYNC` have no bounded request/response completion and are refused upfront with terminal guidance (`redis.rs:607–618`).
5. **SSH Tunnel + TLS SNI Preservation**:
   - `transport::connect_with_timeout` (`transport.rs:133–139`) resolves the TLS server name from `cfg.tls.server_name`, then `cfg.param_str("__tunnel_host")`, then `cfg.host`, ensuring TLS certificate verification and SNI succeed when connecting through an SSH local port forward (`127.0.0.1:<local_port>`).

## Scores (Iteration 08)

| Lens | Score | Evidence Boundary |
|------|------:|-------------------|
| **L13** | 9.8 | Blocking commands (`BLPOP`, `BZPOPMIN`, `XREAD BLOCK`, `WAIT`, `WAITAOF`) supported; continuous push streams (`SUBSCRIBE`, `MONITOR`) explicitly refused. |
| **L14** | 9.8 | Blocking commands work both standalone and inside multi-command batches (`RPUSH` → `BLPOP` → `XREAD BLOCK`). |
| **L15** | 9.8 | Popped `[key, element]` pairs and expired blocking `Nil` replies (`"(nil)"`) render accurately. |
| **L16** | 9.8 | `connect_with_timeout` honors `req.timeout_ms > 30s`; dropping/cancelling `run()` closes the isolated socket immediately. |
