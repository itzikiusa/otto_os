# Backend correctness — quality iteration 4 (2026-10-22)

Lens: logic bugs in the Rust daemon crates (`crates/**`): wrong conditions,
off-by-one errors, broken state transitions, unhandled edge cases, wrong error
mapping, and panics that input can reach. The review was read-only. No cargo,
npm or tests were run. Every finding was confirmed by reading the code at the
cited lines.

## Score

Start at 10.0. Open findings: 2 minor (-0.1 each) and 2 nits (-0.03 each).

- backend-correctness-4-01: minor, -0.1
- backend-correctness-4-02: minor, -0.1
- backend-correctness-4-03: nit, -0.03
- backend-correctness-4-04: nit, -0.03

**10.0 - 2 x 0.1 - 2 x 0.03 = 9.74 / 10**

## Previous findings (iteration 3)

All six open findings from iteration 3 are fixed. None was invalid.

| id | status | evidence in current code |
|----|--------|--------------------------|
| 2-01 (otto-sessions half: huge `session_auto_archive_days` panics or wraps) | **fixed** | `otto-sessions/src/manager.rs:372-380` `auto_archive_cutoff` uses `i64::try_from`, `TimeDelta::try_days` and `checked_sub_signed`. `:5273-5279` treats `None` as "off" and logs a warning. |
| 3-01 (per-agent review Retry wedges a `pending` row) | **fixed** | Three changes close it. (1) `modules/review/retry.rs:283-297`: when the diff can't be rebuilt, the handler puts the settled row back with `set_agent_at(.., &review.agents[index])` after the claim. (2) `otto-state/src/reviews.rs:160-188` `fail_orphaned_agents` runs at boot (`boot/recovery.rs:124`) and settles every `pending/running/waiting` agent of a non-running review. (3) `retry.rs:601-613` `agent_stop_block` lets Stop settle an orphaned `pending` row: one with no live flag on a review that is not running. |
| 3-02 (Postgres `WITH … UPDATE/DELETE` got `LIMIT n`) | **fixed** | `otto-dbviewer/src/types.rs:1261-1267`: `UPDATE`, `DELETE`, `INSERT` and `MERGE` are in the `SKIP` list. |
| 3-03 (shared extractor gave up after a lone `{`; `parse_pr_draft` used first `{` to last `}`) | **fixed** | `otto-core/src/text.rs:81-125`: when a candidate never closes, the scan restarts at `start + 1`. `pr_draft.rs:99` uses `extract_json_where` with a title/description predicate. Iteration 4 found a remaining gap in the *other* callers (see 4-04). |
| 3-04 (multi-run placeholders: Postgres `#`, `$$`, `E''`) | **fixed** | `multirun/params.rs:38-56` adds `PlaceholderMode::Postgres`. `:140-230` `pg_code_mask` handles nested comments, `E'…'`-only backslash escapes and `$tag$` bodies (through `split::try_open_dollar`), and treats `#` and backtick as code. |
| 3-05 (vault link rewrite rewrote frontmatter fences) | **fixed** | `otto-vault/src/parse.rs:416-422` copies `content[..content.len() - body.len()]` verbatim. |

## Areas covered this iteration

