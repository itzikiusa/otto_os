# Test-quality review — iteration 2 (2026-10-22)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, e2e specs that assert nothing meaningful.

Method: read-only (grep / sed / git log / git show). No cargo, npm or e2e
runs. Every finding below was confirmed by reading the code at the cited
lines. Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`,
`docs/reviews/`.

## Score

Start 10.0. Confirmed open findings: 0 blocker, 4 major, 3 minor, 0 nit.

10.0 − (4 × 0.3) − (3 × 0.1) = 10.0 − 1.2 − 0.3 = **8.5 / 10**

(Iteration 1 scored 8.7. Eight of its nine findings are fixed. The lower score
comes from going one level deeper into areas iteration 1 listed as not covered:
the swarm runtime, the state archive, MCP tool governance, and the batched
resource checks.)

## Previous findings

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-1-01 | Fixed | `crates/otto-server/tests/router_mount.rs:164-232` collects every matched probe that does not answer 401 and compares it both ways against the pinned `PUBLIC_ROUTES`. |
| test-quality-1-02 | Fixed (unit) | `ui/unit/uiCommandsWriteMemory.test.ts` runs the real `uiControl` store with in-memory storage: remember→skip, other scope or session still asks, guarded/outward never remembered, `session_meta_updated` off, `session_removed`, `setGrant(false)` and malformed storage. `confirmWrite` (`ui/src/lib/uiCommands.ts:520-523`) calls the same `writeScope` + `writesAllowed`. The optional e2e step was not added (FIXES.md P10). It is not counted as open, because the unit tests now fail on each regression the finding listed. |
| test-quality-1-03 | Fixed | `retiring_direct_credentials_…` now also runs `reconcile_managed_servers_agy` (`crates/otto-sessions/src/mcp.rs:1413`). `retiring_refuses_untracked_agy_entry` (`:1442-1459`) covers the refusal for preview and apply. |
| test-quality-1-04 | Fixed | `agent_cwd_confinement_edges` (`crates/otto-sessions/src/manager/tests/agent_cwd.rs:108`) covers: another workspace's repo, a worktree of a registered or unregistered repo, relative paths, and a symlink that escapes. |
| test-quality-1-05 | **Partly open** | The usage half is fixed (`desktop-usage.spec.ts` now uses `toBeVisible({timeout: 30_000})`). The session-header half is still open; it is carried forward as **test-quality-2-07**. |
| test-quality-1-06 | Fixed | `ui/unit/vaultStore.test.ts:248-259, 315-325` uses `t.mock.timers` with `tick(499)`/`tick(1)` and `tick(400)`. |
| test-quality-1-07 | Fixed | `crates/otto-server/tests/ui_control.rs:652-666` polls with a deadline. |
| test-quality-1-08 | Fixed | The logic moved to `ui/src/modules/insights/reportFrame.ts` and `ui/src/modules/plugins/pluginFrame.ts`, with behaviour tests in `ui/unit/frameMessages.test.ts`. `autoSecurityPins.test.ts` now pins only wiring and markup. |
| test-quality-1-09 | Fixed | `support_bundle_redacts_settings_and_audit` (`crates/otto-server/src/routes/capabilities.rs:729-799`) seeds a password, a bearer token, an email and a detail token, asserts none of them leak, and asserts a non-root caller gets 403. |

## Areas covered

- All nine iteration-1 findings were re-verified against the current code (table above).
- Tests added by the iteration-1 fix commit `66b55a25c` were audited for
  vacuity and timing:
  - `input_queue_tests.rs`: the `sleep(20/30ms)` hand-offs are deterministic on the current-thread runtime, so they are not flaky;
  - `concurrent_submits_do_not_interleave`;
  - `concurrent_creates_of_one_key_dedupe_atomically`: a real 6-connection WAL pool, so the race really runs;
  - `attach_after_cancel_replaces_the_dying_tailer`;
  - `a_crashed_sidecar_is_reaped_and_unpublished`: finding 2-05;
  - `server_update_is_one_statement_that_keeps_omitted_fields`;
  - `brokersStoreScope.test.ts`;
  - `focusReturnAndPollDispose.test.ts`;
  - the support-bundle test;
  - the share revoke tests;
  - the `input_user` authority test.
- FIXES.md items with no "Test:" line, checked one by one:
  - InputAuthority batching (backend-performance-1-07): finding 2-04;
  - mcp_control COALESCE PATCHes: finding 2-03;
  - the Codex env file lifecycle (integrations-1-04): finding 2-06;
  - the plugin watcher generation check (concurrency-async-1-04): finding 2-05;
  - `expand_home`, the blob torn-write repair, improvement backup/rename and design project delete: all have tests.
- From the "not covered" list of iteration 1:
  - `otto-mcp/src/outward/exec.rs` `route_for`: well covered by `outward/tests.rs` (~60 `route_for` assertions, including missing-argument and unknown-tool refusals). No finding.
  - `state_archive/schema.rs` and `state_archive.rs`: finding 2-02.
  - Workflow engine recovery: `classify_resume` (about 20 cases) and `sweep_orphaned_runs` are tested in `tests/unit/workflow_engine.rs`. No finding.
  - Swarm runtime (`engine.rs` 3,500 lines, 6 tests): finding 2-01.
  - `swarm.svelte.ts` / `brokers.svelte.ts`: brokers now has scope tests. The swarm store lost its graph pipeline, and what remains is CRUD glue (low risk).
- Security commits from the last 3 weeks:
  - proof assemble needs a person (`assemble_commands_need_a_person`, plus `is_human` tests covering the share scope);
  - goal-loop temp-file confinement (`tmp_files_stay_directly_in_the_temp_dir`);
  - git option-like refs (`argv_guard_tests.rs`);
  - kubeconfig exec-plugin allow-list and owner opt-in (`kubeconfig_policy.rs`, 9 tests, plus `fake_kubectl.rs`);
  - Linear tracker (`linear.rs` mapper tests plus `tests/linear_client.rs` for paging and clamping).
- Ignored Rust tests: the live-DB ones run in `db-drivers.yml`, the benches in `nightly-bench.yml`, and the telemetry ones in `ci.yml:181`. None is orphaned.
- E2E gate specs (`ui/e2e/gate-specs.ts`): scanned for instantaneous `isVisible()`/`count()` guards and swallowed catches. The only hit (`desktop-rpanel-tabs.spec.ts:230`) is followed by an unconditional click and assertion, so it is harmless.

## Areas NOT covered yet (next iteration)

- `crates/otto-swarm/src/runtime/verify.rs` (1,968 lines, 22 tests) and
  `scheduler.rs`: check what the tests actually assert on the
  verifier→merge path and the scheduler's admission and recovery.
- `crates/otto-mcp/src/http.rs`, `otto-k8s/src/monitor/http.rs`,
  `otto-brokers/src/http.rs`, `otto-vault/src/http.rs`: large route files
  (still carried from iteration 1).
- `otto-product` watcher and studio turns (still carried).
- The rest of the 54 UI unit files that read `src/` as text: only
  `autoSecurityPins`, `focusReturnAndPollDispose` and `brokersStoreScope` were read.
  The biggest regex-heavy ones are `design-primitives.test.ts` (29 matches),
  `siteStudio.test.ts` (22), `journeyFixes.test.ts` (17) and
  `transcriptLifecycle.test.ts` (15).
- `crates/otto-dbviewer/src/access.rs` `check_many` (tree, search and ERD
  filtering): its tests in `tests/resource_access.rs` were not traced.
- UI stores `git.svelte.ts` (beyond auto-fetch/ownership),
  `transcript.svelte.ts`, `sftp.svelte.ts`, `clipHistory.svelte.ts`.
- The session handover, assistant `tasks.rs`, otto-review `session.rs` and
  `claude_pty.rs` paste-then-Enter injectors that FIXES.md says could take
  `submit_lock`.

## Findings

### test-quality-2-01 — major — the swarm result router (who wins, retries, the hop cap) has no test

- File: `crates/otto-swarm/src/runtime/engine.rs:1009-1435` (`route_result_inner`),
  plus `attempt_ceiling_reached` (`:1544`), `block_for_attempts` (`:1562`),
  `task_hops` (`:1436`), `MAX_HANDOFF_HOPS` (`:1429`) and
  `MAX_OPEN_TASKS_PER_PROJECT` (`:1433`).
- Evidence: this 430-line state machine decides what happens to a task after
  every swarm turn. Its comments record incidents it exists to prevent:
  - "a failed turn used to resurrect a cancelled task as `todo`"
    (`:1022-1026`);
  - "re-queueing it as `todo` made Stop behave like a restart on the next
    tick" (`:1053-1056`);
  - "an unmanaged A↔B handoff loop once inflated a 28-task plan to 150
    mostly-blocked tasks" (`:1168-1177`).

  `rg 'route_result|MAX_HANDOFF_HOPS|task_hops|attempt_ceiling_reached|block_for_attempts'`
  finds no test anywhere in the workspace. `engine.rs` has six tests in total,
  covering budgets, delete-halting, the waiting map and a source scanner. None
  of the following has a test:
  - the operator's move or reassign winning (`:1027-1051`);
  - a pause refunding the attempt and parking the task as `todo`, while a
    Stop parks it as `blocked` (`:1057-1087`);
  - retry versus block at the ceiling (`:1090-1114`);
  - planning runs with and without subtasks (`:1137-1166`);
  - a managed IC's proposals turning into ONE manager triage task (`:1186-1217`);
  - handoffs past `hops > 3` escalating instead of creating a task (`:1225-1228`).

  The only nearby test is `refund_task_attempt_never_goes_negative` in
  otto-state, which tests the repo call and not the routing. Any of these
  regressions would pass CI:
  - flipping `"todo"`/`"blocked"` at `:1067`;
  - dropping the `reassigned` guard;
  - changing `hops <= MAX_HANDOFF_HOPS` to `<`;
  - removing the `create_agent_task` dedup.
- Fix: add an otto-server suite (`ServerCtx::for_tests` implements
  `SwarmHost`, `crates/otto-server/src/swarm_host.rs:55`) with a seeded swarm,
  manager, IC and task. Drive `route_result_inner` with a hand-built
  `SwarmTurnResult` and assert the task row and posted messages for each case:
  1. The task was moved to `cancelled` mid-turn. Assert the status stays
     `cancelled`, there is no retry, and one "left as you set it" post exists.
  2. `result=None` with a run stopped with `PAUSED_RUN_REASON`. Assert the
     status is `todo` and `attempts` decremented. A plain Stop gives `blocked`.
  3. A failed turn below or at `max_attempts`. Assert `todo` below the
     ceiling and `blocked` at it.
  4. An IC with `reports_to` returns two subtasks. Assert exactly one triage
     task, assigned to the manager, labelled `hops:1`.
  5. A task labelled `hops:3` returns a handoff. Assert no new task and one
     escalation post.
  6. A second identical handoff. Assert it is deduped.

### test-quality-2-02 — major — the archive / Git-sync export's secret-settings filter is never exercised; the test helper re-implements the export loop

- Files: `crates/otto-server/src/state_archive.rs:157-170` (production export
  loop), `crates/otto-server/src/state_archive/tests.rs:6-31` (test helper
  `snapshot()`), `crates/otto-server/tests/foundation_integrity.rs:262-282`,
  `crates/otto-server/src/routes/backup_git.rs:680-689`
- Evidence: `build_snapshot_parts` drops every `settings` row whose key is
  secret-named (`rows.retain(|row| !…is_some_and(schema::secret_key))`,
  `state_archive.rs:163-168`) before `sanitize`. That retain is the only
  defence for a plain-string secret setting such as
  `slack_bot_token = "xoxb-…"`, because:
  - `sanitize` → `scrub_json` → `scrub_string` (`schema.rs:175-244`) removes
    only URL passwords and secret query keys, `keychain:`/`secret:` refs, PEM
    blocks and `Bearer`/`Basic` prefixes;
  - for a JSON *string* value, `scrub_json` (`schema.rs:288-322`) never looks
    at the row's key.

  The export leaves the machine: `backup_git.rs:681` pushes a portable
  `build_snapshot` to a Git remote, and `GET /state/archive` returns it.

  The unit tests never call the production function. The `snapshot()` helper
  in `state_archive/tests.rs:6-31` copies the loop without the settings
  `retain`. The `do-not-export` password-hash check
  (`tests.rs:53-57`) therefore passes through the column-name path only. The
  tests that do call `build_snapshot` (`foundation_integrity.rs:262-282`,
  `backup_git.rs:1137-1147`) seed no secret at all
  (`rg 'secret|token|password' backup_git.rs` finds no assertion). Deleting
  the `retain`, or narrowing `secret_key`'s settings use, passes every test.
- Fix:
  - Delete the duplicated `snapshot()` helper and call
    `build_snapshot_parts(&pool, tmp, "test", SnapshotOptions{portable, include_files:false})`
    in its place, so the tests run the production loop.
  - Add one test that, for both `portable: true` and `portable: false`, seeds:
    - `settings('slack_bot_token', '"xoxb-plain"')`;
    - `settings('openai_api_key', '"sk-plain"')`;
    - `settings('mailer', '{"smtp":{"password":"p"}}')`;
    - a `users` row with `password_hash`;
    - an `api_tokens`/`auth_sessions` row.

    Assert that `serde_json::to_string(&archive)` contains none of the raw
    values, and that a harmless setting (`default_provider`) survives.

### test-quality-2-03 — major — "rediscovery never lowers a human-pinned risk label" is untested, and a pinned `dangerous` is what keeps the approval gate on

- Files: `crates/otto-state/src/mcp_control.rs:669-703` (`McpToolsRepo::patch`
  sets `risk_overridden = CASE WHEN ? OR ? THEN 1 …`),
  `:588-640` (`upsert_discovered`, `overridden` branch `:604-611`),
  `crates/otto-mcp/src/service_preflight.rs:80`, `crates/otto-mcp/src/service.rs:825`
- Evidence: the approval gate fires when
  `tool.require_approval || (tool.risk_label == "dangerous" && require_approval_dangerous())`
  (`service_preflight.rs:80`). An operator who re-labels a third-party tool
  `dangerous` through PATCH relies on `risk_overridden=1` to keep that label.
  `upsert_discovered` runs on every discover/refresh (`service.rs:546, 1448`).
  On a non-overridden row it overwrites `risk_label`/`injection_risk` with the
  server-advertised values (`:612-618`).

  `rg upsert_discovered` finds only a fixture call in `service.rs:1634`, and
  `rg risk_overridden` finds no test. The only `patch` call in a test
  (`crates/otto-mcp/tests/pipeline_e2e.rs:270`) passes
  `(Some(false), None, None, None)`, which disables a tool. It never sets a
  label and never rediscovers.

  Any of the following passes every test and silently removes a
  human-imposed approval gate on the next refresh:
  - swapping the `CASE` binds;
  - dropping the `risk_overridden` column from the CASE;
  - inverting `if overridden`.

  The iteration-1 COALESCE rewrite of this exact statement
  (data-integrity-1-04) shipped with a test for `McpRegistryRepo::update`
  only, not for `McpToolsRepo::patch` or the policy `update` (`:885-925`).
- Fix: in `mcp_control.rs` tests:
  1. `upsert_discovered` a tool with `risk_label:"read"`.
  2. `patch(id, None, None, Some("dangerous"), None)`.
  3. Assert `risk_overridden` and `dangerous`.
  4. `upsert_discovered` again with `"read"` and a new description. Assert
     the label is still `dangerous` and the description is refreshed.
  5. Control: a second, un-pinned tool does take the new label.

  Add the same omitted-field check for `patch` (`enabled` only leaves
  `require_approval` and the labels as they were) and for the policy `update`.
  Optionally, in `pipeline_e2e.rs`, re-run `svc.discover` after a pin, then
  assert `invoke` returns `PendingApproval`.

### test-quality-2-04 — major — the batched resource filter that hides revoked k8s/AWS/SSH sessions from session lists has no test of its resource-bound branch

- Files: `crates/otto-server/src/resource_sessions.rs:100-146` (`check_many`),
  used as the only filter by `crates/otto-sessions/src/http.rs:391-403`
  (`visible_out`, behind `GET /sessions` and the workspace session list, via
  `SessionsCtx::check_resources`, `crates/otto-server/src/modules.rs:253-259`)
  and by `crates/otto-server/src/routes/projects.rs:111`
- Evidence: F9 rewrote the per-row `check` into `check_many`, which does two
  things:
  - it reloads the caller once (`:114-121`; a failed reload denies every row);
  - it memoizes the decision per `(kind, id, child, op)` (`:125-139`).

  For list routes this is the only place a session whose bound
  resource (SSH connection, k8s cluster or namespace, AWS account, db_assist
  connection) was revoked gets hidden. An Admin's workspace scope includes
  every user's rows (`http.rs:370-374`).

  The only test that reaches `check_many`,
  `sessions_page_shows_owners_their_rows_and_admins_all`
  (`projects.rs:127-235`), seeds agent rows with no binding and one malformed
  `aws` binding. Both take the `None => check_with_pool` arm (`:110`). The
  `Some((resource, op))` arm, which holds the memo, the disabled-account path
  and `check_bound`, is never executed by any test (`rg check_many` shows no
  other test caller). The per-session `input_user` test
  (`modules.rs:7274-7414`) covers `check_with_pool`, not this batched path.

  The following would pass every test:
  - a memo key that drops `child`, so one namespace's allow leaks to another;
  - `None => true` at `:121`, so a disabled or deleted caller sees every
    bound row;
  - an inverted `ok`.

  (Broadcast and relay also use `check_many` through `input_agents`, but
  `submit_session_text` re-checks `input_user` per session at
  `modules.rs:134-137`, so that path keeps a tested guard. The list routes do
  not have one.)
- Fix: extend the projects test, or add a `resource_sessions` unit test,
  with:
  - two k8s-bound sessions on the same cluster, one in namespace `a` and one
    in namespace `b`, with a policy that allows only `a`;
  - one `connection_id` SSH session whose connection grant is revoked.

  Assert that `check_many` returns exactly the `a` row and the unbound rows,
  in order. Then disable the caller and assert every bound row is dropped
  while unbound rows remain. Add one HTTP-level case through
  `GET /workspaces/{id}/sessions` as a workspace Admin who lacks the
  connection's `shell` grant.

### test-quality-2-05 — minor — the plugin watcher's generation guard is never reached by the test that claims to cover it

- File: `crates/otto-server/src/plugins.rs:255-296` (`adopt`), test
  `a_crashed_sidecar_is_reaped_and_unpublished` `:1243-1257`
- Evidence: the second half of the test says "A newer spawn's entry survives
  an older watcher's exit". It adopts `again` with a child that sleeps 0.2 s,
  then immediately adopts `again` again with a 60 s child. The second
  `map.insert` (`:287`) returns and drops the old `RunningPlugin`, which drops
  its `stop` sender. The old watcher's `select!` (`:266-283`) then takes the
  `_ = stop_rx` arm at once, because its child is still sleeping. That arm
  kills the child and never touches the map.

  The guarded arm, `map.get(&owner).is_some_and(|r| r.generation == generation)`
  (`:269`), runs only when the old child's `wait()` wins. In this test it never
  does. Replacing `:269-271` with an unconditional `map.remove(&owner)` still
  passes, and the fixed `sleep(600ms)` (`:1256`) cannot reveal it.
- Fix: make the old child exit before the replacement is published:
  1. Spawn `exit 3`.
  2. On a current-thread runtime, wait for the process to exit with a
     blocking `std::thread::sleep` or a `waitpid`, without yielding.
  3. Adopt it, so that both `wait()` and the stop signal are ready.
  4. Adopt the fresh child.
  5. Yield until the old watcher's `JoinHandle` finishes (keep the handle and
     await it). Do not use a wall-clock sleep.
  6. Assert `port_of("again") == Some(2)`.

  Alternatively, factor the "remove only my generation" step into a small
  function and unit-test it with a stale generation.

### test-quality-2-06 — minor — no test checks that the plaintext credential files on disk die with the session

- File: `crates/otto-sessions/src/manager.rs:180-200`
  (`codex_server_env_path`, `remove_codex_server_env_files`), `:2834-2841`
  (spawn-time cleanup), `:6708-6716` (`revoke_session_credentials`)
- Evidence: integrations-1-04 moved Codex user-server env values, Keychain
  secrets included, into `<tmp>/otto-mcp/<sid>.<server>.env`.
  `write_codex_creds` already writes the session's MCP bearer token to
  `<tmp>/otto-mcp/<sid>.json`. The comments promise that these files are
  removed with the session's credentials, and that a server disabled since
  the last spawn "must not leave its secrets on disk".

  The tests check only the write side: mode 0600 in
  `codex_user_server_env_stays_off_argv`, `mcp.rs:1275-1276`, and
  `codex_creds_preserve_session_source_for_mcp_policy`,
  `manager/tests/mod.rs:190`. `removing_session_revokes_persisted_tokens_…`
  and `retiring_a_session_revokes_its_managed_credentials`
  (`manager/tests/mod.rs:1264-1310`) assert only that the DB token stops
  authenticating. `rg 'codex_creds_path|remove_codex_server_env_files|\.env' manager/tests`
  finds nothing. Both of the following pass every test and leave secrets in
  `/tmp`:
  - dropping the `remove_codex_server_env_files` calls;
  - a prefix or suffix typo in the matcher (`:193`, `:198`), such as matching
    `"{sid}"` without the dot, which would also delete a different session's
    files.
- Fix: in `retiring_a_session_revokes_its_managed_credentials` (and the
  remove test):
  1. Write `write_codex_creds(&id, …)` and two
     `write_env_file(&codex_server_env_path(&id, "a"|"b"), …)`.
  2. Also write `codex_server_env_path(&other_id, "a")`.
  3. Assert that after archive, suspend, kill or remove, the three files for
     `id` are gone and the `other_id` file remains.

### test-quality-2-07 — minor — the session-header details chip is still asserted only when it happens to be visible (carried from 1-05)

- File: `ui/e2e/desktop-session-header.spec.ts:181-189`
- Evidence: the check is still
  `if (await chip.isVisible()) { …title / Folder / Copy folder path assertions… }`.
  `isVisible()` does not wait, and the guard skips every chip assertion when
  the chip is missing. The chip renders unconditionally
  (`ui/src/modules/agents/SessionView.svelte:865-875`). It is hidden only by
  the `@container pane (width < 420px)` rule (`:1516-1518`). The spec already
  knows each pane's tier: it asserts `tiers` equals `['full']` for one pane
  (`:142`). So for `count === 1` the chip must be visible, and the guard turns
  a regression that drops or hides the chip into a pass. FIXES.md P5 lists
  this as not done.
- Fix: for the panes the spec asserts are `full`/`compact` (`count` 1 and 2),
  `await expect(chip).toBeVisible()` and run the assertions unconditionally.
  For `count === 6` (`minimal`), `await expect(chip).toBeHidden()`.
