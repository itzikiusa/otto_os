# Completion audit — not release sign-off

This checklist preserves the user's whole requested outcome. A passing subset
or a high score in one lane does not satisfy the remaining rows.

| Requirement | Current evidence | Decision / next proof |
|---|---|---|
| Mongo distinct and native count/countDocuments/sort semantics | DB iteration reports, live distinct/parity tests, documented parse-only mongosh fallback | Implemented and exercised; final changed-tree gates still apply |
| Useful full Mongo expression/script fallback | Feature contract and shell policy/live fixtures; same connection/database, Extended JSON, no retry after execution error | Supported with installed mongosh; arbitrary JS is not claimed statically read-only |
| MySQL/Redis relevant commands and operation semantics | DB final-verification native/live matrices, transaction/selection/binary/LOCAL/blocking fixtures | Covered to the documented matrix; no universal server/version claim |
| ClickHouse relevant commands and values | Ten review passes; HTTP/native live fixtures; explicit native JSON/Dynamic/Variant limitation | Below target; retain limitation and scores, do not call universal parity complete |
| Consistent new-session dialog across providers | Four desktop/phone light/dark browser cases and screenshots | Verified browser behavior; packaged WKWebView remains deployment boundary |
| Root and privileged-user usability | Manual root cwd, owner-native policy, messaging compatibility regressions; preserved cross-user/agent boundaries | Implemented; no new blanket restrictions authorized |
| Full-product engineering review, not only DB | E01–E20 actual reviewer roster, individual findings/fix reports | Initial coverage recorded; refund regressions passed; final UI/integration closure still pending |
| Ten design/UX reviewers in each original provider lane | design-agy and design-grok reports/IDs | Review provenance recorded; remaining implementation/browser closure is not implied by reviewer count |
| No new sessions; balanced provider work | coordination.md, original Ronaldo/Pele live sessions | Ongoing work uses these two only; closed sessions stay closed |
| Evidence-based 9.8 target, up to ten review/fix iterations | DB ten-pass ledger and product reports | Not all lenses at 9.8; no fabricated uplift or claim of ten product iterations |
| Current non-advisory checks pass | Workspace r3 tests/clippy/docs passed; Python3.13 guard retry passed; UI check/2056 units/build passed | Plugin bundle repaired; affected refund clippy and final consumers pending |
| One PR for all work | Existing PR111 at c009da20; expanded work remains uncommitted | Update same PR after final inventory and checks; old-head CI is insufficient |
| Merge and push main | No merge performed | Required only after expanded-head non-advisory checks pass |
| Rebuild, reinstall, replace running app | Isolated debug restart smoke only | Still pending; deploy receipt, health and installed revision must be verified |

Evidence locations: current-verification.md, verification-evidence.json,
iteration-02-automation-independent.md, the four root iteration-02 UX reports,
../db-parity-2026-10-18/final-verification.md, and PR111's live check rollup.
The active held-shell restart smoke proves reattachment and explicit fixture
cleanup, not termination of every detached application job.
