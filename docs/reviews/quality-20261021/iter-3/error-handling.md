# Error handling: quality iteration 3 (2026-10-22)

Lens: resilience and error UX. This covers swallowed errors, raw exceptions shown instead of an inline Retry, missing recovery after a daemon restart or network drop, retry storms, misleading success or status messages, unhelpful error text and crash paths.

Branch `fix/quality-loop-20261021` at `3dd55d9f2`. The review was read-only (grep, sed and git only; no builds or tests).

## Score

Rubric: start at 10.0 and subtract per open finding (blocker 1.0, major 0.3, minor 0.1, nit 0.03).

- 6 minor × 0.1 = 0.60
- 3 nit × 0.03 = 0.09
- Total deductions: 0.69

**Score: 10.0 − 0.69 = 9.31 / 10** (iteration 2: 9.17; iteration 1: 8.17)

## Previous findings (iteration 2)

| ID | Status | Evidence |
|----|--------|----------|
| 2-01 Rewrite and Test cases pollers | Fixed | `RewriteTab.svelte:52-89` and `TestCasesTab.svelte:184-223` use `pollFailures`/`lastPollError = loadErrorText(e)` and `return false`, so the poller backs off. "Couldn’t check progress" is rendered at `:311-312` and `:646-647`. |
| 2-02 Raw exception text in the listed stores | Fixed as scoped | `apiClient.svelte.ts` (`loadErrorText` at `:725-758`), `git.svelte.ts:248/399/440/550`, `transcript.svelte.ts:286/311`, `k8s`/`aws` (`providerErrorText`), `vault.svelte.ts:1368`, `library.svelte.ts:16` and `PluginsSettings.svelte:54` all go through `loadErrorText`. A `raw-error` ui-guard now ratchets the rest (`ui/scripts/ui-guards.mjs:213, 865, 882`). The baselined debt still covers core surfaces; see 3-01. |
| 2-03 Scheduled-task cursor write dropped | Fixed | `settle_with_retry` (`scheduled_tasks_engine.rs:635-662`) retries 5× with backoff, then holds the claim for `SETTLE_HOLD` (60 s) before a last try. It is used at `:526` and `:582`. |
| 2-04 Dead plugin sidecar invisible | Fixed | `running`/`last_errors` maps (`plugins.rs:139-161, 285-307, 366`). Settings shows "Enabled · not running" with the cause (`PluginsSettings.svelte:60, 202-209`). |
| 2-05 Failed tool probe shown as "not installed" | **Half open** (now 3-06) | The mongosh half is fixed per FIXES P06. The Blender half is untouched: `DesignArena.svelte:771-773` (FIXES lists it as not done). |
| 2-06 Review diff write failure | Fixed | `materialize_review_diff` is fatal. `diff_write_failure` (`review/run.rs:182-196`) settles the review as `error`. A retry returns 409/500 (`review/retry.rs:280-294`). |
| 2-07 Design library memoises failures | Fixed | A key is dropped unless the error is 403/404 (`library.svelte.ts:87`), and `tried` is cleared on `designBus.resyncTick` (`:223-225`). |

## Areas covered this iteration

- Re-verified all 7 iteration-2 findings against the current code.
- **Goal loops** (`otto-automation/src/goal_loop.rs`): the iteration body (plan, execute, evaluate, digest, decision), `finalize`, `block`, `release_honouring_stop` and `stop_loop`; boot recovery `recover_goal_loops`; the LoopDetail stop action.
- **Swarm runtime** (`otto-swarm/src/runtime/engine.rs`):
  - The tick: budget gate (fails closed with `?`), roster and busy reads, claim, `reserve_run` with rollback to `todo`.
  - The waiting-reason registry, `budget_exceeded` and `pause_for_budget` (sound: it refuses to flip paused when the write fails).
  - `reserve_run` agent-busy atomicity (`otto-state/src/swarm.rs:1578`).
- **Session input queue** (`otto-sessions/src/ws.rs:1380-1640, 1790-1806`): the `input_failed` notice is sent once per failing stretch and reset on success. The Terminal renders it (`Terminal.svelte:190, 955`). Sound.
- **Routes**: `routes/projects.rs` (sound), `routes/share.rs` (OTP mail failure revokes the minted share and returns 502 with the cause; sound), and `routes/fs.rs` (`read_file`, the non-root deny list, `filesystem_error` mapping and timeout text).
- **Desktop panes**:
  - `apps/desktop/src-tauri/src/panes.rs` `return_pair` rollback and the `destroyed` deferral.
  - SPA side: `lib/nativePane.ts`, `nativePaneMenu.ts` and `sidePane.svelte.ts`. Failures surface as one toast and the pane stays in place. Sound.
