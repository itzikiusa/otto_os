# Error handling: quality iteration 4 (2026-10-22)

Lens: resilience and error UX. This covers swallowed errors, raw exceptions shown instead of an inline Retry, missing recovery after a daemon restart or network drop, retry storms, misleading success or status messages, unhelpful error text and crash paths.

Branch `fix/quality-loop-20261021` at `6cff08270`. The review was read-only (grep, sed and git only; no builds or tests).

## Score

Rubric: start at 10.0 and subtract per open finding (blocker 1.0, major 0.3, minor 0.1, nit 0.03).

- 5 minor × 0.1 = 0.50
- 3 nit × 0.03 = 0.09
- Total deductions: 0.59

**Score: 10.0 − 0.59 = 9.41 / 10** (iteration 3: 9.31; iteration 2: 9.17; iteration 1: 8.17)

## Previous findings (iteration 3)

| ID | Status | Evidence |
|----|--------|----------|
| 3-01 Raw exception on core surfaces | **Partly open** (now 4-01) | Fixed: `workspace.svelte.ts:1001`, `pushFlow.ts:86`, `pullFlow.ts:65`, `history.svelte.ts:165/192`, `graph-commit-diff.svelte:188`, `DiffViewer.svelte:861`, `GitPage.svelte:220`. Still raw, and both were named in 3-01: `graph-commit-diff.svelte:360` (per-file body) and `DiffViewer.svelte:1005/1018` (the "Couldn’t stage" toast). |
| 3-02 Goal-loop terminal writes discarded | Fixed | `settle_status` retries 5× (`goal_loop.rs:1164-1200`) and raises a Notice. `finalize` emits only the stored status (`:1141-1157`) and returns the error. `stop_loop` propagates it with `?` (`:299-306`). Per-iteration writes go through `warn_on_err` (`:1214`). |
| 3-03 Failed tab fetch closes tabs | Fixed | Ids of a failed `?ids=` chunk go into `unknown` (`workspace.svelte.ts:937-950`). Their rows are kept (`:957`) and `reconcileTabs(unknown)` never prunes them (`:1013-1016`). |
| 3-04 Final output hidden / context 403 | Fixed | `readRunContext` goes through the run-scoped `routes/workflow_run_files.rs`. Only a 404 hides the output. Anything else sets `finalOutputError` with Retry and resets `finalOutputRunId` (`WorkflowsPage.svelte:403-424`). |
| 3-05 Usage tailer stops for good | Fixed | The tailer retries with a 5 s→5 min backoff and reports `Retrying` / `Stopped` / `Running` (`usage_tailer.rs:295-350`). The Usage page shows a banner (`UsagePage.svelte:542-548`). |
| 3-06 Blender probe failure shown as "not installed" | Fixed | `blenderProbeError = loadErrorText(e)` (`DesignArena.svelte:769`). LoadState with Retry and `recheckBlender` (`:772, 1078-1079`). |
| 3-07 Swarm roster read failure read as "no agent fits" | Fixed | `repo.list_agents(&swarm.id).await?` (`engine.rs:589`). The sibling `busy_agents` read in the scheduler is still swallowed; see 4-06. |
| 3-08 RunAgents memoises every failure | Fixed | `attemptedSessions.delete(id)` unless the error is a 404 (`RunAgents.svelte:128`). |
| 3-09 Pull labelled "merge" on a failed read | Fixed | `pullMode` stays `null` until the read lands, so the label is plain "Pull" (`GitToolbar.svelte:74-97`). The catch resets `pullModeFor` (`:88`). |

## Areas covered this iteration

- Re-verified all 9 iteration-3 findings against the current code.
- **MCP outward governance** (`otto-mcp/src/outward/exec.rs` `run_tool`, `self_*`, `parse_self`, `self_call_error`, `self_get_text`; `otto-server/src/mcp_outward.rs`):
  - The approval wait, denial and dry-run envelopes.
  - The executed-error envelope, `unresolved_audit` and the pending-audit arming.
  - Denials (`decision: denied`), errors (`decision: error, is_error`) and pending approvals are distinct and carry the cause.
