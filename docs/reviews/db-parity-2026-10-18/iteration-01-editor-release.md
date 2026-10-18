# Iteration 01 — L21–L24

Reviewed original distinct/session-sizing changes plus engine handoffs against the
query editor, shared splitter/classifier, result notices and published contracts.

| Lens | Score | Evidence and remaining work |
|---|---:|---|
| L21 query routing | 8.8 | MySQL DELIMITER bodies split at internal semicolons; ClickHouse escape/backtick handling incomplete. Fixed shared splitter and whole-buffer editor selection. Initial standalone splitter 24 tests and editor/dialect 16 tests passed. |
| L22 dialog/accessibility | 9.8 | Fixed optional Modal height and NewSession800px within existing backdrop cap. Four named Playwright desktop/phone, light/dark cases passed; screenshots reviewed. Existing keyboard case passed. No new finding. |
| L23 contracts/errors | 8.6 | Existing docs describe pre-parity transport limitations; distinct truncation tooltip recommends unsupported LIMIT. Updates required after driver changes settle. |
| L24 regressions/release | 8.5 | Initial distinct/sizing gate passed2046 Rust and1991 UI tests. Wider parity edits are not covered by those results; live engine fixtures now available. Final combined gate/CI/install pending. |

Fixes are on the shared branch; no commit, PR or deploy occurred in this iteration.
