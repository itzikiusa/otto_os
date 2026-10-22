# Quality iteration 4 — lens: concurrency-async

Reviewer: concurrency-async (Tokio/async correctness). Base: `fix/quality-loop-20261021` @ 6cff08270. This was a read-only review: no cargo, npm, builds or tests were run.

## Score

Rubric: start at 10.0. Subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit, counting only OPEN findings.

| Finding | Severity | Deduction |
|---|---|---|
| concurrency-async-4-01 (new) | minor | 0.10 |
| concurrency-async-4-02 (new) | nit | 0.03 |
| concurrency-async-3-04 (remaining part, still open) | nit | 0.03 |
| concurrency-async-1-05 (carried over, deliberately deferred) | nit | 0.03 |
| **Total** | | **0.19** |

**Score: 10.0 − 0.19 = 9.81 / 10** (iteration 1: 9.34, iteration 2: 9.64, iteration 3: 9.68)

## Previous findings

| Id | Sev | Status | Evidence |
|---|---|---|---|
| 3-01 `submit_prompt` without the submit lock | minor | **Fixed** | `crates/otto-review/src/session.rs:1160-1168` takes `manager.submit_lock(sid)` after `wait_for_tui_strict` and holds it through paste, echo wait, Ctrl-U re-paste, Enter and re-Enter. No caller of `submit_prompt` already holds the lock (no self-deadlock). The optional bounded human wait is deferred (FIXES P04); not counted. The fix did not reach the six other paste+Enter sites: see **4-01**. |
| 3-02 cancel during provisioning leaks the worktree | minor | **Fixed** | `run_workspace.rs:58-91`: `add_worktree_guarded` holds a per-run `KeyedLocks` guard inside the spawned `worktree add` task, so it outlives a dropped stage. `run_service.rs:350-363` awaits it (`await_provisioning`, 60 s) and then removes using `fresh`. `otto-git/src/local.rs:2139-2148` uses `--force --force` and logs real failures. There is a regression test with a slow git shim. |
| 3-03 SSH tunnel probe trusted before ssh binds | nit | **Fixed** | `otto-ssh/src/lib.rs:186-232`: readiness requires ssh's own `Local forwarding listening on 127.0.0.1 port N.` debug1 line (`LogLevel=DEBUG1`, `:384`) **and** a connect. The forward binds `127.0.0.1` explicitly (`:405`, `:430`). Losing the bind exits through `ExitOnForwardFailure`, and the readiness line never appears. |
| 3-04 `write_atomic` (F_FULLFSYNC) on a Tokio worker | nit | **Partly fixed / still open** | Every `otto-context/src/http.rs` writer now runs in `off_runtime` (`spawn_blocking`): `put_skill` `:187`, `put_skill_file` `:292`, `put_soul`, `put_context`, `set_default_soul`. Still on the worker: `otto-server/src/skill_eval.rs:2778-2783` (`backup_library_skill` recursive copy + `put_skill` → two fsyncs) in the async promote handler. FIXES P09 lists it as not done. |
| 3-05 Kafka `max_wait_ms` unclamped | nit | **Fixed** | `otto-brokers/src/service.rs:28-41` `bounded_consume_req` clamps to `MAX_CONSUME_WAIT_MS` (30 s) and is applied at `:1162` before the blocking peek. There are unit tests at `:1601-1615`. |
| 1-05 plugin `free_port` TOCTOU | nit | **Still open** | `otto-server/src/plugins.rs:385-395`: unchanged, documented, deferred (needs a plugin-protocol change). |

## Areas covered (this iteration)

