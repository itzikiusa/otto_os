# Quality iteration 1 — lens: concurrency-async

Reviewer: concurrency-async (Tokio/async correctness). Base: `fix/quality-loop-20261021` @ d447516c4. Read-only review: no cargo, npm or tests were run.

## Score

Rubric: start at 10.0. Subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 3 | 0.30 |
| nit | 2 | 0.06 |

**Score: 10.0 − 0.66 = 9.34 / 10**

## Previous findings

None. This is the first iteration for this lens in `quality-20261021`.

## Areas covered (read in depth)

- `otto-channels`: `mirror.rs` (attach, tailer lifecycle, typing loop, teardown) and `bridge.rs` (turn locks, `submit_to_agent` failure paths, hold-until-done).
- `otto-sessions/manager.rs`: provider-id capture task, lock order (resume lock, then capture lock), nested capture, `submit_text`, broadcast and relay, `human_submit_text_checked`, `input`, the per-handle status/exit watcher task, and deferred adoption retries.
- `otto-sessions/ws.rs`: shared re-auth pass and its registry, and the input queue task.
- `otto-sessions/grid.rs`, `otto-sessions/mcp.rs` (per-cwd lock), and `otto-pty` (`lib.rs` reader, waiter and kill threads; `input_authority.rs`).
- `otto-server`: `game_rooms` (registry, socket, maintain task, lock order registry→room), `rooms/socket.rs` (maintain, end_room, send timeouts), `rooms/recap` (capture writer, terminal task abort on drop), `ws_events.rs` main loop, `ui_bridge.rs` register/unregister/targeting, `plugins.rs` supervisor and proxy, `lsp/pool.rs` actor/reaper, and the `workflow_engine.rs` orphan sweep and owned inner attempt.
- `ottod/main.rs`: shutdown signal fan-out, drain caps, teardown budget, and runtime `shutdown_timeout`.
- `otto-mcp/client.rs`: slot checkout, cancellation, stale-session retry, and transport permits.
- `otto-automation/scheduled_tasks_scheduler.rs`: claim, then the cancel gate, then spawn.
- `otto-brokers/service.rs`: blocking librdkafka calls run on `spawn_blocking`, and the per-cluster open gates.
- `otto-usage/engine.rs`: writer loop and the RETAIN_MAX bound.
- `otto-connections/sftp_pool.rs`, `otto-dbviewer/http.rs` (mongosh probe), `otto-vault/engine.rs` (delete/rename use NOREPLACE renameat), `otto-git/local.rs` (merged-sets memo), and `otto-swarm/runtime/merge.rs`.
- Workspace-wide greps: `unbounded_channel`, cancel-unsafe reads inside `select!`, `exists()` TOCTOU candidates, spawned loops in `otto-server`, and TCP keepalive.

Most of these paths are already well hardened. Examples: generation-checked registry removal, FIFO turn locks, the cancel gate checked before spawn, send timeouts on room sockets, and NOREPLACE renames. The six findings below are what remained.

## Areas NOT covered yet (next iteration)

- `otto-pty/holder.rs` and `held.rs`, the detached holder protocol: reader and ack threads, socket half-close, and re-adoption races.
- `otto-sessions/ws.rs` beyond re-auth and the input queue: credit and back-pressure steps, the grid feed attach/detach race, and the view-only attach path.
- `otto-swarm/runtime/engine.rs` and `run.rs`: cancellation mid-merge and while a turn is in flight (`merge_back` is not wrapped in `while_run_active`). Also the `verify.rs:890` spawn lifecycle.
- `otto-browser/src/live/runtime.rs` and `process.rs` (Chromium process lifecycle), and `otto-k8s` port-forward / exec process lifecycles.
- `otto-assistant` personal agents scheduler and engine, `otto-insights` scheduler, and `otto-improve` engine.
- `otto-workflows` loop checkpoints and trigger dispatch, and the rest of `workflow_engine.rs`.
- `otto-channels`: `slack.rs` and `telegram.rs` socket and poll loops, `manager.rs` supervise and restart.
- UI async races (stale-response ordering in polled views such as `GraphView.loadRefs`), checked across modules.

## Findings

### concurrency-async-1-01 — major — A channel turn's reply is never mirrored when it attaches to a cancelled tailer that has not torn down yet

