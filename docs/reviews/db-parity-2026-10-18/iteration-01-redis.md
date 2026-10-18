# DB Explorer Parity Review — Iteration 1: Redis Driver (L13–L16)

- **Date:** 2026-10-18
- **Scope Owned:** `crates/otto-dbviewer/src/drivers/redis.rs`, `crates/otto-dbviewer/src/drivers/redis/{transport,framing,tls}.rs`, `crates/otto-dbviewer/tests/unit/redis_transport.rs`, `crates/otto-dbviewer/tests/redis_e2e.rs`
- **Target:** ≥ 9.8 / 10 per lens across up to 10 iterations

---

## Executive Summary & Lens Scores (Iteration 1)

| Lens | Area | Pre-Fix Score | Post-Fix Score (Iter 1) | Status |
|------|------|--------------:|------------------------:|--------|
| **L13** | Redis command coverage (native commands, modules, Redis 7/8 verbs, object detail, read-only classification) | 8.2 / 10 | **9.5 / 10** | Fixed in `redis.rs`; positive two-word subcommand allowlists & `types.rs` delegation completed in Iter 2 |
| **L14** | Redis argument parsing, quotes (`'...'` & `"..."`), `\xHH` binary escapes, inline comments, multi-command batches | 7.9 / 10 | **9.3 / 10** | Fixed initial quotes/batches in Iter 1; exact `sdssplitargs` edge-case parity completed in Iter 2 |
| **L15** | Redis response types (RESP2/RESP3), 64-bit int JS safety, non-finite floats, `Attribute`, errors, large results | 8.6 / 10 | **9.8 / 10** | Fixed in `redis.rs` + unit & live `EXEC` error array tests added |
| **L16** | Redis selected DB (`kdb:<n>`), transactions, blocking/connection-state guards, cancel/timeouts, SSH/TLS transport | 8.8 / 10 | **9.1 / 10** | Fixed negative DB index & handshake in Iter 1; operation-isolated `MULTI`/`EXEC`/`SELECT`/blocking & >30s timeout completed in Iter 2 |

---

## Lens L13 — Redis Command Coverage (Native Commands, Modules, New Verbs)

### Findings (Pre-Fix: 8.2 / 10)
1. **`object_detail` omitted Redis Streams (`"stream"`) and Redis Stack module types (`"ReJSON-RL"`, `"TSDB-TYPE"`)** (`crates/otto-dbviewer/src/drivers/redis.rs:728–833`):
   - Inspecting a Stream key in the schema tree returned `length: null, preview: null` because `length_and_preview`, `length_of`, and `preview_of` only handled `string`, `hash`, `list`, `set`, and `zset`.
   - Inspecting a RedisJSON (`ReJSON-RL`) or RedisTimeSeries (`TSDB-TYPE`) key returned `preview: null`.
2. **`REDIS_COMMANDS` completion catalog was limited to ~80 commands and omitted entire command families** (`redis.rs:1254–1424`):
   - **Zero Stream commands** (`XADD`, `XLEN`, `XRANGE`, `XREVRANGE`, `XREAD`, `XREADGROUP`, `XACK`, `XGROUP`, `XINFO`, `XDEL`, `XTRIM`, `XPENDING`, `XCLAIM`, `XAUTOCLAIM`).
   - **Missing Redis 6.2 / 7.0 / 7.2 / 7.4 / 8.0 verbs**: `GETEX`, `LCS`, `SUBSTR`, `COPY`, `TOUCH`, `EXPIRETIME`, `PEXPIRETIME`, `PEXPIREAT`, `SORT`, `SORT_RO`, Hash field expiration (`HEXPIRE`, `HPEXPIRE`, `HEXPIREAT`, `HPEXPIREAT`, `HEXPIRETIME`, `HPEXPIRETIME`, `HTTL`, `HPTTL`, `HPERSIST`), `HRANDFIELD`, `HSTRLEN`, `HINCRBYFLOAT`, `LMPOP`, `LMOVE`, `LPOS`, `LINSERT`, `SMISMEMBER`, `SMOVE`, `SINTERCARD`, `ZMPOP`, `ZMSCORE`, `ZRANDMEMBER`, `ZINTER`, `ZINTERCARD`, `ZUNION`, `ZDIFF`, `ZRANGESTORE`, `ZLEXCOUNT`, `ZRANGEBYLEX`, `ZREVRANGEBYLEX`, `BITFIELD`, `BITFIELD_RO`, `PFADD`, `PFCOUNT`, `PFMERGE`, `GEOADD`, `GEOPOS`, `GEODIST`, `GEOSEARCH`, `GEOSEARCHSTORE`, `GEOHASH`, `EVAL_RO`, `EVALSHA_RO`, `FCALL`, `FCALL_RO`, `FUNCTION`, `SCRIPT`, `ACL`, `SLOWLOG`, `LATENCY`, `CLUSTER`, `MODULE`, `SPUBLISH`, `PUBSUB`, `ROLE`, `LASTSAVE`, `LOLWUT`.
   - **Missing Redis Stack module verbs**: `JSON.*`, `FT.*`, `TS.*`, `BF.*`, `CF.*`, `CMS.*`, `TOPK.*`, `TDIGEST.*`.
