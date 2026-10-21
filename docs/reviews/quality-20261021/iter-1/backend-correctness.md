# Backend correctness — quality iteration 1 (2026-10-21)

Lens: logic bugs in the Rust daemon crates (`crates/**`): wrong conditions,
off-by-one errors, broken state transitions, unhandled edge cases, wrong error
mapping, and panics that input can reach. The review was read-only. No cargo or
tests were run; every finding was confirmed by reading the code at the cited
line.

## Score

Start 10.0. Open findings: 1 major (-0.3), 3 minor (-0.3), 1 nit (-0.03).

**10.0 - 0.3 - 0.1 - 0.1 - 0.1 - 0.03 = 9.37 / 10**

## Previous findings

This is the first iteration of this run, so there are no earlier findings to
track.

## Areas covered

- `otto-core/src/cadence.rs`: due, served-occurrence, next-run and validate,
  with all callers. The callers are the scheduled-tasks scheduler and engine,
  the workflow trigger scheduler, the swarm scheduler and the
  personal-agents host.
- `otto-automation` scheduled-task settlement (`complete_run_with`,
  `settle_schedule`, `settles_schedule`).
- `otto-git/src/local.rs`: `refs_with_base`, the new `integration_bases` /
  `merged_into`, `merged_sets_cached`, `default_branch`,
  `resolve_cleanup_base`, `merge_branch` / `merge_branch_inner`, and the
  `guard_ref` entry points.
- `otto-sessions`: the agy MCP reconcile (`mcp.rs`
  `reconcile_json_launcher`, `retire_user_server`, manager
  `sync_workspace_mcp`), the grid feed (`grid.rs`) and `agent_message_rule`.
- `otto-server/src/game_rooms/*` (registry state machine, socket loop, create
  and join).
- `otto-workflows`: `location.rs` (run-location rules) and `retry.rs` (retry
  budget/classification, with the engine retry loop).
- `otto-transcript` agy protobuf decoder and the Grok usage parser
  (`split_input_tokens`, civil-date math), and the `ottod`
  native usage delta (`usage_tailer/native.rs`).
- `otto-usage/src/ingestion.rs` (journal checkpoint flatten/inflate,
  outbox).
- `otto-state`: `db/pre13.rs` rescue/restore, `workspaces.patch_settings`,
  the transactional connection update, and the session dormant pass.
- `otto-core/src/cancel_signal.rs`; `otto-server` error → HTTP status mapping.
- `otto-server/src/routes/fs.rs` (read/browse helpers).
- A sweep of every crate for string slicing at non-char boundaries. The sites
  checked were either guarded or used ASCII or `find`-derived offsets.

## Areas NOT covered yet (next iteration)

- `otto-sessions/src/manager.rs` lifecycle: restart, kill, archive, resume and
  holder re-adopt state transitions (the file is about 5k LOC).
- `otto-git/src/stash_identity.rs` (new, 841 LOC, journaled stash rewrite) and
  `transport.rs` / askpass credential destinations.
- `otto-dbviewer` drivers changed in f99242740 (mysql/redis/mongodb/clickhouse
  native command and result semantics), plus `service/changes.rs`.
- `otto-server/src/workflow_engine.rs` node execution and checkpoint recovery,
  and `workflow_trigger_scheduler.rs` claim path.
- `otto-swarm` runtime (scheduler reservation and release, verify).
- `otto-channels` mirror and transcript, `otto-product` service and watcher,
  `otto-vault` engine/store durability, and `otto-brokers` kafka.
- `ottod/src/usage_tailer.rs` main loop (2.4k LOC).

## Findings

### backend-correctness-1-01 — major — scheduled tasks replay every missed occurrence back-to-back after a sleep or outage

- **Where:** `crates/otto-core/src/cadence.rs:205-247` (`served_occurrence`)
  with `crates/otto-automation/src/scheduled_tasks_engine.rs:458,526,582`
  (the settled cursor is `served`).
- **Evidence:** `served_occurrence` returns the EARLIEST slot strictly after
  the cursor that is `<= now`:
  - interval: `cursor + every_min` (line 213);
  - daily and weekly: `next_daily_after` / `next_weekly_after`;
  - cron: `sched.next_after(anchor)`.

  The engine then settles the cursor at that slot
  (`settle_schedule(repo, task, "ok", served)`). On the next 60 s tick,
  `is_due_since` sees the next missed slot as due again. One example: for
  interval, `(now - l).num_minutes() >= every_min` with `l = old+every`. So
  the scheduler walks the whole backlog one run at a time:
  - a `*/5` cron or a 5-minute interval after an 8 h sleep runs about 96
    back-to-back agent sessions and deliveries;
  - a daily task after a week offline runs 7 times.

  Failed runs also settle at `served` (line 582), so a failing task does the
  same. The scheduler has no cap (grep for wake/backlog/catch-up in
  `scheduled_tasks_scheduler.rs` finds nothing). The module doc and the test
  name (`daily_and_weekly_catch_up_one_missed_slot_after_the_cursor`) intend
  ONE catch-up, but the tests only cover a backlog of one slot.
