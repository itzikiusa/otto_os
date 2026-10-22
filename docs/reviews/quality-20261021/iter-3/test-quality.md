# Test-quality review: iteration 3 (2026-10-22)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (grep, sed, git log, git show). No cargo, npm or e2e runs.
Every finding below was confirmed by reading the code at the cited lines.
Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`
and `docs/reviews/`.

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 3 major, 3 minor, 0 nit.

10.0 − (3 × 0.3) − (3 × 0.1) = 10.0 − 0.9 − 0.3 = **8.8 / 10**

Iteration 2 scored 8.5. Six of its seven findings are fixed, and the tests that
fixed them are real. The fixes were checked for vacuity: for example, the
swarm submit-lock races would really fail on an interleave, and the
state-archive helper now calls the production loop. The remaining deduction
comes from going one level deeper into areas that earlier passes did not
reach:

- the swarm scheduler's utilization watchdog;
- the reviewed database-change executor gate;
- the Kafka broker mutation guard;
- the DB Explorer tree, search and ERD filters.

## Previous findings

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-2-01 | **Fixed** | `crates/otto-server/tests/swarm_router.rs` (included in `tests/it.rs:79-80`) drives `route_result_inner` through six cases, each asserting the task row and the posts: a cancel mid-turn is left as set (`:206-227`); pause versus stop gives `todo` with a refund versus `blocked` (`:232-262`); a failed turn retries below the ceiling and blocks at it (`:266-291`); one manager triage task labelled `hops:1` (`:296-328`); `hops:3` escalates and creates no task (`:333-353`); a repeated handoff is deduped (`:357-392`). The three submit-lock races (`:450-546`) use a `cat > sink` PTY and compare sorted whole lines, so an interleaved paste would merge two lines and fail. |
| test-quality-2-02 | **Fixed** | `state_archive/tests.rs:8-24`: `snapshot_with` calls the production `build_snapshot_parts`. `export_drops_secret_settings_hashes_and_tokens_portable_or_not` (`:430-462`) seeds `xoxb-plain`, `sk-plain`, an SMTP password, a password hash and two token hashes. For both `portable` values it asserts that none of them leak and that `default_provider` survives. |
| test-quality-2-03 | **Fixed** | `crates/otto-state/src/mcp_control.rs:1640-1684` pins `dangerous`, rediscovers as `read`, and asserts that the label stays while the description refreshes. It also has an un-pinned control tool. `:1687-1719` checks that PATCH leaves omitted fields alone and that an enable toggle does not pin the labels. `:1721-1757` covers policy `update`. |
| test-quality-2-04 | **Partly open** | `resource_sessions.rs:303-355` now runs `check_many_with_pool` with revoked and allowed connections, in order, and with a disabled caller who is reloaded. This kills the `None => true`, inverted-`ok` and id-collision mutants. FIXES.md P02 lists two cases as not done: the namespace `child` in the memo key, and the HTTP list. Both are still untested, so the rest is carried forward as **test-quality-3-05**. |
| test-quality-2-05 | **Fixed** | The guard is factored into `remove_if_generation` (`plugins.rs:118-129`), which the watcher calls (`:295`). `only_the_owning_generation_unpublishes_a_port` (`:1291-1309`) checks that a stale generation leaves the newer entry in place. This is the alternative fix the finding proposed. |
| test-quality-2-06 | **Fixed** | `manager/tests/mod.rs:1266-1347`: `seed_credential_files` writes the creds file, two env files and a neighbour file named `{id}0`, which catches a matcher missing its dot. Remove, archive, suspend and kill each assert that the session's own files are gone and the neighbour's file survives. |
| test-quality-2-07 | **Fixed** | `ui/e2e/desktop-session-header.spec.ts:185-196`: `toBeVisible()` plus the title, folder and copy assertions now run unconditionally for counts 1 and 2. Count 6 asserts `toBeHidden()`. |

## Areas covered

- All seven iteration-2 findings were re-verified (table above). The new
  tests were read for vacuity and timing. The `submissions()` poll in
  `swarm_router.rs` has a deadline: 80 × 50 ms, with no fixed sleep used for
  synchronisation.
- The tests added by iteration-2 fixes and not tied to a finding:
  - goal-loop config ownership (`goal_loops.rs` `owner_tests`: 403 for a
    non-owner Editor, and the config is not changed);
  - `database_changes` `finish_in` and recovery reclassification;
  - the api-client collection cycle and workspace checks, and
    multi-field PATCHes;
  - GitHub and Bitbucket CI paging plus `stale` (`wire_tests.rs`: a page-2
    failure, unread runs give `pending`);
  - `termProtocol`, `cadence` and `confirmOutward` unit suites.

  All of them assert behaviour, not wiring.
- From iteration 2's "not covered" list:
  - `otto-swarm/runtime/verify.rs` is well covered. `run_verification` runs
    against a scripted `MockOps` with 13 cases: merge ordering, a blocking
    verdict that never merges, cancel before merge, budget. There are also
    pure gate tests. One test scans the source as text
    (`run_controller_keeps_lookup_errors_and_pairs_cwd_with_branch`,
    `:1525`), but it backs up the behavioural tests and does not replace them.
  - `scheduler.rs`: the admission helpers (`scheduled_turn_open`,
    `schedule_slot_open`, `is_due`) and the stop-wins race are tested. The
    utilization watchdog is not: **3-01**.
  - `otto-dbviewer/src/access.rs` `check_many`: traced to its four callers.
    The FK filter is tested (`perf_budget_tests.rs:484-554`, mixed scopes,
    order kept). The tree, search and ERD filters are not: **3-04**. Tracing
    the dbviewer tests also found that the reviewed-change executor gate is
    covered only by `#[ignore]`d tests that no CI job runs: **3-02**.
  - Large route files:
    - `otto-k8s/src/monitor/http.rs` namespace scoping is covered by
      `fake_kubectl.rs` (`monitor_routes_enforce_cluster_grants`,
      `scoped_workloads_redact_the_cluster_wide_status_row`,
      `fleet_and_overview_are_filtered_to_the_callers_grants`);
    - `otto-vault` has `tests/http.rs`;
    - `otto-brokers/src/http.rs` has one unit test, and `kafka_e2e.rs` does
      not cover the guarded-cluster refusals: **3-03**.
  - The UI unit files that read `src/` as text were sampled:
    - `transcriptLifecycle.test.ts` is behavioural: it runs the real store
      under `vm` with leases, aborts, an eviction bound and identity reset;
    - `journeyFixes.test.ts` is regex pinning of Svelte markup. This is a
      known, repo-wide choice for components without a DOM harness, and the
      behaviour is also covered by `desktop-ux-onboarding.spec.ts`. It is not
      reported.
  - `otto-product` watcher: 12 tests. No finding.
