# Redis Parity — Iteration 09: `sdssplitargs` Argument Parser, Binary Payloads & Read-Only Classification

Focused review pass on lexical parity with Redis's `sdssplitargs` (`src/sds.c:1017–1064`), binary key/value handling (`Vec<u8>` + base64 display), and positive two-word subcommand read-only classification in `crates/otto-dbviewer/src/drivers/redis.rs`.

## Concrete Cases Traced & Verified

1. **Exact `sdssplitargs` Lexical Parity (`try_split_args`, `redis.rs:1185–1330`)**:
   - Verified line-by-line against `redis/src/sds.c:1017–1064` and live `redis-cli` `7.4.11` (`127.0.0.1:26379`):
     - **Double quotes (`"..."`)**: Recognizes `\n`, `\r`, `\t`, `\b`, `\a`, `\"`, `\\`, and lowercase `\xHH` (two hex digits → raw byte). Any other `\c` (including `"a\q"`, uppercase `"\X41"`, or incomplete hex `"\x4g"`, `"\x4"`) strips the backslash and emits `c` (`aq`, `X41`, `x4g`, `x4`).
     - **Single quotes (`'...'`)**: Only `\'` is unescaped to `'`; every other backslash (including `'a\\b'`) is retained literally (`a\\b`), and a trailing `'a\'` leaves the quote unclosed (`Invalid argument(s)`).
     - **Closing quote boundary**: A closing `"` or `'` must be followed by whitespace, EOL, or an unquoted `#` comment; `ECHO "foo"bar` and `ECHO 'foo'bar` return `Err("redis: Invalid argument(s)")`, matching `sdssplitargs`.
     - **Unquoted-to-quoted concatenation**: `ECHO foo"bar"'baz'` concatenates into a single token `foobarbaz`, matching `sdssplitargs`.
     - **Comments**: Whole-line and whitespace-preceded inline `#` comments (`PING # health check`) are stripped before token emission, while `#` inside quotes (`"#vip"`) or mid-token (`user#1`) is preserved.
2. **Binary Keys & Values (`Vec<u8>` Request → Base64 Response)**:
   - `try_split_args` returns `Vec<Vec<u8>>` and `exec_command` passes raw `&[u8]` slices to `redis::cmd`, so non-UTF-8 bytes (`SET bin "\x00\xff\xfe\x01"`) are sent without UTF-8 replacement corruption.
   - On response, `bytes_to_json` (`redis.rs:1164–1171`) renders valid UTF-8 as a plain JSON string and non-UTF-8 bytes as standard base64 (`"//4AAQ=="`), never a lossy placeholder.
3. **Positive Two-Word Read-Only Classification (`statement_is_read_only`, `redis.rs:1503–1620`)**:
   - Uses `try_split_args` so quoted verbs (`"SET" k v`, `'KEYS' *`) cannot bypass classification, and unclosed/invalid quotes fail closed (`false`).
   - Uses positive subcommand allowlists for `CONFIG` (`GET|HELP`), `CLIENT` (`ID|INFO|LIST|GETNAME|GETREDIR|TRACKINGINFO|HELP`), `MEMORY` (`USAGE|STATS|DOCTOR|MALLOC-STATS|HELP`), `ACL` (`WHOAMI|LIST|USERS|GETUSER|CAT|DRYRUN|HELP` and `LOG [count]`, with `ACL LOG RESET` classified as write), `SLOWLOG`, `LATENCY`, `FUNCTION`, `SCRIPT`, `CLUSTER`, `MODULE`, `OBJECT`, `XINFO`, `PUBSUB`, and `COMMAND`.

## Scores (Iteration 09)

| Lens | Score | Evidence Boundary |
|------|------:|-------------------|
| **L13** | 9.9 | Positive two-word subcommand allowlists delegated from `types::redis_is_write` and `types::redis_uses_keys`. |
| **L14** | 9.9 | All 11 `sdssplitargs` quote/escape/boundary cases verified against `sds.c` and live `redis-cli` 7.4.11. |
| **L15** | 9.9 | Binary bulk strings round-trip via `\xHH` input and base64 output; 64-bit ints use `i64_to_json`. |
| **L16** | 9.8 | Parser errors reject the entire batch before opening a socket or mutating state. |
