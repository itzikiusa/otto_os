# Quality iteration 6, wave 0: concurrency-async lens

Reviewer: concurrency-async (Tokio/async correctness). Worktree `claude_ade-iter6`, branch
`fix/quality-iter6-20261023` @ 02a64019b (main after PR #121). This was a read-only review. No
cargo, npm, build or test command was run.

## Score

Rubric: start at 10.0. Each OPEN finding subtracts 1.0 (blocker), 0.3 (major), 0.1 (minor) or
0.03 (nit).

| Finding | Severity | Deduction |
|---|---|---|
| concurrency-async-6-01 (new) | minor | 0.10 |
| concurrency-async-6-02 (new) | minor | 0.10 |
| concurrency-async-6-03 (new) | minor | 0.10 |
| concurrency-async-6-04 (new) | minor | 0.10 |
| concurrency-async-1-05 (carried over, deliberately deferred) | nit | 0.03 |
| **Total** | | **0.43** |

**Score: 10.0 − 0.43 = 9.57 / 10.** Earlier scores: iteration 1 9.34, 2 9.64, 3 9.68, 4 9.81,
5 9.78.

All four iteration-5 findings are fixed, and none of them regressed. The lower score comes from
areas this lens had not covered before:

- vault rename
- the transcript artifact route
- swarm prompt-editor attribution, which iteration 5 added (access-control-5-01)
- product publish

All four new findings are the same kind of bug. An await that can be cancelled, or a file path
that can be swapped, sits between a durable effect and the check or record that should go with
it.

## Previous findings

| Id | Sev | Status | Evidence |
|---|---|---|---|
| 5-01: broker replay ran inside the request future | minor | **Fixed** | `crates/otto-brokers/src/http.rs:566-594`: the produce loop and the closing `audit_write` run in their own `tokio::spawn`, and the handler awaits the `JoinHandle`. `crates/otto-brokers/src/replay.rs:175-200` adds the `Interrupted` drop guard, which writes the acknowledged evidence with status `interrupted` from a spawned task. Test: `a_dropped_replay_is_marked_interrupted_with_its_acknowledgements` (`replay.rs:284`). |
| 5-02: review retry left the row `pending` when the request was dropped | nit | **Fixed** | `crates/otto-server/src/modules/review/retry.rs:249-254` arms `ClaimGuard` straight after `claim_agent_retry`, with no await in between. It is disarmed only right before the spawn (`:320`). Its `Drop` (`:724-737`) restores the settled row from a spawned task, and the diff-failure path uses `claim.restore()` (`:298`). |
| 5-03: two routes did read-modify-write on one Keychain blob | nit | **Fixed** | `McpService::lock_secrets` (`crates/otto-mcp/src/service.rs:166`) is held by both writers. The workspace route holds it at `routes/mcp_servers.rs:343-346`, from before `read_secret_blob` through `write_secret_env`'s `set_secret_meta`. The control-plane route holds it at `otto-mcp/src/http.rs:502`, re-reads the row under the lock and then writes. Test: `secret_writers_share_one_lock_per_server`. |
| 5-04: `create` ignored `shutting_down` | nit | **Fixed** | `crates/otto-sessions/src/manager.rs:2700` calls `refuse_while_shutting_down()`. `:2962-2974` calls `spawn_unless_shutting_down` (`manager/shutdown_guard.rs:40-54`), which checks before the spawn, checks again after it, and kills the new handle if shutdown began in between. The caller revokes the token and deletes the row. One gap remains: a sync-only window between the post-spawn check and `live.insert`, with no await. It is shared with resume and not reported. |
| 1-05: plugin `free_port` TOCTOU | nit | **Still open** | `crates/otto-server/src/plugins.rs:385-395` is unchanged and documented. Fixing it needs a plugin-protocol change. |

## Areas covered (this iteration)

- **Re-verification:** I re-checked all iteration-5 fixes listed above, including the
  `ClaimGuard` arm/disarm points and both secret-lock holders. I confirmed that `write_secret_env`
  now takes the blob read under the lock.
- **Request-future cancellation sweep:** iteration 5 listed this sweep as not covered. Covered:
  - vault `rename` / `create_folder` / `delete_note` (found **6-01**)
  - product `publish_version` / `publish_testcases` / `publish_as_rfc` / `publish_as_story`
    (found **6-04**)
  - Jira `create_issue_checked` (`otto-issues/src/jira/create.rs`): one POST, then a read-only
    verify. Clean.
  - goal-loop `start_loop`: it is serialised by `LoopRegistry::operation` in every route. Its only
    cancellable window after `mark_running` is a single SQLite read. Boot recovery
    (`boot/recovery.rs:214`) pauses orphaned Running loops, and `stop_loop` finalises directly
    when no controller exists. Clean.
- **Goal-loop controller restart/resume after a crash:** iteration 5 listed this as not covered.
  Covered `start/pause/stop/retry` serialisation, `require_idle`, the `admit_start` and
  `launch_admitted` re-reads, `supersede`, and `recover_goal_loops` (`fail_running` plus executor
  cleanup). Clean.
- **`otto-channels`:** iteration 5 listed this as not covered. Covered:
  - the bridge's per-conversation `ConvLocks`, the `ingress_locks` FIFO `enqueue` with an
    immediate first poll, `acquire_or_notify`, and `hold_until_done_or_idle`
  - Telegram's long-poll loop (exponential backoff capped at `BACKOFF_MAX_MS`, a cancellable held
    poll, per-token `poller_lock`, a monotonic offset) and its `dispatch_prefix` admission
    (Work / Busy / Retry keeps the suffix)

  Clean.
