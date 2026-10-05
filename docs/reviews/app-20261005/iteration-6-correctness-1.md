# Iteration 6 correctness — partition 1 bounded recheck

**Approve with fixes: R5-C1-01 is repaired for completed departure, but one major residual remains during lazy destination preparation. Provisional 9.2/10.**

Reviewed runtime checkpoint `64a850e6` in `/Users/itziklavon/claude_ade-review`. Scope is only the R5 History departure repair, its router/import adjacency, focused regression source and current verification ledger. No tests, builds, browsers, benchmarks, source changes or git mutations were run by this reviewer. The iteration-5 report is preserved.

## R5-C1-01 disposition

**Completed departure: fixed, with inherited mounted RED→GREEN.** `HistoryPage.svelte:75`–79 now invalidates component lifetime on destruction. `resume()` captures generation/workspace/scope at `:175`–176 and checks after import, refresh and resume at `:181`, `:183`, `:188`. A response settling after destruction cannot patch History, change view preferences or navigate. Departed errors are suppressed at `:192`. Explicit selection clears increment the generation synchronously at `:94` and `:102`. Current-origin successful resume still proceeds through the safe endpoint and reactive Chat setter. The mounted normal-open/import-retry controls are recorded green.

**R6-C1-01 — major — accepted navigation can still be superseded while its page chunk loads.** This is a residual branch of R5-C1-01, not a separate broad-scan issue. Confirmed by source hand trace; not executed here.

Locations: `ui/src/modules/agents/history/HistoryPage.svelte:71`–74, `:175`–176, `:187`–190; `ui/src/lib/router.svelte.ts:223`–237.

Intended behavior: after the person chooses another module, the pending History action must not replace that newer navigation, including the interval before the destination is ready to render.

Concrete trace:

1. On History, start session A's resume and hold its response. The component captures generation G, workspace W and scope S.
2. Choose a destination whose page chunk is not loaded, for example Settings, and hold its route-prepare promise. Router accepts the hash change. `parse()` increments `commitSeq` and sets `pendingTarget` at router line 233, but deliberately keeps `parts` on History until the promise settles. This behavior is explicitly documented at router lines 211–217.
3. Resolve A's resume while destination preparation remains pending. History is still mounted; its ownership effect reads `router.parts`, workspace and scope only. None changed, so generation remains G and `alive` remains true. `current()` at History line 176 returns true.
4. Lines 188–190 call `openInChat(A)`, which dispatches `router.go('agents/A')`. That starts a newer route commit, superseding the person's Settings navigation. The old prepare continuation later fails its `commitSeq` check at router line 235.

Actual: a cold/slow destination is replaced by the stale resume. Expected: the accepted destination remains authoritative regardless of chunk readiness. Existing leave guards are not bypassed; this trace uses an accepted clean navigation.

**Fix direction:** invalidate follow-up ownership when a newer navigation is accepted, rather than waiting for rendered `parts` or destruction. Prefer a router navigation/intent epoch captured by the operation; a narrower solution must synchronously account for pending destination/current hash before follow-up publication, including the interval before hashchange processing. Preserve declined-leave behavior and ordinary History canonicalization. Add a deferred-prepare regression that releases resume while History is still mounted, verifies the pending destination wins, then releases preparation. Retain normal-open and completed-departure controls.

The current browser regression at `ui/e2e/desktop-review4-session-recovery.spec.ts:191` explicitly waits for zero History resume buttons before releasing the response. It proves destruction ownership, but cannot exercise the retained-old-page branch. `ui/unit/historyActions.test.ts:59`–73 directly changes the generation for its new-route case, so it does not prove actual router preparation invalidates that generation.

## Adjacent assessment

- Guarded navigation remains in `router.goChecked`; rejected decisions do not commit a route. The repair introduces no direct guard bypass. Automatic History URL replacement remains a canonicalization path; no new dirty-draft-loss finding was established here.
- Import continuation checks prevent starting resume after component departure. The shared `history.importEntry` writer remains unchanged; this limited recheck does not assert or reproduce cross-workspace shared-list corruption.
- Workspace/scope checks handle a currently different identity. The effect generation is intended to handle an observed away/back transition. A mounted workspace A→B→A during held resume is not identified in the ledger; this remains a named execution gap, not a second confirmed finding.

## Fixed PLAN dimensions

