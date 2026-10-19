# Iteration 04 — Mongo and integrated editor/policy review

Read-only integration sweep followed by coordinator-authorized repair of the shared Mongo script detectors. No Cargo invocation in this pass; upgraded-toolchain Rust/live gates remain pending with the coordinator.

## Confirmed finding and repair

`types::looks_like_mongosh_script` and the editor's `looksLikeMongoshScript` recognized a fixed list of whitespace-specific prefixes. A valid `do { print('42'); } while(false)` or `switch(1) { case 1: print('42'); break; }` missed detection, reached the expression fallback, and was placed inside parentheses where a statement is invalid. The editor likewise missed the whole-script execution path. Leading inline comments and tab-separated `if` had the same gap.

Both predicates now strip leading block/line comments while retaining code after a closing comment and recognize complete keyword tokens. They reject keyword-shaped document keys followed by a colon, including intervening comments. UI and Rust regressions cover do/switch, whitespace, declarations after comments, multiline comments, async functions, and native document keys. Four raw-script execution cases were added to the existing Mongo shell integration test.

The predicate recognizes `for await` as script syntax; this is routing only. A direct probe found the installed mongosh rejects top-level `for await` itself. No successful execution claim is made for that shell-unsupported form, and it is not included in the live-success assertions.

## Evidence

- Before implementation, `node --test ui/unit/dbSqlScan.test.ts`: **7 passed, 1 failed**, specifically the new do/while detection assertion.
- After implementation, same command: **8 passed, 0 failed**.
- Direct mongosh against disposable port 27039: do/while, switch, if, and comment-prefixed declaration printed **four expected `42` lines**, exit 0.
- Rust regressions and driver live cases added but **not executed in this pass**, because the coordinator owns the shared Cargo slot and upgraded stable toolchain rebuild.
- Scoped rustfmt and `git diff --check`: passed.

## Integrated paths traced

LOCAL INFILE service preparation clears any saved authorization marker before checking host-file access, requires current root for explicit LOCAL INFILE, and is called on ordinary and approved execution. MySQL uses a request-owned pool with LOCAL_FILES enabled, binds uploaded bytes to the statement's literal filename, and validates size/type. Final compilation and integration remain pending; this pass does not certify the concurrently changing vendor implementation.

The owner/native-ceiling exception applies only to the connection owner's own profile. Alternate credential profiles retain the native ceiling. SQL write classification uses the executor's dialect splitter. The editor sends DELIMITER scripts as a complete buffer. Modal's optional fixed height retains its backdrop-relative max-height, scrollable body, and nonshrinking footer. No additional confirmed defect was found in these traced paths.

## Scores and verification boundary

| Lens | Score | Evidence / remaining condition |
|---|---:|---|
| L01 native Mongo coverage | 9.8 | Prior live/unit evidence retained; no integration regression found. |
| L02 Mongo parser/BSON | 9.8 | Prior parser/BSON evidence retained; reserved-word document-key regressions added. |
| L03 mongosh routing | 9.7 | Confirmed gap repaired; UI RED/GREEN and direct shell success; Rust/driver live gate pending. |
| L04 Mongo results/control | 9.8 | Prior cancellation/timeout/export evidence retained; no new affected execution mechanism. |
| L21 editor routing | 9.7 | Shared detector repair passes targeted UI tests; Rust counterpart pending. |
| L22 dialog sizing | 9.8 | Static sizing invariants and prior browser evidence retained; no new dialog edits. |
| L23 contracts/guidance | 9.7 | Broad script-routing claim repaired; actual shell syntax/version limits remain applicable. |
| L24 release compatibility | 9.5 | New toolchain and LOCAL INFILE integration gates remain coordinator work. |

Scores are scoped review assessments, not claims of universal database compatibility. This pass preserved others' edits, made no commit/push, and touched only delegated predicate/test regions plus this report.
