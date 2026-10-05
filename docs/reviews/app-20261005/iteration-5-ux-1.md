# Iteration 5 — UX, partition 1

Source/runtime checkpoint: `64a850e6`. Bounded independent recheck of shell/navigation, agents/sessions, terminal/conversation and History recovery changes. **9.8/10 at the stated scope; no new confirmed defect.** This does not certify all native or provider journeys.

Read PLAN's fixed rubric, iteration-4-ux-1.md and VERIFICATION.md, then inspected current repair source and the authored session-recovery browser cases. No tests, builds, browsers, source changes or git mutations were performed by this reviewer. Execution below is inherited explicitly from root's ledger, not independently rerun. The iteration-4 provisional 7.1/10 remains historical; this is a new current-source assessment.

## Repair and journey recheck

- **R4-U1-01 closed at tested scope.** `ui/src/modules/agents/NewSession.svelte:327` captures each submitted request; `:337` refuses to redirect retained failures into another workspace; `:364` retains only failures before navigation; `:371` keeps the recovery surface open. The persistent outcome and Retry failed action at `:625` explain what remains. The merged mixed-batch mounted case passed on its selector-corrected rerun; it verifies retained settings and exactly one creation per member. The original failed selector attempt is not counted as product success.
- **R4-U1-02 source repair confirmed.** `ui/src/modules/agents/conversation/ConversationView.svelte:427` derives partial search scope; `:775` reserves definitive “No matches” for a complete transcript; `:789` explicitly limits the result and `:790` offers Load earlier messages. This removes the previously misleading claim. The exact older-only-marker search journey has no named execution in this ledger, retained below as a bounded evidence gap rather than reopened as a defect.
- **History active/stale/open-chat repairs confirmed.** `ui/src/modules/agents/history/HistoryPage.svelte:185` delegates every status to safe resume; `:200` updates the reactive Chat preference before navigation. The merged browser matrix covers five statuses inside one case, stale live rows, warmed Terminal preference and import failure/retry without duplicate import. Backend isolated resume checks passed 3/3, including live-PTY preservation, authorization and concurrent inactive resumes.
- **Departed History resume is closed, not duplicated.** `HistoryPage.svelte:175` captures action generation/workspace/scope; `:176` checks lifetime and context; `:181`, `:183` and `:188` guard asynchronous continuations. Root's mounted same-document departure case established RED then GREEN. No source evidence contradicts that result.
- **Disconnected status repair confirmed in source.** `ConversationView.svelte:316` derives stale state from event connectivity; `:861` suppresses live rendering when stale; `:872` supplies stale status/hint to LiveStatus. This recheck does not claim an executed socket-disconnect/reconnect presentation transition.
- **Long child history remains reachable and responsive in the measured fixture.** Root recorded 100 mounted inspections plus 1,200-turn paging in both directions, p95 inspection 82.78 ms and zero recorded long tasks. Child bodies were absent at GC checkpoints. This supports the inspected navigation task; renderer RSS increased and remains unattributed, so no leak-free claim follows. The additional recovery extension was authored but unrun at this checkpoint.

## Fixed rubric

| Dimension | /2 | Evidence and deductions |
|---|---:|---|
| Task completion/discovery | 2.0 | Named mounted History opening/import/retry and batch completion journeys pass; old/new child pages remain reachable. Partial search now offers an explicit way to extend scope. No remaining material task defect in this bounded matrix. |
| Feedback/state clarity | 2.0 | Persistent mixed-batch outcome, scoped search absence and stale LiveStatus are explicit in current source; mounted batch and History failure cases verify actionable recovery feedback. No known material presentation defect in inspected states. |
| Recovery/retry | 2.0 | Failed members retain submitted settings, successful members are not retried, import retry avoids duplicate import, and stale session status recovers through server authority. Named browser and backend controls pass. |
| Draft/scope/trust preservation | 2.0 | Captured batch workspace/request, preserved live PTY, warmed Chat preference override and departed-navigation ownership all have direct source support plus named passing regressions. No remaining known material trust defect in these repairs. |
| Executed end-to-end journeys | 1.8 | Current merged affected repairs/failure paths pass, including departed History RED→GREEN and mounted long-history paging. Deduct 0.1 for the unmapped older-only-marker search case and 0.1 for the unmapped disconnected LiveStatus transition. These are specific merged feedback paths, not a generic native-coverage ceiling. |
| **Total** | **9.8/10** | No known blocker/major in the stated matrix; execution limits remain explicit. |

To close the two execution deductions: seed a marker only before the initial conversation page, assert the loaded-only absence message, load earlier and reach the marker; disconnect the event stream while a session is working, assert the stale hint and suppressed live rendering, then reconnect and verify recovery. These are verification requests, not new repair findings.

## Inherited verification and limits

VERIFICATION records merged UI check with Svelte 0 errors/0 warnings and all type checks, 1,333/1,333 units, and **15 distinct merged desktop cases passing across the initial run and focused rerun**. Those are shared totals across partitions, not 15 P1 cases and not one clean 15/15 invocation. The current checkpoint also records rustfmt, strict workspace/all-target clippy, daemon build and doc-test commands green; 33 library targets reported zero doc-tests, not 33 executed cases. The two earlier integration failures have focused green repairs; the original 4,694/4,696 run remains a failed run.

No new execution of native focus/window restoration, real provider approval handoff, actual clipboard/image upload or daemon-restart terminal reconnection was performed. Those broader unchanged journeys are outside this bounded repair matrix and are not silently certified by its score. Only this review report was written. Shell slot released.