- **`otto-sessions` `ensure_live` versus the status task and provider-id capture:** iteration 5
  listed this as not covered. Covered the resume lock, re-checking `is_live` under it, adopting a
  deferred holder, the capture loop taking the resume lock and then the generation check and the
  cwd lock, and `spawn_blocking` for rollout scans. Clean.
- **`otto-sessions/src/ws.rs`:** this was the largest diff since iteration 5, refactored into
  `TerminalConn`. Covered `SHARED_REAUTH` subscription versus `unregister_reauth`
  (`remove_if` under the shard lock with the `receiver_count` check), the reauth loop's exit on
  `can_tx.closed()`, and the `InputQueue` budget semaphore. Clean.
- **`otto_core::keyed_lock::KeyedLocks`:** it replaced several hand-rolled registries. Covered
  pruning, poison tolerance, and that a waiter keeps the entry alive. Checked users in
  assistant tasks, channels, git, product, design, workflow progress and api secrets. Clean.
- **Scheduled-task and personal-agent report-write failure paths, and the settle-cursor retry
  loop** (both new): clean.
- **`otto-browser/src/lightpanda.rs` and `otto-telemetry-collector` log rotators** (new
  copy-truncate tasks): aborted on shutdown and on `Drop`, with I/O on the blocking pool. Clean.
- **`otto-k8s/src/monitor/scrape.rs` `read_body_capped`:** bounded, under the per-probe timeout.
  Clean.
- **Workflow `review_run` `await_review`:** subscribed before the first read, the cancel check is
  in the loop, and the timeout path cancels the review. Run cancel is handled by
  `workflow_engine.rs:3117-3150`. Clean.
- **Game rooms** (`game_rooms/registry.rs`, `socket.rs`, `http.rs` `maintain`): the registry lock
  is taken before the room lock and no reversed order exists. The broadcast lag disconnects the
  client, sends time out after 2 s, and the maintain task ends once the room closes (rooms expire
  after 3600 s). Clean.
- **Rooms `registry.rs` capability refactor** (`Digest`, `TokenBucket`): no lock changes. Clean.
- **Swarm** `update_task`, goal create/update, `acting_user`, `reusable_for` and
  `operation_guard` users (found **6-03**).
- **`otto-sessions/src/mcp.rs` codex server keys and the `info/exclude` append:** concurrent
  spawns can at worst append a duplicate ignore line, in O_APPEND mode, which is harmless. Not
  reported.
- **UI** (iteration 5 listed these as not covered):
  - the split database stores (`lib/stores/database/{history,savedQueries,dashboards,assist}`):
    every load has a request token, a scope check and an `accessEpoch` check.
  - Mission Control `reload`: a `reqSeq` and workspace stale guard, plus a `liveDebounce` with a
    maximum wait that is cancelled `onDestroy`.

  Clean.

## Areas NOT covered yet (next iteration)

