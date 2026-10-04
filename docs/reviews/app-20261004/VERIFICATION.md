# Verification log

Baseline (`a16f4c71`): UI check passed; 989 unit tests passed; fresh daemon build passed (12m08s); desktop page-chrome E2E 5/5 passed. See PERFORMANCE.md for real-app sampling and incomplete scale baseline.

## First implementation wave

Parent executes tests; implementation agents edit only. Narrow package tests triggered additional Cargo feature-set compilation in the dedicated target. One heavy command at a time, two Cargo jobs.

| Behavior | Before fix | Latest result / log |
|---|---|---|
| Filesystem error vs missing transcript | Failed Gone vs Unknown | Red `/tmp/otto-review-session-red.log`; green pending |
| Reconnect gap cursor | 2 failures in 17 tests | Fixed cases pass in 31-test UI group |
| Workspace recovery/archive/follow-live | 3 failures in 27 tests | 31/31 passed including Terminal ownership, `/tmp/otto-review-session-ui-green.log` |
| Terminal async image ownership | 3 failures; valid same-target case passed | 4/4 passed in session group above; first harness extraction mistake corrected before valid red |
| Palette failure/stale response | 2/2 failed | 2/2 passed `/tmp/otto-review-palette-green.log` |
| Live transcript byte budget | Retained >8MiB, 1 failure | Fix implemented; expanded tests running |
| Database projection/CH targeting, script variable writes | 5 failures in 30 tests | 35/35 passed `/tmp/otto-review-data-tools-green.log` |
| Expanded data-tool recovery/cache suite | 40 pass,1 fail | Save timing fixture needed explicit start barrier; rerun pending |
| Git auto-stash index preservation | Cached diff lost; regression failed | Fix implemented; green pending |
| Sparse Mongo result expansion | >1M resident cells; regression failed | Fix implemented; green pending |
| Schema registry lazy history | 0 IDs returned vs100; regression failed | Fix implemented; green pending |
| Terminal ring allocation | Initial pointer-address test passed original implementation | Invalid regression measure; replacing with counting allocator before evaluating fix |
| Large K8s backfill | 1.2M-series fixture passed; 3M-series fixture failed at embedded1GiB limit | 32MiB spill/256MiB query trial failed during external merge;64MiB spill/384MiB trial running. Logs `/tmp/otto-review-backfill-before-3m.log`, `-after.log`, `-after-384.log` |

Passing a focused group does not yet establish full UI/Rust integration. Full check, independent re-review, combined-main integration, screenshots, and completed scale/leak measurements remain required.

- First-wave full UI gates: `npm run check` passed (0 errors/warnings); `npm run test:unit` passed 1,015/1,015. Logs `/tmp/otto-review-wave1-ui-check2.log`, `/tmp/otto-review-wave1-ui-unit.log`.
- Expanded session UI group passed40/40, including byte limits. Data recovery/schema cache group passed8/8.
- Robust ring allocation regression now reproduces: 1MiB input causes201,326,592 allocated bytes before repair. `/tmp/otto-review-ring-allocation-red.log`.
- K8s64MiB aggregation spill/384MiB query budget PASSED3M-series migration under embedded1GiB server in42.29s including seeding, exact1m totals and completed-init no-op. `/tmp/otto-review-backfill-after-384.log`. Interrupted retry and other tier totals added afterward; rerun pending.

- Data-tool Rust focused GREEN: schema-history1/1, Mongo shaping2/2, Git auto-stash8/8. `/tmp/otto-review-data-rust-green.log`. Independent iteration2 recheck then identified three remaining cases; see iteration-2-correctness-2.md (identifier folding, explicit LIMIT forms, workspace leave guard).

## First-wave checkpoint (2026-10-04)

- Updated full UI gate: 0 errors/warnings, 1,027 unit tests passed (`/tmp/otto-review-wave1-final-{check,unit}.log`). This includes corrected notification fixture; its earlier failure did not exercise production behavior and is not valid behavioral-red evidence.
- Full `otto-pty` and `otto-transcript` tests including integration/doc targets passed (`/tmp/otto-review-transcript-pty-green.log`).
- Expanded isolated K8s interrupted-backfill/retry scale test is running; final Rust consumer/clippy gates, rendered flows and final performance runs remain pending.

- K8s expanded isolated regression passed (51.59s test runtime): 3 million distinct raw series, injected interruption after first rollup tier, successful retry, exact aggregate counts across all tiers, completed no-op. `/tmp/otto-review-backfill-retry-green.log`. Server retained its 1 GiB hard limit; no installed server data/config changed.
- Main design iteration one integrated at `04917dd7`, including PR75 nightly test headroom. Auto-merges inspected; no conflict markers.
- Role3 real-store red tests: 5/5 expected stale-state failures, `/tmp/otto-review-role3-ui-red.log` (Product selection/collections/workspace and Browser annotations).

- Database type/parser suite passed: 56 tests including explicit LIMIT/FETCH/locking and trailing-comment cases (`/tmp/otto-review-r2-sql-green.log`).
- API persistence browser suite passed 4 tests. Initial History run was invalidated by Vite HMR (trace proves component remount); rerun with HMR disabled passed actual wheel paging with cursors 120 then 60. `/tmp/otto-review-wave1-e2e-stable.log`.
- Added rendered automation leave regression passed: Cancel preserves draft, failed Save stays in editor, successful Save persists before leaving, Discard leaves without overwriting saved steps (`/tmp/otto-review-automation-leave.log`, 1 test, 2.1s). Initial selector assumed expanded Navigator; corrected for collapsed Modules navigation.
- Design stale snapshot CAS regression passed after concurrent implementer fix (`/tmp/otto-review-design-cas-red.log`, despite filename this was a GREEN run).

- Disposable SQL integration: MySQL 8 passed 3 batch tests, PostgreSQL 17 passed 2. Actual server-side counters reached 3 for a max_rows=2 preview, and following statements retained the same backend connection. Explicit limit forms preserved. Created dedicated loopback-only containers (768 MiB/one CPU each, sequential), then removed both; `docker ps` empty. `/tmp/otto-review-sql-fixtures.log`.
- Knowledge recovery unit tests now 5/5 pass: explicit recopy, failed-save leave, pending save drain, publish preview retry, artifact keep-mine failure. `/tmp/otto-review-knowledge-green.log`.
