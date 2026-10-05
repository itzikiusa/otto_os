# Iteration 4 — correctness partition 5

**Verdict: Approve with fixes — 0 blocker, 2 major, 1 minor.** Findings are confirmed by concrete source-level traces; none was executed. Provisional correctness score: **7.7/10**, with runtime acceptance outstanding.

Reviewed current source on `fix/app-review-20261005`, baseline `03f2bc3e`, using the correctness-review skill and this effort's fixed rubric. Prior app-20261004 findings remain closed; the findings here concern adjacent behavior. Read-only review: no tests, builds, servers, source edits, commits, or external operations. This report is the sole authored file. Claude's visual/copy/accessibility and Proof route-selection work is excluded.

## Findings

### R4-C5-01 — major — Parallel Personal Agent loads overwrite other agents' schedules

**Location:** `ui/src/lib/stores/personalAgents.svelte.ts:91`–`:94`; caller `:86`; same construction at `:102` for run history. Consumers: `ui/src/modules/personal-agents/PersonalAgentsPage.svelte:218`, `AgentPage.svelte:93`, `AgentAutonomy.svelte:40`.

**Intent:** The maps retain schedules and recent runs independently for each agent. Loading the list fetches all agents' schedules so each card reports its next scheduled run.

**Confirmed hand trace:** Start with `schedulesByAgent={}` and list result `[A,B]`. Line 86 starts both `loadSchedules` calls. A's object literal copies the empty map at line 92, then suspends on A's API request at line 93. B copies the same empty map and suspends. Resolve A with `[SA]`: the store becomes `{A:[SA]}`. Resolve B with `[SB]`: B's already-evaluated object literal becomes `{B:[SB]}`, dropping A. The final state holds just the last responder on an initial multi-agent load. The same spread-before-await pattern at line 102 loses other agents' run data during concurrent run refreshes.

**Actual versus intended:** A scheduled agent displays “not scheduled”; its schedule-based UI derives an empty list although the server has the schedule. Expected: both A and B remain present regardless of response order. Persistence itself is not changed.

**Repair:** Await the response into a local variable first, then merge it into the current map. Apply the same correction to `loadRuns`. Per-agent request tickets should also prevent an older request for the same agent from replacing a newer result; merely moving the spread fixes cross-agent loss, not same-agent ordering.

**Regression:** Use deferred responses for two agents and assert both schedule keys survive both completion orders. Repeat for concurrent run loads with pre-existing unrelated entries. Include two requests for the same agent completed newest-first. Existing `ui/unit/roomsLive.test.ts` supplies a store-loading harness but exercises messages, not these maps.

### R4-C5-02 — major — Proof filter changes accept older list responses and incompatible pagination cursors

**Location:** `ui/src/lib/stores/proof.svelte.ts:48`–`:65`, especially `:58`; pagination `:76`–`:77`. Caller: `ui/src/modules/proof/ProofPage.svelte:78`–`:89`.

**Intent:** The pack list and cursor belong to the currently selected workspace and status filter. Changing a filter launches a replacement request.

**Confirmed hand trace:** In workspace W start All request A. Select Failed before A finishes, starting request F and setting `lastFilter={status:'failed'}`. Resolve F first: failed packs and F's cursor land. Resolve A second: line 58 accepts it because `wsId` is still W; All packs and A's cursor replace F's results. The selected filter and `lastFilter` remain Failed. Clicking Load more now submits the All cursor with the Failed filter at line 76. Even without pagination, passed packs now appear under Failed.

**Actual versus intended:** The visible filter disagrees with the rows, and subsequent pages are requested with a cursor from a different query. Expected: only the latest list request for the current scope may publish rows, cursor, error, or loading state.

**Repair:** Add a list generation that advances on every replacement load, including same-workspace filter changes. Capture it in list and load-more requests; reject stale success/error/finally publication. Reset the cursor when starting a replacement query so Load more cannot reuse the previous scope. Keep detail and list loading ownership independent where necessary.

**Regression:** Delay All, resolve Failed, then resolve All; assert only failed rows and F's cursor remain. Repeat with a stale rejection, a pending load-more response during filter change, and W→other workspace→W. These are store-data tests and need no changes to Claude's Proof route-selection implementation.