- Recently added, high-risk files (git log `--diff-filter=A` over 3 weeks),
  checked for test counts and then traced:
  - `otto-pty` holder/held: 17 integration tests in `tests/holder.rs`;
  - `dbviewer/service/multirun.rs`: `multirun_tests.rs`, which includes
    another user being unable to see a job;
  - review routes: `approve_comment` declined → 409, local review never
    posts, at-most-once claim tested in `otto-state/reviews.rs:848-853`, and
    the anchor fallback helpers in `otto-review/engine.rs:1024-1040`;
  - rooms `actions.rs` control transfer is exercised over a real WebSocket in
    `tests/rooms_api.rs` (role → grant_control → driver snapshot →
    release_control);
  - git force-push and discard: `push.rs` and `patch.rs` tests.
- FIXES.md "not done" items with a missing test were checked: the
  scheduled-task settle retry is **3-06**. The claim that it needs a
  fault-injection seam does not hold.

## Areas NOT covered yet (next iteration)

- `otto-swarm/runtime/verify.rs` `SwarmVerifyOps` (the real ops,
  `:564-872`): verdict parsing from the agent result, `merge_back`, and
  escalation posts. Only `MockOps` drives the controller.
- `otto-swarm/runtime/scheduler.rs` `tick()` as a whole. The helpers are
  tested; the reserve → emit → spawn sequence is not.
- `otto-mcp/src/http.rs` (1,180 lines, 18 routes, no inline tests): trace
  which routes `pipeline_e2e.rs` and the otto-server `mcp_*` suites reach.
- `otto-server/src/rooms/registry.rs` (560 lines), `recap/mod.rs` (699) and
  `recap/http.rs` (351), none of which has inline tests: check the negative
  admission and recap-consent paths beyond `rooms/tests.rs`.
- UI stores `git.svelte.ts` (894 lines) beyond auto-fetch and ownership, and
  `clipHistory.svelte.ts`.
- `otto-server/src/modules/review/routes.rs` (1,417 lines, one inline test):
  the `approve_comment` handler with a fake forge. The claim is released on a
  failed post, and a second approve after a 5xx does not post twice.
- `otto-assistant` `personal_agents_engine.rs` `advance_cursor` retry: the
  same shape as 3-06.

## Findings

