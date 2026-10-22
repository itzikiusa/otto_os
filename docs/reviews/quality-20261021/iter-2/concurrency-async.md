# Quality iteration 2 — lens: concurrency-async

Reviewer: concurrency-async (Tokio/async correctness). Base: `fix/quality-loop-20261021` @ ab0a86f05. Read-only review: no cargo, npm or tests were run.

## Score

Rubric: start at 10.0. Subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit, counting only OPEN findings.

| Finding | Severity | Deduction |
|---|---|---|
| concurrency-async-1-05 (still open, documented but unfixed) | nit | 0.03 |
| concurrency-async-2-01 | minor | 0.10 |
| concurrency-async-2-02 | minor | 0.10 |
| concurrency-async-2-03 | minor | 0.10 |
| concurrency-async-2-04 | nit | 0.03 |
| **Total** | | **0.36** |

**Score: 10.0 − 0.36 = 9.64 / 10** (iteration 1: 9.34)

## Previous findings (iteration 1)

| Id | Sev | Status | Evidence |
|---|---|---|---|
| 1-01 mirror attach after cancel | major | **Fixed** | `otto-channels/src/mirror.rs:317` skips an entry whose `cancel` is set and installs a new one; teardown goes through `remove_own` (`:717-723`, `Arc::ptr_eq` on the cancel flag) at `:400` and `:709`; regression test `attach_after_cancel_replaces_the_dying_tailer` (`:1195`). |
| 1-02 interleaved paste+Enter | minor | **Fixed for the named paths** | `SessionManager::submit_locks` (`manager.rs:1799`), taken in `submit_text` (`:2205`) and `human_submit_text_checked` (`:4551`), evicted on remove (`:5810`); bridge takes the same lock around paste→Enter and the retry Enter (`bridge.rs:532-548`, `:580`). Other direct submitters still bypass it: see new finding 2-02. |
| 1-03 events/terminal liveness | minor | **Fixed, with a remainder** | `ws_events.rs:49-73` (`SEND_TIMEOUT`, `INBOUND_DEADLINE`, `last_inbound` updated on every frame at `:329`/`:345`); `otto-sessions/src/ws.rs:55-64` `send_frame` with timeout, liveness at `:1915-1921`. The remainder (`grid.rs` `send_grid`) is new finding 2-01. |
| 1-04 plugin sidecar unsupervised | minor | **Fixed** | `plugins.rs` `adopt` moves the `Child` into a watcher task; an unexpected exit removes the entry under a generation check; `stop` signals the watcher and awaits it; a dropped sender also kills the child. |
| 1-05 `free_port` TOCTOU | nit | **Still open** | Skipped deliberately (no identity handshake in the plugin protocol). The race is now documented on `free_port` (`plugins.rs` doc comment), but the code still binds, releases and lets the child bind later. Counted as an open nit. |
| 1-06 mongosh probe orphan | nit | **Fixed** | `otto-dbviewer/src/http.rs:1232-1237` now sets `.kill_on_drop(true)`. |

## Areas covered (this iteration)