- **Workflow run UI**: `RunSteps.svelte` (inline LoadState with Retry for step bodies and checkpoints, node error inline, retry-node toast; sound), `RunAgents.svelte`, and the WorkflowsPage final-output and context panel traced to `/fs/read` policy.
- **Events socket** (`lib/events.svelte.ts`): exponential backoff capped at 30 s, 4401 hand-off, boot-id change triggering `/meta` re-read and DB re-warm, and resync of every store after reconnect. Sound, with no retry storm.
- **Workspace store**: `refreshSessions`, `reconcileTabs`, the scratch and pinned fetches, `sessionsError`, and `refreshOtherSessions`.
- **Git UI**: `pushFlow.ts`, `pullFlow.ts`, `GitToolbar.svelte` pull mode, and the `WipPanel` / `ReviewPanel` empty catches (the WipPanel amend probes are cosmetic and benign).
- **Rooms** (excluding games): recap-capture gap reporting and the room-media cleanup catches (all benign teardown).
- **Usage tailer** (`ottod/src/usage_tailer.rs`): startup, the run loop, and the journal open path in `otto-usage/src/engine.rs`.
- **PTY holder** (`otto-pty/src/holder.rs`, `held.rs`): the discarded results are kill/cleanup on teardown paths. Sound.

## Areas NOT covered yet (next iteration)

- `otto-server/src/mcp_outward.rs` and `otto-mcp/src/outward/exec.rs` (heavy churn): the error text an agent receives for governed tool failures, and whether denials and timeouts are distinguishable.
- `otto-telemetry` and `otto-telemetry-collector` (new crates): exporter failure, buffering and backoff.
- `otto-git/src/local.rs` and `http.rs` (heavy churn): the git CLI stderr mapping to Problems (rate-limit was covered in iteration 2; local command failures were not).
- `otto-dbviewer` drivers for ClickHouse, Mongo and Redis (heavy churn): connection-loss mid-query and pool re-warm after a tunnel drop.
- The AWS module views (`LogsView`, `S3Browser`, `AthenaView`): these hold the largest raw-error baseline counts.
- `otto-sessions/src/manager.rs` respawn and auto-archive paths (`:6463` region).
- e2e coverage of error states (whether Retry is exercised anywhere).

---

## Findings

### error-handling-3-01: Core surfaces still show the raw exception ("Load failed") instead of `loadErrorText` (minor)

**Files:**
- `ui/src/lib/stores/workspace.svelte.ts:986`: `this.sessionsError = e instanceof Error ? e.message : String(e)`. It is rendered by `AgentsPage.svelte:63` as the Agents page's inline LoadState.
- `ui/src/modules/git/pushFlow.ts:65, 85`: `toasts.error('Couldn’t push', msg)`.
- `ui/src/modules/git/pullFlow.ts:44, 64`: `toasts.error('Couldn’t pull', msg)`.
- `ui/src/modules/agents/history/history.svelte.ts:164, 191`: session history list.
- `ui/src/modules/git/graph-commit-diff.svelte:187, 359`, `DiffViewer.svelte:860, 1004` and `GitPage.svelte:219`.

**Evidence:**
- Each site uses the pattern that `loadError.ts` documents as forbidden ("never the raw exception").
- With the daemon restarting, `laneFetch` rethrows WebKit's `TypeError("Load failed")`. The most-visited list in the app then reads "Couldn’t load sessions — Load failed". A push or pull during a network drop toasts "Couldn’t push — Load failed", and a 401 shows the server's raw text instead of "Your session expired. Sign in again, then retry."
- The new `raw-error` guard stops this from growing, but its baseline (`ui/scripts/ui-guards-baseline.json`, about 150 hits) includes these core paths.

**Fix:**
- Replace the sites above with `loadErrorText(e)`. For the push and pull toasts, keep `msg` for the `pushRefusal` / ff-only regex classification and pass `loadErrorText(e)` as the toast body.
- Run `node scripts/ui-guards.mjs --update-baseline` to lock in the reduction.

### error-handling-3-02: Goal-loop terminal writes are discarded, then the UI is told the loop settled, and Stop answers 200 (minor)

**File:** `crates/otto-automation/src/goal_loop.rs`
- `:1081-1108` (`finalize`)
- `:1143-1155` (`block`)
- `:277-310` (`stop_loop`)
- `:781-785, 822-833` (evaluation, ledger and digest writes)

