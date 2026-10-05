# Iteration 6 — UX, partition 1

Final evidence-delta review at integrated source `0c4862ab`, covering the same bounded shell/navigation, agents/sessions, terminal/conversation and History recovery matrix as iteration 5. **Score remains 9.8/10: 2.0 / 2.0 / 2.0 / 2.0 / 1.8.** Additional green execution strengthens existing conclusions but does not close the two named execution gaps.

Read only iteration-5-ux-1.md and the current VERIFICATION.md. No new source audit, tests, builds, browser execution, source edits or git mutations were performed. Prior source citations belong to the iteration-5 inspection and are not represented as fresh final-head line checks. This report inherits root's recorded executions.

## Evidence delta

- **History pending destination residual repaired.** The ledger records mounted RED when a pending Settings hash was replaced by Agents before Settings mounted. The production guard now observes pendingTarget and synchronously compares the originating hash. The same mounted case passed; 13/13 unit controls passed, including two new boundary cases that first failed. The boundary browser run passed 2/2 across History and MCP Audit; that is not two History cases. This closes the additional navigation-ownership issue rather than reopening the earlier departed-resume repair.
- **Long-history evidence expanded.** The 18/18 expanded acceptance run includes eight session/History cases and 200 mounted child inspections with 1,200-turn paging. This supersedes iteration 5's statement that the second 100-cycle extension was unrun. The 18 cases span partitions and are not all P1. Memory plateau is not established; the sustained run aborted at the unchanged safety threshold, and initial runtime measurements belong to `64a850e6`, not an exact final-head benchmark.
- **Integrated gates recorded.** The closing ledger at `0c4862ab` records UI check with 0 errors/0 warnings, 1,335/1,335 units, production build and unchanged bundle-budget checks green. These shared gates support integration but do not execute the missing P1 search and connection-state journeys. Any later Product fix/rerun remains outside this recorded checkpoint until its outcome is logged.
- Existing tested-scope closures carry forward: failed batch retains submitted settings and retries only failures; safe History resume preserves live PTYs and uses Chat despite a warmed Terminal preference; failed import/resume retries without duplicate import; long child history retains bidirectional reachability. Scoped search and stale-status presentation retain iteration-5 source evidence.

## Fixed PLAN rubric

| Dimension | /2 | Evidence and deductions |
|---|---:|---|
| Task completion/discovery | 2.0 | Inherited named batch/History completion passes plus eight session/History cases in expanded acceptance; older/newer child history remains reachable. No new material task defect in this bounded matrix. |
| Feedback/state clarity | 2.0 | Inherited persistent batch outcomes and explicit loaded-search/stale-status source presentation; mounted failure/retry controls remain green. No newly established presentation defect. |
| Recovery/retry | 2.0 | Retained failed requests, import retry without duplication, authoritative session resume and expanded session/History acceptance support the existing score. |
| Draft/scope/trust preservation | 2.0 | Earlier live-PTY and captured-request controls remain; the newly exposed pending-destination navigation issue now has mounted RED→GREEN and passing unit boundary controls. |
| Executed end-to-end journeys | 1.8 | Current affected repairs have named green journeys. Deduct 0.1 for the still-unrun older-only-marker conversation search journey and 0.1 for the still-unrun disconnected LiveStatus transition. Additional child cycles do not substitute for either case. |
| **Total** | **9.8/10** | Unchanged from iteration 5; no automatic uplift from broader green totals. |

## Remaining named verification

1. Put a unique marker before the initial conversation page. Search, assert the loaded-only absence wording, load earlier, and reach the marker. Iteration-5 source evidence: `ui/src/modules/agents/conversation/ConversationView.svelte:427`, `:775`, `:789` and `:790`.
2. Disconnect the event stream while a session is working. Assert the stale hint and suppressed live rendering; reconnect and verify recovery. Iteration-5 source evidence: `ConversationView.svelte:316`, `:861` and `:872`.

These are evidence gaps, not confirmed product defects or requested source repairs. The score certifies only the bounded repair matrix; native focus/window restoration, real-provider approval handoff, actual clipboard/image upload and daemon-restart terminal reconnection remain outside its executed scope. The continuation did not rerun the full desktop suite. Only this report was written; shell slot released.
