# Backend correctness — quality iteration 2 (2026-10-22)

Lens: logic bugs in the Rust daemon crates (`crates/**`): wrong conditions,
off-by-one errors, broken state transitions, unhandled edge cases, wrong error
mapping, and panics that input can reach. The review was read-only. No cargo,
npm or tests were run. Every finding below was confirmed by reading the code
at the cited lines (and, for the chrono panics, the chrono 0.4.45 sources
pinned in `Cargo.lock`).

## Score

Start 10.0. Open findings: 2 minor (-0.1 each), 1 nit (-0.03).

**10.0 - 0.1 - 0.1 - 0.03 = 9.77 / 10**

## Previous findings (iteration 1)

| id | status | evidence in current code |
|----|--------|--------------------------|
| backend-correctness-1-01 (backlog replay) | **fixed** | `otto-core/src/cadence.rs:216-264` `served_occurrence` now returns the LATEST slot `<= now` after the cursor: interval `cursor + floor((now-cursor)/every)*every`, `latest_daily_at_or_before`, `latest_weekly_at_or_before`, and `latest_cron_fire` with widening lookback windows capped at 2,000 steps. |
| backend-correctness-1-02 (huge `every_min` panic) | **fixed** | `validate` rejects `every_min > MAX_INTERVAL_MIN` (525,600), and `add_minutes` uses `checked_add_signed(TimeDelta::try_minutes)`. The interval arm uses `checked_mul`. |
| backend-correctness-1-03 (empty merge commit) | **fixed** | `otto-git/src/local.rs:4495-4502` rejects `source == target`. `:4621-4628` builds the 2-parent commit only when `source_head != target_head`, and otherwise falls through to `up_to_date`. |
| backend-correctness-1-04 (remote-only default branch) | **fixed** | `integration_bases` (`local.rs:815-844`) resolves the base to `origin/<base>` when only the remote ref exists, and dedupes through `integration_name`. `merged_sets_cached` reads the base oid from the listings. |
| backend-correctness-1-05 (`~user` expansion) | **fixed** | `routes/fs.rs:113-120` rejects `~user` paths and expands only `~` and `~/`. |

All five iteration-1 findings are fixed. No iteration-1 finding was rejected.

## Areas covered this iteration

- I re-verified every iteration-1 fix: the cadence latest-occurrence math, the
  merge guard and `integration_bases`.
- **The code changed by the iteration-1 fix commit (66b55a25c)**, read as new
  code that could carry new bugs:
  - `otto-core/src/cancel.rs`: the `CancelRegistryExt` / `CancelGuard`
    ptr-identity unregister, and its callers in automation, swarm, product
    and personal agents.
  - `otto-sessions`:
    - the `submit_locks` serialization: the manager's `submit_text`,
      `human_submit_text_checked` and the channel bridge, checked for
      re-entrancy and deadlock;
    - the `sync_workspace_mcp` per-provider snapshot;
    - the codex user-server env files: written, removed in
      `revoke_session_credentials` and rewritten on restart;
    - `codex_user_server_args` and `write_env_file`.
  - `otto-sessions/src/ws.rs`: the `send_frame` timeout and the inbound
    liveness deadline (the `deferred.reading()` interaction).
  - `workflow_engine.rs`: the event-driven `human_approval` wait
    (`wake_on_event`, `is_gate_decision`) against `approve_run` /
    `record_approval` / `emit_run_decision`, and the `review_run` await loop.
  - `otto-state/src/mcp_control.rs`: the single-statement `COALESCE` updates,
    which I checked are equivalent to the old per-column writes.
  - `otto-swarm`:
    - `delete_swarm_halting`, `delete_project_h` and `halt_runs`;
    - the transactional cascades in `otto-state/src/swarm.rs`;
    - the route merge with `http.rs`, checked for duplicate method and path.
  - `otto-mcp/src/client.rs`: protocol-version negotiation, the
    `MCP-Protocol-Version` header and the `StderrTail` bound.
  - `routes/share.rs`: revoke by minter or session owner, checked against the
    auth cache contract (share tokens are never cached).
  - `routes/projects.rs`: the session paging loop. `next` is always set when
    the page is non-empty, so the loop has no re-read or infinite-loop risk.
  - `otto-design/src/blobs.rs` (torn-blob heal, unique tmp names) and
    `store.rs` (conditional project delete).
  - `otto-improve` `backup_and_replace` and the `run_service`
    untracked-launcher-file gate.
