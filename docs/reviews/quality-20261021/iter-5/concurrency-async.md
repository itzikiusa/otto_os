# Quality iteration 5 — lens: concurrency-async

Reviewer: concurrency-async (Tokio/async correctness). Worktree `claude_ade-iter5`, branch
`fix/quality-iter5-20261022` @ 8177c4d85. Read-only review: no cargo, npm, builds or tests run.

## Score

Rubric: start at 10.0; subtract 1.0 per blocker, 0.3 per major, 0.1 per minor, 0.03 per nit (OPEN findings only).

| Finding | Severity | Deduction |
|---|---|---|
| concurrency-async-5-01 (new) | minor | 0.10 |
| concurrency-async-5-02 (new) | nit | 0.03 |
| concurrency-async-5-03 (new) | nit | 0.03 |
| concurrency-async-5-04 (new) | nit | 0.03 |
| concurrency-async-1-05 (carried over, deliberately deferred) | nit | 0.03 |
| **Total** | | **0.22** |

**Score: 10.0 − 0.22 = 9.78 / 10** (iteration 1: 9.34, 2: 9.64, 3: 9.68, 4: 9.81). Every
iteration-4 finding except the deferred 1-05 is fixed; the dip comes from new territory covered
this pass (request-future cancellation, cross-surface secret RMW, create vs shutdown), not from
regressions.

## Previous findings

| Id | Sev | Status | Evidence |
|---|---|---|---|
| 4-01 paste+Enter submitters bypass the submit lock | minor | **Fixed** | New `crates/otto-sessions/src/submit.rs`: `SessionManager::submit_prompt_at` (`:196-224`) takes `submit_lock` after the TUI wait and holds it through paste / echo wait / Enter / re-Enter; `resubmit_prompt` (`:230-236`) locks one re-paste group. Scheduled tasks (`scheduled_tasks_engine.rs:1061`), goal loops (`goal_loop.rs:2093`), product (`product_host.rs:164/216`), assistant (`assistant_host.rs:371`), insights (`ctx_impls.rs:597`) and the channel bridge (`bridge.rs:416`) all call it. `rg 'bracketed_paste\('` outside otto-sessions finds only swarm (`run.rs:841/876`, `agent_run.rs:365/401`), which goes through the locked `submit_run_prompt` (`engine.rs:111-124`). `scripts/check.sh:240` now refuses new `bracketed_paste(` / raw `b"\r"` writes. Lock order unchanged (submit → authority). |
| 4-02 vault watcher leaked after unregister | nit | **Fixed** | `crates/otto-vault/src/watch.rs:102-108`: after `start_watch` succeeds the task re-reads the vault (`retired_while_starting`, `:318-323`) and `stop_watch`es a NotFound id. `unregister` deletes the row before its own `stop_watch`, so either the re-read sees NotFound or `unregister`'s `stop_watch` runs after the insert — no window left. |
| 3-04 skill promotion fsyncs on a Tokio worker | nit | **Fixed** | `crates/otto-server/src/skill_eval.rs:2780-2790`: `backup_library_skill` + `put_skill` run in one `spawn_blocking`. |
| 1-05 plugin `free_port` TOCTOU | nit | **Still open** | `crates/otto-server/src/plugins.rs:385-395` unchanged; documented; needs a plugin-protocol change. |

## Areas covered (this iteration)

- Re-verified 4-01 end to end: the new `otto-sessions::submit` module, every caller, the swarm
  path, the `check.sh` guard, and that no caller already holds the submit lock (no self-deadlock).
- Re-verified 4-02 against `unregister`'s delete → `stop_watch` order; 3-04; 1-05.
- `otto-sessions/src/manager.rs` shutdown paths (listed uncovered by iter 4): `shutdown_all`,
  `shutdown_ids` (bounded resume-lock waits), `shutdown_for_restart` (holder detach), the resume
  path's pre/post-spawn `shutting_down` checks, `create`, and ottod's teardown ordering
  (`main.rs:440-475`, `HTTP_DRAIN_CAP`, `background_stop.abort()`). Found **5-04**.
