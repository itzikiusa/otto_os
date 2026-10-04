# Iteration 2 — independent partition 4 recheck

**Verdict: Approve with fixes.** Remaining confirmed findings: blocker 0 · major 1 · minor 0. Original defects are addressed on the traced paths; one adjacent Mission Control draft-loss path remains.

Reviewed the combined working tree in `/Users/itziklavon/claude_ade-review` against baseline `a16f4c71`, including the current role-5 approval-decision CAS change. Read the original partition-4 correctness, performance, UX and design reports, `iteration-1-implementation-4.md`, and `iteration-2-integration.md`. Applied the correctness and performance review instructions. This is an independent **source-only** recheck: no builds, tests, servers, provider runs, screenshots, runtime measurements, or source modifications. This report is the only authored file in this review turn.

## Confirmed remaining finding

### C4-R2-02 — [major] Same-item refresh still replaces an active Mission Control draft

**Location:** `ui/src/modules/mission-control/WorkItemDetail.svelte:84`, particularly the draft assignments at `:87`; approval callers at `:146` and `:164`.

**Expected:** A background detail refresh or approval action may update status/approval data, but must preserve unsaved goal, result-summary and risk edits. The new `canLeave` guard protects navigation; staying on an item must not discard the same draft through another path.

**Actual:** Every successful `load()` for the current item/workspace unconditionally assigns all three editor fields from the server. It does not check whether editing began or fields changed after the request started. Approval actions explicitly call this loader while the editor can remain open.

**Confirmed source trace, no timing race required:**

1. Open item I with persisted goal `old`; click Edit (`:278`) and replace the goal with `my unsaved draft`.
2. Click Request approval. Its control is enabled whenever `!busy`, without an editing/dirty condition (`:338–340`). The method posts the approval, then calls `load()` at `:146`.
3. The GET returns I with persisted goal `old`. Identity checks at `:85` pass because neither item nor workspace changed.
4. Lines 86–89 install the refreshed detail and overwrite `editGoal`, `editResult`, and `editRisk`. The editor stays visible with `old`; the user's draft has vanished without confirmation. The new dirty check now compares those reseeded values with the same detail and reports clean. Approve/Deny also calls `load()` at `:164`, so it has the same outcome.

**Confirmed asynchronous variant:** A live event arrives while `editing = false`, so line 118 starts `load()`. Hold its response, enter Edit, and type the draft. Release the old response. The request-time editing check does not run again; lines 87–89 replace the new draft. Thus disabling approval actions alone would leave the background-read variant unfixed.

**Impact:** Manually authored goals/results are lost during normal approval work or a live refresh. This is adjacent to UX4-02 rather than a recurrence of its fixed selection/close path.

**Concrete fix:** Give editor drafts an explicit owner/baseline or edit generation. Preserve active drafts when installing same-item background/approval data; reseed them only for a new item, explicit Cancel, or a successfully saved submitted version. Distinguish read-request ordering from draft ownership so an older response cannot roll back a newer saved result. Protect text typed during an in-flight Save as well, either by disabling editor fields for that operation or by retaining edits newer than the submitted snapshot.

**Regression cases:**

- Edit all three fields, complete Request approval and Approve/Deny, and assert approval data refreshes while the draft and dirty state remain.
- Start a live refresh before entering Edit; defer the GET response, type, resolve it, and assert no draft changes.
- Save, then release an older same-item GET; assert the successful saved state is not replaced with stale values.
- Explicit Cancel resets to the intended persisted baseline; an ordinary clean item switch still seeds the next item's fields.

The existing `ui/unit/orchestrationOwnership.test.ts:98` harness extracts `isDirty` and `canLeave` and exercises selection, but does not extract `load` or either approval method. It therefore cannot catch these response-to-draft paths. This is a test-source observation, not a claim about a test execution.

## Original finding dispositions

“Addressed in source” below means the original concrete trace is prevented by current implementation. It is not a runtime verification claim. The original performance report used titles without explicit IDs; P4-01…05 below label those five findings in their original order.

