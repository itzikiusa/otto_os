# MySQL empty-result metadata: approved implementation direction

Root reviewed and approved the dependency patch before modification.

## Decision and evidence

SQLx 0.9.0's public `Executor::fetch_many` returns `Either<MySqlQueryResult, MySqlRow>`. The former exposed only affected rows/last insert ID, while metadata was attached exclusively to actual rows. Its MySQL executor already holds an Arc<Vec<MySqlColumn>> while processing every resultset EOF/OK; no protocol changes are necessary to preserve it.

Vendoring the 492KB sqlx-mysql package and exposing that already-known metadata is narrower than introducing a second MySQL client or implementing another authentication/TLS/protocol stack. Preparing CALL does not describe its runtime resultsets, and re-executing routines is unacceptable because it repeats their effects.

Implementation:

- `vendor/sqlx-mysql/src/query_result.rs`: optional column snapshot and public `columns()`; Extend follows the last completion.
- `vendor/sqlx-mysql/src/connection/executor.rs`: command acknowledgements carry None; completed rowsets carry Some(shared columns).
- Root Cargo.toml adds the existing repository's established local patch mechanism. No additional runtime dependency.
- `MysqlDriver` reads native completion metadata for ordinary reads, commands and exports. No describe fallback or statement replay remains.
- Test `mysql_call_preserves_empty_result_metadata_and_ack` checks two different empty result schemas, a nonempty rowset and CALL's final acknowledgement in order.
- Upstream licenses, normalized/original manifests, revision and checksum preserved in `vendor/sqlx-mysql/README-OTTO.md`.

A source comparison confirmed exactly those two upstream files changed. Build/live verification awaits the shared Cargo slot; the lockfile has not been manually edited.

## L21 independent review of shared DELIMITER changes

Reported to root for its shared-file ownership:

1. **Major, confirmed native CLI reproduction:** quoted delimiter `DELIMITER '$$'` is accepted by mysql CLI, while the initial new splitter saved the quote characters as part of the delimiter and failed to recognize `SELECT 1$$`. Strip matching delimiter quotes.
2. **Minor, confirmed native CLI reproduction:** `DELIMITER 'x y'` also works in mysql CLI. Initial splitter whitespace rejection and UI `\S+` detection rejected this valid quoted delimiter. Parse complete quoted values and allow their interior whitespace in both layers.
3. **Minor performance, confirmed code trace:** new directive detection runs for every leading whitespace character; repeated prefix/line scans make an N-space input quadratic. Gate on D/d before checking the line and token.

Native reproduction results (MySQL 9.7.1): both scripts below output 1 then 2:

```sql
DELIMITER '$$'
SELECT 1$$
DELIMITER ;
SELECT 2;
```

```sql
DELIMITER 'x y'
SELECT 1x y
DELIMITER ;
SELECT 2;
```

No shared splitter/UI files were edited by this reviewer.

## Implemented and verified

Root approved this proposal; implementation is complete. All30 live MySQL integration tests passed with the vendor patch, including metadata-only CALL boundaries and ordinary empty SELECT/export. The lockfile now selects the local sqlx-mysql patch via normal Cargo resolution. An exact source comparison confirms only the two listed upstream files differ.

Root also applied the quoted/space-containing DELIMITER fixes and the D/d early gate. A second code review confirmed those changes address the three L21 findings above.
