# Iteration 5 — correctness partition 5

**Verdict: Approve with fixes — 0 blocker, 1 major, 0 minor. Provisional score: 8.8/10.**

Bounded review of worktree `fix/app-review-20261005`, HEAD `418e963ac09e9c2bc1db071c23e55720de3bd686`, including then-current working-tree AccountPicker loading presentation and Home accessibility edits. Used correctness-review, AGENTS, PLAN, VERIFICATION, iteration-4-correctness-5 and iteration-4-role5-ui-recheck. No tests, builds, browser sessions, source edits or git mutations were run. This report is the sole authored file. Scores credit inherited executions at their stated revision; they do not certify the merged tree.

## Confirmed finding

### R5-C5-01 — major — Personal Agents list ownership survives workspace ABA and clears another request's loading state

**Location:** `ui/src/lib/stores/personalAgents.svelte.ts:81` (success), `:83` (error), `:85` (finally). Actual caller: `ui/src/modules/personal-agents/PersonalAgentsPage.svelte:38`–`:40`; loading/error/Retry consumed at `:176`–`:181`. Confidence: confirmed by source hand trace; not executed.

**Intent:** Cards and error/loading state describe the newest list request for the selected workspace. Repairing schedule/run maps must preserve that ownership in their parent list loader.

**Reproduction:** Start list request W1 for workspace W with its old snapshot `[A]` held. Switch to V, then back to W, starting V1 and W2 through the page effect. Resolve W2 with current `[A,B]`. Resolve W1: `agentsWs === workspaceId` is W===W, so line 81 replaces the current list with `[A]`. The newly available B disappears until another refresh. Reject W1 instead and line 83 publishes its obsolete error over W2's successful list. Separately, leave W2 pending and resolve V1: V's list is correctly rejected, but line 85 nevertheless sets `loadingAgents=false`, so the current load stops presenting as pending. Line 88 then runs schedule refreshes from the globally current list even though this completion no longer owns the operation.

**Repair:** Add a monotonically increasing list request generation. Capture it at every load; require both generation and workspace match before success, error, finally and schedule fanout. Keep existing per-agent schedule/run generations. Do not use workspace identity alone, because returning to the same workspace is a new request lifetime.

**Acceptance:** Deferred W→V→W requests, newest W first and oldest W last, retain W2 rows, error and loading ownership. Repeat with obsolete W rejection and V completion while W2 is pending; assert no premature loading settlement and no stale-completion schedule fanout. Include the ordinary successful list plus concurrent A/B schedule controls. This is a new adjacent list defect, not a reopening of the repaired per-agent map issue.

## Repair checks and scope

- Personal Agents `loadSchedules`/`loadRuns`: responses are awaited before merging current maps; per-agent generations reject stale same-agent success/error. Both independent-agent completion orders retain their keys. `loadAgents` adjacency produces the finding above.
- Proof `loadPacks`/`loadMore`/`open`/`closeDetail`: list and page generations fence same-workspace filter changes; cursor resets immediately; close advances the detail generation and clears owned state. Traced delayed All after Failed, delayed page after replacement, and close before refresh success/error. No new defect asserted in those repaired paths. Summary loading was only sampled and is not accepted by this pass.
- Insights settings: model callback captures provider; failed provider writes retain a coherent failed pair, and timer state is reactive. Credit the documented 26/26 production-handler/compile regression checks and the focused mounted Retry journey. No real provider execution claimed.
- RoomRecap at `ui/src/modules/rooms/RoomRecap.svelte:75` keys RecapPanel by archive ID. RecapPanel `:18` cleanup sets `live=false`, increments request and stops polling; `:55`–`:59` fence delayed body/revision publication. Traced A page 2 held → key becomes B → B starts with initial cursor → A response rejected. Revision/body retry, exact-page tail growth and polling error recovery inspected. Credit the documented WebKit identity RED→GREEN and authenticated revision HTTP/backend checks; no request-abort claim.
- Inspected the current AccountPicker/Home working-tree diffs: loading presentation import and Home group semantics introduce no state-ownership changes. Earlier AccountPicker provider/unmount, token reveal and shared components are credited only at their documented prior scope.

## Fixed rubric

| Dimension | Score / 2 | Evidence and exact deduction/acceptance gap |
|---|---:|---|
| Contract/data integrity | 1.8 | Per-agent map and Proof query/cursor repairs hold; W1 can overwrite W2 list data. Restore credit after the named W→V→W row-preservation regression passes. |
| State/concurrency ownership | 1.6 | Keyed recap and Proof generations hold, but parent agent list has confirmed ABA/error/finally ownership failure. Restore credit after generation fencing and all three deferred completion controls above pass. |
| Boundary/error behavior | 1.8 | Provider failure keeps coherent retry data and recap polling recovers; obsolete list rejection still contaminates successful state. Require stale W rejection and stale V completion while W2 remains pending, with rendered current-state assertions. |
| Persistence/recovery | 1.9 | Inherited recap external replacement/removal/ctime checks and authenticated owner-scoped revision checks support the inspected recovery protocol. Specific remaining check: mounted revision failure → archive identity switch → retry must keep B selected and reject A recovery completion. Existing identity and transient-failure tests cover those separately, not that combined path. |
| Executed regression coverage | 1.7 | Credit platformOwnership4 26/26, recapPolling4 10/10, backend revision and owner HTTP checks, 12 distinct desktop cases and 1 WebKit recap case at recorded pre-merge scope. Merged UI unit run currently has 1292 pass/38 fail and requires fixture/import triage plus rerun; new list ABA case has not run. Require green affected platform/recap files on merged source and new list ownership regression. |
| **Total** | **8.8** | **New major blocks 9.8 acceptance independently of arithmetic.** |

## Verification limits

Inherited full affected Rust run: 4694 passed, 2 failed, 86 skipped; both failed checks now pass individually. That does not turn the earlier run into a green full gate. Merged UI check reported 0 errors/10 warnings with cleanup pending; merged units reported 1292 passed/38 failed under triage. C1/C3 mounted departure repairs subsequently passed 2/2; these are other partitions and do not establish C5 list ownership. No all-green claim is made.

This roughly two-minute pass targeted existing repairs and adjacent ownership only. It did not newly audit settings backup/restore, plugins, usage ingestion, auth/cloud, assistant execution, Share persistence, native media/two-device rooms, or the entire shared-component catalog. Those omissions delimit this report; deductions above name concrete acceptance work rather than applying a generic breadth cap. Root and the coordinator were notified immediately of R5-C5-01. Reviewer shell released after writing this report.