### test-quality-3-01: major. The swarm utilization watchdog has no test, although it reassigns tasks and starts paid manager turns on its own

- File: `crates/otto-swarm/src/runtime/scheduler.rs:283-524`
  (`utilization_pass`, `check_utilization`); constants `UTIL_EVERY` and
  `UTIL_ESCALATE_EVERY` (`:271-272`).
- Evidence: every 60 s tick runs `utilization_pass` (`:100`). For each active
  swarm, `check_utilization` does three things:
  1. It reassigns ready tasks whose assignee is busy or inactive to the
     best-fitting idle teammate. It writes `assignee_agent_id` and posts a
     "rebalanced" status (`:352-407`).
  2. When nothing is schedulable but open work exists, it reserves a
     `trigger: "utilization"` run for the manager, with a directive
     (`:447-479`).
  3. It spawns `run::run_turn` for that run (`:520-523`). This is an LLM turn
     that costs tokens. A 30-minute per-swarm rate limit is supposed to
     bound it (`:425-434`).

  `grep -rn 'check_utilization|utilization_pass|utilization_rebalance|utilization_escalation' crates`
  finds only `scheduler.rs`. The module's nine tests (`:600-737`) cover
  `scheduled_turn_open`, `schedule_slot_open`, `is_due` and the stop race.
  None of them reaches this function. Each of the following passes every
  test:
  - dropping the `assignee_is_idle` skip (`:363-367`), which would take ready
    work away from idle agents;
  - inverting the `active >= cap` early return;
  - removing the `!busy.contains` or `agent_under_verification` filter, which
    would reassign onto an agent that is running or verifying;
  - breaking the escalation rate limit (`e.1` check, `:428-433`), which would
    wake the manager, and spend a paid turn, every 5 minutes on a stuck
    board.
- Fix: make `check_utilization` `pub(crate)` or expose a test hook. Then add
  a case to `crates/otto-server/tests/swarm_router.rs`; its `world()` already
  builds a swarm with a manager and an IC, and `ctx.swarm_rt()`:
  1. Use a cap of 2. Give the IC an active run (busy), and give it a `todo`
     task. Add a second idle IC. Assert that the task moves to the idle IC,
     and that one `utilization_rebalance` post exists.
  2. A ready task already on an idle agent: assert it is not moved.
  3. No ready tasks and one `blocked` task: assert that one
     `trigger='utilization'` run is reserved for the manager. Call the
     function again at once and assert that no second run is created (the
     rate limit).
  4. The board is at its cap: assert that nothing changes.

  So that the test does not start a real turn, either assert on the reserved
  row before the spawn by passing a cancelled `CancelSignal` after the
  reserve, or factor the reassignment choice into a pure
  `plan_rebalance(ready, idle, cap, active)` function and unit-test it.

### test-quality-3-02: major. The reviewed database-change executor gate is covered only by `#[ignore]`d tests that no CI job runs

- Files: `crates/otto-dbviewer/src/service/changes.rs:51-113`
  (`approved_attempt`) and `:199-254` (`execute_approved_change`);
  `crates/otto-dbviewer/tests/resource_access.rs:326-335, 337-519, 521-530, 532-611, 613-621`;
  `.github/workflows/db-drivers.yml:12-14, 156-157`.
- Evidence: `execute_approved_change` is the only way an approved schema
  change runs (`crates/otto-server/src/database_changes.rs:286`). Before it
  sends any SQL, and again every 500 ms while the SQL runs, `approved_attempt`
  refuses an attempt in any of these cases:
  - the attempt's `script` or `content_hash` no longer matches the approved
    change (the single SQL at `:57`);
  - the executor is different, or `cancellation_requested=1`;
  - an explicit deny now matches (`:79-96`);
  - the policy revision or the connection or credential fingerprint changed
    (`:97-111`).

  The only tests of this gate are `approved_native_fixture`, which tampers
  with the script, then makes the hash stale, then replays a finished or
  unknown attempt (`resource_access.rs:412-505`). It is reached only through
  the seven `#[ignore = "requires explicitly provisioned disposable … fixture"]`
  tests. `db-drivers.yml` excludes them explicitly
  (`-E 'not binary(resource_access)'`, `:157`), and its header says so:
  "resource_access.rs's native-credential cases are NOT run … no seed in the
  repo creates yet" (`:12-14`).

  The ledger in `otto-state` is well tested (`database_changes_tests.rs`, 10
  cases), but those tests never call the dbviewer gate.
  `grep -rn execute_approved_change crates` finds no other caller in any
  test. Any of the following passes CI:
  - dropping `AND a.script=c.script` or `a.content_hash=c.content_hash` from
    the `:57` query, so a tampered artifact would run;
  - removing the revision or fingerprint comparison;
  - removing the mid-execution re-check.

  Each refusal fires before any network call (`approved_attempt` is the first
  statement; `change_target_fingerprint`, `:18-49`, only reads rows and the
  Keychain). So these cases do not need a live database at all.

  The same applies to `governed_root_reads_cannot_call_a_mutating_builtin_overload`
  (`:532-611`), the only test of the native read-only gate for root reads on
  prod. It does need Postgres, and it never runs.
