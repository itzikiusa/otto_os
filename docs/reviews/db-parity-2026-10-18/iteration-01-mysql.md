# Iteration 01 — MySQL native parity (L05–L08)

Correctness review skill applied. Scores assess demonstrated parity, not percentage of MySQL's command grammar tested. The requested 9.8 target is **not established** by this pass.

| Lens | Before | After | Evidence and remaining limits |
| --- | ---: | ---: | --- |
| L05 native statement coverage | 6.5 | 9.3 | COM_QUERY replaces prepared execution for user reads/commands. USE and SQL PREPARE/EXECUTE now work; EXECUTE/CALL/admin rowsets are displayed. Client DELIMITER/routine script splitting belongs to the shared splitter review. LOCAL INFILE remains unsupported by SQLx. |
| L06 multiple results/affected rows | 5.5 | 9.2 | CALL result boundaries, final acknowledgement, caps, and trailing batch results survive; CTE UPDATE retains affected rows. Empty CALL result sets still lack columns because SQLx exposes metadata only with rows. |
| L07 values/metadata | 8.0 | 9.4 | Full-range signed TIME durations, exact unsigned/decimal, binary, NULL and timestamp microseconds validated live. Empty SELECT columns recovered through describe without re-execution. Empty dynamic/native result sets remain a metadata limitation. |
| L08 timeout/session/database | 7.0 | 9.4 | Whole-request timeout covers lowercase SELECT, DO and batches, with native cancellation while connection remains checked out; timed sessions retire. Same-session batches and database selection validated. Cross-request transaction/session pinning is deliberately not advertised or implemented. |

## Confirmed findings and fixes

1. **Major, confirmed by code trace and live regression:** `exec_write_conn` previously called prepared `.execute()`. Thus commands returning rows (CALL, EXECUTE, CHECK/ANALYZE) lost their output, and commands unavailable in prepared protocol failed. At `crates/otto-dbviewer/src/drivers/mysql.rs:2323`, native `raw_sql().fetch_many()` now separates rows and acknowledgements, retains each boundary and affected count, and drains capped routine rows so later statements execute.
2. **Major, confirmed by live regression:** WITH was always a read; CTE UPDATE therefore discarded affected rows. `is_read_statement` now sends only provable read CTEs to the optimized read path. Other CTEs use the native command collector, preserving native success/errors rather than rejecting syntax.
3. **Major, confirmed by live regression:** chrono NaiveTime cannot represent negative or >=24-hour TIME values. `mysql_value_to_json` now attempts SQLx MySqlTime before raw fallback. Text protocol additionally avoids exposing binary temporal payloads.
4. **Major, confirmed by live regression:** timeout_ms previously depended on an uppercase-only SELECT hint and did nothing for DO or batches. `run_tracked` now imposes a request deadline, invokes KILL QUERY while the target connection is still owned, and closes timed-session connections on drop. Native cancel has its existing 10-second cleanup bound; a timeout cannot undo already committed writes.
5. **Minor, confirmed by live regression:** zero-row SELECT had no column names. `exec_read_conn` retrieves metadata through an optional non-executing describe when no row supplied it. Unsupported native describe variants keep their successful result.

## Verification

Against existing local dev MySQL **9.7.1**, `OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test mysql_e2e -- --ignored --nocapture`: **25 passed**. Six new regression tests cover native commands/dynamic results, CTE write counts, duration/precise values, CALL result boundaries, empty SELECT metadata and all-class timeouts. Existing truncation/session-reuse, same-session batches, partial errors, governed reads, timestamps, schema and completion tests also passed.

`cargo test -p otto-dbviewer --lib`: **488 passed, 7 ignored**. MySQL focused unit tests: **13 passed**. `cargo clippy -p otto-dbviewer --all-targets -- -D warnings`: passed. A final read-CTE classification refinement preserves the prior optimized read path and was followed by the focused MySQL unit rerun.

Docker was initially unavailable, then root started Docker; this pass started only the repository's existing isolated MySQL dev service. No user database was removed. The CALL test uses a unique fixture routine and drops that exact routine after collecting the response. No commit or push performed.

## Limits needing separate decisions/evidence

- SQLx's stream omits independent metadata events: zero-row stored-procedure/dynamic results cannot be reconstructed faithfully without driver-library support. Re-executing CALL to infer columns would be incorrect.
- LOCAL INFILE protocol handling is not implemented by the SQLx MySQL executor. Native client file-transfer features are not equivalent to a SQL-only query API.
- Session state persists within one submitted batch. Separate Run requests intentionally obtain independent pooled sessions; capabilities.transactions remains false.
- Cancellation and timeout do not reverse already committed DML or implicit-commit DDL. The timeout is a whole-request budget including pool acquisition and batch execution.
- Successful timed reads retire their sessions, trading connection reuse for safe abort behavior. Untimed bounded reads retain pooling (live regression passes).
- No exhaustive MariaDB/Percona version matrix, mode-dependent lexical matrix (e.g. changing NO_BACKSLASH_ESCAPES mid-batch), privilege-level admin suite or SQLx-empty-result metadata proof exists yet; these prevent a defensible universal 9.8 score.

Official references: [MySQL TIME range](https://dev.mysql.com/doc/mysql/8.0/en/time.html), [prepared statement restrictions and CALL result protocol](https://dev.mysql.com/doc/refman/8.0/en/sql-prepared-statements.html). SQLx implementation was inspected locally in sqlx-mysql 0.9.0 connection/executor.rs, types/mysql_time.rs and value.rs.