| Original ID | Disposition | Independent evidence |
|---|---|---|
| C4-01 — caller approval shadowing | Addressed in source | `McpApprovalRepo::find_usable` at `crates/otto-state/src/mcp_control.rs:1468` now includes nullable `requested_by IS ?`; both service and outward gateway supply the effective requester. A's lookup cannot select B's newer approval. Ownership verification remains, and consume checks both approved status and expiry. |
| C4-02 — dispatch after swarm stop | Addressed on traced producers/lifecycle paths | `swarm_runtime.rs:35` supplies one per-swarm operation boundary. Coordinator, scheduler, utilization, explicit task run, meta-agent setup and lifecycle actions use it. `SwarmRepo::reserve_run` additionally checks current lifecycle during insertion. Each session input rechecks the live run under this boundary (`swarm_runtime.rs:77`). Abort writes aborted status before cleanup. |
| C4-03 — duplicate agent/capacity reservation | Addressed in source | `crates/otto-state/src/swarm.rs:1436` uses one INSERT…SELECT requiring an available agent and swarm slot, current workspace/lifecycle and remaining total-run budget. All production `create_run` callers in the inspected `swarm_*` files now reserve; the remaining runtime `create_run` occurrence is in a test. A losing scheduled reservation does not advance its cadence cursor. |
| C4-04 — wrong workflow after Save | Addressed in source | `WorkflowsPage.svelte:839` captures workflow ID, workspace and view generation before validation/save, rechecks ownership before POST, and guards response installation. Navigating to another or no workflow aborts the pending launch; a late dispatched run response does not install into another editor. |
| C4-05 — duplicate Run preflight | Addressed in source | The same method sets `running = true` before its first await and releases it in its enclosing `finally`. Two synchronous activations cannot both enter validation. Failure/cancelled preflight also releases the guard. |
| C4-06 — cross-workflow history restore | Addressed in source | `WorkflowsPage.svelte:1514` binds history reads to workflow plus view/request generation. Restore validates `v.workflow_id`, captures that identity before confirmation, rechecks before POST and guards editor installation. |
| P4-01 — MCP pool spills into unbounded processes | Addressed mechanism; measurement pending | `otto-mcp/src/client.rs:125` waits for one of two slot locks instead of one-shot fallback. Health takes the same slots; current service construction rechecks cache insertion and keeps busy clients. A global 64-permit transport budget also covers parked/config-replacement transports. Cancellation owns/drops the live transport outside the slot, avoiding reuse with unread replies. Active transport count is bounded; queue latency and CPU/RSS remain unmeasured here. |
| P4-02 — response cap after unbounded read | Addressed mechanism; measurement pending | `client.rs:491` rejects excessive Content-Length, consumes chunks under the remaining cap, and does not append an over-cap chunk. `:545` bounds stdio fill-buffer consumption across lines and skipped notifications. Operation failure drops the transport. Large-body memory now scales with admitted readers × cap, not arbitrary response length. HTTP transport-level chunk buffering was not independently measured. |
| P4-03 — unbounded scheduled shell output | Addressed mechanism; measurement pending | `scheduled_tasks_engine.rs:939` drains each pipe while retaining at most 512 KiB plus the omission marker. `:988` joins both drainers with child wait; excess output continues draining. Existing process-group timeout/drop cleanup remains. Capture/report payload memory is bounded independently of command duration/output volume. |
| P4-04 — per-tool capability HTTP/SQL fan-out | Addressed mechanism; measurement pending | `resource_access.rs:528` bounds batches to 1,000 children and 4,096 bytes per name; `decisions_batch` shares membership/policy and operation ceilings, preserving the pure per-child evaluator. Tools retains owned batches; release removes unused entries and invalidates in-flight installs. Refresh batches up to 1,000 children per request; source checks no longer make one HTTP request per tool. Identity/generation guards remain. |
| P4-05 — serial per-session usage refresh | Addressed in source; usage fixture pending | `workgraph_projector.rs:887` requests one lifetime aggregate for workspace-derived session/external-trigger IDs, bounded by the 500-item list. `otto-usage/src/engine.rs:1140` uses IN plus GROUP BY without dashboard date/row truncation. Missing totals stay absent; finite nonnegative changed costs, including zero, write back. |
| UX4-01 — understated global replace scope | Addressed in source | `PoliciesTab.svelte:113` snapshots replace intent, fetches the complete unscoped policy list, always confirms instance-wide replacement with global/workspace counts, and does not POST after fetch failure or cancellation. The backend's global replacement scope is accurately disclosed. |
| UX4-02 — item selection/close loses draft | Original selection/close path addressed; adjacent draft loss remains as C4-R2-02 | `MissionControlPage.svelte:64` awaits `detailPane.canLeave()` before replacing selection/URL, with a selection generation. `WorkItemDetail.svelte:61` compares actual fields and blocks departure while busy; its router guard covers module navigation. Same-item refreshes bypass this navigation guard, as detailed above. |
| UX4-03 — Extend/Resume partial failure hidden | Addressed in source | `LoopDetail.svelte:143` keeps the extension modal open until Resume succeeds, remembers a successful limits PATCH, exposes a partial-success explanation, and retries Resume without resending the same PATCH. A response after leaving the modal uses a toast. |
| D4-01 — failed skill discovery appears empty | Addressed in source | `swarm.svelte.ts:882` exposes loading/error/loaded state and retains the old suggestions on failure. `SkillPicker.svelte:68` displays loading, error with Retry, and successful-empty state without altering typed or selected skills. Mount-triggered loading is untracked, avoiding a reactive retry loop. |
| External-owned Mission Control fullscreen focus | Source wiring present; browser/accessibility verification pending | `MissionControlPage.svelte:273` attaches `dialogFocus` to the small-screen detail with named `role=dialog`/`aria-modal` markup at `:406`; Escape calls the guarded selection close. Shared `dialogFocus` checks the topmost dialog for nested ownership and supplies return-focus behavior. No accessibility-tree, inert-background, or real keyboard behavior claim is made from source alone. |
| Integration C4-R2-01 — stop blocked by readiness mutex | Addressed in source | Ordinary run setup drops its operation guard at `swarm_run.rs:759`, before readiness/landing waits; meta setup drops at `swarm_agent_run.rs:248`. `while_run_active` waits without holding that lock and checks terminal state every 250 ms. Individual prompt writes reacquire the guard and reject stopped rows. The specific multiple-minute readiness critical section is absent. |

