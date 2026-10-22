# Quality iteration 3 — lens: concurrency-async

Reviewer: concurrency-async (Tokio/async correctness). Base: `fix/quality-loop-20261021` @ 3dd55d9f2. This was a read-only review: no cargo, npm or tests were run.

## Score

Rubric: start at 10.0. Subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit, counting only OPEN findings.

| Finding | Severity | Deduction |
|---|---|---|
| concurrency-async-1-05 (carried over, deliberately deferred) | nit | 0.03 |
| concurrency-async-3-01 | minor | 0.10 |
| concurrency-async-3-02 | minor | 0.10 |
| concurrency-async-3-03 | nit | 0.03 |
| concurrency-async-3-04 | nit | 0.03 |
| concurrency-async-3-05 | nit | 0.03 |
| **Total** | | **0.32** |

**Score: 10.0 − 0.32 = 9.68 / 10** (iteration 1: 9.34, iteration 2: 9.64)

## Previous findings

| Id | Sev | Status | Evidence |
|---|---|---|---|
| 2-01 `send_grid` unbounded | minor | **Fixed** | `otto-sessions/src/grid.rs` `send_grid` now wraps the send in `tokio::time::timeout(SEND_GRID_TIMEOUT, …)` (10 s, the same bound as `ws::SEND_TIMEOUT`). A timeout maps to `Err(())`. |
| 2-02 paste+Enter submitters bypass the submit lock | minor | **Fixed for the named sites** | Swarm uses `submit_run_prompt` (`otto-swarm/src/runtime/engine.rs:115-138`), which takes `manager().submit_lock` around paste, wait, Enter and the retry Enter. Lock order is submit lock, then `operation_guard`; no `operation_guard` holder takes a submit lock. The handover and delegation-answer paths now call `manager.submit_text` (`handover.rs:795-813`, `tasks.rs:888`). The shared engine helper `otto_review::session::submit_prompt` was not covered: see new finding **3-01**. |
| 2-03 improvement rollback order | minor | **Fixed** | `otto-improve/src/engine.rs:782-829` claims `Applied → RolledBack` with `decide` (a CAS) before any file I/O. Both conflict exits use `set_edit_status_if(RolledBack → Conflict)`, and a failed write reverts with `set_edit_status_if(RolledBack → Applied)`. |
| 2-04 version probes without `kill_on_drop` | nit | **Fixed** | `routes/meta.rs` (`which`, `<tool> --version`) and `routes/capabilities.rs:163` now all set `.kill_on_drop(true)`. |
| 1-05 plugin `free_port` TOCTOU | nit | **Still open** | `plugins.rs:385-395`: the race is documented, but the code still binds, releases and lets the sidecar bind later. FIXES lists it as deferred (it needs a plugin-protocol change). |

## Areas covered (this iteration)

- Re-verified all five open findings from iterations 1 and 2 (table above). Also checked the swarm lock order: `submit_lock` → `operation_guard` in `submit_run_prompt`. `channels.rs:136`, `scheduler.rs:127` and the `service.rs` guards never take a submit lock, so there is no ABBA deadlock.
- Every paste-then-Enter submitter in the workspace (`rg '\x1b\[200~'` plus `submit_prompt(` callers). This found 3-01.
- `run_engine.rs` / `run_service::cancel` / `run_workspace.rs` stage cancellation, followed down into `otto-git` `spawn_capture`. Detached `LocalWrite` tasks keep running when the stage future is dropped. This found 3-02.
- `otto-sessions/src/ws.rs` deeper (listed as not covered last time):
  - `revive_viewer`
  - `shared_reauth` / `reauth_loop_shared` / `unregister_reauth`: generation-checked `remove_if` under the shard lock; a joiner at the "unwatched" instant keeps the pass alive.
  - `off_worker` and the `SNAPSHOT_PERMITS` bound
  - `resync_frame` / `resume_frame` capture-and-subscribe hand-over
  - `CreditGate` step plumbing
  - No defects found.