- **otto-git** `upstream_err` (`local.rs:5312`): the meaningful-line pick, local-refusal → 409 and the auth hint. Sound.
- **Bitbucket 202 merge polling** (`providers/bitbucket.rs:225-275`): same-origin task URL, poll errors tolerated within the budget, and an explicit "hasn’t finished yet" conflict. Sound.
- **otto-dbviewer**:
  - SSH tunnel liveness (`service.rs:1567`, `otto-ssh is_alive`), idle eviction and `close_connection`.
  - The Redis `BoundedConnection` liveness and retire path (`drivers/redis/transport.rs`), and `get_or_try_init` with `is_alive`.
  - Sound: a dead connection is rebuilt on the next query, with "retry to reconnect" text.
- **otto-telemetry-collector**: startup recovery and the child lifecycle (recovery tests exist). Not traced deeper.
- **Sessions manager**: `auto_archive_stale` (checked cutoff, per-row failures logged, pagination advances past failures), `restart_locked` and `respawn_failed`, and `stamp_suspended`.
- **Swarm utilization scheduler** (`runtime/scheduler.rs:304-470`).
- **Server routes**: discarded writes across `routes/` (scheduled-task convert and create, findings, proof, notifications, API automation runs), `workflow_run_files.rs` error mapping, and `error.rs` (500s logged; the Problem body carries the message).
- **otto-channels Telegram** listener backoff (3 s doubling to 60 s, reset on success) and `dispatch_prefix`. Sound.
- **UI**:
  - AWS views (`S3Browser`, `LogsView`, `AthenaView`) and `awsErrorText`.
  - The Jira issue editor state (`issueEditorState.svelte.ts`).
  - `ScheduledTasksPage` convert and destination probes, `GraphView` side-list fallbacks, the canvas store and the rooms client.
  - The swarm store and GoalsPanel, `workflowRuns`, bulk archive and delete, the notifications store and FloatingBar.
- **e2e**: error states are exercised (`desktop-load-errors.spec.ts`, `desktop-db-error-panel.spec.ts` and about 20 specs that click Retry). No finding.

## Areas NOT covered yet (next iteration)

- `otto-assistant` (personal agents scheduler, task reminders): delivery failure and claim recovery.
- `otto-product` runners (analysis, rewrite, test-case and plan runs) on the server side: what a failed agent turn leaves on the story.
- `otto-review` summarizer fallback: the error text shown when both the managed summarizer and the deterministic fallback fail.
- `otto-browser` live tabs (Lightpanda sidecar crash and restart) and `RemoteLiveView` raw errors (2 baselined).
- The k8s monitor scheduler (`k8s_monitor_scheduler.rs`): a cluster that is unreachable for a long time, and the backoff.
- The remaining raw-error baseline: `PublishDialog`, `MultiRunDialog`, `NetworkProfiles`, `AccountWizard`, `ClusterWizard` and the settings pages (`ui/scripts/ui-guards-baseline.json`).
- The desktop shell (`apps/desktop/src-tauri`) sidecar supervision: an ottod crash loop and what the window shows.

---

## Findings

### error-handling-4-01: Raw exception text still reaches the user on core surfaces (minor; partial carry-over of 3-01)

**Files:**
- `ui/src/modules/git/graph-commit-diff.svelte:360`: `bodies.set(f.path, { kind: 'error', message: e instanceof Error ? e.message : String(e) })`. It is rendered at `:542` as "Couldn’t load this file’s changes: {body.message}".
- `ui/src/modules/git/DiffViewer.svelte:1005, 1018`: `const msg = e instanceof Error ? e.message : String(e)` … `toasts.error('Couldn’t stage', msg)`.
- `ui/src/lib/stores/notifications.svelte.ts:332`: `this.error = (e instanceof Error ? e.message : String(e)) || 'Request failed'`. It is rendered by `NotificationBell.svelte:359, 383`.
- `ui/src/lib/components/FloatingBar.svelte:315`: the ⌘K command failure `detail: e instanceof Error ? e.message : String(e)`.

**Evidence:**
- Each site uses the pattern that `lib/loadError.ts` forbids.
- The bell's own comment (`notifications.svelte.ts:329-331`) names its main failure case: "Backend may not be ready yet". That is exactly when `fetch` rejects with WebKit's `TypeError("Load failed")`, so the bell's inline error reads "Load failed".
- A per-file diff fetched during a daemon restart reads "Couldn’t load this file’s changes: Load failed", and a staging 401 shows the server's raw text instead of the sign-in line.
- Iteration 3's FIXES marked 3-01 fixed for the store part only. The two git sites above were named in 3-01.