- **Re-verified** the six iteration-3 findings above.
- **Code added by the iteration-3 fix commits (`e1c4a8f29`, `00943460a`), read as new code:**
  - `otto-core/src/keyed_lock.rs`: Weak-entry pruning and a poison-tolerant map. Also its users in `otto-git/src/http.rs` (`repo_lock`, `remote_lock`, `forget_repo_locks`), product, and swarm merge and workspace.
  - `otto-git/src/providers/bitbucket.rs` `merge_polling`: the 202 path, the same-origin `Location` check, the poll deadline and the FAILED reason.
  - `otto-git/src/local.rs`: `worktree_remove` (`--force --force --`) and `output_noise.rs`.
  - `otto-rbac/src/tokens.rs` `rotate_mcp_token`: DELETE … RETURNING as the guard, and an insert in the same transaction.
  - `otto-state/src/workflows.rs` `run_acting_user`: the union over the version author and trigger editors. Both timestamps use the same `fmt`.
  - `routes/workflow_run_files.rs`: `resolve`, `browse_sync`, `read_sync` and `read_at_most`.
  - `otto-state/src/memory.rs` `soft_forget` and `undo_forget_scoped` (finding 4-01), plus the `save_one` reactivation path.
  - `otto-assistant/src/assistant/tasks.rs`: attachment removal, `in_inbox`, and the orphan inbox sweep, checked against the inbox writers in `routes/assistant.rs:229`.
  - `otto-automation/src/goal_loop.rs`: `finalize`, `block` and `warn_on_err` call sites.
  - `otto-swarm/src/runtime/lifecycle.rs` (activate, pause, abort) and `scheduler.rs` (`idle_agents`, `plan_rebalance`, `take_escalation_slot`).
  - `otto-sessions/src/manager/mcp_sync.rs`: the per-provider delivered set, the codex env-file sweep, and the browser union.
  - `ottod/src/mcp_tools/budgets.rs`: `wait_session_budget` against the daemon's `min(25)`.
  - `ottod/src/usage_tailer.rs`: the startup retry loop.
  - `otto-ssh/src/lib.rs`: the readiness `StderrTap` (finding 4-02).
  - `routes/settings.rs` and `SettingsRepo::put_many`.
- **Areas iteration 3 listed as not covered:**
  - `workflow_engine/nodes/loop_node.rs` end to end, with `otto-workflows/src/checkpoint.rs` `execute_observed`: adoption, a restart on a Running checkpoint, and the retry budget.
  - The JSON candidate selection in `otto-swarm/src/runtime/engine.rs` (plan), and `verify.rs` (`Verdict::parse`, `classify`, `run_verification`), with `otto-agent-run/src/agent_run.rs` `watch_for_result` and `transcript_ok` (finding 4-04).
  - `otto-product/src/run.rs` `retry_analysis_agent_owned` (including the `set_agent_status` merge semantics), and the JSON callers in `swarm.rs` and `refine.rs`.
  - `otto-dbviewer`:
    - the MySQL `run_script` loop with `StatementScanner`;
    - the `DELIMITER` directive (quoted values, `has_more`);
    - the ClickHouse single, batch and export entry points (`drivers/clickhouse.rs:1020-1075`, `:2855-2905`, `:3050-3085`) (finding 4-03).
  - `otto-pty` holder protocol: frame length cap, `serve_client`, `client_reader`, the superseded drain, the exit barrier and `resize` validation.
  - `otto-channels`:
    - `bridge.rs` keyed locks and queue order;
    - `mirror.rs` `render_feed`, `ReplyDelivery` and the directive extractors.
  - `otto-core/src/cadence.rs`:
    - `is_due_since`, `effective_cursor`, `served_occurrence` and `latest_cron_fire`;
    - the next-daily and next-weekly logic;
    - `next_run`, `validate` and DST `resolve_local`.
  - `otto-mcp/src/client.rs`: `list_all_tools` paging limits and `op_checked` stale-session retry.
  - `policy.rs` head (exempt list, rooms/game-rooms rules), checked against the registered `/game-rooms` route (POST only).
- **Sweeps:**
  - `len() - 1` slicing and indexing in recently changed non-test code. Every site is guarded.
  - `.unwrap()` / `.expect()` in recently changed non-test code. All remaining sites are mutex-poison unwraps or provably present handles.

## Areas NOT covered yet (next iteration)

- `otto-server/src/workflow_engine.rs` `resume_queued_runs` and run admission after a crash. Only the loop node and checkpoint primitive were traced this time.
- `otto-product/src/run.rs` `run_analysis` fan-out and summarizer, `run_generate_plan`, and `chat.rs` action parsing.
- `otto-dbviewer` MongoDB driver: the mongosh script path and aggregation paging.
- `otto-server/src/policy.rs` past the exempt block (about 3,000 lines), checked for logic. Also `mcp_outward.rs` route mapping.
- `otto-usage/src/engine.rs`: budget evaluation, the totals memo and the day rollover (`totals_window_day`). Only the SQL window was read.
- `otto-channels/src/slack.rs` (14 changes in 3 weeks) and the Telegram update loop.
- `otto-sessions/src/ws.rs` and `http.rs`: changes since iteration 2.
- `otto-server/src/state_archive/*` export and import, `otto-aws/src/s3.rs`, and `otto-k8s` actions.

