# Mongo current-head verification — 2026-10-18

After all iteration04–10 changes, including the trailing-comment integration repair and stable1.99 unnecessary-unwrap fix, the scheduled Mongo Cargo gate completed successfully. No further Mongo implementation changes followed these checks.

| Command | Actual result |
|---|---|
| `cargo test -p otto-dbviewer --lib drivers::mongo` | **98 passed,0 failed,5 ignored** |
| `cargo test -p otto-dbviewer --lib mongosh_` | **10 passed,0 failed** (overlaps some driver tests; not an additive unique-test count) |
| `OTTO_DBV_E2E=1 OTTO_MONGO_PARITY_PORT=27039 cargo test -p otto-dbviewer --test mongodb_e2e -- --ignored --nocapture` | **13 passed,0 failed**, Mongo8.3.13 disposable fixture |
| `cargo test -p otto-dbviewer --lib mongo_distinct_e2e -- --ignored --nocapture` | **1 passed,0 failed**, test-owned disposable container |

The current unit pass includes separated legacy count parsing/command fields/numeric validation, bounded native cancellation timeout/success/error paths, comment-trivia acceptance and malformed-tail rejection. The script-policy pass includes the exact previously failing trailing-comment input and literal shell markers staying native/read-only.

The live suite now covers legacy count with `$where` succeeding while countDocuments with the same predicate fails, JSON legacy count, raw do/switch/if/comment-prefixed scripts, native explain settings, sparse typed exports, fallback options/results, script timeout, and two concurrent operations with the same client ID remaining cancellation-isolated. Existing distinct live regression also passed separately.

UI detector tests last ran after the latest predicate changes: `node --test ui/unit/dbSqlScan.test.ts` **9 passed,0 failed**. The later parser-trivia and cancellation changes are Rust-only. Scoped rustfmt and diff checks passed during implementation; root owns whole-workspace clippy/check, CI and release.

## Final scoped Mongo assessment

L01 native methods **9.8**; L02 parser/BSON fidelity **9.8**; L03 shell coverage **9.8**; L04 results/timeouts/cancellation **9.8**. These replace the pending-gate assessments in iterations04–10. They are scoped engineering assessments, not proof of every server-version/command permutation.

Limits remain explicit: finite native parser, installed mongosh for unsupported expressions, conservative write policy for unproven JavaScript, raw scripts' own print behavior, shell-version syntax limits, native find/aggregate-only streaming export, fixed CSV schema, SQL-to-Mongo convenience subset, and no newly exercised remote SSH/TLS fixture. No user databases were touched. Mongo Cargo slot was released to root immediately after verification; no commit/push/deployment performed by this worker.
