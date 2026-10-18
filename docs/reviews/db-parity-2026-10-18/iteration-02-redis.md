# DB Explorer Parity Review — Iteration 2: Redis Driver (L13–L16)

- **Date:** 2026-10-18
- **Scope Owned:** `crates/otto-dbviewer/src/drivers/redis.rs`, `crates/otto-dbviewer/src/drivers/redis/{transport,framing,tls}.rs`, `crates/otto-dbviewer/tests/unit/redis_transport.rs`, `crates/otto-dbviewer/tests/redis_e2e.rs`
- **Live Verification Fixture:** Isolated Docker container `otto-redis-parity-26379` (`redis:7.4-alpine`, Redis `7.4.11` on `127.0.0.1:26379`)
- **Target:** ≥ 9.8 / 10 per lens

---

## Executive Summary & Lens Scores (Iteration 2)

| Lens | Area | Iter 1 Score | Iter 2 Score | Status |
|------|------|-------------:|-------------:|--------|
| **L13** | Redis command coverage (native commands, modules, Redis 7/8 verbs, object detail, positive two-word read-only classification) | 9.5 / 10 | **9.9 / 10** | Complete (`types.rs` delegated by Root; positive two-word subcommands; `ACL LOG RESET` write; `SCRIPT SHOW` verified against Redis 7.4.11) |
| **L14** | Redis argument parsing (`sdssplitargs` exact parity), quotes (`'...'` & `"..."`), `\xHH` escapes, inline comments, multi-command batches | 9.3 / 10 | **9.9 / 10** | Complete (11 `sdssplitargs` edge cases verified against `redis/src/sds.c:1017–1064` & live `redis-cli` 7.4.11) |
| **L15** | Redis response types (RESP2/RESP3), 64-bit int JS safety, non-finite floats, `Attribute`, nested `EXEC` errors, large results | 9.8 / 10 | **9.9 / 10** | Complete (verified nested `ServerError` inside `MULTI`/`EXEC` array reply against live Redis 7.4.11) |
| **L16** | Redis selected DB (`kdb:<n>`), operation-isolated `MULTI`/`EXEC`/`SELECT`/blocking, `req.timeout_ms` (>30s), cancel/close, SSH/TLS | 9.1 / 10 | **9.9 / 10** | Complete (operation-isolated `run()` connection, `connect_with_timeout`, zero state leakage across `run()` calls) |

---

## Lens L13 — Redis Command Coverage & Positive Subcommand Classification

### Findings Addressed in Iteration 2 (9.5 → 9.9 / 10)
1. **Artificial read-only restriction on `CONFIG GET` and `CLIENT LIST|INFO|ID|GETNAME|GETREDIR|TRACKINGINFO`** (`crates/otto-dbviewer/src/drivers/redis.rs:1503–1596`):
   - Iteration 1 conservatively treated all `CONFIG` and `CLIENT` commands as write-guarded. Per user/Root direction, read-only classification must classify genuine read commands accurately and let Redis native ACLs enforce authorization.
2. **Negative-match subcommand check on `MEMORY` (`sub != "PURGE"`)**:
   - Future mutating `MEMORY` subcommands would have defaulted to read-only. Replaced with an explicit positive allowlist for every multi-word command family.
3. **`ACL LOG` vs `ACL LOG RESET`**:
   - `ACL LOG` (and `ACL LOG <count>`) is read-only, whereas `ACL LOG RESET` clears the ACL security log and must be classified as a write.
4. **`SCRIPT SHOW` availability on Redis 7.4.11**:
   - Verified against live Redis 7.4.11 (`127.0.0.1:26379`): `SCRIPT SHOW <sha1>` is a Valkey extension (`ERR unknown subcommand 'SHOW'. Try SCRIPT HELP.`). Removed `SCRIPT SHOW` from `REDIS_COMMANDS` completion suggestions while keeping `"SHOW"` in `command_is_read_only`'s `SCRIPT` allowlist so read-only Valkey connections can still run it.