- Re-verified all six iteration-1 findings against the current code (above).
- The iteration-1 fix commit `66b55a25c`, read for new concurrency defects: `otto-core/src/cancel.rs` (the new `CancelRegistryExt`/`CancelGuard`), its `join`/`register` users in `modules.rs` (`run_review`, retry and PR review launch), `otto-mcp/src/client.rs` (the stderr tail task and the handshake), `otto-improve/src/engine.rs` (`approve_edit` / `rollback_edit` / `backup_and_replace`), `plugins.rs` (the watcher), and `ws_events.rs` / `ws.rs` (liveness).
- `otto-pty/src/holder.rs` (`serve`, `serve_client`, `client_reader`, the drain after a kick) and `otto-pty/src/held.rs` (`send` misframe hang-up, kill/release fallback thread, `request_input` single pending ack). These were not covered last time, and they are well hardened.
- `otto-swarm/src/runtime`: `engine.rs` (`start_coordinator` / `stop_coordinator`, `tick_lock`, `while_run_active`, `operation_guard`), `merge.rs` (`merge_or_back_out`, branch lock), and `verify.rs` (`start_verification` registry guard, and `merge_back` under `operation_guard`).
- `otto-channels/src/manager.rs` (the generation supervisor), `telegram.rs` (offset store, cancel mid-poll, 409 hand-off) and `slack.rs` (send timeout, watchdog).
- `otto-browser/src/live/process.rs` (Chromium start/setup failure reap, shutdown, guard admission semaphore), and `otto-k8s` process spawns (all `kill_on_drop`).
- `otto-assistant` personal-agent engine (`in_flight().claim`, spawned runs), `otto-insights` scheduler (`in_flight` set, `ACTIVE_RUNS` reservation and `Unreserve` guard).
- `otto-workflows/src/triggers.rs` event listener, and `workflow_trigger_scheduler.rs`.
- `run_engine.rs` (`RunEngine` in-flight claim, cancellation token, `advance` select).
- `otto-sessions/src/manager.rs` attach/detach and size authority (`attach`, `detach`, `claim_/revert_input_authority`), plus the `activation_gate` RwLock.
- Workspace-wide sweeps:
  - Every `tokio::process::Command::new` site checked for `kill_on_drop`, cross-checked with `timeout` wrappers.
  - Every global `tokio::sync::Mutex`/`RwLock`.
  - Every `unbounded_channel` outside tests: `dbviewer` export, `git/watch.rs`, `holder.rs`, `ui_bridge` (bounded) and the workflow engine.
  - All bracketed-paste (`\x1b[200~`) and bare-Enter submitters.
- UI async races: the `GraphView.loadRefs` / `LocalReviewPanel.loadRefs` path. It is safe, because `GitPage.svelte` `{#key activeRepo.id}` remounts `RepoView` per repo and `GraphView` guards on `id !== repoId`. `design-hall/Lobby.svelte` `loadRefs` uses an AbortController plus an identity check.

## Areas NOT covered yet (next iteration)

- `otto-sessions/src/ws.rs` deeper: the view-only/share attach path, the credit/back-pressure state machine (`CreditGate`, `apply_credit_step`) under reconnect, and `revive_viewer`.
- `workflow_engine.rs` loop checkpoints and the `unbounded_channel` progress/activity pumps (`:474`, `:2600-2608`, `:6845`, `:7200`). Check that each producer stops when its consumer exits.
- `ottod/src/mcp_tools.rs:4858` (unbounded `Value` channel) and the MCP stdio bridge lifecycle.
- `otto-connections` SSH/DB session lifecycles and `otto-ssh` tunnel children (`lib.rs` early-exit path, the `sftp.rs:1229` `kill` helper).
- `run_engine.rs` stage cancellation safety: whether dropping `stage_provision` mid-`git worktree add` leaves locks that the cancel cleanup does not remove.
- `lsp/pool.rs` writer channel (`:291`, unbounded `Vec<u8>`) against a hung language server.
- UI: polled/streamed views outside git, such as database explorer result pagination, mission-control and swarm boards, checked for stale-response ordering.

## Findings

### concurrency-async-2-01 — minor — `send_grid` is the last unbounded send in the terminal socket loop

- **Where:**
  - `crates/otto-sessions/src/grid.rs:132-148`: `socket.send(Message::Text(frame.into())).await` with no timeout.
  - Called from the main loop at `crates/otto-sessions/src/ws.rs:2106` (`Resync`) and `:2175` (`Scrollback`), and on attach at `:1705`.
- **Evidence:**
  - Iteration 1's fix routed every other terminal-socket write through `send_frame` (`ws.rs:59-64`, `timeout(SEND_TIMEOUT, socket.send(frame))`). That includes the grid-change arm at `:1901`, which builds the same frame with `grid_frame` and sends it through `send_frame`.
  - `send_grid` was left on the raw `socket.send`. FIXES.md lists it as deferred ("`send_grid` in `otto-sessions/src/grid.rs` still sends without a timeout").
  - The call sits inside the `select!` arm body, so while it is blocked no other arm runs. That includes the ping/liveness arm (`:1912-1925`), the evict arm, and the re-auth narrowing.