**Evidence:**
- `finalize` reads the loop with `Err(_) => return` (no log, no emit). It then writes with `let _ = ctx.goal_loops_repo().finalize(loop_id, status, summary, error).await;` and unconditionally `emit(… status …)`.
- When the `UPDATE` fails (SQLite busy, disk full):
  - The controller has already `deregister`ed, so the row stays `running` (or `blocked`) with no controller until a daemon restart runs `recover_goal_loops`.
  - Every open view still receives `GoalLoopUpdated{status: succeeded|failed|stopped}` and shows the loop as settled. A reload then shows it running again.
  - `stop_loop` calls the same `finalize` for a loop without a controller and returns `Ok(())`. LoopDetail's `act(() => loops.stop(loopId), …)` therefore reports no error, the emitted event says "stopped", and the row is unchanged.
- `block` has the same `let _ =` on its status write.
- The per-iteration `set_ledger`, `set_iter_evaluation`, `set_context_digest` and `update_iteration_status` writes are all `let _ =` with no log. A failed ledger write loses the "needs input" question the loop then blocks on (`:766-781`), so the user sees "Blocked — needs input" with no question to answer.

**Fix:**
- Make `finalize` and `block` return `Result` and retry the status write (the `settle_with_retry` shape from the scheduled-task engine).
- Emit only the status that was actually stored. On a persistent failure, emit the stored status and a `Notice` ("Couldn’t save the goal loop’s result: <cause>").
- Have `stop_loop` propagate the error so the route returns a Problem and LoopDetail toasts it.
- At minimum, `warn!` on every discarded per-iteration write.

### error-handling-3-03: A failed open-tab or scratch fetch is read as "these sessions are gone" and permanently closes the user's tabs and panes (minor)

**File:** `ui/src/lib/stores/workspace.svelte.ts`
- `:928-939`: the `pinned` and `scratch` fetches, each `.catch(() => [] as Session[])`
- `:946`: merge
- `:984, 996-1006`: `reconcileTabs`

**Evidence:**
- `refreshSessions` loads the shown list (`archived=false&foreground=true…`) and, separately, the open tabs and panes by id (`/sessions?ids=…`, in 64-id chunks). The by-id rows are what keep tabs of archived, background (review, workflow or swarm agents) and other non-listed sessions in `sessionById`.
- When a pinned chunk or the scratch list fails but the own list succeeds (a deadline timeout on one request, a 5xx, or the daemon going down between requests), the failure becomes `[]`. `reconcileTabs()` then drops every tab not in `sessionById`, calls `persistTabs()` (localStorage), and `layout.retain(exists)` closes those split panes.
- The next successful refresh cannot bring them back, because they are no longer pinned. A transient blip silently and durably rearranges the user's workspace.
- This runs on every resync after reconnect (`events.svelte.ts:665`), which is exactly when partial failures are most likely.

**Fix:** Track whether every pinned chunk and the scratch request succeeded. When one failed, skip `reconcileTabs()` for that refresh (keep the rows that are already known), or only prune ids that the daemon positively omitted from a successful `?ids=` response.

### error-handling-3-04: A workflow run's final output is silently hidden on any error, and for non-root users the context panel always fails (minor)

**Files:**
- `ui/src/modules/workflows/WorkflowsPage.svelte:394-406` (`loadFinalOutput`)
- `ui/src/modules/workflows/WorkflowsPage.svelte:3641-3653` and `:2572-2583` (FileTree on `run.context_dir`)
- `crates/otto-server/src/routes/workflows.rs:875-885` (`context_dir = <data_dir>/workflow-context/<run_id>`)
- `crates/otto-server/src/routes/fs.rs:218-229, 430-438` (the non-root deny list includes the Otto data dir)

**Evidence:**
- `loadFinalOutput` marks the run attempted up front (`finalOutputRunId = runId`) and then `catch { /* no final-output.md (or unreadable) — the panel just stays hidden */ }`. A missing file, a timeout, an unreachable daemon and a 403 all look the same: no panel, and it never retries for that run.
- For a non-root user (policy `Require(Agents, Edit)`, `policy.rs:239`), every `/fs/read` and `/fs/browse` under `<data_dir>/…` is refused by `non_root_canonical_denied` with "…holds credentials or Otto's own state; only the root user can open it".
- So for every non-root team member:
  - A successful run's final output never appears.
  - The run's Context panel (FileTree) is offered, but always renders that 403 inline. They cannot act on it.
- No run-scoped file route exists. `/workflow-runs/{id}/…` mounts only progress, checkpoints, nodes, cancel, retry-node and approve (`routes/mod.rs:752-775`).

**Fix:**
- Serve the run's context files through a run-scoped, RBAC-checked route (for example `GET /workflow-runs/{id}/files?path=` confined to the run's context dir) and point both the final-output fetch and the FileTree at it.
- In `loadFinalOutput`, only a 404 should hide the panel silently. Any other failure should show an inline "Couldn’t load the final output: <loadErrorText>" with Retry and reset `finalOutputRunId`.

### error-handling-3-05: The usage tailer stops ingesting for the life of the daemon on a transient journal failure at boot, and nothing tells the user (minor)