- `workflow_engine.rs`:
  - The per-node `sess_tx` / `log_tx` / `activity_tx` unbounded channels are drained in the same `drive` select as the node future, so producer and consumer share a lifetime.
  - `spawn_progress_pump` is awaited at run end.
  - `workflow_node_driver::drive`
  - The cancel, skip and retry-backoff selects. On skip, the step's reviews are cancelled and its sessions killed.
- `ottod/src/mcp_tools.rs` `serve`: a single writer task, a `MAX_CONCURRENT_CALLS` semaphore for back-pressure on reads, the 6 s in-flight drain then the 2 s writer drain, and `send_notification`. No defects found.
- `otto-ssh/src/lib.rs` tunnel launch (`launch_with_retry`, `launch`, stderr drain, early-exit path) and `free_local_port`. This found 3-03.
- `otto-connections`: `sftp_pool.rs` (async-mutex scope; the session constructor is sync, with no network I/O under the lock) and `service.rs` `credentials_lock`.
- `otto-server/src/lsp/pool.rs`: the writer channel has `write_cap` plus a backlog counter (the `a_stalled_server_backpressures_client_sends` test). Not an issue.
- `otto-server/src/plugins.rs` after the iteration-2 change:
  - `adopt` publishes under the map lock before the watcher can reap.
  - The watcher is generation-checked.
  - `stop` signals the watcher and awaits it.
- `modules/review/lifecycle.rs`: `transition` is unconditional, but `ReviewsRepo::set_status` refuses to overwrite `cancelled` in SQL, so a late completion cannot undo a cancel.
- `otto-automation/src/scheduled_tasks_engine.rs`: the `spawn_run` in-flight claim moves into the task, the cancel guard is registered before the row is returned, and the `select!` with `until_cancelled` is sound.
- `rooms/socket.rs`: the per-room async mutex is held across `set_room_driver` / `room_authority_snapshot`. Those take only the in-memory authority entry lock, and `manager.input` drops that permit before the PTY write (`manager.rs:4587-4596`), so a hung child cannot stall the room. Every send is bounded at 2 s.
- `otto-brokers/src/kafka.rs` peek loop and `service.rs` `consume_with`. This found 3-05.
- `otto-context/src/library.rs` `write_atomic` (new in iteration 2). This found 3-04.
- UI: `mission-control/WorkItemDetail.svelte` (generation and ownership guard), `MissionControlPage.svelte` (`reqSeq`/workspace stale guard), the database store (`objectSearchSeq`, `objectOpenSeq`, per-tab `AbortController` in `runQuery`), and the swarm `KanbanBoard` / `RunInspector` (`AbortController`). No stale-response defect was confirmed.

## Areas NOT covered yet (next iteration)

- `otto-vault/src/engine.rs` + `watch.rs`: per-vault write locks against the scan/index publication lock order, and the watcher lifecycle (`watches`, `watch_starting`).
- `otto-canvas/src/assist.rs`: the `busy_scenes` gate and the `PreviewCleanup` Drop that spawns `spawn_blocking` from Drop during runtime shutdown.
- `otto-design-assist` and `otto-product` runners (story watcher, refinement): spawned task supervision and cancel.
- `otto-browser` Lightpanda sidecar fetch engine (reader tabs) process lifecycle.
- `otto-usage/src/engine.rs` ingestion driver under ClickHouse restart (17 recent commits).
- `game_rooms/socket.rs` (excluded UI games aside, the server half was not read).
- UI: `SwarmSettings.svelte` `loadGoals` reads the store-wide `swarm.standingGoals` after an await. Check whether any other component can load a different swarm's goals while the modal is open. Not confirmed this pass.

## Findings

### concurrency-async-3-01 — minor — The shared `submit_prompt` helper still pastes and presses Enter without the per-session submit lock