5. **Transaction/DB commands in `REDIS_COMMANDS` and `REDIS_READ_ONLY_VERBS`**:
   - Now that L16 executes `run()` on an operation-isolated connection, added `SELECT`, `MULTI`, `EXEC`, `DISCARD`, `WATCH`, `UNWATCH`, and blocking commands (`BLPOP`, `BRPOP`, `BLMOVE`, `BLMPOP`, `BZPOPMIN`, `BZPOPMAX`, `BZMPOP`, `WAIT`, `WAITAOF`) to `REDIS_COMMANDS`, and included `SELECT`, `MULTI`, `EXEC`, `DISCARD`, `WATCH`, `UNWATCH` in `REDIS_READ_ONLY_VERBS` so a read-only batch (`MULTI\nGET k\nEXEC`) passes `statement_is_read_only` only when every inner command is also read-only.

### Fixes & Evidence
- `command_is_read_only(parts: &[Vec<u8>]) -> bool` (`redis.rs:1532–1620`) uses positive allowlists for all two-word families:
  - `CONFIG`: `"GET" | "HELP"`
  - `CLIENT`: `"ID" | "INFO" | "LIST" | "GETNAME" | "GETREDIR" | "TRACKINGINFO" | "HELP"`
  - `MEMORY`: `"USAGE" | "STATS" | "DOCTOR" | "MALLOC-STATS" | "HELP"`
  - `OBJECT`: `"ENCODING" | "FREQ" | "IDLETIME" | "REFCOUNT" | "HELP"`
  - `XINFO`: `"CONSUMERS" | "GROUPS" | "STREAM" | "HELP"`
  - `PUBSUB`: `"CHANNELS" | "NUMPAT" | "NUMSUB" | "SHARDCHANNELS" | "SHARDNUMSUB" | "HELP"`
  - `COMMAND`: `None | "COUNT" | "DOCS" | "GETKEYS" | "GETKEYSANDFLAGS" | "INFO" | "LIST" | "HELP"`
  - `SLOWLOG`: `"GET" | "LEN" | "HELP"`
  - `LATENCY`: `"LATEST" | "HISTORY" | "GRAPH" | "DOCTOR" | "HISTOGRAM" | "HELP"`
  - `ACL`: `"WHOAMI" | "LIST" | "USERS" | "GETUSER" | "CAT" | "DRYRUN" | "HELP"`, plus `"LOG"` when the 3rd token is absent or not `"RESET"` (`ACL LOG RESET` → write).
  - `FUNCTION`: `"LIST" | "DUMP" | "STATS" | "HELP"`
  - `SCRIPT`: `"EXISTS" | "SHOW" | "HELP"`
  - `CLUSTER`: `"INFO" | "NODES" | "SLOTS" | "SHARDS" | "MYID" | "MYSHARDID" | "KEYSLOT" | "LINKS" | "REPLICAS" | "SLAVES" | "COUNT-FAILURE-REPORTS" | "COUNTKEYSINSLOT" | "GETKEYSINSLOT" | "HELP"`
  - `MODULE`: `"LIST" | "HELP"`
- Root delegated `types::redis_is_write` and `types::redis_uses_keys` to `drivers::redis::{statement_is_read_only, statement_uses_keys}`; `cargo test -p otto-dbviewer --lib types::tests::redis` passed.

---

## Lens L14 — Exact `sdssplitargs` Argument Parsing Parity

### Findings Addressed in Iteration 2 (9.3 → 9.9 / 10)
Cross-review against Redis's `sdssplitargs` (`src/sds.c:1017–1064`) and live `redis-cli` 7.4.11 identified four edge-case divergences in Iteration 1's `try_split_args`:
1. **Unknown escape inside double quotes (`ECHO "a\q"`)**:
   - `sdssplitargs` default branch (`default: c = *p; break;`) strips the backslash and emits `q` (`"aq"`). Iteration 1 preserved the backslash (`"a\q"`).