**Files:**
- `crates/ottod/src/usage_tailer.rs:290-314` (startup)
- `crates/otto-usage/src/engine.rs:629-655` (`ingestion()` and `ingestion_checkpoint`)
- `crates/ottod/src/main.rs:548-549`

**Evidence:**
- At startup, the task does `Err(e) => { tracing::error!("usage tailer: journal unavailable; ingestion stopped: {e}"); return; }`. It does the same on a failed initial `persist()` ("could not commit initial checkpoint").
- `ingestion()` opens `usage-ingestion.sqlite3` through `get_or_try_init`, so a later call could succeed. But the task has returned, and nothing re-spawns it. `main.rs` logs "usage tailer: started" regardless.
- The module doc (`:18-19`) intends only corrupt journals to stop ingestion. A locked database (SQLITE_BUSY during a parallel migration or backup) or a full disk at boot gets the same permanent stop.
- Nothing in the UI reflects ingestion health (`rg -i ingestion ui/src` finds nothing related). The Usage page keeps showing stale totals until the next daemon restart, with no indication.

**Fix:**
- Distinguish a corrupt checkpoint (`restore_checkpoint` error: stop) from an unavailable journal or a failed commit. For the latter, retry with capped backoff (for example 5 s doubling to 5 min) inside the task.
- Expose the tailer state (`running` / `retrying: <cause>` / `stopped: <cause>`) on the usage status route, and render an inline warning on the Usage page ("Usage isn’t being recorded: <cause>").

### error-handling-3-06: Blender probe failure is still shown as "Blender isn’t installed" and is never re-checked (minor; carry-over of the 2-05 Blender half)

**File:** `ui/src/modules/product/design/DesignArena.svelte:220-221, 771-773` (rendered at `:1081-1090` and the else branch)

**Evidence:**
- `void product.blenderStatus().then((s) => (blender = s)).catch(() => (blender = { installed: false, path: null, version: null }));`
- `blenderChecked = true` is set before the request. A timeout, 403 or unreachable daemon therefore renders the install hint ("set OTTO_BLENDER or install it in /Applications") for a user who has Blender, and there is no Retry while the arena is open.
- FIXES P06 lists this half as not done.

**Fix:** Mirror the mongosh fix. Keep `blenderProbeError = loadErrorText(e)` and render "Couldn’t check for Blender: <cause>" with a Retry that resets `blenderChecked`. Show the install hint only when a successful probe says `installed: false`.

### error-handling-3-07: A failed swarm roster read tells the board "No available agent fits this task" (nit)

**File:** `crates/otto-swarm/src/runtime/engine.rs:606, 609, 631-636`

**Evidence:**
- `let agents = repo.list_agents(&swarm.id).await.unwrap_or_default();`
- With an empty roster, `pick_agent_from` returns `None` for every ready task, and each gets `waiting.insert(…, ("no_agent_fit", "No available agent fits this task"))`. That reason is served on `/utilization` and shown on the board.
- A transient DB error is presented as a configuration problem. (Dispatch itself stays safe: `reserve_run` re-checks agent busyness atomically, and the `busy_agents` fallback cannot double-dispatch.)

**Fix:** Propagate the error with `?`, as the rest of `tick_inner` does. The coordinator loop already logs tick errors and retries on the next tick, and the previous tick's waiting reasons stay in place.

### error-handling-3-08: RunAgents memoises every failed session lookup, not only deleted sessions (nit)

**File:** `ui/src/modules/workflows/RunAgents.svelte:107-124`

**Evidence:**
- The comment says "a 404 (deleted session) is remembered instead of re-requested forever". But `attemptedSessions.add(id)` happens before the request, and the `catch { … }` keeps it for every error.
- A daemon restart while the run view mounts leaves the sub-agent rows without title, provider or elapsed time for the rest of the mount. This is the same shape as 1-15 and 2-07, which were fixed elsewhere.

**Fix:** In the catch, `attemptedSessions.delete(id)` unless `e instanceof ApiError && e.status === 404`.

### error-handling-3-09: The Git toolbar labels Pull "merge" when the repo's pull-mode read fails (nit)

**File:** `ui/src/modules/git/GitToolbar.svelte:73-86, 130, 267-275`

**Evidence:**
- `pullMode = 'merge'` is set before the request, and `.catch(() => {})` keeps it. `pullModeFor = id` stops it from being re-read for that repo.
- On a failed read, a repo configured for rebase shows "Pull (merge)", while `runPull` (`pullFlow.ts:31-38`) sends no mode and the daemon rebases. The label states the wrong behaviour for a history-changing action.

**Fix:** Keep `pullMode` as `null` until it is known and label the button "Pull". In the catch, reset `pullModeFor = ''` so the next render retries.