- The rest of the request-future cancellation sweep:
  - `otto-design` writes and assist
  - `otto-canvas` scene writes
  - `routes/api_client/automations.rs` (new)
  - `otto-git/src/collections.rs` (new)
  - workflow `git_pr` node PR open versus run cancel
- `otto-server/src/budget/mod.rs` (reworked) under concurrent session spawns: is the budget
  check-then-spawn atomic?
- The `otto-product` story watcher and refinement runners after the recent `service.rs` changes.
- `otto-assistant` delegation (`pick_directable`) versus concurrent agent edits.
- UI: `product.svelte.ts` and `apiClient.svelte.ts` stale-response ordering, and the new kart and
  shooter `net.ts` reconnect paths.

## Findings

### concurrency-async-6-01: minor. Vault rename rewrites backlinks inside the request future, so a dropped request leaves the file moved but backlinks half-rewritten, with no rescan and no report

- **Where:**
  - The route awaits the engine inline: `crates/otto-vault/src/http.rs:276-283`.
  - The engine: `crates/otto-vault/src/engine.rs:1607` (`rename`). `:1738` moves the file
    (`renameat NOREPLACE`). `:1743-1762` rewrites each affected source in a sequential loop:
    `read_to_string`, `prepare_revision`, `atomic_replace_at` (which does `sync_all` at `:1245`),
    then `commit_revision`. `:1768` is the final `rescan_after_mutation`.
  - The MCP path: `crates/otto-mcp/src/outward/exec.rs:1320-1327` (`vault_rename` is a self-call
    POST to the same route), with the bridge's 20 s budget at
    `crates/otto-core/src/mcp_budgets.rs:15`.
- **Evidence:**
  - The code's own invariant at `engine.rs:1648-1650` says: "once the file has moved, nothing below
    may return early (the index would keep the old path while the disk has the new one)". Dropping
    the future is exactly that early return.
  - Every per-source rewrite awaits file I/O and an fsync, so each one is a cancellation point.
    hyper drops the handler future when the client closes the connection. This is the premise the
    5-01 fix accepted.
  - A folder rename with hundreds of linking notes takes one fsynced atomic replace plus a
    revision commit per source. That is several seconds, so an MCP rename can outlive the
    bridge's 20 s `CALL_TIMEOUT`, and the bridge then gives up on the request.
- **Failure scenario:**
  1. An agent calls `otto_vault_rename` to move `docs/api/` to `reference/api/` in a large vault.
     300 notes link into it.
  2. The bridge times out at 20 s, after 180 sources have been rewritten.
  3. The folder has moved, the remaining 120 notes still link to `docs/api/...`, and nothing
     reports them.
  4. `links_failed` is never returned and the agent sees a timeout error. It may retry, and the
     retry then fails with NotFound for `docs/api`.
  5. The index keeps the old paths until some later scan kicks in.

  A user who closes or reloads the window mid-rename gets the same result.
- **Fix:** follow the `state_archive` / broker-replay convention.
  - Run everything from the move onwards (the link-rewrite loop and `rescan_after_mutation`) in a
    `tokio::spawn`ed task that owns the vault write-lock guard and the publication guard. The
    handler awaits its `JoinHandle`.
  - Alternatively, spawn the whole `rename` body and await it, so a dropped request still
    finishes the rewrite and the rescan.
  - Add a test that drops the `rename` future after the move and asserts that every backlink was
    rewritten and the index shows the new path.

### concurrency-async-6-02: minor. `get_artifact` checks the canonical path, then stats and reads by path, so a symlink, FIFO or growing file swapped in afterwards bypasses the confinement and the 25 MB cap

- **Where:** `crates/otto-server/src/routes/transcript.rs:683-705`. The sequence is:
  1. `target.is_file()`
  2. `target.canonicalize()`
  3. `artifact_path_allowed(&canonical, ...)` (`:640-659`), a deny-list and cwd/data-dir check
  4. `tokio::fs::metadata(&canonical)`, the `ARTIFACT_CAP` check
  5. `tokio::fs::read(&canonical)`