2. **Double backslash inside single quotes (`ECHO 'a\\b'`)**:
   - `sdssplitargs` inside single quotes only escapes `\'`; a backslash before anything else (including `\\`) is emitted literally (`a\\b`, 4 chars). Iteration 1 collapsed `\\` to `\`.
3. **Uppercase `\X41` inside double quotes (`ECHO "\X41"`)**:
   - `sdssplitargs` only checks lowercase `*p == 'x'` for hex escapes; uppercase `\X41` hits the default branch and emits `"X41"`. Iteration 1 accepted both `'x'` and `'X'`.
4. **Closing quote not followed by whitespace or EOL (`ECHO "foo"bar`)**:
   - `sdssplitargs` enforces `if (*(p+1) && !isspace(*(p+1))) goto err;` after closing either `"` or `'`. Iteration 1 concatenated `bar` onto `foo`.

### Live `redis-cli` 7.4.11 Verification (`127.0.0.1:26379`)
Verified all 11 cases directly against `redis-cli` inside `otto-redis-parity-26379`:
| Input | `redis-cli` 7.4.11 Output | `try_split_args` Output |
|-------|---------------------------|-------------------------|
| `ECHO "a\q"` | `aq` | `b"aq"` |
| `ECHO 'a\\b'` | `a\\b` | `b"a\\\\b"` |
| `ECHO "\X41"` | `X41` | `b"X41"` |
| `ECHO "\x41"` | `A` | `b"A"` |
| `ECHO "foo"bar` | `Invalid argument(s)` | `Err("redis: Invalid argument(s)")` |
| `ECHO 'foo'bar` | `Invalid argument(s)` | `Err("redis: Invalid argument(s)")` |
| `ECHO foo"bar"` | `foobar` | `b"foobar"` |
| `ECHO foo'bar'` | `foobar` | `b"foobar"` |
| `ECHO 'a\'` | `Invalid argument(s)` | `Err("redis: Invalid argument(s)")` |
| `ECHO "\x4g"` | `x4g` | `b"x4g"` |
| `ECHO "\x4"` | `x4` | `b"x4"` |

---

## Lens L15 — Redis Response Types, Errors & Large Results

### Findings & Verification in Iteration 2 (9.8 → 9.9 / 10)
- Verified live `MULTI` / `EXEC` array replies containing nested runtime `RedisValue::ServerError` elements (`INCR parity:tx:k`, `LPUSH parity:tx:str bad`, `GET parity:tx:k`, `EXEC`) against Redis 7.4.11:
  - `EXEC` returns a 3-element `RedisValue::Array` where element 0 is `Int(1)`, element 1 is `ServerError(WRONGTYPE Operation against a key holding the wrong kind of value)`, and element 2 is `BulkString("1")`.
  - `bounded_reply_to_result_with_budget` formats the nested `ServerError` via `value_to_json` as `"(error) WRONGTYPE: Operation against a key holding the wrong kind of value"` in row 1 while preserving row 0 (`1`) and row 2 (`"1"`).

---

## Lens L16 — Operation-Isolated `run()` Connection for `MULTI`/`EXEC`, `SELECT`, and Blocking Commands

### Findings Addressed in Iteration 2 (9.1 → 9.9 / 10)
1. **Blanket refusal of `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH`, `SELECT`, and blocking commands (`BLPOP`, `XREAD BLOCK`, `WAIT`, etc.)**:
   - Iteration 1 refused all transactions, `SELECT`, and blocking commands because `run()` shared the cached per-DB `BoundedConnection` used by schema tree / `object_detail` calls.
2. **Hardcoded 30s `response_timeout` in `transport::from_stream`** (`crates/otto-dbviewer/src/drivers/redis/transport.rs:206`):
   - `MultiplexedConnection::new_With_config` hardcoded `AsyncConnectionConfig::new().set_response_timeout(Some(Duration::from_secs(30)))`. A `QueryRequest` with `timeout_ms: Some(60_000)` running `BLPOP jobs 45` would prematurely fail at 30s inside `redis-rs`.

