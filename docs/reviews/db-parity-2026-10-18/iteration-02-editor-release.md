# Iteration 02 — L21–L24

Independent MySQL reviewer examined shared DELIMITER implementation. Root traced
classifier, session-state detection and result guidance against current drivers.

| Lens | Score | Evidence and remaining work |
|---|---:|---|
| L21 query routing | 9.5 | Fixed quoted DELIMITER tokens (including embedded spaces), removed repeated whitespace prefix scans, whole-buffer editor selection retained. Standalone splitter26/26 and editor/dialect17/17 passed. Engine classifier now uses executor dialect, avoiding phantom writes in quoted identifiers/dollar strings; crate validation pending. |
| L22 dialog/accessibility | 9.8 | Re-read fixed-height implementation against existing viewport cap and optional-default behavior. No further layout edits; prior four browser cases remain applicable. |
| L23 contracts/errors | 9.2 | Truncation tooltip now says raise row cap/narrow query, without promising SQL LIMIT or streaming export. Documented native vs shell fallback, same-batch session behavior, ClickHouse transports. Final driver review still needed. |
| L24 regressions/release | 9.0 | Live engine tests underway; MySQL empty-procedure metadata gap required minimal vendored SQLx patch. Shared session-state detector now recognizes whitespace/comments before GET_LOCK/SET_CONFIG and transaction chaining. Full combined gate/CI/install pending. |

No universal database-compatibility claim is made. Protocol behavior, supported
versions, policy and client-only commands have distinct verification boundaries.