- Re-verified every open finding from iterations 1–3 (table above), including the lock order of the new 3-01 hold. `submit_prompt` → `submit_lock` → `manager.input` takes the authority permit only inside `input`. `human_submit_text_checked` takes the submit lock before the authority permit. The channel bridge and `submit_run_prompt` take the submit lock first. There is no ABBA.
- **Every** paste+Enter submitter, not only the literal `\x1b[200~` grep the last pass used. This pass also searched `bracketed_paste(` callers across `otto-automation`, `otto-swarm`, `otto-server` hosts and `otto-insights`. Swarm (`run.rs:779/815`, `agent_run.rs:360/396`) goes through `submit_run_prompt` (locked). Six sites do not lock: **4-01**.
- `otto-vault`:
  - `watch.rs`: `ensure_watch` / `start_watch` / `watch_loop` / `stop_watch`, the bounded queue with overflow-then-rescan, and the burst cap.
  - `engine.rs`: `unregister`, `kick_scan` coalescing (rerun bit under the same mutex), `scan` and `rescan_after_mutation`.
  - Lock order: `write_lock(root)` → `publication` everywhere (`:359/362`, `:1353/1361`, `:1436/1440`, `:1510/1515`, `:1583/1592`, `recovery.rs:867/885`). `scan` takes `scan_lock` → `publication` and never the write lock. `delete_note` drops `publication` before `rescan_after_mutation`, so there is no self-deadlock.
  - Found **4-02**.
