# Correctness review — orchestration partition 4

**Verdict: Approve with fixes.** Counts: blocker 0 · major 6 · minor 0 · question 0.

Baseline: `a16f4c71`. Read-only source review using the correctness-review skill. Every finding below is **confirmed by hand-tracing concrete inputs/interleavings**, not by executing the application. No builds, tests, servers, or source edits were performed; this report is the only file written.

Intended behavior comes from the lifecycle/dispatch comments and guards in the cited implementations: stopping a swarm stops its work, one agent runs one turn at a time within the swarm cap, Run starts the selected workflow once, version restore applies the selected workflow's history, and approvals authorize their own requester.

## Findings

### C4-01 — [major] Another caller's approval shadows the caller's valid approval

**Location:** `crates/otto-mcp/src/service.rs:796`, `crates/otto-state/src/mcp_control.rs:1477`.

**Actual versus expected:** An approved tool call is denied with “approval belongs to another caller” even when this caller has a matching approved, unconsumed authorization. The matching query should select only this caller's approval; another caller's authorization should not prevent requesting a new one either.

**Trace:** Users A and B request the same tool, server, workspace, and argument hash before either request is approved. An authorized approver approves A's request, then B's. `find_usable` filters the call identity but not `requested_by`, orders by newest `decided_at`, and returns B's ID. A retries; `invoke` loads B's approval and immediately denies at lines 805–817. A's own valid approval remains unused and every retry selects B again until B consumes it or it expires. If only B has an approval, A cannot even enter the branch creating A's own pending request.

**Impact:** Independent callers block each other's approved MCP work in shared workspaces.

**Fix:** Add requester identity to the usable-approval lookup, including explicit null matching for anonymous/internal callers. Retain the ownership verification and atomic consume as defense in depth. Do not fall back to using another caller's approval.

**Regression:** Create same-call approvals for A and B, approve A then B, invoke as A and assert A's ID is consumed and B remains usable. Also assert that A receives a new pending approval when only B has an approved one.

### C4-02 — [major] A coordinator already in a tick can start work after Pause or Abort completes

**Location:** `crates/otto-server/src/swarm_runtime.rs:399`, lifecycle handlers at `:2066` and `:2090`.

**Actual versus expected:** Pause/Abort can return with the swarm stopped while a new agent turn is subsequently enqueued and executed. A completed lifecycle stop should exclude any later dispatch from the old coordinator.

**Trace:** An active swarm has a ready task and no current run. `tick_inner` reads `active` at lines 266–269, reads ready tasks, and suspends before `create_run`. Pause writes `paused`, sets the handle flag, stops existing runs, suspends existing sessions, and returns (2072–2087); the new run does not exist during either cleanup pass. The old tick resumes and inserts a queued run at 399–409, then spawns `run_turn` at 439. The tick checks neither the handle nor fresh swarm status inside this dispatch section. `swarm_run.rs:258` only checks the run's status; its subsequent swarm lookup at 268 checks existence, not active status. The newly queued row passes, and the turn starts. Abort has the same gap: cancelling the coordinator only takes effect between ticks (`coordinator_loop`, 139–150), and its cleanup can finish before the stale tick inserts the row.

**Impact:** Agent work and spend continue after the user has stopped the swarm, potentially modifying a repository the user believes idle.

**Fix:** Serialize lifecycle changes and dispatch under a common per-swarm operation boundary; make status validation and enqueue atomic relative to stopping. Ensure the scheduler and other producers use this boundary too. Merely rereading status before an unrelated insert leaves another check-to-insert race. A defensive active-state check at turn start is useful but does not replace atomic dispatch/lifecycle coordination.

**Regression:** With a barrier immediately before enqueue, let a tick pass its initial active-state read, complete Pause/Abort, then release the tick. Assert no new live run/session is created, the task remains resumable, and no prompt is dispatched. Cover both lifecycle operations.

### C4-03 — [major] Scheduler and coordinator independently reserve the same agent and capacity slot

**Location:** `crates/otto-server/src/swarm_scheduler.rs:72`, `crates/otto-server/src/swarm_runtime.rs:321`.

**Actual versus expected:** An agent can receive two concurrent turns, and the swarm can exceed its configured parallel cap. Both paths intend one turn per agent and enforce capacity using separate stale reads.

**Trace:** Set `max_parallel_sessions = 1`; agent A is active, has a due schedule, and owns a ready task. The scheduler reads active count 0 and `agent_has_active_run(A) = false` at lines 72–87. Before it inserts, the coordinator reads active count 0 and a busy set without A, then claims the task. The scheduler inserts a scheduled run at 108; the coordinator inserts a task run at 399. The coordinator's `tick_lock` at runtime lines 117–148 only serializes coordinator ticks; the scheduler never acquires it. `SwarmRepo::create_run` (`crates/otto-state/src/swarm.rs:1433`) is an unconditional insert, with no active-agent/capacity reservation. Both rows therefore execute. `run_turn` has no per-agent execution lock and deliberately reuses agent sessions (`swarm_run.rs:663` onward), so concurrent turns may also inject competing prompts into the same session when their cwd matches.

**Impact:** Wrong task results, concurrent work on the same agent worktree/session, and violated user concurrency limits.

**Fix:** Route all swarm run producers through one atomic dispatch/reservation operation that checks lifecycle, agent ownership, and swarm capacity, then creates the run. Include scheduled and utilization-triggered runs, not only coordinator tasks; advance a schedule cursor only once its reservation succeeds.

**Regression:** Barrier both producers after their eligibility reads with cap 1 and the same agent. Release together and assert exactly one live run, one dispatched prompt, and the losing task/schedule remains eligible for a future turn.

