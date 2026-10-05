# Iteration 6 — correctness partition 2

**Verdict: Approve the bounded source recheck.** New confirmed findings: **0 blocker, 0 major, 0 minor**. Provisional **9.6/10**; final evidence rescore pending the queued acceptance results.

Runtime checkpoint: `64a850e6`, 2026-10-05, `/Users/itziklavon/claude_ade-review`. Same reviewer and PLAN dimensions as iteration 5. Read the existing iteration-5 report and current VERIFICATION ledger; reused the previously read AGENTS.md and correctness-review instructions. Scope was only the existing guard/paging repairs and their merge interactions. No broad feature discovery, tests, builds, browsers, source edits, git mutations or delegation. Only this new report was written; iteration 5 remains intact.

## Bounded source traces

- **Broker integrity:** `crates/otto-brokers/src/service.rs:1015`, `:1051` and `crates/otto-brokers/src/kafka.rs:1528` retain character-safe preview and raw optional key/value/header handling. Binary bytes pass unchanged; absent payload remains absent; present-empty remains a supplied payload. Explicit transforms affect their selected fields. No source regression found.
- **Database write approval:** `ui/src/lib/stores/database.svelte.ts:3408` captures connection identity and scope before awaiting. A refusal arriving after selection moves to B still confirms A and retries A; changed access/connection epochs reject the retry. No source regression in the existing origin-binding repair.
- **API ownership and draft preservation:** `ui/src/lib/stores/apiClient.svelte.ts:650` retains workspace/generation publication ownership. `ui/src/modules/api/EnvironmentsView.svelte:103` retains submitted rows and editor generation: newer typing stays dirty, while clean A→B→A returns reconcile from the saved object; stored-secret renames retain the intermediate persisted key. `ui/src/modules/api/SaveRequestDialog.svelte:48` checks alive/workspace/tab ownership after collection creation and before request persistence. The earlier synchronous close/onDestroy invalidation remains the controlling lifecycle rule. No new confirmed defect in these finite traces.
- **Navigation guard:** `ui/src/modules/api/ApiPage.svelte:72` awaits automation leave approval and rejects superseded view transitions; routed request opening at `:193` proceeds only after acceptance. Cancel does not call the subsequent `openRequest`. Existing automatic initial request selection retains the dirty-draft/persisted-tab checks documented in iteration 5. Current mounted automation acceptance strengthens evidence for the normal leave flow; overlapping routed transitions are not inferred from it.
- **Workbench cursor/retry:** `ui/src/modules/workbench/HistoryPanel.svelte:89`, `:109`, `:114`, `:236` retains document-visit/load-generation publication checks, one metadata page, explicit cursor trail, and Retry using the failed request's cursor/trail. `crates/otto-state/src/workbench.rs:681` uses exclusive descending sequence paging and clamps limits to 1–200. With page ending at sequence 101, the older request uses `before_seq=101`; an inserted newer sequence does not shift that older page. A failed request retains its attempted cursor for Retry. These are hand traces, not the newly authored mounted failed-page/concurrent-insert test execution.

## Evidence reconciled since iteration 5

The following are actual central results reported in VERIFICATION/coordinator handoff, not commands executed by this reviewer:

- Broker replay final focused **6/6 passed** through the isolated Kafka MockCluster, including nullable headers, binary data, tombstones, transforms, Unicode and public-produce controls.
- Grouped state/Git/Product controls **106 passed**; authenticated history/recap **3/3 passed**, including exclusive history cursor after a new save, oldest restore and 50,000-revision bounds. MCP history **2/2 passed**. These resolve the contract/integrity uncertainty identified in iteration 5 without requiring a new live-production broker campaign.
- Merged UI **0 errors/0 warnings**, **1,333/1,333 unit tests passed**, production build and unchanged bundle budgets green.
- All **15 distinct merged desktop cases** have passing executions, with the original 14/15 attempt and corrected single-case rerun preserved. Additional mounted API automation **Cancel, failed Save, successful Save and Discard** acceptance is green. These counts are not presented as one combined run.
- The two previous full-run failures, route inventory and MCP catalog byte budget, each passed a focused rerun. The earlier 4,694-pass/2-fail/86-skip run remains historical; it is not relabeled as a green full run.
- At checkpoint `64a850e6`, workspace formatting, strict all-target clippy, workspace doc-test commands and fresh daemon build passed. The doc-test commands reported zero cases across 33 library targets, not 33 executed tests.

## Remaining bounded acceptance gaps

These are evidence gaps, not confirmed defects or a request for another broad audit:

