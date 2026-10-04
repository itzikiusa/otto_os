# Iteration 1 — correctness partition 5

**Verdict: Block — 1 blocker, 4 major, 0 minor.** All five findings are confirmed by source-level hand traces; none was executed against a daemon or external service.

Baseline: `a16f4c71d5b158359fc3507c9f883c500c1f88a3`. Read-only review of selected high-risk paths; only this report was written. No builds, test runs, server processes, source changes, or external mutations.

## Ranked findings

### C5-01 — blocker — Assistant can approve an action already denied in the canonical approvals queue

**Location:** `crates/otto-server/src/assistant/tasks.rs:575` (ignored approval decision result); related `:548`–`:566`, `:669`, `:481`; `crates/otto-state/src/mcp_control.rs:1443`.

**Intent:** The Assistant task and linked MCP approval represent one decision; either surface can decide it (`docs/contracts/api.md:4343`, `assistant/tasks.rs:5`). A recorded denial must not become an affirmative tool response through the other surface.

**Confirmed trace:** Create an Assistant approval with `wait_seconds:30`. Deny its linked MCP approval, then click Approve on the still-open Assistant card before the 30-second synchronization tick. `act()` reads the still-`needs_you` task, computes `done` and `result.decision="approved"`. MCP `decide()` returns Conflict because its status is already `denied`, but line 575 discards that error. The Assistant row is nevertheless set to `done/approved`; `request_approval()` reads that result and returns `decision:"approved"` to the agent. `open_approvals()` only selects `needs_you`, so later synchronization never repairs the disagreement. If Always allow was selected, the persistent grant is also installed before the rejected decision.

**Actual vs expected:** The same action is denied in MCP but approved in the Assistant/tool response. Expected: first canonical decision wins; a later contradictory decision fails or returns the existing denial, without creating an allow grant.

**Fix:** Settle against the canonical approval first and propagate decision errors. On an already-decided row, reconcile the Assistant task from its actual decision rather than the requested action. Create any persistent grant only after successful approval. Use an atomic pending-to-decided transition in the canonical repo and synchronize the Assistant row so parallel decisions cannot overwrite each other.

**Regression:** Create linked approval/task rows, deny the MCP row, immediately approve the Assistant row with `always_allow:true`, and assert no approved Assistant result, no allow grant, and a denied tool result. Also test approval/denial racing from the two surfaces.

### C5-02 — major — Cancelling an Assistant task does not stop its execution

**Location:** `crates/otto-server/src/assistant/tasks.rs:658`–`:671` (`cancel` falls through to a state write); related `:327`, `:972`; `crates/otto-server/src/personal_agents_engine.rs:535`, `:738`–`:762`; `ui/src/modules/assistant/cards/TaskCard.svelte:22`.

**Intent:** The public cancel action ends queued/running/needs-you work (`docs/contracts/api.md:4343`), and the UI Stop confirmation promises “Otto drops what it has not finished.”

**Confirmed trace:** Delegate a long-running directive. `delegate()` starts a real Personal Agent run and records `agent_run_id`. POST the Assistant task's `cancel` action while that run is active. `next_state()` accepts it, but the action match has no cancel arm; it sets only `assistant_tasks.state='cancelled'`. It never calls the existing `personal_agents_engine::cancel_run(agent_run_id)` or interrupts a thread execution. The independent run continues through `execute_agent()` and may perform the remaining work. Delegation settlement queries only `state='running'`, so the cancelled card stops tracking the still-running job.

**Actual vs expected:** The UI/API reports cancelled while execution continues. Expected: propagate cancellation to the execution represented by the task and then settle the task consistently.

**Fix:** Add kind-aware cancellation before terminal settlement. For delegation, cancel the recorded Personal Agent run through its run-cancel mechanism; for thread-backed tasks, interrupt the owned current operation through the thread execution mechanism. Keep queued reminder cancellation as state-only. Handle already-finished runs and serialize cancellation against completion so completion cannot revive or overwrite cancellation.

**Regression:** Start a controllably blocked delegated run, cancel through `/assistant/tasks/{id}/cancel`, and assert the underlying run receives cancellation, its session stops, and no post-cancel work executes. Add the corresponding running thread-task Stop case.

### C5-03 — major — Taking over a delegation interrupts the parent Assistant session instead of the delegated agent

**Location:** `crates/otto-server/src/assistant/tasks.rs:640`–`:656`; related `:690`–`:692`, `:327`–`:345`; `crates/otto-state/src/assistant.rs:836`.

**Intent:** `takeover` interrupts the agent and parks its task until `handback` (`docs/contracts/api.md:4343`). Delegations are accepted by the same state machine.

**Confirmed trace:** Assistant thread T delegates to Personal Agent run R, which owns a separate session S. The delegation task records both T and R. POST `takeover` for that task. `thread_session()` resolves T's session, ignoring `agent_run_id`; Esc goes to the parent Assistant, while R/S continues running. The task is now `needs_you`, so `running_delegations()` stops observing R. `handback` sends its continuation into T again, not S. With a delegation created without a thread, takeover sends nothing at all while still returning success.

