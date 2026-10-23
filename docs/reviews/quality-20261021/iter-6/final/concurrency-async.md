# Quality iteration 6, final review: concurrency-async lens

Reviewer: concurrency-async (Tokio/async correctness). Worktree `claude_ade-iter6`, branch
`fix/quality-iter6-20261023` @ 2065f00e9 (after fix waves 1 to 3). This was a read-only review: no
cargo, npm, build or test command was run. Every finding below was checked against the current
code.

## Score

Rubric: start at 10.0. Each OPEN finding subtracts 1.0 (blocker), 0.3 (major), 0.1 (minor) or
0.03 (nit).

| Finding | Severity | Deduction |
|---|---|---|
| concurrency-async-6f-01 (new; what is left of the 6-02 class) | minor | 0.10 |
| concurrency-async-1-05 (carried over, deliberately deferred) | nit | 0.03 |
| **Total** | | **0.13** |

**Score: 10.0 − 0.13 = 9.87 / 10.** Wave 0 of this iteration scored 9.57. Earlier iterations
scored 9.34, 9.64, 9.68, 9.81 and 9.78.

## Previous findings (wave-0 report)

| Id | Sev | Status | Evidence |
|---|---|---|---|
| 6-01: vault rename rewrote backlinks inside the request future | minor | **Fixed** | `crates/otto-vault/src/engine.rs:1625-1640`: `rename` now `tokio::spawn`s `rename_to_completion` and awaits the `JoinHandle`. The spawned task takes the vault write lock (`:1651`) and the index publication guard (`:1659`), so a dropped request no longer stops the move, the backlink loop or the rescan. Test: `rename_finishes_after_its_request_is_dropped` (`performance_tests.rs:1119`). |
| 6-02: `get_artifact` checked the path, then stat-ed and read it by path | minor | **Fixed (as scoped)** | `crates/otto-server/src/routes/transcript.rs:718-730`: `confine_artifact` (`is_file`, `canonicalize`, allow-list) and `read_artifact_capped` now run in one `offload::blocking` hop. The read goes through `routes::fs::read_at_most` (`O_NOFOLLOW` and `O_NONBLOCK`, regular files only, `cap + 1` bytes). The final-component symlink, FIFO and growing-file cases are closed, and `is_file`/`canonicalize` are off the runtime. A swap of a parent directory is still possible. That gap is shared by every `read_at_most` caller and is reported as **6f-01**. |
| 6-03: swarm prompt-editor stamp written separately from the content; goal handlers ran without `operation_guard` | minor | **Fixed** | `crates/otto-swarm/src/service.rs:447`: `TaskPatch.prompt_updated_by` is part of the same `UPDATE`. `crates/otto-state/src/swarm.rs:2548-2596` (`create_goal_by`) and `:2646-2717` (`update_goal`) stamp the editor inside the goal's own `BEGIN IMMEDIATE` transaction (`stamp_goal_editor`). `engine.rs` takes `operation_guard` in `create_task_goal` (`:2105`), `create_project_goal` (`:2136`), `update_goal_h` (`:2194`, which re-reads under the guard) and `delete_goal_h` (`:2241`). The verifier's own `update_goal` writes (`verify.rs:751-790`) also run under `operation_guard`, so no lost update can bring back a stale `verify_cmd`. |
| 6-04: product publish ran the outward create and the local record in the request future, with no per-story lock | minor | **Fixed** | `crates/otto-product/src/service.rs:45-60`: `PUBLISH_LOCKS` (`KeyedLocks<String>`) and `run_detached` (spawn and await). `publish_version` (`:641-655`), `publish_testcases` (`:866-886`), `publish_as_rfc` (`:1660-1676`) and `publish_as_story` (`:1770-1782`) each take the story lock inside the detached task. The `*_locked` bodies re-read the run or story, and they compare the reviewed snapshot under the lock (`publish_as_rfc_locked` / `publish_as_story_locked` → `reviewed_publication`, `:1426`), so a draft that was just rebound is refused. Tests: `publish_as_story_dropped_mid_create_still_records_the_binding` (`:3588`) and `publish_as_story_double_submit_creates_one_issue` (`:3621`). |
| 1-05: plugin `free_port` TOCTOU | nit | **Still open** | `crates/otto-server/src/plugins.rs:385-395` is unchanged and documented. Closing it needs a plugin-protocol change. |

Counts: 4 fixed, 1 still open (deferred), 0 invalid.

## Areas covered (this review)

- **Re-verification** of all five wave-0 findings, at every location they listed (table above).
  This included the goal-update lost-update path between `update_goal_h` and the verifier's
  `record` / `record_iterations`.
- **`otto-agent-run`**: new code this iteration.
  - `fanout.rs` (`run_with_quorum`, `fan_in`) and `fanout/retry.rs` (`attempts`,
    `wait_for_retry`, `run_slots`, `summarize_with_retries`):
    - the `JoinSet` per-slot ids, and panic mapping through `join_next_with_id`
    - the retry key is closed on every exit
    - the cancel predicate is checked before and after each attempt
  - Every production caller drives the fan-out from a spawned task:
    - skill review `:283`
    - product analysis `analysis.rs:236`
    - product plan
    - vault docs `run.rs`

    Dropping the `JoinSet` therefore happens only at shutdown. Clean.
