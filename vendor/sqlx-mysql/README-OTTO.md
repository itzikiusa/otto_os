# Otto patch: metadata on MySQL result completion

Vendored from the crates.io `sqlx-mysql` **0.9.0** release, package checksum
`90b8020fe17c5f2c245bfa2505d7ef59c5604839527c740266ad2214acebea27`.
Upstream repository: https://github.com/launchbadge/sqlx
Upstream revision: `003b698e99e024f3621b8043a2426fde5b741171`, subdirectory `sqlx-mysql`.
The upstream MIT and Apache-2.0 licenses are preserved alongside this file.
`Cargo.toml` is the registry-normalized manifest; `Cargo.toml.orig` preserves the
upstream workspace manifest. Registry cache markers and the package lockfile are
not vendored. This is a local patch, not a claim of upstream acceptance.

## Why

`Executor::fetch_many` emits a `MySqlQueryResult` at every native result boundary,
but upstream retains only affected rows and last insert ID there. Column metadata
is visible only on actual rows. Consequently, a zero-row SELECT in CALL is
indistinguishable from CALL's final acknowledgement. Preparing CALL cannot reveal
the runtime result schemas; repeating it would repeat side effects.

## Metadata patch

The metadata patch changes `src/query_result.rs` and `src/connection/executor.rs`:

- Add an optional shared column snapshot to `MySqlQueryResult` and a public
  `columns() -> Option<&[MySqlColumn]>` accessor.
- Standalone command OK packets carry `None`; resultset EOF/OK packets carry
  `Some(Arc::clone(&columns))`, even when the set contains no rows.
- `Extend` follows the last result's metadata, matching its existing last-ID
  behavior. Existing `rows_affected()` and `last_insert_id()` behavior is unchanged.

The metadata patch does not change protocol parsing, authentication, TLS, query
execution or row representation. One Arc clone per completed rowset; no per-row allocation added. Otto's
MySQL reader/exporter can now use native metadata without a second statement.

## Verification and upgrades

Live regressions reside in `crates/otto-dbviewer/tests/mysql_e2e.rs`, including
`mysql_call_preserves_empty_result_metadata_and_ack` and empty SELECT/export
metadata checks. Run against the isolated dev MySQL server with
`OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test mysql_e2e -- --ignored`.

When updating SQLx, compare the metadata changes in those two files against the new release and
retain this narrow patch only until upstream exposes equivalent metadata. Keep
all workspace SQLx packages at compatible versions. Do not replay user commands
as a metadata fallback.

## Additional Otto patch: explicit LOCAL INFILE bytes

The native query console additionally needs LOAD DATA/XML LOCAL INFILE. This
patch adds opt-in capability negotiation and a **one-command bytes grant**:

- `MySqlConnectOptions::local_infile(bool)` defaults false.
- `MySqlConnection::no_backslash_escapes()` reads server status without issuing
  SQL that could alter session ROW_COUNT()/FOUND_ROWS() counters.
- `MySqlConnection::authorize_local_infile(filename, bytes)` grants only those
  supplied bytes for the following execution; SQLx never opens files.
- The executor consumes the grant and accepts a server0xFB request only when its
  filename bytes exactly match. Missing/mismatched/repeated requests shut down
  the connection without sending file contents.
- Successful uploads use bounded64KiB protocol packets, preserving sequence IDs,
  an empty terminator, and the server's ordinary OK/error acknowledgement.
- The grant is consumed even if the query produces an error before requesting
  the file; there is no retry/replay.

Beyond the two metadata files, this affects `src/options/mod.rs` and
`src/connection/{mod,stream,establish}.rs`. Otto performs its existing root-only
host-file authorization before reading the explicitly submitted filename, and
uses a dedicated root-authorized request pool with the capability enabled. This
allows later SQL-mode changes without predicting the entire script upfront, at
the cost of connection setup for scripts that may contain LOCAL. A conservative,
case-insensitive byte scan gates the host-authority lookup independently of SQL
mode; ordinary queries retain shared pools with the capability disabled.

Regression: `crates/otto-dbviewer/tests/mysql_local_infile.rs` loads native
FIELDS/LINES/IGNORE/SET data and uses a protocol proxy that replaces the server's
requested filename with another existing file, asserting zero uploaded bytes.