- **Where:** `crates/otto-review/src/session.rs:1153-1204` (`submit_prompt`): paste at `:1169`, `Ctrl-U` re-paste clear at `:1166`, Enter and the re-Enter at `:1200-1202`.
- **Evidence:**
  - The doc comment calls it "the one submit path every engine should use". It writes paste → wait for echo (`PASTE_ECHO_WAIT` 8 s + `PASTE_LATE_WAIT` 20 s + a 2 s settle) → optional `\x15` and re-paste → `\r` → optional second `\r`. Every write is a raw `manager.input`, and `rg submit_lock crates/otto-review` finds nothing.
  - Callers whose sessions are ordinary live agent sessions in the workspace, so `broadcast_message` / `relay` / a human composer reach them:
    - `agent_session.rs:707` (`submit_once` → workflow agent steps, non-claude Run-with-Otto)
    - `modules/session_input.rs:305` (the opening prompt of an `open_session` delegation, spawned)
    - `routes/personal_agents.rs:971`
    - `assistant_host.rs:71` (assistant threads)
    - `finding_agent.rs:369`
    - `skill_eval.rs:1025`, `skill_review.rs:935/1103`
    - `otto-insights` `lib.rs:1106`
  - Iteration 2's fix (FIXES: "all six paste-then-Enter sites now hold the session's submit lock") serialises `submit_text`, `human_submit_text_checked`, the channel bridge and `submit_run_prompt`, but not this helper. It has the widest window of all of them: up to about 30 s between paste and Enter.
- **Failure scenario:**
  1. A lead agent calls `otto_open_session` with a prompt; the worker's `submit_prompt` is spawned and waits for its paste to echo.
  2. The lead immediately `/message`s the worker, or the user broadcasts "all: stop", or a workflow agent step is running when a broadcast lands. That path runs `submit_text`, which takes the submit lock that `submit_prompt` never holds.
  3. The worker's TUI receives paste A, paste B, Enter, Enter. The worker then gets one merged prompt and one empty prompt, which is the defect 1-02 and 2-02 described.
  4. If the echo check fails because the probe is now buried under paste B, `submit_prompt` sends `\x15` (Ctrl-U). That erases the other submitter's paste, so a message the caller recorded as delivered (`record_user_message`) never reaches the agent.
- **Fix:**
  - In `submit_prompt`, after `wait_for_tui_strict`, take `let lock = manager.submit_lock(sid); let _submit = lock.lock().await;` and hold it through the paste/echo/Enter/re-Enter sequence, the same way `submit_run_prompt` does.
  - If an up-to-30 s hold is too long for a human composer queued behind it, bound the human path's wait (for example `timeout(…, lock.lock())` → 409 "agent is receiving a prompt").
  - Add a regression test in the style of `concurrent_submits_do_not_interleave`: a `submit_prompt` and a `submit_text` on one session produce two contiguous paste+Enter groups.

### concurrency-async-3-02 — minor — Cancelling a run during provisioning races the detached `git worktree add` and leaks the worktree

- **Where:**
  - `crates/otto-server/src/run_service.rs:312` (stale `run` read), `:341-357` (`cancel` → `wait_idle` → `remove_worktree(ctx, &run)`)
  - `crates/otto-server/src/run_workspace.rs:56-57` (`worktree_add_if_absent`)
  - `crates/otto-git/src/local.rs:2136-2142` (`worktree_remove`: a single `--force`) and `:1563-1567` (`LocalWrite` runs in a detached `tokio::spawn`)