- **`otto-agent-run/session_turn/lifecycle.rs`**:
  - `SESSION_LEASES` (`KeyedLocks`)
  - the `SessionGuard` arm/disarm/transfer and its `Drop` (it kills only a session the turn
    created)
  - `retire` keeps the lease through the cleanup
  - `create` / `resume` publish ownership from a spawned task through a oneshot
  - `Startup::run`: `drain`, plus the cancelled flag checked again after the lease

  Clean.
- **`otto-agent-run/assist/mod.rs`** (`run_assist_job`, the one assist engine):
  - the claim key check
  - `select!` between the capped turn and `live_poll`
  - the kill on a cap overrun
  - a stopped job applies nothing

  Stop does not interrupt the PTY. That matches the previous Canvas design, where the UI
  interrupts it (`otto-canvas` docs on origin/main), so it is not a regression. Clean.
- **`otto-design-assist/src/job.rs`** apply path: an optimistic `base` on `commit_bytes`, and a
  `Conflict` becomes a side variant plus a head restore. Clean.
- **`otto-sessions/src/submit.rs`** `RunGuard` (the async run guard for swarm submits):
  - checked during the TUI wait, and under the per-session submit lock before the paste, the
    re-paste, Enter and the re-Enter
  - `otto-swarm` `run_guard` (`engine.rs:70`) = the cancel flag plus a DB status read

  Clean.
- **`otto-sessions/src/manager.rs`** diff (transcript roots, `tail_open` from `otto-transcript`):
  scans stay on `spawn_blocking`, and the capture lock order is unchanged. Clean.
- **`otto-dbviewer/src/export.rs`** (bounded export channel):
  - `ExportWriter::pace` / `close`
  - `drain_to_file` on the blocking pool, run concurrently with the driver through `tokio::join!`
    (`driver.rs:299-303`), so there is no producer/consumer deadlock
  - the HTTP stream: a dropped body → `BrokenPipe` on the next `pace` → the driver task ends
  - the error oneshot is sent after the writer drops
  - every streaming driver calls `finish_paced`, and the buffered fallback calls `close`

  Clean.
- **`otto-server/src/budget/mod.rs`**: the admission check is spend-based (measured usage), so a
  non-atomic check-then-spawn cannot overshoot anything a spawn consumes. Clean.
  (This item was listed as not covered in wave 0.)
- **The `otto-rooms` extraction** (`authority.rs`, `terminal.rs`, the otto-server `rooms/socket.rs`
  diff): moved code. Same lock order: the room lock is held across authority calls, and no
  registry await happens under it. The maintain task still exits on `ended`. Clean.
- **`otto-mcp`**:
  - `governance.rs` / `service_stages.rs`: a pure stage pipeline with no locks.
  - `otto.query_db` (new):
    - the self-call budget is 75 s, above the 60 s engine deadline
    - the bridge proxies governed tools with `GOVERNED_WAIT_SECS + GOVERNED_RUN_BUDGET`
      (`ottod/src/mcp_tools.rs:1312-1320`)

  Clean.
- **`otto-apiclient` engine** (moved from otto-server): history prune throttle and spawn, and the
  std mutexes for the cookie jar and client cache, none of which are held across an await. Clean.
- **`otto-insights`**: `write_atomic` with unique temp names, and `write_schedule_status`
  serialised by a std mutex on the blocking pool. Clean.
- **`otto-usage` tailer driver and `history_index.rs`** (transcript roots): listing stays in one
  blocking task, with dedup by canonical path. Clean.
- **`routes/channel_webhook.rs`**: `webhook_key_guard` is an extraction only. Clean.
- **`otto-git/src/collections.rs`** (listed as not covered in wave 0): every target is validated
  before any write, and writes go through a temp file and rename. Clean in this lens.
- **Workflow `git_pr` node vs run cancel** (listed as not covered in wave 0):
  - a cancel drops the node future (`run/schedule.rs:442-448`)
  - finalisation kills the run's sessions
  - a PR already opened is not duplicated on a re-run, because the provider refuses a second open
    PR from the same head

  Not reported.
- **Product story-run reaper** (`run/reap.rs`) and the analysis/plan runners on the shared
  fan-out. Clean.
- **`otto-assistant` `delegate`** (listed as not covered in wave 0):
  - `spawn_directive_run` claims `in_flight` and checks for a running run, which blocks a
    duplicate
  - the task-row writes that follow are two SQLite writes, too small a window to report

  Clean.
- **UI**:
  - the split `apiClient` stores: `execute.svelte.ts` uses per-tab controller ownership,
    `ownsExecution` checks after every await, and a GraphQL generation counter
  - `kart/net.ts` and `shooter/net.ts` are codecs only (no reconnect logic, contrary to the
    wave-0 note)
  - `room-client.ts` is unchanged

  Clean.

## Areas NOT covered