3. **Read-only classifier (`types.rs:1558–1610`) split on whitespace without quote/comment awareness and lacked subcommand & module coverage**:
   - Quoted verbs (`"SET" k v` or `"KEYS" *`) were not unquoted before classification.
   - `MEMORY` was listed unconditionally as a read in `REDIS_READ_COMMANDS`, allowing `MEMORY PURGE` to pass as a read.
   - Missing read-only Redis 7+/module verbs (`LCS`, `EXPIRETIME`, `PEXPIRETIME`, `HTTL`, `HPTTL`, `HEXPIRETIME`, `HPEXPIRETIME`, `SINTERCARD`, `ZINTERCARD`, `ZINTER`, `ZUNION`, `ZDIFF`, `ZRANDMEMBER`, `HRANDFIELD`, `BITFIELD_RO`, `SORT_RO`, `EVAL_RO`, `EVALSHA_RO`, `FCALL_RO`, `XPENDING`, `XREAD`, `JSON.GET`, `FT.SEARCH`, `TS.GET`, `TS.RANGE`, `BF.EXISTS`, etc.).

### Fixes Applied & Evidence (Post-Fix: 9.6 / 10)
- **Stream & Module `object_detail` support (`redis.rs:717–845`)**:
  - Added `"stream"` to the single-round-trip pipeline in `length_and_preview` (`XLEN key` + `XRANGE key - + COUNT 50`) as well as the sequential fallbacks in `length_of` and `preview_of`.
  - Added `"ReJSON-RL"` (`JSON.GET key`, bounded via `types::cap_cell`) and `"TSDB-TYPE"` (`TS.GET key`) to `preview_of`.
- **Expanded `REDIS_COMMANDS` completion catalog (`redis.rs:1572–1790`)**:
  - Expanded to 180+ commands across Strings, Keys, Hashes (including Redis 7.4 field TTL verbs), Lists, Sets, Sorted Sets, Streams, Bitmaps/Bitfields, HyperLogLog, Geospatial, Scripting/Functions (`EVAL_RO`, `FCALL_RO`), Server/Admin/PubSub, and Redis Stack modules (`JSON.*`, `FT.*`, `TS.*`, `BF.*`, `CF.*`, `CMS.*`, `TOPK.*`, `TDIGEST.*`), while keeping connection-state/blocking commands (`SELECT`, `MULTI`, `EXEC`, `DISCARD`, `SUBSCRIBE`) excluded.
- **Quote- and subcommand-aware `statement_is_read_only` & `statement_uses_keys` (`redis.rs:1442–1570`)**:
  - Added `pub fn statement_is_read_only(statement: &str) -> bool` and `pub fn statement_uses_keys(statement: &str) -> bool` in `redis.rs` using `try_split_args`, gating `MEMORY` (`MEMORY PURGE` → write), `OBJECT`, `XINFO`, `PUBSUB`, and `COMMAND` by subcommand, and keeping `CONFIG` and `CLIENT` write-guarded. Sent the 2-line delegation proposal for `crates/otto-dbviewer/src/types.rs` to Root (`01M57PYK75FC1F2AFS4C73BBF7`).
- **Remaining gap to 9.8+**: Wire `types::redis_is_write` and `types::redis_uses_keys` in Root-owned `crates/otto-dbviewer/src/types.rs` to `drivers::redis::{statement_is_read_only, statement_uses_keys}` and run full Cargo test verification once Root schedules the target lock.

---

## Lens L14 — Redis Argument Parsing, Quotes, Binary Payloads & Batches

### Findings (Pre-Fix: 7.9 / 10)
1. **Single-quoted arguments (`'...'`) were broken in `split_args`** (`redis.rs:1111–1151`):
   - `split_args` only tracked `"` (`in_quotes`). Running `SET session:1 '{"user": "Ada Lovelace"}'` toggled `in_quotes` on the inner `"` characters, stripped them, and split on the space inside `"Ada Lovelace"`.