## Findings

### backend-correctness-4-01 — minor — undoing a memory forget re-activates a memory that was already inactive before the forget

- **Where:**
  - `crates/otto-state/src/memory.rs:951-972` (`soft_forget`);
  - `crates/otto-state/src/memory.rs:1009-1022` (`undo_forget_scoped`);
  - reachable through `POST /workspaces/{ws}/memory/{mid}/forget` and `/forget/undo` (`crates/otto-server/src/memory_gov.rs:56-89`).
- **Evidence:**
  - `soft_forget` saves only the lifecycle `state` (`prev_state = CASE WHEN forgotten_at IS NULL THEN state ELSE prev_state END`). It does not save `active`.
  - The undo then works `active` out from the state: `active = (COALESCE(prev_state, 'accepted') <> 'contradicted')`.
  - But `active` is independent of `state` everywhere else:
    - `DELETE /workspaces/{ws}/memories/{id}` (`otto-memory/src/http.rs:229-240`) calls `MemoriesRepo::forget` (`memory.rs:717-727`), which sets `active=0` and leaves `state` as it was;
    - `PATCH` with `{"active": false}` does the same (`memory.rs:687`, `MemoryPatch.active`);
    - `set_state` (`:923-934`) never touches `active`.
  - Such a row stays listable (`include_inactive`) and visible to `get_visible`. So the governance Forget route can soft-forget it.
  - Sequence: the memory is `accepted` and deactivated (`active=0`). Forget saves `prev_state='accepted'`. Undo then sets `active=1`. A memory the user had removed from recall comes back into recall.
  - The regression test `undo_restores_the_pre_forget_state` (`memory.rs:1690-1712`) only seeds the pairs `(suggested,true)`, `(contradicted,false)` and `(accepted,true)`, so `(accepted,false)` is not covered.
- **Fix:**
  - Remember `active` too. Add a nullable `prev_active INTEGER` (append-only migration, rollback-safe).
  - In `soft_forget`, set it with `CASE WHEN forgotten_at IS NULL THEN active ELSE prev_active END`.
  - On undo, restore `active = COALESCE(prev_active, COALESCE(prev_state,'accepted') <> 'contradicted')`.
  - Add `("off", "accepted", false)` to the test table.

### backend-correctness-4-02 — minor — an SSH tunnel through a host with an existing ControlMaster connection never becomes "ready" and fails after 12 s

- **Where:**
  - `crates/otto-ssh/src/lib.rs:198-245` (`launch`: readiness requires `tap.forward_ready()`);
  - `:246-252` (`is_forward_ready_line`);
  - `:364-393` (`base_args`).