- **Evidence:**
  - The artifact path comes from the session's folded transcript, so the agent chose it, and it
    sits under the agent's own writable cwd.
  - Between the deny-list check and the read there is an await (`metadata`). Nothing stops
    the final component from being replaced in that window:
    - with a symlink, which `tokio::fs::read` follows
    - with a FIFO, which blocks the blocking-pool thread in `open` until a writer appears,
      possibly forever
    - by a file that grows after the size check, which `read` then loads in full with no cap
  - The module doc (`:11-13`) claims the `routes/fs.rs` discipline. That discipline's reader,
    `routes::fs::read_at_most` (`fs.rs:497`), exists for exactly this swap (S8-09). It opens with
    `O_NOFOLLOW|O_NONBLOCK`, checks `is_file()` on the handle, and reads with `take(limit)`.
    `session_files` and `workflow_run_files` (`workflow_run_files.rs:263`) use it. `get_artifact`
    does not.
  - Separately, `is_file()` and `canonicalize()` at `:683/:687` are blocking syscalls on a Tokio
    worker. The same file's `history_path_gate` (`:100-101`) offloads `canonicalize` for this
    reason.
- **Failure scenario:**
  1. A prompt-injected agent in a Seatbelt-sandboxed session writes `out/report.html` and
     announces it as an artifact.
  2. It then loops renaming `out/report.html` back and forth with a symlink to a file the
     daemon's deny-list and the sandbox refuse to it (for example the data dir's
     `secrets.json`).
  3. When the user opens the artifact, a request that lands in the window serves the denied file
     to the viewer. Swapping in a FIFO instead parks a blocking-pool thread for good.
- **Fix:**
  - Replace the `metadata` + `tokio::fs::read` pair with
    `crate::offload::blocking(move || routes::fs::read_at_most(&canonical, ARTIFACT_CAP as usize + 1))`.
    Map a result longer than the cap to `PayloadTooLarge`.
  - Run `is_file`/`canonicalize` inside the same blocking closure.
  - Add a test that swaps the final component to a symlink after canonicalisation and asserts a
    refusal (the `session_files` tests have the pattern).

### concurrency-async-6-03: minor. Swarm prompt-editor attribution is a separate write after the prompt or goal write, and goal edits skip `operation_guard`, so a rewritten prompt or `verify_cmd` can run as the previous editor

- **Where:**
  - `crates/otto-swarm/src/service.rs:402-422` (`update_task`): `repo.update_task(...)` commits
    title, description and labels. Only after that does `set_task_prompt_editor(id, editor)` run,
    as its own statement.
  - `crates/otto-swarm/src/runtime/engine.rs:2066-2080` (`create_task_goal`) and `:2137-2179`
    (`update_goal_h`): `create_goal` / `update_goal` commit the goal text and `verify_cmd`. Only
    after that does `note_goal_editor` (`:2186`) issue two more separate writes, `set_goal_editor`
    and `set_task_prompt_editor`.
  - Neither goal handler takes `operation_guard(swarm_id)`.
  - Readers: `run.rs:280` (`acting_user` reads `task_prompt_editor`) and `verify.rs:1008`.
- **Evidence:**
  - access-control-5-01 made the latest prompt or goal editor the identity a task's turns act as.
    That invariant needs the content and the editor to change together. They don't:
    - Between the two writes there is an await that can be cancelled. hyper drops the handler on
      client EOF.
    - The editor write can fail on its own (SQLITE_BUSY, a pool timeout). The handler then returns
      an error, but the prompt change has already committed.
  - `update_task` at least holds `operation_guard`, so the scheduler cannot start a turn between
    the two writes. The goal handlers hold nothing, so a verification turn that starts in that gap
    computes `acting_user` from the old editor.
- **Failure scenario:**
  1. An Editor changes the `verify_cmd` of a goal on a task the Owner wrote, or the description
     of the Owner's task.
  2. The request is dropped, or `set_task_prompt_editor` hits a busy database, right after the
     content commit. On the goal path, the verifier could instead pick the task up in the gap.
  3. The new prompt or command runs with `acting_user` resolving to the Owner. That is the
     escalation access-control-5-01 set out to close.
  4. In the failure case the Editor also sees an error for an edit that did land.
- **Fix:**
  - Write the editor in the same statement or transaction as the content:
    - add `prompt_updated_by: Option<&str>` to `TaskPatch`, set in the same `UPDATE swarm_tasks`
      when `prompt_changed`
    - give `create_goal` / `update_goal` an `updated_by` that sets `swarm_goals.updated_by` and,
      for a task goal, `swarm_tasks.prompt_updated_by` inside one transaction
  - Take `operation_guard(&goal.swarm_id)` in `create_task_goal`, `create_project_goal`,
    `update_goal_h` and `delete_goal_h`, as `update_task` does.
  - Add a test where the editor write fails and the content write must not stick, or check both
    columns atomically.