- `otto-server/src/vault_docs_agent/*` (review loop, retry and authors after the split): I only
  scanned cancellation-flag usage and the `JoinSet` sites. The round coordinator was not traced.
- `crates/otto-server/src/workflow_engine/run/admission.rs` and `run/state.rs`: these appear to
  be moved code. I did not diff them line by line against the old `workflow_engine.rs`.
- `otto-product/src/run/*` beyond `reap.rs`, analysis and plan (`rewrite.rs`, `narrative.rs`,
  `scratch.rs`).
- The `ui/src/lib/stores/database/*` split (`queryRuns`, `connections`, `workbenchPersist`):
  wave 0 covered the earlier `database` stores, but the new split files were not re-traced.
- `ottod/src/mcp_tools/dispatch.rs`: only the budget plumbing for governed and native writes was
  checked.

## Findings

### concurrency-async-6f-01: minor. The artifact and session-file readers refuse only a final-component swap, so swapping a parent directory between the check and the open still reads outside the allow-list

- **Where:**
  - `crates/otto-server/src/routes/fs.rs:497-522` (`read_at_most`), and its copy at
    `crates/otto-server/src/routes/workflow_run_files.rs:260-283`.
  - Callers that check a canonical path and then open it by path:
    - `routes/transcript.rs:669-679` + `:725-728` (`confine_artifact` → `read_artifact_capped`)
    - `routes/session_files.rs:296-302` (`resolve` → `Restrict::check` → `read_at_most`)
    - `routes/workflow_run_files.rs:205`
    - `routes/fs.rs:452`
- **Evidence:**
  - The helper's comment states the assumption: "`path` is canonical, so its last component is
    never a symlink: refuse one that appeared since". `O_NOFOLLOW` applies only to the last
    component. `open(2)` still follows a symlink in any parent directory.
  - The open handle is then checked only with `is_file()`. Nothing re-validates where the handle
    actually points: there is no `F_GETPATH` re-check against the allow-list, and no
    `(dev, ino)` comparison with the checked path.
  - For `get_artifact` and `session_files`, the directories in the path are under the agent's own
    writable cwd. The path is agent-chosen: it comes from the transcript, or names a file the
    agent wrote.
  - The 6-02 fix moved the check and the read into one blocking closure. That narrows the window,
    but the check and the open are still separate path walks.
- **Failure scenario:**
  1. A prompt-injected agent in a sandboxed session writes `cwd/out/hosts.yml` and announces it
     as an artifact. `hosts.yml` is not a denied file name.
  2. It then loops `rename(out, out.real)`, `symlink(~/.config/gh, out)`, and back again. Creating
     a symlink does not need read access to its target, so the Seatbelt profile does not stop
     this.
  3. A `get_artifact` (or `/sessions/{id}/files/read?path=out/hosts.yml`) request whose
     `canonicalize` + allow-list check lands on the real directory, and whose `open` lands on
     the symlink, returns the daemon user's GitHub OAuth token from `~/.config/gh/hosts.yml`.
     `~/.config` is on `HOME_DENY_DIRS`, so the check would have refused that path.
  4. The non-owner callers the allow-list exists to bind are workspace viewers and the agent's
     own session or MCP credential (`transcript.rs:634-639`). A caller that is also the swapper
     can retry until the race is won.
- **Fix:**
  - After `open` in `read_at_most`, read the handle's real path with `fcntl(F_GETPATH)` on macOS
    (`/proc/self/fd` on Linux), and re-run the caller's allow/deny predicate on it. Pass the
    predicate in as a closure. Refuse on a mismatch.
  - Or: `lstat` each component of the checked canonical path, open the file, then compare the
    handle's `(st_dev, st_ino)` with a `stat` of the canonical path taken after the open, and
    refuse when they differ. This is weaker.
  - Fold the duplicate in `workflow_run_files.rs` into the shared helper.
  - Add a test that swaps a parent directory for a symlink between `confine_artifact` and the
    read, and asserts the read is refused.

### concurrency-async-1-05: nit. Still open (carried over)

- `crates/otto-server/src/plugins.rs:385-395` `free_port()` binds, reads the port and releases it,
  and the sidecar binds later. This is documented in the code. Fixing it needs a plugin-protocol
  change: the plugin binds `:0` and reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`.

## Checked and dropped (not findings)

- **`publish_as_story` / `publish_as_rfc` on a non-draft story:** a second submit creates a
  second converted issue or page. This is the explicit "convert again" action. The draft
  double-submit that 6-04 described is refused under the lock.
- **Assist Stop does not kill the agent's PTY:** this is unchanged from the old per-engine jobs,
  where the UI sends the interrupt. It is not a regression of the shared engine.
- **`RunGuard` check-then-paste:** the run can stop just after a check passes. The guard bounds
  this to one write per check, and the next check stops the submit. This is inherent and
  documented.
- **`read_collection_files` checks the type, then reads by path:** the threat model there is a
  static malicious repo, which the no-follow `file_type` check handles. No concurrent swapper is
  in scope.
- **Budget check-then-spawn:** the budget is spend-based, so atomicity is irrelevant.