**Fix:**
- Use `loadErrorText(e)` at `graph-commit-diff.svelte:360`, `notifications.svelte.ts:332` and `FloatingBar.svelte:315`.
- In `DiffViewer.svelte`, keep `msg` for the 400/409 classification and pass `loadErrorText(e)` to the fallback `toasts.error('Couldn’t stage', …)`.
- Run `node scripts/ui-guards.mjs --update-baseline` afterwards.

### error-handling-4-02: AWS views show "Load failed" and raw 401 text in their error states (minor)

**Files:**
- `ui/src/modules/aws/S3Browser.svelte:80, 151, 217, 245, 284`
- `ui/src/modules/aws/LogsView.svelte:105, 242, 427, 458`
- `ui/src/modules/aws/AthenaView.svelte:127, 290, 343, 377`

**Evidence:**
- Every load error is stored as `e instanceof Error ? e.message : String(e)` and rendered through `awsErrorText(...)` (for example the EmptyState body "Couldn’t list buckets" at `S3Browser.svelte:589`, and `LogsView.svelte:546, 628`).
- `awsErrorText` (`aws/util.ts:79-92`) only rewrites the boto `An error occurred (Code) when calling …` shape. It passes every other string through unchanged.
- So, with the daemon restarting or the network dropping, the S3, CloudWatch Logs and Athena panes show "Couldn’t list buckets — Load failed". A daemon 401 or 404 shows the server's raw text instead of `loadErrorText`'s human line.
- These three files hold 14 of the baselined `raw-error` hits (`ui-guards-baseline.json`: 6 + 4 + 4). The same pattern is in `RdsView`, `EksView`, `Ec2View` and `AccountWizard`.

**Fix:**
- Store two values: the raw message, still used for `isLoginRequired` (which needs the provider text), and a display text.
- Compute the display text as `awsErrorText(providerErrorText(e))`. The AWS store already uses `providerErrorText`, which keeps a provider 403's actionable text and turns a `TypeError` into "Otto can’t reach the daemon."
- Lower the baseline afterwards.

### error-handling-4-03: "Convert to workflow" reports success and pauses the task even when the schedule trigger wasn't created (minor)

**Files:**
- `crates/otto-server/src/routes/scheduled_tasks.rs:150-175`
- `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:700-708`

**Evidence:**
- The trigger is created with `.create(NewWorkflowTrigger { … kind: "schedule" … }).await.ok();`, which discards the error.
- Then `if req.disable_task { let _ = ctx.scheduled_tasks.update(&task.id, ScheduledTaskPatch { enabled: Some(false), .. }).await; }` disables the task unconditionally, and also discards that write's error.
- The route returns 200 with `trigger_id: None`.
- The UI ignores `trigger_id` and toasts "Converted to workflow — Created a workflow from “X” and paused the task."
- When the trigger INSERT fails (SQLite busy, disk full), the user's recurring job stops running entirely: the task is paused, and the new workflow has no schedule. Nothing says so.
- When the disable write fails instead, the toast claims "paused the task" while the task keeps running, so every run is delivered twice. The confirm dialog (`:686`) warns about exactly that outcome.

**Fix:**
- Propagate the trigger error with `map_err(ApiError)?`, before touching the task.
- Only disable the task after the trigger exists, and propagate that error too.
- Better still, do the workflow, the trigger and the disable in one transaction, or delete the just-created workflow on failure.
- In the UI, treat a missing `trigger_id` as a failure ("The workflow was created without its schedule").

### error-handling-4-04: A failed Jira editmeta load leaves every story field read-only for the rest of the visit, with only a toast (minor)

**Files:**
- `ui/src/modules/product/jira/issueEditorState.svelte.ts:272-289`
- `ui/src/modules/product/OverviewTab.svelte:299-303, 245-253, 1620`

**Evidence:**
- `ensureEditmeta` catches the error, calls `toastError('Couldn’t load editable fields', e)` and sets `this.editmeta = []` ("loaded-but-empty: every field stays read-only").
- The OverviewTab effect only re-fetches while `jira.editmeta === null`. `jira.reset()` runs only on a story switch (`:253`).
- So a single timeout or daemon restart while the story opens silently removes every inline editor (story points, sprint, labels, priority…) until the user navigates to another story and back.
- The toast disappears. Nothing in the pane says why the fields can't be edited, and there is no Retry.
- This is the "a load failure renders inline with Retry, not as a toast" rule from `lib/loadError.ts`.