| Dimension | Score /2 | Evidence and deduction |
|---|---:|---|
| Contract/data integrity | 2.0 | Reviewed repair changes follow-up ownership without altering safe-resume/status contracts. Existing status/open and import-retry controls pass. |
| State/concurrency ownership | 1.5 | Destruction, changed selection and current scope ownership are repaired; confirmed pending-route residual still overrides a newer accepted operation. |
| Boundary/error behavior | 1.8 | Stale error suppression and failed-resume retry remain; lazy destination preparation is the concrete uncovered boundary. |
| Persistence/recovery | 2.0 | Normal warmed Terminal→Chat and import-once/retry controls remain green; no persistence repair regression identified in this bounded scope. |
| Executed regression coverage | 1.9 | Root recorded mounted departure RED→GREEN and merged controls, plus current unit/type/build gates. Pending-prepare and mounted workspace ABA remain unexecuted named cases. |
| **Total** | **9.2/10** | **Provisional. Confirmed major residual prevents acceptance regardless of arithmetic.** |

## Execution provenance

Inherited from current `VERIFICATION.md`, not performed by this role:

- Departed History and Product browser cases passed together 2/2 after their recorded intended RED failures; History normal-open, stale-status, import/retry controls are among 15 distinct merged desktop cases with passing executions.
- Merged unit rerun passed 1333/1333. UI guards/typecheck passed with Svelte 0 errors/0 warnings; production UI build and unchanged bundle budgets passed.
- Runtime `64a850e6`: workspace rustfmt and strict all-target clippy passed; scheduled controls after migration passed 57/57; fresh daemon build passed. Workspace doc-test command passed with zero doc-tests across 33 library targets, not 33 executed tests.
- Both old full-gate failures individually passed their corrected targeted reruns. The historical 4694-pass/2-failure aggregate is not rewritten as a new full-suite pass.
- Mounted child measurement also recorded 1200-turn bidirectional paging and 100 inspections, with zero retained child bodies at checkpoints. This is inherited correctness support for the previously reviewed paging behavior, not new source inspection or proof that RSS cannot leak.

Further root acceptance is queued and unrun at this review snapshot. This report does not reopen unrelated source surfaces or impose a generic whole-app breadth deduction. Concrete remaining correctness cases are the pending-page preparation residual and mounted workspace ABA. Shell slot released on report completion.

**final evidence rescore pending**

## 2026-10-05 follow-up — pending-route repair reviewed

**R6-C1-01 closed at the inspected current working-tree checkpoint above `64a850e6`; no remaining confirmed correctness finding in this bounded repair scope.** The earlier 9.2 assessment remains above as the pre-repair record. This follow-up inspected only the changed ownership guard, targeted regression source and supplied green logs; the reviewer ran no tests or builds.

`HistoryPage.svelte:175` now captures `window.location.hash`. Its `current()` predicate at `:178` requires that exact origin hash and no `router.pendingTarget`. The ownership effect at `:72` additionally tracks pendingTarget, invalidating the generation when pending navigation is observed. Hand traces now close both reported intervals: before hashchange parses the destination, the hash mismatch rejects stale completion; after parse establishes a deferred destination while History remains mounted, pendingTarget rejects it. Normal unchanged-origin completion still passes the predicate, and existing post-await guards cover import, refresh and resume. A declined navigation that preserves the original hash and never establishes a pending destination does not gain a false departure from these two checks.

Inherited execution, directly inspected in the logs:

- `/tmp/otto-review06-history-unit-green.log`: **13/13 passed**, 315 ms, including pending-route and hash-before-parse cases alongside existing live/stale status, warmed preference and departed-import controls. Root reports the two added cases first failed before repair; this reviewer read their green output and current assertions.
- `/tmp/otto-review06-boundary-green.log`: **2/2 passed**, 4.5 s overall. One is the relevant mounted History deferred-readiness case (1.8 s); the other is the separately owned MCP audit-details case and is not counted as History coverage. The History case establishes pending Settings while the old History page remains mounted before settling resume, addressing the exact branch missed by the original destruction test. Root reports its prior intended RED.

Updated provisional fixed dimensions:

| Dimension | Score /2 | Evidence and remaining deduction |
|---|---:|---|
| Contract/data integrity | 2.0 | Safe resume/status behavior unchanged; focused controls remain green. |
| State/concurrency ownership | 2.0 | Source and focused execution now cover destruction, pending destination and hash-before-parse ownership; no confirmed residual in the repaired trace. |
| Boundary/error behavior | 2.0 | Reported deferred-page boundary closes; ordinary success and prior retry controls are retained. |
| Persistence/recovery | 2.0 | Ownership predicate does not change persistence; existing warmed preference and import/retry evidence remains applicable. |
| Executed regression coverage | 1.9 | Exact new boundary executed green; current broader 18-case acceptance batch is still running and mounted workspace ABA is not yet credited. |
| **Total** | **9.9/10** | **Provisional targeted rescore; final aggregate belongs to the coordinator.** |

This closes the concrete major without silently replacing the historical score or treating the pending broader batch as passing. Shell slot released.

**final evidence rescore pending**