- **Evidence:**
  - The iteration-3 change made readiness depend only on this child's own stderr line, `debug1: Local forwarding listening on 127.0.0.1 port N.`: `if tap.forward_ready() && TcpStream::connect(..)`. Otherwise it waits until the 12-second deadline and returns "did not become ready within 12s".
  - `base_args` passes `BatchMode`, `StrictHostKeyChecking`, `ExitOnForwardFailure`, `ConnectTimeout`, `ServerAlive*` and `LogLevel=DEBUG1`. It does not pass `ControlMaster=no` / `ControlPath=none`. So the user's `~/.ssh/config` multiplexing settings apply (`ControlMaster auto`, `ControlPath …`, `ControlPersist …`, a common bastion setup).
  - When a master connection to the bastion already exists (the user's own terminal session, or an earlier tunnel left running by `ControlPersist`), the spawned `ssh -N -L …` is only a mux client. The master binds the forward and logs its `debug1` line (at the master's own log level, on the master's stderr). Otto's child never prints the readiness line.
  - Before this change, the TCP probe plus "child still alive" accepted that tunnel. Now every DB Explorer, Kafka or SFTP tunnel through such a host fails to open.
  - Note: even when Otto's ssh *becomes* the master under `ControlPersist`, killing the child (`kill_on_drop`) leaves the backgrounded master owning the port. That is another reason to opt out of multiplexing.
- **Fix:**
  - Add `-o ControlMaster=no -o ControlPath=none` to `base_args`. Otto's tunnel then always owns its own connection, the readiness line is always its own, and `kill_on_drop` really tears the forward down.
  - Assert both options in `local_forward_args_shape` and in the socks args test.

### backend-correctness-4-03 — nit — a ClickHouse multi-statement run panics when the connection's `params` is a JSON array or string

- **Where:**
  - `crates/otto-dbviewer/src/drivers/clickhouse.rs:1035-1036` and `:1043` (`batch_cfg.params["__ch_session"] = …`);
  - params come from `crates/otto-dbviewer/src/config.rs:103-109`, which keeps a non-object `conn.params` as it is.
- **Evidence:**
  - `serde_json::Value`'s `IndexMut<&str>` panics unless the value is an object or `null`.
  - `validate_params` (`otto-connections/src/builders.rs:444-480`) only builds the CLI command. For ClickHouse, `opt_str(p, "host")` on an array is `None`, so the command falls back to `login_shell()` and the params pass.
  - `config::parse` copies `conn.params` unchanged when it is not an object (only `as_object_mut()` strips `__` keys).
  - `force_read_only_execution` (`service.rs:195-201`) normalises non-object params, but only for read-only connections.
  - So a ClickHouse profile saved through the API with `"params": []` panics the request task on any query that splits into two or more statements. That panic happens before any connection attempt.
  - Only root can create or update such a profile, which is why this is a nit.
- **Fix:**
  - Normalise in `config::parse`: `if !params.is_object() { params = Value::Object(Map::new()) }`. Alternatively, write the session keys through `as_object_mut()` in `run_ch_batch`.
  - Optionally have `validate_params` reject non-object params for every DB kind.

### backend-correctness-4-04 — nit — swarm and product callers still take the first parseable JSON span or fence, so an earlier `{}` or JSON example shadows the real reply object

- **Where:**
  - `crates/otto-swarm/src/runtime/verify.rs:45-72` (`Verdict::parse`);
  - `crates/otto-swarm/src/runtime/run.rs:120-130` (`parse_turn_result`, also the `transcript_ok` predicate);
  - `crates/otto-swarm/src/runtime/engine.rs:3188-3229` (planner and plan summary);
  - `crates/otto-product/src/swarm.rs:244` and `:725` (planners);
  - shared extractor: `crates/otto-core/src/text.rs:135-158`.
- **Evidence:**
  - `extract_json` returns the first fenced ```` ```json ```` body that parses. Failing that, it returns the first balanced `{…}` span that parses. `{}` parses, as an empty object.
  - Iteration 3 added `extract_json_where(text, accept)` because "a `{}` in a preamble (`fn x() {}`) does not shadow the object the caller actually wants". It is used only by `pr_draft.rs:99`.
  - The callers above still use the schema-less `extract_json`:
    - `Verdict::parse` defaults a missing `target_met` to `false` and `blocker` to `true`. So a leader reply that skips the fence and mentions `` `format!("{}", x)` `` before its verdict, or that quotes a JSON config in an earlier ```` ```json ```` block, is read as a blocking miss. `classify` then asks for a fix, or marks a passing goal `unmet` and blocks the merge.
    - `parse_turn_result` returns `None` when the first span has no valid `status`, even if a valid result object follows. The transcript-based completion then never fires.
    - The planners create zero tasks from `{}`.
  - The prompts ask for a single fenced block (`verify.rs:1173-1177`), so this only bites when the model departs from the format. Hence a nit.
- **Fix:** switch each caller to `extract_json_where` with its schema predicate:
  - verdict: `v.get("target_met").is_some_and(Value::is_boolean)`;
  - turn result: `status ∈ {done, blocked, needs_review, in_progress}`;
  - planners: `v.get("tasks").is_some_and(Value::is_array)`.

  Use the same predicate in the matching `transcript_ok` closures. Add one test per caller with a `{}` preamble.