- **Fix:** have `served_occurrence` return the LATEST occurrence `<= now`, not
  the earliest one after the cursor:
  - interval: `cursor + floor((now-cursor)/every)*every`;
  - cron/daily/weekly: iterate `next_after` until the next one is `> now` and
    keep the last, bounded to N steps.

  This keeps the in-flight guarantee (a run started at 09:05 still serves
  09:05, and 09:10 stays due) and collapses a backlog into one catch-up run.
  Add a test with a multi-slot gap (for example `*/5` and 8 h asleep) that
  asserts exactly one more due run after settlement.

### backend-correctness-1-02 — minor — a huge `every_min` passes validation, then panics in chrono arithmetic

- **Where:** `crates/otto-core/src/cadence.rs:395-406` (validate only checks
  `>= MIN_INTERVAL_MIN`), `:308` (`next_run`:
  `from + Duration::minutes(every_min(spec))`) and `:213`. It is reachable
  from `crates/otto-server/src/routes/scheduled_tasks.rs:418-425` (create),
  the update path, and the personal-agents host
  (`assistant_host.rs:188`).
- **Evidence:** `{"cadence":"interval","every_min":1000000000000}` validates.
  `Utc::now() + Duration::minutes(1e12)` overflows chrono's DateTime range and
  panics (``DateTime + TimeDelta overflowed``). Above about 1.5e14,
  `Duration::minutes` itself panics. The create handler panics before any
  response. No `CatchPanicLayer` is mounted (grep in `otto-server/src` finds
  none), so the client sees a dropped connection instead of a 400.
- **Fix:** bound `every_min` in `validate`, for example to 525 600 (one year)
  or to the largest value whose `Duration` add fits. Use
  `checked_add_signed` and `TimeDelta::try_minutes` in `next_run` and
  `served_occurrence`, falling back to `None` or `now`.

### backend-correctness-1-03 — minor — merging a branch whose tip already equals the target makes an empty "Merge branch" commit

- **Where:** `crates/otto-git/src/local.rs:4563-4590` (`merge_branch_inner`,
  MergeCommit with "Already up to date").
- **Evidence:** when `git merge --no-ff` reports "Already up to date", Otto
  builds a 2-parent commit by hand:
  `commit-tree <HEAD tree> -p <target_head> -p <source_head>`, then
  `merge --ff-only`. Nothing checks `source_head != target_head`. Two
  everyday cases hit this:
  - a branch just created from `main` with no commits, merged back with
    MergeCommit;
  - `source == target` (no guard in `merge_branch` or in the route,
    `otto-git/src/http.rs:2735-2748`).

  In both cases `-p X -p X` collapses to one parent ("duplicate parent
  ignored"). Otto then fast-forwards the target onto an empty commit titled
  "Merge branch 'feature' into main" and reports `status: "merged"`.
- **Fix:** in the up-to-date branch, if
  `source_head == target_head` (or `source == target`), return the
  `up_to_date` result without creating a commit. Also reject
  `source == target` in `merge_branch` with `Error::Invalid`.

### backend-correctness-1-04 — minor — `merged_into` never reports the default branch when only `origin/<default>` exists locally

- **Where:** `crates/otto-git/src/local.rs:810-841` (`integration_bases`),
  `:1902-1924` (`default_branch`), `:2331-2337` and `:2496-2498`.
- **Evidence:** `default_branch()` returns the bare name (`"main"`) from
  `origin/HEAD` even when no local `main` exists. `integration_bases` pushes the
  base as `(label, rev) = ("main", "main")`, then skips the `main` candidate
  because the label is already present. So the documented
  "local else `origin/`" fallback is never applied to the base. With no local
  `main` (a clone made with `-b develop`, or a deleted local main):
  - `resolve_commit("main")` fails;
  - `merged_sets("main")` runs `git branch --merged main`, which fails, so the
    set is empty;
  - because `complete` is false, nothing is memoized and two failing git
    spawns are repeated on every 30 s refs poll.

  The base's twin `origin/main` is never consulted, so no branch ever shows
  "main" in `merged_into` (or `merged_into_base`). A base override such as
  `origin/develop` also gets a duplicate label pair (`origin/develop` and
  `develop`).
- **Fix:** resolve the base's rev the same way as the candidates. If
  `local_has(base)` is false and `remote_has(base)` is true, use
  `origin/<base>` as its rev. Dedupe labels whose `origin/` prefix-stripped
  name matches an existing label.

### backend-correctness-1-05 — nit — `expand_home` turns `~user/...` into `$HOMEuser/...`

- **Where:** `crates/otto-server/src/routes/fs.rs:110-117`.
- **Evidence:** `path.strip_prefix('~')` matches any leading `~`, so
  `~bob/notes.txt` becomes `/Users/alicebob/notes.txt`. The result is a
  confusing "resolve file" not-found for a path the user typed. The sibling
  helper in `otto-workflows/src/location.rs:11-23` correctly handles only
  `~` and `~/`.
- **Fix:** expand only `~` exactly or a `~/` prefix (reuse the `expand_tilde`
  logic). Leave `~user` untouched, or reject it with a clear `Invalid`.