- Fix:
  1. Add a non-ignored test in `resource_access.rs` that uses the existing
     `Fixture::new(kind, 1)` (port 1 is never reached). Create, validate,
     submit, approve and claim a change through `DatabaseChangesRepo`, as
     `approved_native_fixture` does. Then assert that `execute_approved_change`
     returns `Forbidden` or `Conflict` in each of these cases, without
     touching the driver:
     - the tampered `script`;
     - a stale `content_hash`;
     - a different executor;
     - `cancellation_requested=1`;
     - a policy edited after the claim (revision bump);
     - a finished attempt.
  2. Add the `otto_reader` user and the `shop` and `hidden` seeds to
     `dev/dbviewer`, and drop the `not binary(resource_access)` filter in
     `db-drivers.yml`, so the seven native cases run nightly and on dbviewer
     PRs.

### test-quality-3-03: major. The Kafka prod and read-only guard on destructive broker operations, and the global-cluster grant, have no test

- File: `crates/otto-brokers/src/http.rs:156-209` (`authorize`, `guard`).
  `guard` is called by:
  - `create_topic`, `:333`;
  - `delete_topic`, `:378`;
  - `put_configs`, `:407`;
  - `produce`, `:445`;
  - `reset_group_offsets`, `:496`;
  - `replay`, `:568`.
- Evidence: `guard` is the only check that stops an Editor from deleting a
  topic, rewriting topic configs, producing to, resetting consumer-group
  offsets on, or replaying into a `prod` or `read_only` cluster without
  `confirm=true`. These operations are irreversible against production
  streams. `reset_group_offsets` has its own extra rule: a dry run is allowed
  without confirm (`:486-494`).

  The crate has one unit test, `lag_alerts_list_is_200_when_wired_like_the_daemon`
  (`:674-716`). `kafka_e2e.rs` never creates a prod or read-only cluster
  (`grep -n 'confirm|prod|read_only' crates/otto-brokers/tests/kafka_e2e.rs`
  finds no such case). Outside the crate,
  `grep -rln 'brokers/clusters' crates/*/tests` finds only `kafka_e2e.rs`.

  The global-cluster branch of `authorize` (`:166-189`, the Database grant
  for a workspace-less cluster, the fail-closed `None` pool) has no test
  either. The policy snapshot only pins the route-level
  `Require(Database, …)`.

  Any of the following passes every test:
  - deleting the `guard` call from any handler;
  - changing `row.read_only || row.environment == "prod"` to `&&`;
  - moving `reset_group_offsets`'s guard above the dry-run branch.
- Fix: in `http.rs` `mod tests`, reuse the existing `Ctx`/`AllowAll`
  harness and a cluster at `127.0.0.1:1`. The guard runs before any broker
  call, so no Kafka is needed. For a cluster with `environment: "prod"` and
  one with `read_only: true`:
  1. Assert that each of the six mutating routes answers 403 without
     `confirm`. The message must name "production" or "read-only".
  2. Assert that `reset` with `dry_run=true` is not refused by the guard. It
     then fails later on the unreachable broker with a non-403 status.
  3. Assert that a `dev` cluster passes the guard (also a non-403 status).

  Add a global-cluster case: a non-root user without the Database `edit`
  grant gets 403 on `DELETE /brokers/clusters/{id}`, and a user with it
  does not.

### test-quality-3-04: minor. The DB Explorer tree-root, object-search and ERD-edge reader filters are never exercised by a running test

- File: `crates/otto-dbviewer/src/service.rs:1700-1722` (`schema_root`),
  `:1765-1800` (`search_objects`, including the `scanned` recount),
  `:2016-2040` (`schema_graph` edge filter); all three go through
  `authorize_nodes` → `access::check_many` (`:820-840`,
  `access.rs:148-180`).