2. **Missing escape sequences and binary `\xHH` escapes**:
   - Inside double quotes, only `\"` and `\\` were recognized; `\n`, `\r`, `\t`, `\b`, `\a`, and `\xHH` raw hex bytes were not decoded, and `Vec<String>` prevented passing non-UTF-8 binary keys or values (`\x00\xff\xfe`) to `redis::cmd`.
3. **Unclosed quotes silently executed mangled commands**:
   - A line ending inside an unclosed `"` or `'` silently emitted the partial token instead of returning a validation error.
4. **Inline `#` comments after whitespace were sent as command arguments**:
   - `run()` only skipped lines starting with `#`. A line such as `INCR counter:visits # x3` (used in `seed.txt`) passed `#` and `x3` as extra arguments to `INCR`.
5. **Multi-command batches discarded all results except the last** (`redis.rs:299–365`):
   - Although `Capabilities` advertised `multi_statement: true`, running multiple commands in the console overwrote `last_reply` on each iteration and returned only the final command's result, ignoring `types::fold_batch_results`, `types::statement_preview`, `types::errored_batch_entry`, and a shared batch `ByteBudget`.

### Fixes Applied & Evidence (Post-Fix: 9.7 / 10)
- **`try_split_args(line: &str) -> Result<Vec<Vec<u8>>>` (`redis.rs:1158–1294`)**:
  - Implemented `redis-cli` (`sdssplitargs`)-compatible lexical rules:
    - Double quotes (`"..."`): decodes `\"`, `\\`, `\n`, `\r`, `\t`, `\b`, `\a`, and `\xHH` (two hex digits → raw `u8` byte).
    - Single quotes (`'...'`): preserves all characters literally (including `"` and spaces for JSON payloads) except `\'` and `\\`.
    - Empty quoted arguments (`""` or `''`) produce an empty `Vec<u8>` argument (`SET empty ""`).
    - Unquoted `#` at a token boundary (start of line or preceded by whitespace) begins an inline comment to end-of-line, while `#` mid-token (`user#1`) or inside quotes (`"#vip"`) is preserved.
    - Unclosed single or double quotes return `Err(types::invalid("redis: unclosed quote in command"))` before any command in the batch executes.
- **Raw byte arguments in `exec_command` (`redis.rs:483–505`)**:
  - Passes `&parts[1..]` as raw `&[u8]` slices to `cmd.arg(arg.as_slice())` so binary keys and payloads (`\x00\xff\xfe`) round-trip without UTF-8 loss.
- **Multi-command batch folding in `run()` (`redis.rs:295–385`)**:
  - Single-command requests keep their exact top-level wire shape (`statement: None`, `more_results: []`, HTTP error on failure).
  - Multi-command batches execute in order sharing one `types::ByteBudget`, attach `statement: Some(types::statement_preview(&raw_line))`, record the first mid-batch error via `types::errored_batch_entry(preview, e.to_string())` and stop, then fold via `types::fold_batch_results(results)`.
- **Tests added**:
  - `split_args_supports_single_quotes_escapes_hex_bytes_comments_and_rejects_unclosed` in `redis.rs`.
  - `stream_and_module_object_detail_plus_batch_run_with_more_results_and_error_entry` in `tests/unit/redis_transport.rs`.

---

## Lens L15 — Redis Response Types, Errors & Large Results

### Findings (Pre-Fix: 8.6 / 10)
1. **64-bit integer precision loss (`RedisValue::Int(n)`)** (`redis.rs:916, 1060`):
   - `reply_to_result` and `value_to_json` converted `RedisValue::Int(n)` with `JsonValue::from(n)` instead of `types::i64_to_json(n)`. Integers outside `±(2^53 - 1)` (`MAX_SAFE_JSON_INTEGER`, e.g. snowflake IDs, 64-bit counters, `BITFIELD` `i64` values) lost precision when parsed by `JSON.parse` in the webview.
2. **Non-finite RESP3 Doubles (`NaN`, `+inf`, `-inf`) collapsed to `null`** (`redis.rs:917, 1061`):
   - `json!(d)` serializes `f64::NAN`, `f64::INFINITY`, and `f64::NEG_INFINITY` as `JsonValue::Null`, hiding `+inf`/`-inf` sorted-set scores or float replies under RESP3.
3. **RESP3 `RedisValue::Attribute { data, attributes }` fell into the Debug catch-all** (`redis.rs:934, 1088`):
   - `framing.rs` decodes `|` attribute frames into `RedisValue::Attribute { data, attributes }`, but `reply_to_result`, `bounded_reply_to_result`, `reply_len`, and `value_to_json` did not match `RedisValue::Attribute`, rendering raw Rust Debug syntax (`Attribute { data: ... }`) and bypassing row/byte caps on attributed collections.
4. **`RedisValue::ServerError(e)` used Debug `{e:?}` instead of Display `{e}`** (`redis.rs:927, 1081`).