**Fix:**
- Keep `editmetaError = loadErrorText(e)` instead of faking an empty list (leave `editmeta` null, and gate the effect on `editmetaError` to avoid a loop).
- Render a compact `LoadState` ("Couldn’t load editable fields") with a Retry that clears the error.
- Drop the toast.

### error-handling-4-05: A failed restart labels the session "released to free memory" (minor)

**File:** `crates/otto-sessions/src/manager.rs`
- `:6283-6302` (`respawn_failed`)
- `:5829, 5865, 5873` (callers)
- `:1147-1159` (reason constants and lifecycle line)
- UI: `ui/src/lib/status.ts:333-338`

**Evidence:**
- When an explicit restart of a live session fails after the old process was killed, `respawn_failed` runs. The failure can be account apply, a spawn error, the daemon shutting down, or the provider-id write.
- `respawn_failed` sets the row to `Reconnectable` and calls `self.stamp_suspended(session, Some(SUSPEND_REASON_RELEASED))`.
- `released` is documented as "An explicit `SessionManager::suspend` call (engines, API)". The UI renders it as "Suspended … · released to free memory", and the trail line reads "Suspended (freed memory, still resumable)".
- The restart error toast is transient. Afterwards the pane permanently attributes the dead session to a memory-saving suspend, so the user has no hint that the agent failed to start and why.
- `respawn_failed` also emits `SessionStatus{Reconnectable}` even when `update_status` failed (`:6291-6298`). This is the "emit only what was stored" shape fixed for goal loops in 3-02.

**Fix:**
- Add a `SUSPEND_REASON_RESPAWN_FAILED` ("restart failed") reason that carries the cause in `meta.suspended.error`. Map it in `status.ts` ("Couldn’t restart: <cause>") and in `suspend_lifecycle_line`.
- Emit the status only after `update_status` succeeds.

### error-handling-4-06: The swarm utilization check swallows the busy-agents read and announces reassignments that weren't saved (nit)

**File:** `crates/otto-swarm/src/runtime/scheduler.rs:341, 354-388`

**Evidence:**
- `let busy = repo.busy_agents(&swarm.id).await.unwrap_or_default();` turns a failed read into "nobody is busy". This is the same shape as 3-07, which was fixed in `engine.rs` but not here.
- Each rebalance move is written with `let _ = repo.update_task(… assignee_agent_id …).await;`. The code then calls `emit_task_pub` and posts "⚖️ Utilization check: … rebalanced N ready task(s): “T” → Agent" to the board, whether or not the write landed.
- On a failed write, the board announces a reassignment the task list doesn't show. (Dispatch stays safe because `reserve_run` re-checks busyness atomically.)

**Fix:**
- `busy_agents(...).await?` (the tick already logs and retries on the next check).
- Only count and announce the moves whose `update_task` returned `Ok`, and `warn!` the rest.

### error-handling-4-07: An agent's `k8s_logs` call reads a broken response body as "no logs" (nit)

**File:** `crates/otto-mcp/src/outward/exec.rs:2057-2085` (`self_get_text`, line 2070)

**Evidence:**
- `let text = resp.text().await.unwrap_or_default();` runs before the status check. On a 2xx whose body breaks off mid-read, the tool returns `{"text": "", "truncated": false}` as a success.
- `parse_self` (`:2003-2018`) was fixed for exactly this case. Its comment says "A body that broke off mid-read is NOT success". The text path was not.
- An agent investigating an incident is told the pod printed nothing.

**Fix:** Mirror `parse_self`: on a 2xx, map a `text()` error to `Error::Upstream(format!("{status}: reading the response failed: {e}"))`.

### error-handling-4-08: Bulk archive and bulk delete drop every failure's cause (nit)

**File:** `ui/src/lib/stores/workspace.svelte.ts:1680-1703` (`archiveSessions`, `killSessions`)

**Evidence:**
- Each failure is `catch { failed++; }`. The only report is "Couldn’t archive — 2 sessions couldn’t be archived." (or "Couldn’t delete …").
- It says neither which sessions failed nor why. A 403 (another user's session), a 409 and the daemon being unreachable all read the same, so the user can't tell whether to retry.

**Fix:**
- Collect `{title, loadErrorText(e)}` per failure.
- Name them in the toast body: one line, or the first cause plus "and N more".
- When every failure has the same cause, say it once ("Otto can’t reach the daemon.").