- Evidence: `check_many` itself is tested through the FK filter
  (`perf_budget_tests.rs:484-554`, an enforced reader limited to `shop`, with
  mixed scopes). The three other call sites decide which databases, search
  hits and ERD edges an enforced reader sees, and none of them is run with a
  restricted user:
  - `schema_root`: the only reader assertion is in the ignored
    `native_fixture` (`resource_access.rs:267-271`, never run; see 3-02). The
    running `schema_root_reads_do_not_scale_with_database_count`
    (`perf_budget_tests.rs:207-233`) uses a caller who is allowed every
    root, and asserts `nodes.len() == roots`.
  - `search_objects`: no test calls it on the service.
  - `schema_graph`: both stubs return `edges: vec![]`
    (`perf_budget_tests.rs:156`, `scoped_metadata_tests.rs:66`).

  Any of the following passes every test and shows hidden database names to a
  scoped reader:
  - returning `nodes` unfiltered;
  - leaving `result.scanned` as the driver's count, which would reveal how
    many hidden databases were inspected;
  - skipping the edge `zip`.
- Fix: in `perf_budget_tests.rs`, reuse the `shop`-only reader from
  `object_detail_filters_mixed_fk_scopes_without_changing_order`. Make the
  stub return:
  - roots `shop` and `hidden`;
  - search hits in `shop`, `hidden` and `shop`;
  - one graph edge into `shop` and one into `hidden`.

  Assert that `schema_root` returns only `shop`, and that `search_objects`
  returns the two `shop` hits in order with `scanned == 1`. Assert that
  `schema_graph` keeps only the `shop` edge.

### test-quality-3-05: minor. The resource-session memo's namespace key and the HTTP list filter are still untested (rest of 2-04)

- File: `crates/otto-server/src/resource_sessions.rs:105-150`, memo key
  `(kind, id, child, op)` at `:128-133`; `crates/otto-sessions/src/http.rs:391-403`.
- Evidence: the new test (`:303-355`) seeds only `connection_id` rows, whose
  binding has `child: None` (`:38-46`). A k8s binding carries
  `child = Some("namespace:<ns>")` (`:15-26`).
  `grep -rn 'namespace:' crates/otto-server/tests` finds no session-list
  case. Two mutants still pass every test:
  - a memo key without `resource.child`, so one namespace's allow would show
    another namespace's sessions;
  - dropping `ctx.check_resources` from `visible_out`, so
    `GET /workspaces/{id}/sessions` would list revoked rows.

  FIXES.md P02 records both cases as not done.
- Fix:
  1. In the same unit test, add two `meta.k8s` sessions on one cluster, in
     namespaces `a` and `b`, under a policy that grants `exec` on
     `namespace:a` only. Assert that only the `a` row survives, whichever
     order the two rows come in.
  2. Add one HTTP case to `tests/projects.rs` or `tests/sessions_*.rs`
     through `build_router`: a workspace Admin without the connection's
     `shell` grant does not see the bound row in
     `GET /workspaces/{id}/sessions`.

### test-quality-3-06: minor. The scheduled-task settle retry has no test, and the stated reason (it "needs a fault-injection seam") does not hold

- File: `crates/otto-automation/src/scheduled_tasks_engine.rs:635-666`
  (`settle_with_retry`, `SETTLE_HOLD`), called at `:526` and `:582`.
- Evidence: this loop is the fix for error-handling-2-03. Without it, a
  failed cursor write re-ran and re-delivered the same occurrence every
  minute (comment at `:629-634`). FIXES.md P04 says it is untested because "a
  settle-failure test needs a fault-injection seam in otto-state".
  `grep settle_with_retry` finds no test.

  `settle_schedule` → `settle_generation` is a plain
  `UPDATE scheduled_tasks …` (`crates/otto-state/src/scheduled_tasks.rs:388, 404`).
  A SQLite trigger can fail it without any seam:
  `CREATE TEMP TRIGGER fail BEFORE UPDATE OF last_run_at ON scheduled_tasks BEGIN SELECT RAISE(ABORT,'busy'); END;`.

  Each of the following passes CI:
  - removing the retry loop;
  - making it give up after the first error;
  - dropping the final post-hold attempt.
- Fix: add a `#[tokio::test(start_paused = true)]` in the engine's
  `mod tests`:
  1. Install the trigger, and spawn `settle_with_retry` (make it
     `pub(crate)`).
  2. `tokio::time::advance` past two backoff steps, then drop the trigger.
  3. Advance again, and assert that `last_run_at` equals the served
     occurrence.

  Add a second case that keeps the trigger through all five attempts, drops
  it before `SETTLE_HOLD` ends, and asserts that the final attempt settles
  the cursor.