- `otto-sessions/src/manager/session_files.rs` (new): orphan sweep (grace + NotFound only),
  symlink-safe removal. `routes/session_files.rs` (new): canonicalize → deny check → `read_at_most`
  (`O_NOFOLLOW|O_NONBLOCK`, regular-file check on the handle), all on the blocking pool. Clean.
- Workflow schedule triggers moved to `otto-workflows/src/triggers/schedule.rs`: cancel polled
  between steps, `claim_scheduled_run` is one `BEGIN IMMEDIATE` with cursor CAS + overlap guard +
  run insert. Event-trigger validation. `checkpoint.rs` loop checkpoints vs cancel (Running +
  `!restart_safe` → explicit operator retry) and the engine's `drive`/`select!` around it.
- `otto-swarm/src/runtime/scheduler.rs` tick (listed uncovered): `launch_after_await` after every
  await, operation guard, `reserve_scheduled_run` + `release_unstarted_scheduled`; `run.rs` per-user
  session reuse (new `acting_user`), `submit_run_prompt` locking.
- `otto-brokers` replay intent/checkpoint rewrite (data-integrity-4-05) and offset-reset audit.
  Found **5-01**.
- `otto-dbviewer/src/service/multirun.rs`: detached job task, worker pull loop, cancel via
  `watch` + native cancel of a dropped run future, lock order store → job (never job → store),
  TTL expiry. Clean.
- `otto-server/src/modules/review/retry.rs` (listed uncovered): claim → spawn sequence, per-agent
  cancel registration, temp-diff sweep ordering. Found **5-02**.
- `otto-server/src/lsp/pool.rs` + `mod.rs` (listed uncovered): attach under the pool lock vs the
  idle reaper, actor shutdown (`kill_on_drop`, polite `shutdown`/`exit`, bounded waits), client
  backpressure (`try_send` + detach, bounded data channel). Clean (fork/exec under the std mutex
  is a small blocking call, not reported).