- **Areas iteration 1 marked as not covered:**
  - `otto-sessions/src/manager.rs` lifecycle: `ensure_live`, `restart` /
    `restart_locked`, `kill_session_locked`, `suspend_inner` /
    `suspend_if_idle` / `kill_if_idle`, `remove`, `unarchive`,
    `auto_archive_stale`, and the status task's exit / superseded-handle
    logic (`evict_if_same`, `maps_handle`). This includes credential
    revocation on exit, kill and suspend, and MCP token rotation on respawn.
  - `otto-git/src/stash_identity.rs` `rewrite` (lock, journal, pins, commit).
  - `otto-git/src/patch.rs` partial-hunk builder: multi-block type changes,
    the selection, `\ No newline` handling, count recomputation and
    creation/deletion-to-modification rewrite.
  - `otto-dbviewer/src/split.rs` (the statement scanner, including the MySQL
    DELIMITER handling) and the MySQL `run_script` loop.
  - `workflow_trigger_scheduler.rs`: the tick, the overlap guard and
    `admit_schedule_run`; and `resume_queued_runs`.
  - `otto-swarm/src/runtime/scheduler.rs`: the tick, `reserve_scheduled_run`,
    `release_unstarted_scheduled_occurrence`, the cap and busy gates, and
    `swarm_session_ids`.
  - `otto-channels`: the bridge `submit_to_agent` and the mirror
    `remove_own` / cancelled-entry replacement.
  - `otto-product/src/watcher.rs` `poll_story`, which keeps the cursor until
    commit.
  - `otto-brokers/src/service.rs` replay selector and offset-range validation.
  - `ottod/src/usage_tailer.rs` `read_new_lines` / `read_lines_window`.
- **Sweeps:**
  - unchecked `len() - x` subtraction sites across crates (about 60);
  - chrono `Duration::{days,hours,minutes,seconds}(var)` built from
    user-controlled values;
  - SQL `LIKE` patterns built from user input without `ESCAPE`;
  - `unwrap` / `expect` in route handlers.

## Areas NOT covered yet (next iteration)

- `otto-sessions/src/manager.rs`:
  - `create` (3009-3525) end to end;
  - `adopt_holders` / `adopt_one` / `adopt_deferred` (holder re-adopt);
  - `enforce_live_cap`;
  - `suspend_idle_unattached` sweep guards;
  - `capture_nested_agents`.
- `otto-sessions/src/ws.rs` input queue / deferred-read state machine beyond
  the liveness change. `input_queue_tests.rs` was not cross-checked against
  the implementation.
- `otto-dbviewer` drivers: the postgres, clickhouse and mongodb native command
  paths, `service/changes.rs`, and `types.rs` `inject_row_limit` /
  `sql_is_write` parity.
- `otto-vault` engine/store durability (rename link rewrite, watch,
  recovery).
- `otto-automation` goal-loops controller; `otto-assistant` threads and
  routing.
- `otto-review` summarizer / fallback; `otto-server/src/modules.rs` review
  routes (the `agents.len() - 1` sites are guarded, but retry/stop state
  transitions were not traced).
- `ottod/src/usage_tailer.rs` main loop beyond the windowed reader
  (attribution, held responses, checkpoint restore).
- `otto-git/src/http.rs` worktree / PR routes and `transport.rs` askpass.

## Findings

### backend-correctness-2-01 — minor — a large API-history retention ("keep N days") panics every prune

- **Where:**
  - the panic: `crates/otto-state/src/api_client.rs:870-871`
    (`prune_history`: `Utc::now() - chrono::Duration::days(max_age_days)`);
  - the unbounded read: `crates/otto-server/src/routes/api_client.rs:1837-1851`
    (`history_retention`: any `as_i64() >= 0` is accepted);
  - the UI input: `ui/src/modules/api/RetentionDialog.svelte:22-25` (accepts
    any non-negative integer).