## Combined approval and adjacent lifecycle checks

Role 5's decision change composes with C4-01: `McpApprovalRepo::decide` updates only `status = 'pending'` and requires exactly one affected row. Two reviewers deciding one request cannot overwrite the winning decision; `consume` is still single-use and refuses newly expired rows. Matching caller identity is applied before selection, so the new decision CAS does not reintroduce caller shadowing. I inspected both the requester-lookup and simultaneous-decision fixture source but did not run them.

Swarm's ordinary-paused manual execution remains intentionally allowed by the reservation API, while automatic producers require active status and budget-paused/aborted swarms reject admission. Stop and input use the same boundary; the long TUI/Claude landing waits do not retain it. Meta-run callbacks and terminal success now CAS live statuses, preventing a stopped row from being resurrected by late output on those paths. No new confirmed lifecycle defect was found in this bounded recheck.

## Verification limits

Reviewed source/diffs and regression-test source only. Test outcomes quoted by the implementation report remain parent-owned evidence; they were not rerun or independently observed by this reviewer. Current HTTP/SSE cap test source is present even though the earlier implementation report listed that fixture as a pending gap. Concurrent MCP N=1/2/8/20 CPU/RSS, scheduled-output N=1/2 RSS, 1,000-tool HTTP/SQL counts, 500-session usage parity, real-app memory-over-time sampling, and light/dark/mobile behavior still require central evidence before claiming measured performance or visual completion.

This pass did not exhaustively inspect every workflow node/provider, all swarm merge/verification/recruiting logic, every access resource kind, browser workspace-switch behavior, or all Goal Loop controller phases. No further broad-scope hunt was performed after confirming C4-R2-02 so implementation can repair the exact remaining path promptly.
