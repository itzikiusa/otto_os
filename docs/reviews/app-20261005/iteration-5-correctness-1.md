# Iteration 5 correctness — partition 1

**Approve with fixes: 0 blockers, 1 major, 0 minor. Provisional 8.8/10; acceptance is blocked by the major finding and pending integrated execution.**

Source checkpoint: supplied `418e963a`, `/Users/itziklavon/claude_ade-review`. Bounded source-only recheck of shell/navigation, agents/sessions and conversation/history repairs plus their merged interactions. No tests, builds, benchmarks, source edits or git mutations were performed. This report is the sole authored file.

## R5-C1-01 — major — a departed History resume steals subsequent navigation

**Confirmed by hand trace, not execution.** Location: `ui/src/modules/agents/history/HistoryPage.svelte:168`–170; global side effects at `:179`–182; destination dispatch at `ui/src/lib/stores/workspace.svelte.ts:1049`–1050.

Intended behavior: an explicit History open safely resumes its selected session and opens Chat while that interaction still owns the screen. A later user navigation should remain authoritative.

Reproduction trace:

1. In workspace W, select session A in History and click Open in Otto. Hold the successful `/sessions/A/resume` response. `resume()` sets only its component-local `busy` flag and waits at line 168.
2. Navigate to another module, such as Settings. There is no History leave guard, destruction invalidation or captured operation generation in this component; leaving destroys the component but does not cancel its asynchronous function.
3. Resolve the held response. The continuation patches History and unconditionally calls `openInChat(A)` at line 170. That updates the transcript preference, changes the global workspace view to tabs, and invokes `navigateToSession(A)`.
4. The latter calls `router.go('agents/A')`. With a clean destination, routing succeeds and replaces the user's newer Settings navigation. If the destination instead has a dirty editor, this stale action initiates an unsolicited leave decision; it does **not** bypass that editor's guard. Scope/workspace changes during the await likewise have no local ownership fence.

Actual: completion of a departed operation takes over the current UI. Expected: the already-requested server resume may finish, but the departed operation must not navigate or change global view preferences.

Fix direction: capture origin workspace/scope and an operation/lifecycle generation; invalidate ownership synchronously on departure/destruction and context changes. Check ownership after each await before starting subsequent import/resume stages, patching shared History selection state, changing preferences or navigating. Preserve current-operation error/retry behavior. Add a deferred-response regression for leave-to-Settings and workspace A→B→A, plus an unchanged-origin successful-open control. A mounted regression should assert the newer route remains after response settlement. No claim is made that this was executed.

## Repairs and adjacent traces

- **Working/stale statuses and Chat preference remain repaired.** `HistoryPage.svelte:152` recognizes working/running/idle. Every explicit existing-session open reaches safe resume at `:168`, independent of stale row status. `openInChat` uses `transcript.setView` at `:180`, updating the reactive cache. `workspace.svelte.ts:1611` dispatches `/resume`; the inspected manager `ensure_live` retains both its early live check and locked recheck. No unconditional restart regression found.
- **History URL selection:** inspected deep-link resolution and `pick`/`clearSelection`/automatic selection at `HistoryPage.svelte:72`–115 against router guarded navigation and unguarded canonicalization at `router.svelte.ts:294`–415. Automatic selection canonicalizes with `replace`; it does not itself represent a dirty History editor in the inspected source. No independent dirty-draft-loss finding was proved. Interaction between delayed page preparation, a concurrent list refresh and automatic replacement remains unexecuted; do not equate this bounded read with acceptance of every navigation interleaving.
- **History list ownership:** `history.svelte.ts:145`–194 fences first-page responses by sequence and additional pages by sequence/workspace/query. New load invalidates an old page append. `importEntry` at `:217`–227 remains an unowned asynchronous shared-state writer; it is included in the origin-fencing direction above, without asserting a separate reproduced cross-workspace corruption case.
- **Child transcript paging:** `transcript.svelte.ts:418`–525 preserves reference-counted release, controller/epoch ownership and pre-request admission. For three fixed 60-turn pages, earlier records each request cursor and later replays the preceding cursor; the displayed body is replaced. Collapse aborts and removes the charge/body; reopen uses a new controller, so old completion fails `current()` and cannot delete the new reservation. Aggregate rejection retains the previous body and retry target. Parent earlier paging at `:294`–312 checks read epoch and original cursor before publication. No new confirmed paging defect in these bounded traces.
- **Prior terminal question:** the iteration-4 unproven non-parking delayed socket-close concern is not promoted to a finding. This pass did not establish the missing concrete caller path or execute native/WebGL behavior.

## Fixed PLAN rubric

| Dimension | Score /2 | Evidence and deduction |
|---|---:|---|
| Contract/data integrity | 1.9 | Safe resume/status and reactive Chat repairs remain; sampled scope, not a complete provider contract matrix. |
| State/concurrency ownership | 1.4 | Confirmed departed-resume navigation takeover; transcript/list ownership guards remain intact in inspected paths. |
| Boundary/error behavior | 1.9 | Guarded navigation, stale status dispatch and child failure retention traced; merged async routing interactions unexecuted. |
| Persistence/recovery | 1.9 | Reactive/persisted Chat preference and child cursor/reopen recovery traced; native relaunch/provider recovery not run. |
| Executed regression coverage | 1.7 | Inherited relevant green units/Rust/mounted journeys support the repairs, but this role executed nothing and integrated merge is unverified. |
| **Total** | **8.8/10** | **Provisional, bounded source review; major prevents 9.8 acceptance.** |

## Execution evidence and limits

Inherited from `VERIFICATION.md`, not rerun here: pre-merge UI check 0 errors/0 warnings, 1314 unit passes, 12 distinct passing desktop journeys and one passing WebKit recap identity case. The earlier safe-resume checkpoint has 3/3 passing isolated Rust tests. These executions support their named checkpoints; they do not certify source `418e963a` after the design merge or the newly identified departure race.

Latest full affected gate: rustfmt and strict clippy passed; nextest **4694 passed, 2 failed, 86 skipped**. The route-inventory fixture and MCP catalog budget failures are source-repaired with reruns pending. The gate exited before doc-tests/UI phases. No all-green or whole-app acceptance claim is made.

Read: supplied/repository AGENTS.md; correctness-review skill; PLAN.md fixed rubric; VERIFICATION.md checkpoints; iteration-4-correctness-1.md; iteration-4-role1-recheck.md; selected current HistoryPage/history store, router/leaveGuard, workspace resume/navigation, transcript parent/child paging, session manager ensure_live, shell route references and focused History regression declarations. Omitted: full shell and terminal implementations, all provider parsers, complete PTY lifecycle, native/multi-window behavior, exhaustive transcript mutation matrices and runtime execution. Report-only review; shell slot released on completion.
