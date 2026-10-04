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

## Second-wave combined Rust gate

`cargo test -p otto-server -p otto-design -p otto-vault -p otto-state -p otto-mcp -p otto-usage --lib --no-fail-fast -- --test-threads=2` passed: Design123, MCP35, server1,283, State371, Usage42, Vault74 =1,928 tests (6 ignored). `/tmp/otto-review-wave2-rust-libs2.log`. Includes Product summary migration, concurrent design publication, Vault bounds/rename, swarm stop readiness and capability child-decision equivalence. Earlier run stopped on the batch test helper's 1MiB response cap; bounded1000-child response fixture now uses/asserts8MiB and verifies every decision against single-child endpoint.

Main design PR76 (`98012f47`) integrated at `431ed3fd`; automatic merge inspected. Upstream introduced trailing whitespace in CSS-only blank lines; our diff against updated main has no whitespace errors. Combined post-merge UI check underway. Added root HTTP/SSE endless-chunk cap regression afterward; pending focused execution. Assistant/platform work has started, so this is not the final full-branch gate.

- Post-PR76 UI integration passed: zero errors/warnings and 1,063 unit tests (`/tmp/otto-review-merge2-check2.log`, `/tmp/otto-review-merge2-unit.log`). Merge required restoring the `confirmer` import used by our Mission Control leave guard.
- Endless chunked HTTP and SSE regression passed (1 test, both content types): the MCP response cap rejects before EOF; `/tmp/otto-review-mcp-stream-cap.log`.

## Independent rechecks and rendered regressions

- Partition2 final source recheck approves all13 original/follow-up findings; report `iteration-3-partition-2.md`. This does not replace runtime checks.
- Composer deferred-upload remount browser test reproduces the exact bug: Send becomes enabled after Terminal→Chat while inbox response remains pending. `/tmp/otto-review-composer-remount-red.log`. Claude owns the pending final-branch repair.
- Product real-draft browser flow cannot reach Overview after selecting a story (`/tmp/otto-review-product-focus-red.log`). Independent trace confirms the workspace effect subscribes to selectedId through the new ownership helper, then clears selection. Fix and browser rerun pending. The earlier Jira fixture attempt did not reach its keyboard assertion either.
- Assistant state focused tests pass: simultaneous approval decisions yield one winner, and late completion cannot revive canceled task. `/tmp/otto-review-platform-state.log`. Subsequent schedule additions exposed missing Error imports during server compilation; implementation in progress.

- Product follow-up exposed a separate missing-source defect: real draft details looked up `source` instead of `draft` revisions, disabling dirty detection. Backend lookup and legacy-null editor fallback repaired. Browser now passes both existing Jira edit/save/reload and real draft canceled keyboard tab-focus preservation: 2/2, `/tmp/otto-review-product-final.log` (6.4s).
- Final focused recovery group passed36/36, including true Vault status-generation refresh and null-source draft dirtiness: `/tmp/otto-review-recovery-final-unit.log`.
- Assistant focused Rust tests passed49 server +7 state tests: `/tmp/otto-review-platform-assistant2.log`. Later cancellation registration/setup changes still require recompilation.
- Broad clippy attempt compiled state before a concurrent new method, then saw its server caller (E0599). The method exists in current source; freezing Rust source and rerunning is required. `/tmp/otto-review-clippy.log`.

- Seven-package Rust library gate passed1,968 tests with6 ignored: Design123, MCP36, Product30, Server1,289, State374, Usage42, Vault74 (`/tmp/otto-review-platform-rust-libs.log`). Includes real Product saved-revision reopening, canonical approval races, plugin replacement and bounded Insights filesystem counters. New Insights route-policy classification and the independently discovered resumed-task ownership follow-up were added/identified after compilation and need the closing gate.

## Closing integration gate

- `scripts/check.sh --base origin/main --check` with two Cargo jobs and nextest's four-process `ci` profile passed rustfmt and clippy. Nextest ran 4,592 tests: 4,591 passed, one failed, 85 skipped (`/tmp/otto-review-final-gate.log`). Both final task-resume/foreign-thread cancellation regressions passed. The sole failure was policy inventory: the new exact `/access/{kind}/{id}/capabilities/batch` path lacked the same handler-enforced classification as single-item capabilities. Added that exact path; independent source recheck confirms resource and child authorization remain enforced. Closing gate rerun pending; the failed run did not reach doc-tests or UI gates.
- Pre-closing UI validation passed zero check errors/warnings and 1,081 unit tests (`/tmp/otto-review-ui-closing-{check,unit}.log`). Platform browser run passed 38 tests and failed five. Four failures traced to stale report-status summary fixtures or retired History title selectors; the fifth injected a socket event before the chat view acquired history. Fixtures now wait for loaded history and model regenerated summaries. Browser rerun pending; no production repair is inferred from these fixture corrections.
- Exact external Composer file from commit `32d9b161` imported with its owner's authorization for early validation. Includes persistent per-session pending uploads, a pane-based textarea budget and removal of the popup minimum-height floor. This is pending browser verification and final upstream merge.