- **Failure scenario:**
  1. A remote viewer (phone, or a share link over the network listener) sends `scrollback` or `resync` right after attaching, while heavy agent output has filled its TCP send buffer.
  2. The viewer then loses its network without a FIN.
  3. The grid frame blocks in `socket.send` until TCP gives up, which takes minutes.
  4. During that time the liveness deadline that 1-03 added never fires, an admin terminate or a share revoke (`next_evict`) is not delivered, and the attachment keeps holding its `AttachGuard`, size-authority entry and output subscription.
- **Fix:** In `send_grid`, replace the raw send with the bounded helper. Either make `send_frame` `pub(crate)` and call `send_frame(socket, Message::Text(frame.into())).await`, or wrap the send in `tokio::time::timeout(SEND_TIMEOUT, …)` and map a timeout to `Err(())`. Then no write on this socket is unbounded.

### concurrency-async-2-02 — minor — Several paste-then-Enter submitters bypass the per-session submit lock added for 1-02

- **Where:**
  - `crates/otto-swarm/src/runtime/run.rs:779-797` and `crates/otto-swarm/src/runtime/agent_run.rs:360-381`: `send_run_input(paste)`, sleep `paste_to_enter`, then `send_run_input(b"\r")`, each a separate `manager.input`.
  - `crates/otto-assistant/src/assistant/tasks.rs:890-893` (`deliver_delegation_answer`): `manager().input(sid, paste)`, then `manager().input(sid, b"\r")` immediately, with no paste-to-Enter delay.
  - `crates/otto-server/src/routes/handover.rs:805-813` (`inject_handover_prompt`): paste, sleep 200 ms, Enter.
- **Evidence:**
  - The fix for 1-02 serialises submitters through `SessionManager::submit_lock(id)`, but only `submit_text`, `human_submit_text_checked` and the channel bridge take it (`rg submit_lock` finds no other callers).
  - The paths above write the same two-step paste/Enter sequence through raw `manager.input`, which takes only the per-write `room_authority` permit.
  - These sessions are not private to their submitter:
    - `broadcast_message(ws, text, None)` (`manager.rs`, used by broadcast and `/message`) targets every live agent session in the workspace, swarm members and delegated personal-agent sessions included.
    - The delegated personal-agent session is a live session a human can also type into.
- **Failure scenario:**
  1. A swarm turn's paste is in its `paste_to_enter` window.
  2. A workspace broadcast, or an agent→agent `/message`, runs `submit_text` on the same session. It takes the submit lock, which the swarm path never holds.
  3. The PTY receives paste A, paste B, Enter, Enter, so the member agent gets one merged prompt and then an empty one. This is the exact defect 1-02 described, on a path that fix did not reach.
  4. In `deliver_delegation_answer`, the Enter also follows the paste with no delay. `submit_text`'s own doc comment explains why that can leave the text pasted but never submitted.
- **Fix:** Route these through the shared primitive. Either call `manager.submit_text(&sid, text)` (it sanitises, holds the lock, waits and sends Enter), or take `manager.submit_lock(&sid)` around the paste → sleep → Enter sequence, and around the dispatch-retry Enter as the bridge does. For swarm, the lock can be taken inside the `guarded_dispatch` closure, or `send_run_input` can grow a `submit` variant. Add one regression test per path in the style of `concurrent_submits_do_not_interleave`.

### concurrency-async-2-03 — minor — Improvement rollback writes the file before claiming the edit, and its conflict path overwrites the status unconditionally

- **Where:** `crates/otto-improve/src/engine.rs:782-846` (`rollback_edit`).
  - The status check is a plain read (`:784`).
  - The file is restored at `:803-835` (`create_new` restore or `backup_and_replace`).
  - The claim comes only afterwards (`decide(Applied → RolledBack)`, `:839-845`).
  - Both conflict exits use `set_edit_status(…Conflict…)` (`:797-800`, `:817-820`). That is the unconditional `UPDATE … WHERE id = ?` at `crates/otto-state/src/improvements.rs:371-393`, not `set_edit_status_if`.