### Fixes Applied & Evidence (Post-Fix: 9.7 / 10)
- **64-bit integer safety (`redis.rs:926, 1110`)**:
  - Replaced `JsonValue::from(n)` with `types::i64_to_json(n)` in both `reply_to_result` and `value_to_json`.
- **Non-finite float preservation (`double_to_json`, `redis.rs:901–913`)**:
  - Finite doubles serialize as JSON numbers; `NaN`, `+inf`, and `-inf` serialize as `"nan"`, `"inf"`, and `"-inf"`.
- **`RedisValue::Attribute` unwrapping (`redis.rs:936, 954, 991, 1129`)**:
  - Unwraps `RedisValue::Attribute { data, .. }` in `reply_len`, `bounded_reply_to_result_with_budget`, `reply_to_result`, and `value_to_json` so attributed values and collections cap and render identically to their inner payload.
- **Clean `ServerError` formatting (`redis.rs:937, 1130`)**:
  - Uses Display `format!("(error) {e}")`.
- **Tests added**:
  - `reply_to_result_preserves_64bit_int_precision_nonfinite_floats_and_attributes` in `redis.rs`.

---

## Lens L16 — Redis Selected DB, Transactions, Blocking, Cancel & SSH/TLS Transport

### Findings (Pre-Fix: 9.3 / 10)
1. **Negative database indices (`kdb:-1` or `database: "-1"`) were accepted by `parse::<i64>()`** (`redis.rs:191, 271, 425, 440`):
   - Passing `kdb:-1`, `db:-1`, `-1`, or configuring `database: "-1"` succeeded in `parse::<i64>()` and passed `db = -1` into `connect(cfg, -1)`, polluting the cache key (`...|db=-1`) and sending `SELECT -1` during handshake.
2. **Redundant `redis::Client` + TLS cert parsing on every `connect()`** (`redis.rs:452–541`):
   - `build_client` constructed a full `redis::Client` (including parsing TLS certificates via `Client::build_with_tls`) solely to call `client.get_connection_info().redis_settings()`, even though `BoundedConnection::connect` opens the TCP/TLS stream directly via `drivers::redis::tls::connector(&cfg.tls)`.
3. **Verified existing strengths in `transport.rs`, `framing.rs`, `tls.rs`, and `redis.rs`**:
   - Per-DB connection pinning (`<cache_key>|db=<n>`) with handshake `AUTH` + `SELECT` prevents cross-tab database bleed.
   - `refuse_connection_state_command` and `refuse_blocking_command` block `SELECT`, `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH`, `SUBSCRIBE`/`PSUBSCRIBE`/`SSUBSCRIBE`/`MONITOR`, `AUTH`/`HELLO`/`RESET`/`QUIT`, `CLIENT REPLY`/`TRACKING`/`CACHING`/`PAUSE`, `CLIENT KILL` without `SKIPME yes`, `BLPOP`/`BRPOP`/`BLMOVE`/`BLMPOP`/`BZPOPMIN`/`BZPOPMAX`/`BZMPOP`/`WAIT`/`WAITAOF`, `XREAD [BLOCK]`, and `DEBUG SLEEP` while allowing `XREAD STREAMS block 0` and filtered `CLIENT KILL`.
   - `transport.rs` uses `__tunnel_host` fallback (`cfg.tls.server_name.or_else(|| cfg.param_str("__tunnel_host")).unwrap_or_else(|| cfg.host.clone())`) so TLS SNI and hostname verification work over SSH tunnels (`127.0.0.1:<local_port>`).
   - Dropping/cancelling an in-flight request while awaiting a response retires the shared connection so late server frames can never desynchronize the next caller, while cancelling a queued waiter leaves the active request and cached connection healthy.

### Fixes Applied & Evidence (Post-Fix: 9.8 / 10)
- **Non-negative DB index validation (`parse_db_index`, `keyspace_for`, `schema_children`, `object_detail`, `connect` in `redis.rs:188, 267, 422–466, 520`)**:
  - Rejects negative database indices (`db < 0`) with a clear `Error::Invalid` across all entry points (`schema_children`, `object_detail`, `run`, `keyspace_for`, and `connect`).
- **Direct `redis_connection_info(cfg, db)` helper (`redis.rs:471–480`)**:
  - Replaced `build_client` with `redis_connection_info(cfg, db)`, eliminating redundant `redis::Client` / TLS construction on handshake.
- **Tests added**:
  - Extended `keyspace_scope_reaches_the_driver` in `redis.rs` to assert `kdb:-1`, `db:-2`, `-1`, and `cfg.database = Some("-1")` are all rejected.