- **Evidence:**
  - `advance` races each stage against the cancel token and `break`s at once (`run_engine.rs:154-157`), so `wait_idle` returns as soon as the stage future is dropped.
  - Dropping the stage does not stop `git worktree add`. `spawn_capture` runs `LocalWrite` in a detached task: "dropping the request future stops the WAIT, not the probe + git" (`local.rs:1563-1566`).
  - `cancel` then calls `remove_worktree`, which runs `git worktree remove --force <path>` (`local.rs:2140`) and ignores the result. While `git worktree add` is checking out, git holds `.git/worktrees/<id>/locked` ("initializing"), and a single `--force` refuses a locked tree. The sibling `worktree_remove_checked` (`:2216-2219`) says so: "a locked worktree needs --force --force". The doc comment on `worktree_remove` ("force-removes dirty/locked trees") is wrong.
  - `remove_worktree` is also passed the `run` read at `:312`, not `fresh` (`:352`). A cancel whose first read lands before `provision_worktree` records `worktree_path` (`run_workspace.rs:42-52`) removes nothing.
  - Nothing reaps it later. A repeat Stop on a Cancelled run calls only `stop_owned_work` (`:316-318`); there is no run delete route (`routes/runs.rs:26-32`); and nothing sweeps `otto-runs/`.
- **Failure scenario:**
  1. A user launches a Run with Otto on a large repo and presses Stop while it shows "Provisioning an isolated branch/worktree…".
  2. The status flips to Cancelled at once, and `remove_worktree` fails silently against the "initializing" lock.
  3. The detached checkout completes, leaving a full working copy under `<data_dir>/otto-runs/<id>/work`, registered in the repo's `git worktree list`, for a run whose UI says it was cleaned up. Every such cancel leaks one checkout.
- **Fix:**
  - In `cancel`, pass `&fresh` to `remove_worktree`.
  - Make `worktree_remove` pass `--force --force`, as `worktree_remove_checked` does, and log a non-"not a working tree" failure.
  - Make provisioning cancel-aware. Either run `worktree_add_if_absent` inside the stage future with a class that is killed on drop (a `SpawnClass` / budget override that is not detached, so the add dies with the stage), then `git worktree prune`; or have `cancel` wait for the detached add, for example by having `stage_provision` hold a per-run "provisioning" guard that `cancel` awaits (bounded) before removing.
  - Add a test with a slow `git` shim (`with_git_bin`): cancel during `worktree add` → no worktree is left registered or on disk.

### concurrency-async-3-03 — nit — The SSH tunnel's "TOCTOU guard" does not cover the authentication window

- **Where:** `crates/otto-ssh/src/lib.rs:203-217` (`launch`) and `:285-295` (`free_local_port`).
- **Evidence:**
  - `free_local_port` binds `127.0.0.1:0`, reads the port and drops the listener; `ssh -L <port>:…` binds it later.
  - The readiness loop treats any successful `TcpStream::connect(port)` as "forward is live". The guard then calls `child.try_wait()` once, with the comment: "Only bless the tunnel if the ssh child is still running after the probe … (ssh then exits — it uses ExitOnForwardFailure)".
  - OpenSSH sets up `-L` forwards only after authentication (`ssh_init_forwarding` runs post-login). For the whole connect, host-key and auth phase (seconds with a jump host or slow DNS), ssh is alive and has not yet tried to bind. So if another local process grabbed the released port, the probe reaches that foreign listener, `try_wait` returns `None`, and the tunnel is returned.
- **Failure scenario:** a DB connection over SSH opens its MySQL or Postgres client to `127.0.0.1:<port>`, which is now a foreign process. That process receives the handshake, and with Postgres `password` auth, the cleartext password. Later ssh exits with "bind: Address already in use", `is_alive()` flips and the next open retries, but the first session already talked to the wrong peer. This is the same class as 1-05; it needs a local racer, hence a nit.
- **Fix:** do not trust the TCP probe until ssh reports the forward. Either:
  - run ssh with `-o LogLevel=VERBOSE` and wait for its "Local forwarding listening on 127.0.0.1 port N" stderr line before probing; or
  - forward to a Unix socket in a 0700 per-tunnel directory (`-L /dir/sock:host:port`), which no other process can pre-bind.

  Then correct the comment.

### concurrency-async-3-04 — nit — The new library `write_atomic` runs `F_FULLFSYNC` on a Tokio worker