- **Evidence:**
  - `approve_edit` (same file, `:690-745`) was fixed to "claim the decision BEFORE touching the file, so a concurrent reject (or second approve) loses cleanly" through the `set_edit_status_if` CAS.
  - `rollback_edit` keeps the read → write → claim order and uses a blind write on its conflict exits.
  - Rollback is reachable concurrently from the UI and from the outward MCP tool `rollback_improvement_edit` (`mcp_outward.rs:2172`).
- **Failure scenario:**
  1. Two rollbacks of the same applied edit (a double click, or the UI plus an agent's MCP call) both read `status = Applied`.
  2. The first restores `before_content` and CASes to `RolledBack`.
  3. The second reads the file only after that restore, finds `current != after_content`, and runs `set_edit_status(Conflict)`. That overwrites the `RolledBack` row, so the queue now shows a conflict for an edit that was cleanly rolled back. The user is asked to resolve a conflict that does not exist, and the audit trail is wrong.
  4. The same blind write lets a rollback that races an in-flight `approve_edit` mark it `Conflict` in the gap between approve's CAS and its file write. That file then ends with the applied content under a `Conflict` row.
- **Fix:** Mirror `approve_edit`:
  - CAS `Applied → RollingBack`, or claim `Applied → RolledBack` first and revert to `Applied` on a failed write.
  - Then do the file I/O.
  - Make both conflict exits `set_edit_status_if(edit_id, Applied, Conflict, …)`, so a decision another caller already made is never overwritten.
  - Add a test: two concurrent `rollback_edit` calls end as one `RolledBack` and one "no longer applied" error.

### concurrency-async-2-04 — nit — Tool-presence probes time out without `kill_on_drop`, the same pattern fixed in 1-06

- **Where:**
  - `crates/otto-server/src/routes/meta.rs:208-211`: `timeout(2s, Command::new(program).arg("--version").output())`, where `program` is `claude`, `codex` and similar Node CLIs.
  - `crates/otto-server/src/routes/meta.rs:190-193` and `crates/otto-server/src/routes/capabilities.rs:161-164`: `timeout(2s, Command::new("which")…output())`.
- **Evidence:**
  - None of these commands sets `.kill_on_drop(true)`.
  - The neighbouring `shell_knows` (`meta.rs:240-252`) does set it, and so did the 1-06 fix for `mongosh --version`.
  - On timeout, or when the request future is dropped, the child keeps running unsupervised.
  - `probe_tools_cached` (`meta.rs:42-60`) holds a global async mutex across the probe, so a cancelled request still drops an in-flight `join_all` of these children.
- **Fix:** Add `.kill_on_drop(true)` to the three commands.

### concurrency-async-1-05 — nit — still open (carried over)

- `crates/otto-server/src/plugins.rs`, `free_port()`: bind `127.0.0.1:0`, read the port, drop the listener, and let the sidecar bind later. The race is now documented but not closed.
- Fix as before: let the plugin bind `:0` and report its port, or echo `OTTO_PLUGIN_TOKEN` in `/health` so the readiness probe can verify identity.

## Checked and dropped (not findings)

- `CancelRegistryExt::join` plus `CancelGuard` drop. A pre-registered launch guard dropping before the run would unregister the shared flag, but no caller pre-registers before `join`: `run_review` is the only `join` user, and its launchers register nothing. A cancel landing before `join` is covered by the `review_is_cancelled` DB re-check (`modules.rs:1842-1847`).
- `/db/export` unbounded body channel (`dbviewer/http.rs:780`). This is deliberate and documented: a sync sink cannot await, and the buffering is no worse than the old fully buffered path.
- `git/watch.rs` unbounded dispatcher. It is bounded per repo by the `pending` flag.
- Insights `in_flight` check-then-insert. The ticks run sequentially, so there is no concurrent admitter.
- The workflow event-trigger listener parks on `recv()` and checks its cancel flag only per event. It runs only until daemon shutdown, and runtime teardown drops it.
- `merge_back` under `operation_guard`. It is intentional, so a stop cannot interleave a half-recorded merge, and local git merges are short.