- **Where:** `crates/otto-channels/src/mirror.rs:312` (`attach` reuses any existing entry); `crates/otto-channels/src/mirror.rs:703` and `:394` (the tailer's teardown removes the entry by key, without checking whose entry it is); `crates/otto-channels/src/bridge.rs:523`, `:531` and `:543` (`mirror.cancel` on a delivery failure).
- **Evidence:**
  - `attach` does `if let Some(entry) = guard.get(&session_id) { *entry.dest… = destination; return; }`. It never checks `entry.cancel`.
  - `Mirror::cancel` only stores `true` in the entry's flag. The entry is removed only later, when the tailer wakes, sees the flag and runs `self.sessions.lock().await.remove(&session_id)`.
  - While a turn is live the tailer sleeps up to `STATUS_TICK` (3.5 s), or `IDLE_LIVENESS` (30 s) between turns. In the `wait_for_artifact` failure path it first awaits a network `send_notice`.
- **Failure scenario:**
  1. Turn N fails in `submit_to_agent` at step 1, 2 or 3 (not live, paste failed or Enter failed). That calls `mirror.cancel` and returns, which releases the session's turn lock.
  2. Turn N+1 is already queued on that FIFO lock, so it proceeds at once. Its `attach` finds the cancelled entry, updates `dest`, and returns without spawning a tailer.
  3. Its `begin_turn` then flips `typing_active`. That wakes the old tailer, which sees `cancel` and breaks, then removes the entry.
  4. N+1's prompt is delivered to the agent, but nothing tails its transcript. `wait_turn_done` returns at once because the session is untracked, so the reply never reaches Slack, Telegram or the webhook, and no notice is sent.
- The same happens for any message that arrives within the 3.5–30 s window after a cancel.
- **Fix:**
  - In `attach`, treat an entry whose `cancel` is set as absent: replace it with a fresh entry and spawn a new tailer.
  - Make the teardown remove only its own entry, for example `remove_if` keyed on `Arc::ptr_eq(&entry.cancel, &cancel)`, at both `:394` and `:703`. A newer tailer's entry must never be dropped.
  - Add a regression test: cancel, then attach before the tailer wakes, then assert a tailer is running for the second attach.

### concurrency-async-1-02 — minor — Concurrent paste+Enter submissions into one session interleave and merge into one prompt

- **Where:** `crates/otto-sessions/src/manager.rs:2174-2185` (`submit_text`: paste, `sleep(200ms)`, then `\r`); `crates/otto-sessions/src/manager.rs:~4420-4455` (`human_submit_text_checked`, same shape); callers at `crates/otto-server/src/modules.rs:95` (`submit_session_text`, used by agent→agent `/message` and broadcast), `crates/otto-server/src/agent_tasks_nudge.rs:312`, and `manager.rs` `broadcast_message` and `relay`.
- **Evidence:**
  - Each submit is two separate `input`/`write_pty` calls about 200 ms apart.
  - The `room_authority` permit is dropped right after the grant, before the paste. No per-session lock spans the pair.
  - The channel bridge already serializes its own turns for exactly this reason. Its comment reads "two quick messages used to merge into one prompt". The REST, nudge and relay paths share no such lock with the bridge or with each other.
- **Failure scenario:**
  1. Two worker agents report to the same coordinator session within 200 ms. This is the agents-messaging-each-other flow fixed in #110. A nudge colliding with a human message triggers the same race.
  2. The PTY receives paste A, paste B, Enter, Enter.
  3. The TUI submits A+B as a single prompt with no separator between the senders, and the second Enter submits an empty line.
  4. The activity trail still records two separate messages.
- **Fix:** Add a per-session `tokio::sync::Mutex` (for example a `submit_locks: DashMap<Id, Arc<Mutex<()>>>`, evicted with the session) acquired around paste → delay → Enter in both `submit_text` and `human_submit_text_checked`. Let the bridge's `submit_to_agent` take the same lock around its paste/submit, so all submitters serialize.

### concurrency-async-1-03 — minor — `/ws/events` and terminal sockets have no liveness deadline, so half-open peers linger as UI-control targets

- **Where:** `crates/otto-server/src/ws_events.rs:239` and `:260` (sends a Ping every 30 s but never tracks Pong or any inbound activity, and `sink.send` has no timeout). No `Pong` handling exists in `crates/otto-sessions/src/ws.rs` or `ws_events.rs` (grep finds none). No TCP keepalive is set on any listener (grep `keepalive` finds none in `ottod` or `otto-server`).
- **Evidence:**
  - The room and game sockets bound every send with `timeout(2s)` and drop a member after 10–15 s without a heartbeat (`rooms/socket.rs:41-46`, `game_rooms/socket.rs`).
  - The events and terminal sockets do neither. A small frame such as a Ping or an event "succeeds" into the kernel send buffer even when the peer is gone.
- **Failure scenario:**
  1. With the opt-in network listener, a phone or remote browser loses its network without sending a FIN.
  2. The server keeps that socket until TCP retransmission gives up, which takes minutes.
  3. Meanwhile its `ui_bridge` registration stays with `focused`/`activity` intact (`ui_bridge.rs:489` is removed only on loop exit). An agent's `ui_command` can be routed to the ghost connection and wait out its budget.
  4. The re-auth beat cannot evict it while `sink.send` is blocked.
  5. On loopback, peer death delivers RST/FIN, so the impact is limited to network/remote use.
- **Fix:**
  - Record `last_inbound` on any client frame (Pong, Text, Binary) and close the socket when it exceeds about 2× the ping interval.
  - Wrap `sink.send` and `socket.send` in a timeout, as the room sockets do.
  - Optionally enable TCP keepalive on the network listener's accepted sockets.

### concurrency-async-1-04 — minor — Plugin sidecars are not supervised after startup: a crash leaves a zombie and a stale port

- **Where:** `crates/otto-server/src/plugins.rs:229` (`running.insert(slug, RunningPlugin{port, child})`), `:266` (`port_of`), and `:353` (`proxy` forwards to that port).
- **Evidence:**
  - `try_wait` is called only during the startup readiness loop (`:197` and `:219`).
  - After that nothing awaits or polls the `Child`, and there is no restart or backoff.
  - `port_of` returns the port for as long as the entry exists. The entry is removed only by `stop`, which runs on disable, remove or reinstall.
- **Failure scenario:**
  1. An enabled plugin crashes an hour after boot.
  2. Its process stays a zombie, because the owned tokio `Child` is never waited.
  3. Every `/api/v1/plugins/{slug}/*` call returns a connect error or 502 until a human disables and re-enables the plugin.
  4. The freed port is now unowned. If any local process later binds that port, the proxy forwards plugin requests, including the `x-otto-user` identity headers and request bodies, to that process.
- **Fix:** Spawn a per-plugin watcher that awaits `child.wait()`, for example by moving the `Child` into a task and keeping a `JoinHandle` or kill handle in `RunningPlugin`. On exit it removes the `running` entry (matched by a generation), logs and emits status, and optionally restarts with capped backoff. `stop` signals the watcher instead of waiting itself.

### concurrency-async-1-05 — nit — `free_port()` bind-then-release TOCTOU for plugin ports

- **Where:** `crates/otto-server/src/plugins.rs:271-274`, used at `:170`.
- **Evidence:** The function binds `127.0.0.1:0`, reads the port, then drops the listener. The plugin binds it later, after process spawn. In that gap another process can take the port, and the readiness probe (`GET /health` on that port) would then accept whatever answers.
- **Fix:** Prefer a handshake: let the plugin bind `:0` itself and report its port on stdout or in a file, or pass an inherited listening fd. Alternatively, verify plugin identity in the readiness probe, for example by echoing `OTTO_PLUGIN_TOKEN`.

### concurrency-async-1-06 — nit — mongosh version probe orphans its child on timeout

- **Where:** `crates/otto-dbviewer/src/http.rs:1231-1237`.
- **Evidence:** `tokio::time::timeout(10s, Command::new("mongosh").arg("--version").output())` is called without `kill_on_drop(true)`. When the timeout fires, the `output()` future is dropped and the `mongosh` process keeps running unsupervised. The global cache mutex is held for the full 10 s, so concurrent editor probes queue behind it.
- **Fix:** Add `.kill_on_drop(true)` to the command, matching the other tokio commands in the repo.