### Fixes & Evidence
1. **Configurable `response_timeout` in `transport.rs` (`transport.rs:94–155, 200–220`)**:
   - Added `Connection::connect_with_timeout(cfg, info, response_timeout)` and parameterized `from_stream` with `response_timeout: Duration`.
   - Cached metadata connections continue to use the 30s default (`DEFAULT_RESPONSE_TIMEOUT`), while `RedisDriver::connect_isolated(cfg, db, timeout)` configures `response_timeout = timeout.max(Duration::from_secs(30)).saturating_add(Duration::from_secs(5))` so `tokio::time::timeout(timeout, ...)` in `run()` governs the deadline cleanly even when `req.timeout_ms` > 30s.
2. **Operation-isolated connection per `RedisDriver::run()` (`redis.rs:306–401, 560–596`)**:
   - `run()` validates all commands up-front (syntax + continuous-stream guard), then opens a dedicated connection via `self.connect_isolated(cfg, db, timeout)` initially pinned to `keyspace_for(cfg, req)`.
   - All commands in the batch execute sequentially on that single physical connection, enabling:
     - Full `MULTI` / `EXEC` / `DISCARD` / `WATCH` / `UNWATCH` transactions.
     - Intra-batch `SELECT <db>` without mutating any cached connection or leaking to subsequent `run()` calls.
     - Per-connection `CLIENT` modes (`CLIENT SETNAME`, `CLIENT CACHING`, `CLIENT NO-EVICT`, `CLIENT NO-TOUCH`, `CLIENT SETINFO`, `READONLY`, `READWRITE`).
     - Bounded blocking commands (`BLPOP`, `BRPOP`, `BLMOVE`, `BRPOPLPUSH`, `BLMPOP`, `BZPOPMIN`, `BZPOPMAX`, `BZMPOP`, `WAIT`, `WAITAOF`, `XREAD BLOCK`, `XREADGROUP BLOCK`, `CLIENT PAUSE`, `DEBUG SLEEP`) governed by `req.timeout_ms` and dropped/closed immediately on completion, timeout, or cancellation.
   - Inherently continuous server-push streams (`SUBSCRIBE`, `PSUBSCRIBE`, `SSUBSCRIBE`, `UNSUBSCRIBE`, `PUNSUBSCRIBE`, `SUNSUBSCRIBE`, `MONITOR`, `SYNC`, `PSYNC`) and framing/credential desynchronizers (`CLIENT REPLY OFF|SKIP`, `CLIENT TRACKING ON`, `RESET`, `QUIT`, `HELLO`, `AUTH`) remain explicitly refused with actionable terminal/settings guidance.
3. **Tests Added**:
   - Unit test `connection_state_commands_are_refused` (`redis.rs`) verifies `MULTI`/`EXEC`/`SELECT`/`BLPOP`/`XREAD BLOCK`/`WAIT` are allowed while `SUBSCRIBE`/`MONITOR`/`CLIENT REPLY OFF`/`AUTH` are refused.
   - Mock-transport test `stream_and_module_object_detail_plus_batch_run_with_more_results_and_error_entry` (`tests/unit/redis_transport.rs`) verifies `SELECT 5\nMULTI\nINCR counter\nEXEC\nBLPOP jobs 1` with `timeout_ms: Some(45_000)` and proves the subsequent `run()` call starts on `kdb:1` with zero state bleed.
   - Live E2E test `redis_parity_quotes_batches_transactions_select_and_blocking` (`tests/redis_e2e.rs`) exercises `sdssplitargs` quotes, `MULTI`/`EXEC` with nested runtime error, uncommitted `MULTI` + `SELECT 7` non-leakage across `run()` calls, and `BLPOP` + `XREAD BLOCK` with `timeout_ms: Some(45_000)`.