### R4-C5-03 — minor — A pending Proof refresh reopens detail after Back

**Location:** `ui/src/lib/stores/proof.svelte.ts:266`–`:268`; response publication `:194`–`:199`, event refresh `:252`–`:263`. User path: `ui/src/modules/proof/ProofPage.svelte:708`.

**Intent:** Back to list closes the phone's detail view and cancels ownership of its pending result.

**Confirmed hand trace:** With pack P open and the page watched, a `proof_pack_updated` event schedules `refreshDetail()`. The refresh calls `open(P)`, capturing sequence N and starting GET P. Before it returns, press Back. `closeDetail()` clears `detail`, but leaves `openSeq=N`. Resolve GET P: line 198 still accepts N and line 199 restores the detail. The phone returns to P without another selection.

**Actual versus intended:** Closing is undone by a request issued before the close. Expected: the list stays open until a new explicit selection. This is a narrow asynchronous navigation defect, not a visual or URL-routing finding.

**Repair:** Advance the detail generation on close and settle its owned loading state. Ensure delayed errors also cannot resurrect detail error state after close.

**Regression:** Open P, begin a deferred refresh, close, then resolve or reject the refresh. Assert `detail` and `detailError` remain null and no detail loading state remains. Test store semantics independently of route-selection changes.

## Fixed rubric — provisional source-only score

| Dimension | Score / 2 | Evidence and deduction |
|---|---:|---|
| Contract/data integrity | 1.6 | Traced per-agent cache ownership and Proof query/cursor contract; both have material result-integrity gaps. Sampled usage grouping, AWS account/region cache keys, and recap upload format boundaries. |
| State/concurrency ownership | 1.3 | Two confirmed major publication races and the close/refresh race. Existing Assistant turn reader lifetime, AWS access revision, and usage attribution request tokens provide positive source evidence, but do not cover these gaps. |
| Boundary/error behavior | 1.7 | Inspected auth capability denial/recovery branches, settings export disabled-during-request behavior, recap consent epochs, and Kubernetes action target capture. External failure matrices and most settings panels remain unverified. |
| Persistence/recovery | 1.8 | Read Personal Agent room tail-resync/eviction paths and existing recovery-test source; inspected recap withdrawal/stopped epochs and share extension failure handling. No persistence or restart execution in this pass; full backup/restore omitted. |
| Executed regression coverage | 1.3 | No new execution permitted in this review slot. Prior baseline verification and current `platformRecovery`, `roomsLive`, and `assistantModel` test sources provide bounded historical/source evidence. New interleavings require execution by root before acceptance. |
| **Total** | **7.7** | **Provisional judgment over the inspected matrix, not a whole-application reliability measurement. Major findings prevent 9.8 acceptance.** |

## Inspected and omitted

Substantive reads/traces: Personal Agents store list/schedule/run/message lifecycle and schedule consumers; Proof store list/filter/page/detail/event/summary behavior and page callers; Assistant store request tickets, turn-reader lifecycle, task and thread loads; auth boot/login/impersonation/401 recovery; AWS store permission/account/service-cache ownership; plugin navigation loading; Home Today loading/derived source aggregation. Sampled Kubernetes action registry and scale/sync request construction, settings connection export and database compaction/retention handlers, usage attribution loader/export grouping, Insights report and archive loading, shared PathField and confirmation store, server Share extension/mail failure path, room recap consent and capture upload queue. Read relevant existing test source without executing it.

Positive bounded checks: recap epoch changes reject old consent/upload epochs and stopped recaps cannot restart; Kubernetes action calls retain explicit cluster/resource parameters across confirmation; usage attribution associates responses with a request sequence; connection-export controls freeze scope during preparation. These are source traces, not runtime acceptance.

Omitted or only sampled: complete Rust Assistant execution and Personal Agent scheduling engines (prior repairs not reopened), complete plugin supervisor/install lifecycle, usage ingestion/pricing/ClickHouse queries, all settings panels and backup/restore implementation, Kubernetes command execution and monitoring backend, AWS external operations, full Proof assembly/persistence, Share token persistence semantics, room media negotiation/two-device/native capture, exhaustive shared components, multi-user/browser/native execution, light/dark renders, and performance measurements. The partition is substantially larger than this bounded pass. No off-lens issues are asserted.