### C4-04 — [major] Run can execute a different workflow after waiting for Save

**Location:** `ui/src/modules/workflows/WorkflowsPage.svelte:783`.

**Actual versus expected:** Run initiated for workflow A can POST a run for newly selected workflow B using A's input. A run request should retain its original workflow identity or stop if the view changes.

**Trace:** Open A with dirty graph edits and choose Run. Validation succeeds and `startRun` awaits `save()` at 783; Save correctly captures A's ID at 571. While its PATCH is pending, select B and accept discarding A's draft. `open(B)` sets `current = B` and `dirty = false` (486–509). A's PATCH resolves; `applySavedWorkflow` correctly leaves B open, and `save` returns. `startRun` then sees B's false `dirty`, reads **B's** ID at 786, and posts `/workflows/B/run` with the original A body at 788. The server accepts full runs for B without requiring any A identity (`routes/workflows.rs:505–534`). Separately, switching workflows while the final POST is pending lets its response unconditionally install A's run under B at 791–792.

**Impact:** The user may execute an unintended automation or view/cancel a run under the wrong workflow.

**Fix:** Capture workflow/workspace identity and an operation generation before the first await. Validate/save that captured workflow, then recheck ownership before dispatch and before installing the response. Decide explicitly whether navigation cancels the pending launch or lets A launch in the background; never retarget it to B. Handle navigation to an empty workspace without dereferencing a now-null `current`.

**Regression:** Hold A's Save response, select clean B, release A's response, and assert no POST to B. Hold A's Run response, select B, release it, and assert B never displays A's run. Repeat with workspace navigation leaving no selected workflow.

### C4-05 — [major] The Run double-click guard starts after the asynchronous validation

**Location:** `ui/src/modules/workflows/WorkflowsPage.svelte:781`.

**Actual versus expected:** Two quick Run actions create two independent runs; the documented double-click guard should admit one launch.

**Trace:** A is clean (`dirty = false`), `running = false`. Invoke `startRun` twice before either `/validate` response completes. Both pass the guard at 781 and await validation at 782. First validation resolves: it sets `running = true` at 785 and posts the run. Second validation resolves while that POST is pending: there is no second `running` check, so it also posts. Run controls such as the node's “From here” / “Only this” buttons at 2113–2114 disable on `running`, which remained false during validation. The backend manual-run route creates a new run for each POST (`routes/workflows.rs:531–534`), so it does not deduplicate them.

**Impact:** Duplicate automation effects and duplicate paid agent work from a common double click or repeated activation during a slow preflight.

**Fix:** Reserve the launch operation synchronously before validation and release it in an outer `finally` covering validation, save, and POST. Keep the view/run ownership checks from C4-04; a late response must not clear a newer operation's guard.

**Regression:** Delay validation; activate “Only this” twice on the same node, then release both validation responses. Assert exactly one run POST and one created run. Include failed validation to verify the guard is released.

### C4-06 — [major] A stale version-history response can make Restore modify the wrong workflow revision

**Location:** `ui/src/modules/workflows/WorkflowsPage.svelte:1452` and `:1466`.

**Actual versus expected:** The Versions drawer can display A's history under B and restore B's same-numbered revision when the user selects an A entry. Displayed history and restore target should share one workflow identity.

**Trace:** Request A's version history and hold the response. Open B and its Versions drawer; B's history returns first. Opening B cleared `versions`, but it did not invalidate A's pending loader. A's late result then assigns `versions` unconditionally at 1452. The drawer renders every entry without an ownership check at 1984–1989. Click displayed A revision 2 while B is current. `restoreVersion(v)` sends `restoreWorkflowVersion(current.id, v.version)` at 1466, so the request restores **B revision 2**. If B has that revision, its current graph/instructions are replaced with a different state than the displayed history describes. This operation can also retarget after the discard-confirmation await because the ID is not captured before 1464.

**Impact:** A user action based on another workflow's history replaces B's current definition; a new revision preserves recovery history but does not prevent the incorrect restore.

**Fix:** Guard the complete version loader (success, error, and loading state) with a captured workflow ID plus generation. Capture the version's workflow identity before confirmation, verify it still owns the drawer, and restore using that identity. Guard response installation so a completed restore cannot reopen an editor the user has left.

**Regression:** Delay A's versions read, fully load B's versions, release A, and assert only B entries remain. Exercise restore while switching the selected workflow during discard confirmation and while the restore POST is pending; assert no mutation is sent to the newly selected workflow.

## Coverage and limits

Substantive traces covered swarm coordinator scheduling, scheduled-agent dispatch, turn startup/CAS settlement, Pause/Abort cleanup; workflow UI save/run/version ownership plus backend manual run creation and queued/restart execution; MCP approval lookup/consume and outbound invocation. Additional reads sampled Goal Loop lifecycle serialization, pause/retry/controller decisions, loop detail polling, scheduled-task in-flight ownership/cancellation/workflow handoff, and Mission Control detail loading/workgraph service mutation broadcasts.

The existing safeguards correctly reject executor retry while a Goal Loop is running, keep scheduled manual execution outside the HTTP request lifetime, prevent a failed workflow graph save from running an older persisted graph, and preserve stopped swarm-run status through CAS settlement. Those checks do not eliminate the separate traces above.

This is a bounded highest-risk pass, not full coverage of every workflow node, swarm verification/merge path, cadence form, Mission Control projector, MCP transport/policy combination, or UI state. No dynamic reproduction or browser confirmation was performed. MCP error-state UI and scheduled-task Run-now confirmation were deliberately excluded because another reviewer owns them. No security/performance/style findings are included.