- **Evidence:**
  - The retention dialog saves `settings.api_client.history_max_days`
    unbounded. The backend passes it straight to
    `Utc::now() - Duration::days(n)`.
  - In chrono 0.4.45, `DateTime - TimeDelta` is
    `checked_sub_signed(rhs).expect("`DateTime - TimeDelta` overflowed")`
    (`datetime/mod.rs:1658-1659`). Its range ends near year -262,143, which
    is about 95.7 M days back. `TimeDelta::days` itself panics above about
    1.07e11 (`time_delta.rs:138`).
  - So a user who types `99999999` days, meaning "keep forever", gets a panic
    in three places:
    - the `POST …/api-client/storage/prune` handler (`prune_storage`,
      `api_client.rs:1168-1170`): no response, the connection drops, and the
      dialog shows a generic error;
    - the background prune spawned after every Send (`api_client.rs:1753-1763`):
      the task dies silently, so no history is pruned by rows either;
    - `api_automation_runs.rs:318-326` after every automation run.
  - The same unguarded pattern is in
    `crates/otto-sessions/src/manager.rs:5450`
    (`session_auto_archive_days`, a free-form global setting with no UI). A
    huge value there kills the hourly archiver task for the daemon's
    lifetime (`boot/tasks.rs:279-290`).
- **Fix:**
  - In `prune_history`, compute the cutoff with
    `TimeDelta::try_days(max_age_days).and_then(|d| Utc::now().checked_sub_signed(d))`
    and treat `None` as "no age limit" (skip the DELETE).
  - Clamp `history_max_days` in `history_retention`, for example to
    36,500 days, and reject values over that in the dialog.
  - Apply the same `try_days` / `checked_sub_signed` to
    `auto_archive_stale`.
  - Add a unit test with `max_age_days = 99_999_999`.

### backend-correctness-2-02 — minor — the k8s monitor `?window=` parser panics on a non-ASCII unit or a huge count

- **Where:** `crates/otto-k8s/src/monitor/queries.rs:196-216`
  (`parse_window`). It is reachable from `GET /k8s/monitor/overview?window=`
  (`monitor/http.rs:552-559`) and
  `GET /k8s/clusters/{id}/monitor/workloads?window=` (`:706-713`).
- **Evidence:** there are two panics.
  - `let (num, unit) = t.split_at(t.len() - 1);` splits at a BYTE offset. For
    `window=1é`, `t.len()` is 3 and byte 2 is inside `é`. `str::split_at`
    panics ("byte index 2 is not a char boundary"). Any multi-byte final
    character (`5µ`, `1日`) does the same.
  - `Duration::minutes(n)`, `Duration::hours(n)` and `Duration::days(n)` run
    BEFORE the 90-day check (`:208-215`). A parseable but huge `n`, such as
    `window=9999999999999d`, panics in `TimeDelta::days` ("out of bounds")
    instead of returning the intended `Invalid("window must be at most 90d")`.

  The handler task panics and the client gets a dropped connection instead of
  a 400. No `CatchPanicLayer` is mounted in `otto-server`.
- **Fix:**
  - Split on the last char:
    `let unit_at = t.char_indices().last().map(|(i, _)| i)?;` then
    `t.split_at(unit_at)`.
  - Use `TimeDelta::try_minutes`, `try_hours` and `try_days`, mapping `None`
    to the same `Invalid("window must be at most 90d")`. Alternatively, bound
    `n` (for example `n <= 90*24*60`) before building the duration.
  - Add the `"1é"` and `"9999999999999d"` cases to the existing
    `parse_window` test.

### backend-correctness-2-03 — nit — Mission Control's work-item search treats `%` and `_` in the query as wildcards

- **Where:** `crates/otto-state/src/workgraph.rs:747` and `:755`
  (`list_items`).
- **Evidence:**
  - `let like = f.q.as_ref().map(|s| format!("%{s}%"));` is bound into
    `title LIKE ?` with no `ESCAPE` clause and no escaping of `%` / `_`.
  - Searching `100%` matches every title containing `100`.
  - Searching `my_task` also matches `myXtask`, `my-task` and so on.
  - The vault (`otto-vault/src/store.rs:1205-1219`, `:1388-1391`) and design
    (`otto-design/src/store.rs:2358-2361`) searches already escape the
    pattern and add `ESCAPE '\'`, so this site is the odd one out.
- **Fix:** escape the user text
  (`s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")`) and
  write `title LIKE ? ESCAPE '\'`. The same backslash-first escaping should
  also be used by the vault and design helpers, which currently do not escape
  a literal `\` in the needle.
