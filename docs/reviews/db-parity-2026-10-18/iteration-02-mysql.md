# Iteration 02 — independent MySQL follow-up (L05–L08)

Scope: re-traced native read/command dispatch, export, multi-result completion, zero-row metadata, timed cancellation and pooled session cleanup after the first implementation. Review remains evidence-driven: no universal 9.8 claim while observed metadata limitations remain.

| Lens | Prior | Current assessment | Evidence |
| --- | ---: | ---: | --- |
| L05 native statement coverage | 9.3 | 9.5 | Root added DELIMITER-aware scripts; new live test creates a multi-SELECT procedure and CALLs it through the actual driver. Export now uses native text protocol too. |
| L06 result fidelity | 9.2 | 9.3 | Empty command/result acknowledgements now explicitly say "0 row(s) affected / empty result set" instead of falsely implying a write-only completion. SQLx cannot distinguish an empty CALL rowset from its status packet through the current public stream. |
| L07 values/metadata | 9.4 | 9.5 | Empty and max_rows=0 CSV exports retain column headers; duration export uses the same native text decoding. Empty CALL metadata limitation remains. |
| L08 timeout/session/database | 9.4 | 9.6 | Governed reads now retire sessions that retain nontransactional variables/locks; bounded exports retire unread sessions. Added transaction rollback/temp-table/same-batch variable and subsequent-request isolation regression. |

Scores are provisional until the scheduled iteration-02 Cargo/live tests finish; iteration-01's 25 live tests are the last completed live suite at report creation.

## Findings fixed

- **Major, confirmed code trace:** export still used the prepared protocol while interactive Run used COM_QUERY. Replaced with text protocol. A dedicated one-statement validation preserves the export's existing single-table contract instead of silently flattening multiple result schemas.
- **Minor, confirmed code trace:** empty export wrote `write_header(&[])`, and a cap of zero stopped before extracting the first row's columns. Empty schema is now retrieved without re-execution; the zero-cap path captures metadata before stopping. New live export test checks exact CSV output.
- **Major, confirmed code trace:** capped export's dropped read-only transaction queued rollback against unread rows, forcing the pool's later cleanup to drain a potentially enormous result. Retire that session on drop whenever export has a cap; rollback occurs when its connection closes.
- **Major, confirmed code trace:** governed_read assumed rollback removed all query effects, but MySQL user variables and advisory locks are session state, not transactional. It now applies the same sql_leaves_session_state retirement used by ordinary reads. The direct-driver regression assigns a variable inside the governed transaction and checks its absence on a subsequent request.
- **Minor, confirmed SQLx source trace:** a zero-row CALL result boundary is exposed identically to an acknowledgement by SQLx fetch_many. Preserve the boundary and explicitly ambiguous message. No repeated CALL, guessed columns or fabricated metadata was introduced.

## Added verification cases

- DELIMITER CREATE PROCEDURE with two internal SELECT statements, followed by CALL through the driver.
- Empty CSV header, zero-row cap header, full-range signed TIME exported as text; multi-statement export refused.
- CREATE TEMPORARY TABLE + SET variable + START TRANSACTION + INSERT + ROLLBACK + SELECT shares one session; subsequent Run sees no assigned variable.
- Governed read session variable retirement.

Direct native CLI verification on MySQL 9.7.1 confirmed signed TIME and 838-hour values. The driver live suite is now 29 tests, awaiting the shared Cargo slot. `git diff --check` passed before the final governed-read addition; rustfmt ran on both owned Rust files.

## Remaining limits

SQLx lacks metadata-only events for zero-row stored-procedure result sets; LOCAL INFILE is unsupported in its executor; independent Run requests intentionally lack session-pinned transactions. Root is reviewing the shared session-state detector's whitespace-sensitive lock-function recognition. Vendor-matrix coverage alone is not treated as a defect or reason to withhold 9.8; the concrete empty-procedure metadata gap is.

## Completed follow-up: native empty-result metadata

The provisional limitation above was subsequently removed through the root-approved narrow SQLx patch described in `iteration-02-mysql-metadata-proposal.md`. Completion packets now carry native rowset columns, including zero-row sets. The ordinary reader, CALL collector and exporter consume those packets; the describe fallback and ambiguous zero-result message were removed. No command replay occurs.

Final live verification: **30/30 tests passed** on MySQL9.7.1, including two empty CALL result schemas distinguished from its final acknowledgement, native DELIMITER procedure creation, CSV empty headers, negative TIME with a single-digit hour, transaction rollback and governed-session isolation. The first30-test run had28 passes and2 fixture failures caused by reserved aliases FIRST_VALUE/LAST_VALUE; renaming those aliases fixed the tests, with no implementation workaround.

Current lens assessment: **L05 9.7, L06 9.8, L07 9.8, L08 9.8**. L05 retains the explicit SQLx LOCAL INFILE protocol limitation; whether client-local file transfer is outside the agreed query-console scope is a product decision, not a reason to silently claim universal parity. Cross-request sessions remain intentionally unpinned and accurately advertised.

The focused MySQL unit rerun was temporarily blocked by a concurrent Redis test raw-string syntax error at redis.rs:2072; this is separate from the successfully compiled and executed live integration suite. Final unit rerun status will be appended after the shared source is parsable.

Final focused unit rerun after the Redis syntax correction: **13/13 MySQL unit tests passed**, exit0. Cargo slot released to Mongo. Live integration suite remains30/30 passed with the vendored metadata implementation.