1. **Workbench mounted failure recovery:** the authored failed older-page → Retry → back navigation and concurrent-insert cases are still **unrun**. Backend exclusive-cursor/insert behavior already has passing evidence; the remaining question is the rendered cursor/trail and Retry interaction.
2. **API routed concurrency:** mounted normal automation leave decisions are green. The narrower deferred routed-navigation case—request URL changes while an earlier leave approval is pending—still needs evidence that the stale transition cannot take over and the visible draft remains the accepted one.
3. **Environment/save-sheet lifecycle:** production-handler/unit evidence covers Save followed by newer typing, clean A→B→A during Save including secret rename, and Cancel during collection creation. The supplied current browser evidence does not explicitly establish these mounted component/reactivity cases. Existing evidence may satisfy them if root supplies the exact case/result; do not count an unrelated automation Save as environment acceptance.

Historical broad native-engine, partial-production and Git-recovery limitations are scope disclosures rather than extra requirements to reach the bounded target. Do not repeat already passing contract/byte-preservation checks solely for this review.

## Provisional score

| Correctness dimension | /2 | Evidence and remaining deduction |
|---|---:|---|
| contract/data integrity | 2.0 | Raw broker fixtures and actual history/MCP/backend cursor cases now have direct passing evidence; inspected source retains the repaired contracts. |
| state/concurrency ownership | 1.9 | Deferred unit guards and normal mounted leave acceptance pass; overlapping routed leave completion remains the bounded gap in item 2. |
| boundary/error behavior | 1.9 | Unicode/error/ownership controls and repaired failing gates pass; mounted failed-page Retry/back behavior in item 1 is queued, not green. |
| persistence/recovery | 1.9 | Draft/secret/cancel handler regressions and mounted automation Save failure/recovery pass; the specific environment/save-sheet mounted cases in item 3 lack supplied execution evidence. |
| executed regression coverage | 1.9 | Integrated UI, focused backend and affected browser evidence closes the prior major execution gap; the enumerated queued mounted cases prevent full coverage credit. |
| **Total** | **9.6/10** | **Provisional; final evidence rescore pending. No arbitrary 1.9 cap is applied.** |

Root can append a final evidence adjustment when the queued proof arrives, naming each actual case and result. This provisional report does not claim unrun cases passed.

## Final evidence addendum — checkpoint `0b685e73`

The **9.6/10 provisional score above remains preserved**. This addendum reconciles central execution evidence only; it is not a new source audit. Read the closing VERIFICATION entries on 2026-10-05. No tests/builds/browser sessions, source edits or git mutations were performed by this reviewer.

- `/tmp/otto-review06-acceptance.log`: **18/18 passed**, including the specific mounted Workbench failed older-page → Retry → concurrent insert → back/restore journey. This closes the previously queued mounted paging/recovery gap; the backend cursor evidence was already green.
- `/tmp/otto-review06-recovery.log`: **7/7 passed**, including two mounted environment controls covering newer input during Save, double secret rename, and clean A→B→A selection. This closes the environment reactivity/draft-recovery uncertainty identified above. These cases used no production secret or real network request.
- Closing integrated UI: **0 errors/0 warnings**, **1,335/1,335 unit tests passed**, production build and unchanged bundle budgets passed. Results are recorded in `/tmp/otto-review06-integrated-{ui-check,units,build}.log` and `/tmp/otto-review06-integrated-bundle-budget.log`.

The **overlapping API routed leave/superseding pending confirmation remains unrun**. Normal automation Cancel/failed Save/successful Save/Discard acceptance does not establish that concurrent route case. Nor does the supplied environment run establish a mounted SaveRequestDialog cancellation during collection creation; that path retains source-hand-trace and production-handler/unit evidence only. These bounded execution limitations remain explicit; no whole-desktop-suite, native-device or sustained-memory acceptance is inferred.

| Correctness dimension | Final /2 | Evidence adjustment |
|---|---:|---|
| contract/data integrity | 2.0 | Existing direct broker/backend/MCP evidence remains; mounted Workbench continuation now also passes. |
| state/concurrency ownership | 1.9 | Normal ownership and mounted environment ABA are covered; concurrent routed leave remains the specific residual gap. |
| boundary/error behavior | 2.0 | The previously missing mounted Workbench failed-page Retry/back case now passes alongside existing Unicode/error controls. |
| persistence/recovery | 2.0 | Direct mounted newer-draft, secret double-rename and clean-return controls now corroborate the repaired persistence behavior; cancellation's subsequent-write prevention retains direct production-handler coverage. |
| executed regression coverage | 1.9 | Final integrated gates and mapped mounted recovery cases pass; overlapping routed leave and mounted save-sheet cancellation remain explicit uncovered combinations. |
| **Final total** | **9.8/10** | **Bounded evidence-adjusted judgment; no new confirmed findings and no claim that every combination was executed.** |

The persistence increase reflects newly executed draft/recovery cases, not an assertion that the save-sheet mounted case ran. Remaining lifecycle/route execution breadth is retained in the coverage deduction; the concurrent route ownership uncertainty also remains in its specific dimension.