- **Where:**
  - `crates/otto-context/src/library.rs:70-106` (`write_atomic`: `f.sync_all()` at `:94`, directory `sync_all` at `:103`). It is called synchronously from `put_skill`, `put_skill_file`, `put_soul`, `put_context`, `set_default_soul` and `write_md_entry`.
  - Those are reached from async axum handlers with no `spawn_blocking`: `otto-context/src/http.rs:187` (`put_skill`), `:424`, `:474`, `:514`, and `otto-server/src/skill_eval.rs:2779-2781` (`promote_skill`, after the sync `backup_library_skill` copy).
- **Evidence:**
  - Iteration 2 replaced `fs::write` with `write_atomic`. On macOS, `File::sync_all` is `fcntl(F_FULLFSYNC)`: a full device-cache flush that commonly costs tens to hundreds of ms under I/O load. Here it runs twice per save, on the request's worker thread.
  - The workspace `clippy.toml` bans blocking calls on workers (perf/03 F5), but `otto-context` opts out with an empty list ("S9-305 RATCHET"), so nothing flags this new cost.
- **Fix:** run the library writes off the runtime. In `otto-context/src/http.rs`, wrap `put_*`/`set_default_soul` in the crate's existing `spawn_blocking` helper (`http.rs:226`), and do the same for `backup_library_skill` + `put_skill` in `promote_skill`.

### concurrency-async-3-05 — nit — Kafka peek `max_wait_ms` is client-controlled and unclamped on an uncancellable blocking thread

- **Where:**
  - `crates/otto-brokers/src/kafka.rs:1198`: `Instant::now() + Duration::from_millis(req.max_wait_ms.unwrap_or(5000))`.
  - `crates/otto-brokers/src/service.rs:1148-1160`: `limit` is clamped to `MAX_CONSUME`; `max_wait_ms` is passed through. The call runs in `spawn_blocking`.
- **Evidence:**
  - `ConsumeReq.max_wait_ms` (`types.rs:396`) is deserialized straight from the `POST /brokers/clusters/{id}/topics/{topic}/consume` body (`http.rs:423-432`).
  - The poll loop exits early only on `want`/`raw_cap`/all-partitions-EOF. When a broker goes unreachable after the watermark fetch, `poll` returns `None` until the deadline.
  - Dropping the HTTP request cannot stop a `spawn_blocking` closure. The thread, and the consumer it took from `peek_pool`, stay busy for whatever `max_wait_ms` the caller sent.
  - The UI and the MCP tool never send the field (`exec.rs:712-723`), so only a direct API caller can hit this.
- **Fix:** clamp it in `consume_with` next to `limit`, for example `max_wait_ms: req.max_wait_ms.map(|w| w.min(MAX_CONSUME_WAIT_MS))` with a 30 s cap (the largest internal caller uses 30 000).

### concurrency-async-1-05 — nit — still open (carried over)

- `crates/otto-server/src/plugins.rs:385-395`, `free_port()`: bind, read, release, then the sidecar binds later. The race is documented; the fix needs a protocol change, either the plugin binding `:0` and reporting its port, or `/health` echoing `OTTO_PLUGIN_TOKEN`.

## Checked and dropped (not findings)

- `workflow_engine.rs` per-node unbounded channels: they share a lifetime with their consumer (the `drive` select), and the senders live in the node's scope.
- `ottod` MCP `serve` unbounded writer channel: the producers are bounded by the `MAX_CONCURRENT_CALLS` semaphore (one reply per call) and by rare `tools/list_changed` notifications.
- `reauth_loop_shared` join/leave race: `remove_if` checks `receiver_count()` under the DashMap shard lock that `subscribe()` also takes.
- `review::lifecycle::transition` being unconditional: `set_status` SQL refuses to overwrite `cancelled` unless the new status is `running`.
- Room socket lock held across `manager.set_room_driver`: those take only the in-memory authority entry lock; PTY writes drop the permit first.
- `stage_provision` recording the path: `provision_worktree` persists `worktree_path` BEFORE `worktree add`. The leak in 3-02 comes from the stale read plus the detached add, not from a missing record.