- `otto-canvas/src/assist.rs`: the `SceneBusy` claim/Drop (pointer-equal removal, so a later turn's slot is never freed), `request_stop`, `AbortOnDrop`, and the `PreviewCleanup` Drop (`spawn_blocking` from Drop when a runtime exists, sync fallback otherwise, kill spawned). There was no defect beyond cosmetic ordering.
- `otto-design-assist/src/lib.rs`: the turn spawn with its slot guard, `spawn_live_poll` abort on the normal path, the variants fan-out joining every handle, and the learning pass after accept (`otto-improve/src/design.rs:192+` is serialised per rule by `claim_evidence`).
- `otto-product`:
  - `watcher.rs` supervisor: a `JoinSet`-owned bounded poll set with an in-flight claim, cancelled through `CancelSignal` on handle drop.
  - `analysis.rs` `spawn_supervised` crash hook and the `story_start_lock` `KeyedLocks`.
  - `agent_attempt.rs` claim/stop/drain.
- `otto-browser`:
  - `lightpanda.rs` supervisor. Its child lock is held across `wait()`, but `shutdown()` aborts the supervisor first and that releases the guard. Drop falls back to `kill_on_drop` through the last `Arc`. The respawn readiness kill.
  - `cdp.rs` `PendingGuard` (removal on future drop) and reader/writer abort.
- `otto-usage/src/engine.rs` `reinit` / `stop_writer` / `shutdown`, plus the `ottod/src/usage_tailer.rs` startup retry loop added in iteration 3. The tailer is unjoined; on stop it persists its checkpoint while the teardown's 12 s session step runs, and a lost flush is re-read on the next start by design.
- `ottod/src/mcp_tools.rs` iteration-3 change: per-request `.timeout(budget)` replaces the client-wide timeout, and `wait_session_budget`.
- `otto-server/src/game_rooms/socket.rs`: 2 s bounded sends, heartbeat expiry, per-room tokio mutex only around `connect`/`apply`/`disconnect`, and a non-blocking broadcast.
- `otto-dbviewer/src/service.rs`:
  - The detached query task with `InFlightGuard` (dropped after parking).
  - The abort-handle publication.
  - `close_connection` cancelling natively while the pool and tunnel are alive and running phases under timeout.
  - The governed inline path.
- `otto-apiclient/src/scripts.rs`: script child with `kill_on_drop`, `env_clear`, a wall-clock timeout plus `start_kill`, a concurrency semaphore, RLIMITs and an RSS watchdog.
- `otto-git/src/watch.rs`: pending-flag/touched-set hand-off ordering (path recorded before `pending` is set; set cleared before take) and the TTL/MAX eviction.
- `otto-orchestrator/src/claude_pty.rs`: the PTY is killed on every path, including a dropped turn future (`PtyHandle` Drop kills `Local` backends).
- `otto-server/src/agent_session.rs` `run_session_turn_with`: the `SessionGuard` / `Startup` drain on cancel.
- `otto-server/src/routes/workflow_run_files.rs` (new): canonicalize-then-`O_NOFOLLOW|O_NONBLOCK` open. Intermediate-component swap is theoretically racy, but the only writer able to race it is the run's own agent, which can already copy any file it can read into the dir. That is no escalation, so it was dropped.
- UI: `SwarmSettings.svelte` `loadGoals`, the item flagged last iteration. `loadStandingGoals` has a single caller, and the only other writer (`mergeGoal` through `swarm_goal_updated`) is filtered by `detail.id === ev.swarm_id`. Not a defect.

## Areas NOT covered yet (next iteration)

- `otto-sessions/src/manager.rs` `shutdown_for_restart` versus in-flight `create`/`resume`, where a spawn races the daemon stop: is a session created after the kill sweep left orphaned?
- `otto-connections` SSH/DB connection sessions (`service.rs` beyond `credentials_lock`), and the `otto-k8s` port-forward reconnect loop.
- `otto-issues` (`jira.rs`, `atlassian.rs`, `confluence.rs`: 600+ changed lines in iteration 3): retry/backoff loops and token refresh under concurrent requests.
- `otto-swarm/src/runtime/scheduler.rs` and `lifecycle.rs` (iteration-3 changes, about 320 lines).
- `otto-server/src/lsp/mod.rs` iteration-3 change and `modules/review/retry.rs`.
- `otto-workflows` loop checkpoints versus concurrent cancel; `workflow_engine.rs` checkpoint resume.
- UI: database explorer result paging and multi-run polling for stale-response ordering; mission-control polling.

## Findings

### concurrency-async-4-01 — minor — Six more paste+Enter submitters still bypass the per-session submit lock

- **Where:**
  - `crates/otto-automation/src/scheduled_tasks_engine.rs:1057-1067`: scheduled-task run (paste, `paste_to_enter`, Enter, `dispatched` wait up to 6 s, re-Enter).
  - `crates/otto-automation/src/goal_loop.rs:2079-2093`: goal-loop executor prompt (same sequence).
  - `crates/otto-server/src/product_host.rs:181-189` and the re-paste loop `:197-250`: product runs. This one re-pastes and re-Enters up to `REPASTE_MAX_ATTEMPTS` = 3 times over `REPASTE_PHASE_BUDGET` = 25 s.
  - `crates/otto-server/src/assistant_host.rs:369-376`: personal-agent run.
  - `crates/otto-server/src/modules/ctx_impls.rs:593-610`: `InsightsCtx::submit_prompt`, the insights run's prompt, called from `otto-insights/src/lib.rs:1106`.
- **Evidence:**
  - Each site writes `manager.input(&sid, &bracketed_paste(prompt))`, sleeps `PASTE_TO_ENTER` (250 ms), writes `\r`, then waits up to `DISPATCH_WAIT` (6 s) and may write a second `\r`. All of these are raw `manager.input` calls.
  - `rg 'submit_lock\(' crates` finds only `otto-review/src/session.rs:1167`, `otto-swarm/src/runtime/engine.rs:112`, `otto-channels/src/bridge.rs:532` and `otto-sessions/src/manager.rs:2218/4371`. None of the six sites above takes it.
  - All six create ordinary `SessionKind::Agent` sessions through `manager.create` in the workspace, so `SessionManager::relay` (`manager.rs:2280-2332`, every live agent session), agent→agent `/message` (`submit_text`) and the human composer (`human_submit_text_checked`) can reach them while the unlocked sequence is mid-flight.
  - Iteration 3's sweep grepped only the literal `\x1b[200~`. These sites go through the `bracketed_paste()` helper, so they were missed. The 3-01 fix covers `otto_review::session::submit_prompt` only. This is the same defect class as 1-02, 2-02 and 3-01.
- **Failure scenario:**
  1. A scheduled task, or a product analysis run, spawns its session and pastes its prompt.
  2. Within the next 250 ms to 6 s, the user broadcasts "all: pause and commit", or a lead agent `/message`s every live session. `submit_text` takes the submit lock, which the scheduled-task path does not hold.
  3. The PTY receives paste A, paste B, Enter, Enter. The agent gets one merged prompt and one empty prompt.
  4. For product runs, a 7 s-idle re-paste can also land on top of the other sender's text. The broadcast is recorded as delivered (`record_user_message`) but runs as part of the wrong prompt.
- **Fix:** route all six through one locked helper.
  - Simplest: call `otto_review::session::submit_prompt` (already locked, with echo verification) instead of the hand-rolled sequence. For `InsightsCtx` and `assistant_host`, which already import `review_session`, this is a drop-in.
  - Alternatively, add `SessionManager::with_submit_lock(id, || async { … })` and wrap each paste → Enter → re-Enter group, including each product re-paste iteration (take the lock per group, not across the 25 s poll).
  - Add a `scripts/check.sh` guard that refuses `bracketed_paste(` outside the locked helpers, so the next submitter cannot regress silently.
  - Extend `submit_prompt_and_submit_text_do_not_interleave` to a scheduled-task-style caller.

### concurrency-async-4-02 — nit — A vault unregistered while its watcher is starting leaks the FSEvents watcher until the daemon restarts

- **Where:**
  - `crates/otto-vault/src/watch.rs:85-105` (`ensure_watch`: the spawned task reads `get_vault(id)`, then calls `start_watch`).
  - `crates/otto-vault/src/watch.rs:158-171` (`start_watch` inserts into `watches` and spawns `watch_loop`).
  - `crates/otto-vault/src/engine.rs:357-373` (`unregister` → `delete_vault` → `stop_watch(id)` at `:371`).
- **Evidence:**
  - `unregister` removes the watcher only if it is already in `watches`. It neither consults `watch_starting` nor marks the id retired.
  - The starter task's window is the `get_vault` read plus the synchronous `start_watch`, which does `std::fs::canonicalize`, `recommended_watcher` and `watcher.watch(root, Recursive)` (FSEvents stream setup). If `unregister` commits its delete and runs `stop_watch` inside that window, `stop_watch` removes nothing, and `start_watch` then inserts a live watcher for a vault that no longer exists.
  - Nothing removes it later. `ensure_watch` sees `contains_key(&id)` and returns, and vault ids are `AUTOINCREMENT` (migration `0105`), so the id is never reused or re-unregistered.
  - Every external edit under the old root then runs `kick_scan(id)` → `scan` → `get_vault` `NotFound`. The FSEvents stream, the debounce task and the `scan_locks`/`indexes` entries recreated by `index_state(id)` live until restart.
- **Failure scenario:** a user opens a vault (a read calls `ensure_fresh` → `ensure_watch`) and immediately unregisters it, for example to fix the root path. The old root keeps an FSEvents stream and a background task. Each save in that folder (it is often still in use, for example a repo) triggers a failing scan. This repeats for every such race until the daemon restarts.
- **Fix:**
  - In the `ensure_watch` task, after `start_watch` returns true, re-read `store().get_vault(id)` and `stop_watch(id)` if it is gone.
  - Or, more robustly, have `unregister` insert `id` into a `retired` set under the `watch_starting` lock, and have `start_watch` refuse to insert (and drop the watcher) when `id` is retired, checked under the `watches` lock.

### concurrency-async-3-04 — nit — still open (remaining part): skill promotion fsyncs on a Tokio worker

- `crates/otto-server/src/skill_eval.rs:2778-2783`: the async promote handler calls `backup_library_skill` (a recursive directory copy) and `ctx.context_library.put_skill` (`write_atomic` → two `F_FULLFSYNC`s on macOS) inline.
- Fix: wrap both in `tokio::task::spawn_blocking`, as `otto-context/src/http.rs` now does with `off_runtime`.

### concurrency-async-1-05 — nit — still open (carried over)

- `crates/otto-server/src/plugins.rs:385-395` `free_port()`: bind, read, release; the sidecar binds later. The race is documented. The fix needs a protocol change: either the plugin binds `:0` and reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`.

## Checked and dropped (not findings)

- **Lightpanda supervisor holding the child mutex across `wait()`.** `shutdown()` aborts the supervisor before it locks, and abort drops the guard. In Drop, a failed `try_lock` still kills the child when the last `Arc` drops (`kill_on_drop`).
- **Canvas `PreviewCleanup` removing the dir before the kill.** Both are spawned at once and the session is being killed. This is cosmetic.
- **`design-assist` `spawn_live_poll` not aborted on a panic.** It is only reachable through a panic in the turn, and it polls a missing file cheaply. Not confirmed reachable, so not reported.
- **`workflow_run_files.rs` intermediate-dir swap after canonicalize.** The racer would be the run's own agent, which can already copy the file in, so there is no privilege gain.
- **3-01's unbounded human wait behind a 30 s `submit_prompt` hold.** It was explicitly deferred, and a dropped composer request just abandons its wait (no partial write). Not counted.
- **`usage_tailer` startup retry sleeping up to 300 s without selecting on `wake`.** The task is unjoined; runtime shutdown drops it, and the loop re-checks `cancel` each round.