- MCP: `otto-mcp/src/profiles` + `otto-state/src/mcp_profiles.rs` (default hand-back races resolve
  to Full via `default_profile`'s fallback — benign), `ottod/src/mcp_tools.rs` single-flight
  caches, `ottod/src/mcp_server.rs` profile filter, `routes/mcp_servers.rs` Keychain PC-6 change.
  Found **5-03**.
- `otto-issues/src/atlassian.rs` (listed uncovered): one bounded rate-limit retry, reads only.
- `otto-k8s`: `pod_http.rs` gateway pool (lock not held across proxy start; loser dropped),
  reaper, port-forward fallback (`kill_on_drop` + explicit kill); kubeconfig validation.
- `otto-connections/src/sftp_pool.rs`: lease-aware eviction, ControlPersist bound.
- `otto-vault/src/engine.rs` batched scan publication (tokio `publication` lock; std `cache`
  RwLock never held across `.await`).
- Session rooms (`rooms/registry.rs`, `socket.rs`, `http.rs`, `recap/media.rs`): registry → room
  lock order everywhere; `maintain` drops the room lock before `forget`; no room-lock holder
  touches the registry. Game rooms UI (`KartGame.svelte`, `ShooterGame.svelte`: `disposed` flag,
  rAF cancel, renderer/input dispose, abort) and the classroom (`ClassroomsBox.svelte`: scene
  mount `dead` guard, context-lost remount, poller stop). Clean.

## Areas NOT covered yet (next iteration)

- Other long write loops that run inside an HTTP request future (the 5-01/5-02 class): a
  systematic sweep of handlers that claim/write state, then await, then spawn — e.g. product
  publish, Jira create-with-attachments (`otto-issues/src/jira/create.rs`), canvas/design writes,
  vault bulk rename.
- `otto-channels` bridge receive loop vs `hold_turn` cap under many threads; Telegram long-poll
  reconnect.
- `otto-sessions` status task vs `ensure_live` idle-resume under rapid attach/detach.
- `otto-automation` goal-loop controller restart/resume after daemon crash mid-iteration.
- UI: database store split (`lib/stores/database/*.svelte.ts`, ~1.7k lines moved) for
  stale-response ordering; mission-control polling.

## Findings

### concurrency-async-5-01 — minor — Broker replay runs inside the request future: a dropped request stops mid-publish, leaves the row `running` and up to 99 produced messages unrecorded

- **Where:** `crates/otto-brokers/src/http.rs:558-580` (route awaits `ctx.brokers().replay(..)`
  inline), `crates/otto-brokers/src/service.rs:1108-1166` (consume, then `replay::publish` with a
  per-message `produce_raw(..).await`), `crates/otto-brokers/src/replay.rs:65` (`on_start`),
  `:106` (checkpoint every `CHECKPOINT_EVERY` = 100 acks), final `update_replay` after the loop.
- **Evidence:** the whole produce loop is polled by the axum handler. hyper's HTTP/1 dispatcher
  closes the connection on client EOF (`half_close` off by default) and drops the service future,
  so a window reload/close or a dropped connection cancels `publish` at whatever `.await` it is on.
  Nothing marks the row afterwards: `mark_interrupted_replays` (`otto-state/src/broker_ops.rs:255`)
  runs only at boot for rows created before boot. The trailing `audit_write(.."replay"..)` in the
  route never runs. The module doc (`replay.rs:5-10`) only plans for a *daemon stop*.
  The codebase's own convention for this exact case is to detach: `state_archive.rs:676-681`
  ("Own the transaction/publication even if the initiating HTTP client disconnects") and the
  DB-explorer query task.
- **Failure scenario:** a user replays a 16 MiB DLQ selection (thousands of messages, one
  sequential produce each), then reloads the window. Kafka has received e.g. 1,250 messages; the
  evidence row holds 1,200 and still reads `running` until the next daemon restart; the
  `replay` audit entry (count/error) is never written. The operator cannot tell which 50 were
  re-produced, and the History list shows a replay "still running" that is dead.
- **Fix:** run `publish` in `tokio::spawn` (like `restore_snapshot`) and `await` the
  `JoinHandle` in the handler, moving the final `audit_write` into the task; or keep it inline but
  hold a drop guard that, when not disarmed, checkpoints `evidence` and sets the row to
  `interrupted` from a spawned task. Add a test that drops the `replay` future after N sends and
  asserts the row is not left `running`.

### concurrency-async-5-02 — nit — Review agent retry: claim → several awaits → spawn; a dropped request leaves the agent `pending` until restart

- **Where:** `crates/otto-server/src/modules/review/retry.rs:233-240` (`claim_agent_retry` flips
  the row to `pending`), then `:248` `ctx.manager.archive(&sid).await` (takes the session's resume
  lock, which an in-flight resume can hold for seconds), `:280-302` `get_diff` + blocking
  `std::fs::write`, `:316` `stage_all_review_skills(..).await` (whole-library copy), and only then
  `:320` `tokio::spawn`.
- **Evidence:** every await between the claim and the spawn is a cancellation point of the HTTP
  handler. The code already restores the settled row on the diff-failure path (`:295-302`,
  "`pending` blocks all three" — retry, stop, summarize), but not on cancellation. The only
  sweep of orphaned agents is boot recovery (`boot/recovery.rs:119-131`).
- **Failure scenario:** user clicks Retry on a reviewer whose old session is mid-resume; the
  archive waits on the resume lock; the user reloads the window. The agent row stays `pending`
  with no session: Retry answers 409/"still pending", Stop and Re-summarize are blocked, until
  the daemon restarts.
- **Fix:** move everything after the claim into the spawned task (the handler returns the
  claimed row), or hold a guard from the claim that, if dropped before the spawn, writes back
  `review.agents[index]` (the same restore the diff-failure path does).

### concurrency-async-5-03 — nit — Two routes read-modify-write the same Keychain secret blob `mcp-{id}` with no shared lock

- **Where:** `crates/otto-server/src/routes/mcp_servers.rs:283+` (`update`: `read_secret_blob`,
  then `write_secret_env` `:146-185` sets `blob["env"]`, keeps `headers`, `put_async`) and
  `crates/otto-mcp/src/http.rs:455-505` (`update_server`: `resolve_secrets` for the omitted map,
  builds `{env, headers}`, `put_async`, `set_secret_meta`).
- **Evidence:** both surfaces own one half of the same blob (`mcp_servers.rs:10-15` documents the
  shared `{"env","headers"}` convention) and each does read → merge → put across awaits with no
  per-server lock. The connections crate serialises the identical pattern with
  `credentials_lock` (`otto-connections/src/service.rs:282/330/560/648`); this pair has none.
- **Failure scenario:** the owner saves an Authorization header in the MCP Control Plane while
  (another tab) saving a new `API_KEY` env on the workspace MCP-servers page. Each read the old
  blob; the later `put` wins, silently reverting the other map. The row's
  `secret_env_keys`/`secret_header_keys` (set by each route's own `set_secret_meta`) then no
  longer match the blob, and every governed call fails with "Saved MCP credentials are
  incomplete" until the credentials are re-entered.
- **Fix:** one per-server `KeyedLocks` (keyed by server id) shared by both writers (expose it
  from `McpService`), held from the read through `set_secret_meta`; and have
  `write_secret_env` re-read under it rather than take a pre-read `blob`.

### concurrency-async-5-04 — nit — `SessionManager::create` ignores `shutting_down`; a create in flight during teardown publishes a live session after the kill sweep

- **Where:** `crates/otto-sessions/src/manager.rs:2957-2982` (`create`: `spawn_session_pty`
  then `live.insert`, no `shutting_down` check) vs the resume path `:5728-5739` (checks the flag
  before and after the spawn and kills a handle that finished after shutdown began) and
  `shutdown_for_restart` `:6078-6104` (snapshots `live` once).
- **Evidence:** ottod aborts only the supervisors before teardown (`ottod/src/main.rs:453-455`);
  tasks they already spawned (scheduled-task runs, swarm turns, review agents) and HTTP handlers
  abandoned at `HTTP_DRAIN_CAP` keep running and can call `create` while
  `shutdown_for_restart` runs. The resume comment names the consequence: "a live CLI nobody
  kills, its row later counted as a suspended live one". `create` has the same window.
- **Failure scenario:** with session persistence off, a scheduled task creates its session during
  `launchctl stop`; the spawn completes after `shutdown_for_restart` iterated `live`. The handle
  is inserted, never killed or marked exited, and its MCP token is not revoked; the CLI dies only
  by SIGHUP when the daemon exits, and the row says `running` until boot recovery.
- **Fix:** mirror the resume guard in `create`: refuse with `Conflict("the daemon is shutting
  down")` before `spawn_session_pty`, and after it kill the handle, revoke the token and delete
  the row when the flag is set.

### concurrency-async-1-05 — nit — still open (carried over)

- `crates/otto-server/src/plugins.rs:385-395` `free_port()`: bind, read, release; the sidecar binds
  later. Documented; needs a plugin-protocol change (plugin binds `:0` and reports, or `/health`
  echoes `OTTO_PLUGIN_TOKEN`).

## Checked and dropped (not findings)

- **MCP profile `delete` vs `set_default` race leaving zero defaults.** `default_profile` falls back
  to Full (`mcp_profiles.rs:115-125`), which is what deleting the default does anyway. Benign.
- **`session_files` canonicalize → read swap.** `read_at_most` opens `O_NOFOLLOW|O_NONBLOCK` and
  checks the handle is a regular file; the only racer is the session's own agent.
- **Kubeconfig `metadata` → `read_to_string` FIFO swap.** The path is root-chosen; no escalation.
- **LSP spawn under the pool's std mutex.** A short fork/exec; no await under the lock.
- **ottod `enabled_cache`/`gateway_cache` held across the fetch.** Intentional single-flight with
  per-request timeouts.
- **Rooms `create` holding the registry lock while awaiting each room lock.** No room-lock holder
  awaits the registry, so no ABBA; worst case is a bounded stall behind `end_room`'s DB write.