### concurrency-async-6-04: minor. Product publish does an outward create, then the local record or rebind, inside the request future with no per-story lock, so a dropped request or a double submit creates a duplicate Jira issue or Confluence page

- **Where:** `crates/otto-product/src/service.rs`:
  - `publish_as_story`: `:1720` `create_issue`, then `:1728` `update_story` rebinds the draft to
    the new key.
  - `publish_as_rfc`: `:1626` `create_page`, then `:1635` `add_version` and `:1661`
    `update_story` rebind.
  - `publish_testcases`: `:897` `create_page`, then `:902-903` `set_testcase_run` (page id),
    and `:922` the story's `confluence_tests_page_id`.
  - `publish_version`: `:660` `update_page`, then `:682` `add_version` with `raw_json` (the new
    page version that the next publish uses as its conflict base).

  The routes (`crates/otto-product/src/http.rs:588-606`, `:1011-1036`, `:1229-1255`) await these
  inline, and no lock is held for the story or run.
- **Evidence:**
  - Each create is a remote call lasting hundreds of milliseconds or more. If the handler future
    is dropped during that call, or between it and the local write, the remote object can exist
    while Otto never records it.
  - Two publishes of the same draft (two tabs, or a retry while the first is still in flight)
    both pass `reviewed_publication` (`:1364-1378`, a read-only snapshot compare) and both create
    the remote object.
  - The code already knows the retry consequence. `:870-874` notes that creating a second
    "Test Cases — …" page fails with Confluence's duplicate-title 400. That fallback only helps
    once `confluence_tests_page_id` was recorded, which a dropped request skips.
- **Failure scenarios:**
  - **Draft published as a Jira story:** the user reloads during publish. The issue `PROJ-812` is
    created, but the draft stays a draft. Publishing again creates `PROJ-813`, so the team sees
    two identical issues. Duplicates also appear when two tabs submit together.
  - **Test cases:** a dropped first publish leaves the page id unrecorded. Every later publish
    then fails with Confluence's duplicate-title error until someone deletes the page by hand.
  - **`publish_version`:** a dropped request leaves the stored base version stale. The next
    publish then reports a false "the Confluence page changed since Otto last synced it"
    conflict.
- **Fix:**
  - Serialise publishes per story with a `KeyedLocks<String>`, as `agent_attempt.rs` does, and
    re-read the story's binding under the lock, so a second publish of an already-rebound draft
    is refused.
  - Run the remote call and its local record in a spawned task that the handler awaits (the
    5-01 convention), so a dropped request still records what was created.
  - Add a test that drops the future after the remote create and asserts the binding is recorded.

### concurrency-async-1-05: nit. Still open (carried over)

- `crates/otto-server/src/plugins.rs:385-395` `free_port()` binds, reads the port and releases it,
  and the sidecar binds later. This is documented. Fixing it needs a plugin-protocol change: the
  plugin binds `:0` and reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`.

## Checked and dropped (not findings)

- **`review_run` returns "run canceled" without cancelling its review.** The engine's canceled
  finalize (`workflow_engine.rs:3117-3150`) cancels the run's reviews.
- **Goal loop `start_loop` dropped between `mark_running` and spawn.** The window is one SQLite
  read. `stop_loop` finalises a loop with no controller, and boot recovery pauses it.
- **Swarm `update_task` racing the scheduler.** Both hold `operation_guard(swarm_id)`. Only the
  cancellation/failure half and the unguarded goal handlers are reported, in 6-03.
- **Concurrent `info/exclude` appends** (`otto-sessions/src/mcp.rs:564`): O_APPEND, so the worst
  case is a duplicate ignore line.
- **Game room `create` holds the registry lock while awaiting room locks.** No room-lock holder
  awaits the registry, so there is no reversed lock order. This was checked in iteration 5 and is
  unchanged.
- **`SHARED_REAUTH` join-while-exiting.** `remove_if` decides under the shard lock with the
  `receiver_count` check, and a joiner keeps the pass alive.