**Actual vs expected:** The task claims the user has control, but its delegated work has never paused. Expected: control the execution attached to this task, or reject takeover when that execution cannot safely support it.

**Fix:** Resolve delegation controls through `agent_run_id` and its run/session lifecycle, with explicit pause/resume semantics. If Personal Agent runs do not support resumable takeover, reject takeover for delegation tasks and offer cancellation/open-session instead. Do not mutate the task to takeover after acting on the wrong session.

**Regression:** Give parent thread and delegated run different spy sessions; takeover must pause the delegated session and leave the parent untouched, or return a documented conflict. Test threadless delegations and handback as well.

### C5-04 — major — Reinstalling an enabled plugin rotates its credentials without restarting its sidecar

**Location:** `crates/otto-server/src/plugins.rs:503`–`:525`; `crates/otto-state/src/plugins.rs:88`–`:91`; related `crates/otto-server/src/plugins.rs:167`, `:679`.

**Intent:** Local installation can replace an existing plugin through the upsert path. Enabled state, installed metadata, and the actual sidecar must describe the same installation.

**Confirmed trace:** Install a local plugin, enable it, then install that same local folder again. The running sidecar received token A in `OTTO_PLUGIN_TOKEN`. Reinstall upserts token B (`new_id()` at line 513); the SQL conflict clause preserves `enabled=1`. The handler neither stops nor respawns the existing sidecar. It returns an enabled plugin while the running process still carries A. Its next host API call fails `find_enabled_by_token(A)`, returning 401. Changed exec/source metadata also takes effect only in the DB while the old process keeps running.

**Actual vs expected:** A successful reinstall leaves an apparently enabled plugin broken until manually disabled/re-enabled or daemon restart. Expected: a consistent stopped installation awaiting enable, or a restarted enabled installation using the new metadata and token.

**Fix:** Serialize install/replace with enable/disable for the slug. Stop the old process before replacing a live installation, then either explicitly set disabled (matching install's existing UI instruction) or restart with the new record and roll back enabled status on failure. Do not rotate a live token without coordinating the process that uses it.

**Regression:** Install and enable a fixture plugin, reinstall from the same local path, and assert enabled state matches process state and every live process uses the current token. Include a changed exec command and failed restart.

### C5-05 — major — Athena history opens query results in the wrong region

**Location:** `ui/src/modules/aws/AthenaView.svelte:304`–`:316`; related `:73`, `:259`, `:284`, `:294`; `ui/src/lib/api/aws.ts:199`–`:213`.

**Intent:** Athena catalog/history and executions are region-specific. The component explicitly tracks `qRegion` so status and cancellation follow the execution's region.

**Confirmed trace:** Account default is `us-east-1`. Select `eu-west-1`, load History, and open a completed execution from that list without having submitted a query in this component. History is fetched with `region=eu-west-1`, but `openExecution()` sets `qid` without setting `qRegion`. It remains the initial empty string. The scheduled `poll()` calls `athenaStatus(..., qRegion || undefined)`, so the request uses the account default `us-east-1` and cannot find the EU execution. If an earlier run used another region, that old region is reused instead; Cancel uses the same wrong value.

**Actual vs expected:** Opening valid non-default-region history fails to load its result, and active historical executions cannot be cancelled correctly. Expected: status/cancel use the region of the selected history item.

**Fix:** Retain the region associated with each loaded history response and assign it to `qRegion` when opening that response's execution. Fence stale history responses on region changes so an old list cannot be labelled with the new region. Merely reading the current selector during open is insufficient if a stale list remains visible.

**Regression:** Mock different regions, open a historical execution from a non-default region before any run, and assert status/cancel include that region. Repeat after running in another region and with delayed history responses during region switching.

## Coverage and limits

Substantive traces: Assistant task creation/delegation, cancel/takeover/handback, canonical approval synchronization and waiting; Personal Agent scheduler/in-flight and run cancellation paths; plugin install/upsert/enable/disable/authentication; Athena query/history/status/cancel region ownership. Sampled additionally: auth boot/impersonation/401 recovery, settings hot-reload, usage summary/report aggregation and budget deduplication, Insights period/report loading, history cursor pagination, room registry connect/disconnect/admission and recap consent/capture, proof artifact/waiver/assembly handlers, Share mint guards, Kubernetes resource polling and Pod HTTP helpers.

The sampled history cursor and recap consent branches had no confirmed bug in this pass. That is not an exhaustive clean bill of health for those subsystems. No runtime validation was performed; concurrency findings use explicit source-level interleavings, and external service behavior was not exercised. The broad partition is much larger than one focused pass: AWS services other than Athena, Kubernetes command execution/monitoring, full usage ingestion/pricing, backup/restore, every settings panel, complete room media networking, all proof assembly sources, and physical/two-device flows remain unreviewed or only sampled.

Excluded as assigned: the separately owned `k8s_monitor_clickhouse` test; production Kubernetes/exec/SQS/Share confirmations; home error states; visual, accessibility, and copy findings. No off-lens findings are asserted here.