- Closing rerun passed rustfmt, clippy, all 4,592 nextest cases (85 skipped) and the doc-test command (`/tmp/otto-review-final-gate2.log`). The UI phase correctly rejected the partial Composer import's missing `toastError.ts` dependency. Imported the exact shared helper from external commit `483212b0`, plus its History phone-title repair and `Canceled` label/test pair. UI check then passed with zero errors/warnings; all 1,081 unit tests passed (`/tmp/otto-review-import-check.log`, `/tmp/otto-review-import-unit2.log`). The initial copy-label unit failure expected the old spelling and was resolved by importing the matching upstream expectation.

- Rebuilt the actual daemon in 3m53s (`/tmp/otto-review-daemon-final-build.log`), with the existing macOS debug unwind-section linker warning. Seven focused desktop specs ran 63 cases: 61 passed, including the real local-draft HTTP save/reopen, pending-upload remount, nine-tile long drafts in light/dark, and phone full-title checks. Two failures exposed actual integration bugs: same-ID Assistant metadata invalidated the history acquisition, and final History markup hid scope controls during an initial load. Logs `/tmp/otto-review-closing-browser.log` and `/tmp/otto-review-assistant-acquire-red.log`; the latter's trace shows an unwanted history response at 1945ms immediately after the live event at 1940ms.
- The Assistant effect now depends on a primitive derived thread ID; its browser regression requires both incremental messages and no extra history GET. The History fix and Assistant copy label were imported from final design snapshot `f4a3d6fa`. Both remaining cases passed in 3.4s (`/tmp/otto-review-final-two-browser.log`). UI check and all 1,081 units passed afterward (`/tmp/otto-review-ui-final-{check,unit}.log`). Light/dark tile screenshots were inspected and copied to `screenshots/`.


### External design effort's reported acceptance (PR 77)

Claude supplied this ledger; these are external results, not executions by this worktree. Its UI check had zero errors/warnings, 999 unit tests passed, and the expanded style guards passed. Its full desktop-browser run covered 210 non-performance specs at two workers: 1,188 tests, 952 passed, 137 failed, 82 skipped and 17 not run in 51.5 minutes. This was **not a clean full-suite pass**.

The external effort reran 67 failing files on main `98012f47`, reported 79 baseline failures and 54 branch-only failures, then repaired/reran the latter. It reported 38 passing in the first follow-up and 13 more in a one-worker follow-up of 18 cases. These follow-up counts are recorded as supplied and are not combined into a fabricated full-suite total. Remaining reports concerned the Rooms tail fixture and terminal text-coordinate helper handled here; four Docker Mongo cases; Help/tour film fixtures also failing on main; and database pool contention at two workers that passed at one worker. The fixture fixes in this worktree require their own executed result below.

External limits: no local WebKit performance subset (CI runs it), no real-app visual pass, and no light/dark screenshot attachments in its PRs. This worktree provides inspected constrained-chat light/dark captures, not a complete native-app visual or assistive-technology acceptance claim. Reviewer design scores used different models between iterations and are judgments, not comparable performance measurements.


### Final main integration

Integrated design PR77 (`01f0c290`) as `65e13b4f`, with no conflicts. Reviewed overlap hunks; all execution/draft ownership repairs remain. The merge changes no Rust/build inputs, so the completed Rust integration gate remains applicable. Combined UI check passes with zero errors/warnings, all 1,081 unit tests pass, and production build succeeds. Logs: `/tmp/otto-review-merged-ui-{check,unit,build}.log`.

The bundle guard identified one deliberate growth: Snip 9364 → 9815 gzip bytes (+451, +4.8%) from serialized persistence, explicit-copy retry and failed-save navigation/native-close guards. Recorded only that measured page budget; the 3% tolerance and every other budget are unchanged. This is an acknowledged feature-cost increase, not a claimed optimization.


Combined post-merge browser run: **97 passed / 1 failed** in 3.3 minutes across 12 affected specs (`/tmp/otto-review-merged-browser.log`). The failure was the History scroll fixture's assumption of a single startup transcript request: trace proved three newest-page reads during workspace restoration/selection, followed by the correct `before=120` after the wheel. The fixture now verifies startup reads separately and still requires exact scroll-driven cursors `120`, then `60`, retained scroll anchoring, the oldest turn, and removal of the earlier-page button. No production change was needed.

Closing repeated History + terminal run: **33/33 passed** in 28.5 seconds (`/tmp/otto-review-paging-links-repeat.log`): each of 11 cases repeated three times. Terminal geometry waits for connected nonzero text ranges, and the HTTP case asserts the exact real popup URL rather than an obsolete anchor-click observer. Rooms tail/earlier pagination also passed **3/3** before integration and passed again in the merged group (`/tmp/otto-review-rooms-repeat.log`). Thus all **98 distinct cases** in the combined group are verified across the 97-case pass and repeated History repair; this is not described as one all-green 98-case invocation.

The final E2E TypeScript check passed after the fixture corrections. Light/dark chat-tile captures were refreshed from the combined merged run and visually inspected. The merged bundle guard passes after the documented 451-byte Snip baseline adjustment.


Final merged-source 0/1/3/5-session scale confirmation completed in 375.4 seconds, 66 samples, no page errors/swap growth/safety abort and zero leftovers. Peak sampled host load 6.66; estimated available memory at least 36.0 GiB. Results and scope are in [PERFORMANCE.md](PERFORMANCE.md) and `measurements/final-scale-summary.csv`. No production source changed after the combined checks.
