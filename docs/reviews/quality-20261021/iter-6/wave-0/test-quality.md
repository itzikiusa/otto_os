# Test-quality review: iteration 6, wave 0 (2026-10-23)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (grep, sed, git log). No cargo, npm or e2e runs. Every finding
was confirmed by reading the code at the cited lines. Excluded: `target/`,
`node_modules/`, `docs/reviews/`. Branch `fix/quality-iter6-20261023` at
`02a64019b` (main after PR #121).

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 4 major, 2 minor, 0 nit.

10.0 − (4 × 0.3) − (2 × 0.1) = 10.0 − 1.2 − 0.2 = **8.6 / 10**

Eight of the ten iteration-5 findings are fixed, and the new tests are real:
each one was read to check it would fail without the code it covers. Two are
partly fixed. Their remaining parts are minors 6-05 and 6-06.

The four majors come from areas no earlier pass read. Each is the same
pattern: the helper is unit-tested, but the gate that wires it in is not
reached by any test.

- **Unauthenticated entry points:** webhooks, DB-Assist `q`, `/ingest/*`,
  plugin-host and the chat allow-list.
- **Database-change workflow:** the per-step authority checks.
- **Budget admission:** the measured "cap exceeded" path.
- **`review_run` gate:** scoring, threshold and multi-repo aggregate.

## Previous findings (iteration 5)

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-5-01 | **Fixed** | `mcp_auto_approve.rs:640-735` `a_server_bound_approval_needs_approve_access_on_the_server`: a server-bound card (`server_id: Some`) under an Enforced policy. `discover` only → 403, and the card stays `pending`. Adding `approve` → 200 `approved`. Deleting `require_resource` fails the first assertion. |
| test-quality-5-02 | **Fixed** | `db-drivers.yml:84-215` provisions the MySQL/Postgres fixtures and runs `--run-ignored only` over `otto-dbviewer`; the `not binary(resource_access)` filter is gone. The new ignored `mysql_cancellation_mid_run_revokes_the_executing_claim` (`resource_access.rs:701-`) writes the cancel only after the statement shows in `processlist`, so only the in-flight recheck can refuse it. (Verified on the first CI `db-drivers` run, as FIXES says.) |
| test-quality-5-03 | **Fixed** | `personal_agents_engine.rs:1137-1175`: `settle_cursor_with_retry` uses `SETTLE_HOLD` = 2 s under `cfg(test)`. Two trigger-driven tests (`:1424`, `:1452`) cover a transient failure and a final attempt after the hold. |
| test-quality-5-04 | **Fixed** | `tests/session_files.rs:58-64` calls `protect_dir_for_tests` on the temp data dir. In `a_home_cwd_session_cannot_reach_another_persona_folder` (`:284-311`), read and browse of another persona folder get 403, the owner's own read still gets 200, and LSP on the other persona dir gets 403. Both mutants from 5-04 fail it. |
| test-quality-5-05 | **Partly fixed** (rest is **6-06**) | `routes/api_client/automations.rs` `agent_run_step_refuses_a_script_retargeted_secret_and_a_person_run_sends_it` runs `run_step` with a binding. The pre-script retarget to B is refused with `.expect(0)` on B, and a person's run sends the bearer. That kills the "delete the block" and "move it above the pre-script" mutants. But the test builds `StepSecretBinding` itself, copying `api_automation_runs.rs:108-121`, so the `is_agent_caller → None` mutant in `start` still passes. |
| test-quality-5-06 | **Partly fixed** (rest is **6-05**) | `reusable_for` was extracted and unit-tested (`run.rs:1251-1285`: another user, another agent, archived). `find_agent_session` (`:225-235`) and the create-as identity (`:812`, `let creator: Id = acting_user`) still have no test. FIXES P06 lists this as not done. |
| test-quality-5-07 | **Fixed** | `nodes/git_pr_tests.rs:139-201` runs `git_pr` through `execute_node`. In the repo's own dirty checkout, HEAD and the untracked file are unchanged. A dirty `git worktree add` gets exactly one `chore: commit workflow changes` commit, and the main HEAD is untouched. R13 is tested on the pure `require_opened` (`:209-225`); its one call site is `git_pr.rs:346`. |
| test-quality-5-08 | **Fixed** | `channel_notify.rs:105` calls the pure `notify_targets`, and `:117` calls `reply_target`. Tests `:248-305` check that only the workspace's own integrations are kept, that webhook and blank-channel rows are dropped, and that a Slack thread is kept on Slack but a Telegram target gets the default chat. |
| test-quality-5-09 | **Fixed** | `mcp_auto_approve.rs:600-612`: bob, an Admin of `ws-bob`, now gets `[]` from the list and `count == 0`. `otto-state mcp_control.rs:2181-2215` inserts `other-ws` rows (including the caller's own) and asserts they are absent from both `list` and `stats`. |
| test-quality-5-10 | **Fixed** | The new `it` suite `tests/mcp_profiles_edges.rs` (`it.rs:45-46`) covers two edges. On the session PATCH: an agent change gets 403, an owner change gets 200, an agent round-trip gets 200, and agent narrowing gets 403. On `/mcp/http?profile=database`, `otto.k8s_logs` is hidden and the call is denied. The verifier fixed the bare-name bug that made the hidden-tool assertion always pass. |

## Areas covered

- **All ten iteration-5 findings:** re-verified (table above), and every new
  test was read to check it would fail without the code it covers.
- **Iteration 5's "not covered" list:**
  - `workflow_engine/nodes/review_run.rs` (925 lines): only the empty-diff
    path runs in a test (**6-04**).
  - `loop_node`: has the recovery and checkpoint tests
    (`workflow_recovery_tests.rs:18, 409, 632`). No finding.
  - `swarm_task`, `self_improve`, `product`: thin dispatchers; not deepened.
  - The classroom (`modules/home/school/**`): six unit suites (980 lines) were
    read. `school-actions.test.ts` covers kick-out, detention and release with
    fakes: confirm text, cancel, failure → restore, and `canManage=false` →
    nothing called. No finding.
  - Shooter and royale sims: eight shooter suites exist; not deepened (the
    games team is splitting these modules).
  - `otto-channels/src/bridge.rs`: 20 tests on the pure gates and the
    per-conversation locks. `Bridge::handle`'s allow-list gate is never
    reached with a stranger (part of **6-01**).
  - `otto-issues/src/atlassian.rs`: `rate_limit_wait` plus a loopback test.
    A 429 on a read retries once, and a write is never re-sent (`:391-433`).
    No finding.
  - Broker consumer-group reset 409: `reset_is_refused_for_groups_with_live_members`
    (`service.rs:2309`), and the replay drop guard is tested. No finding.
  - `patch_session` server-owned meta keys: `otto-sessions/tests/isolation.rs:857-875`
    checks that an agent token gets 403 on each confinement key, and
    `ui_control.rs` covers the grant. No finding.
- **New code from the iteration-5 fixes:**
  - `rooms/capability.rs`: issue/digest/malformed tokens, the TokenBucket
    clock and the protocol pair are all tested. No finding.
  - Game-room relay: host-only and guest-only commands, stale epochs, replays
    and capacity races (`game_rooms/tests.rs`, 12 tests). No finding.
  - Budget fail-closed (`budget/mod.rs:315-415`): config-unavailable and
    spend-unavailable are tested. The measured exceedance is not (**6-03**).
  - Swarm prompt-editor identity, the directive gate and the channel-trigger
    actor: `tests/swarm_identity.rs:177-415`. No finding.
  - The workflow `approve_run` agent-token refusal: `workflow_product_publish_tests.rs:113-`.
    No finding.
  - The UI helpers `relaySocket`, `termGridSync`, `storyReset`,
    `attachmentUrls`, `reviewFallback` and `userLabel` each have a unit suite.
- **Unauthenticated routes:** every route in `public_routes()` was mapped to
  its tests (**6-01**). The workflow webhook `/workflows/{id}/webhook/{token}`
  is tested (`run_repo_scope.rs:271-320`). Share/OTP has `share_otp.rs` and
  `share_scope_guard.rs`.
- **The database-change review workflow:** the repo is strongly tested
  (`database_changes_tests.rs`, 10 tests), and so are the `authorize`
  matrix and the executor gate (`resource_access.rs:337-`). The route layer
  is not tested (**6-02**).
- **Git PR merge:** `merge_pinned` wire tests for GitHub, GitLab and
  Bitbucket, and the fork-namesake cleanup (`wire_tests.rs:95-131`,
  `:1800-1870`). No finding.
- **e2e:** a script listed every `test(` body with no `expect`/`assert`.
  Two hits are the opt-in screenshot suite (`desktop-conversation-shots`,
  which documents this). The third, `desktop-home-classrooms-3d.spec.ts:266`
  "dark theme renders the school too", has only implicit checks: its
  `boot`/`enterOurRoom` helpers poll the room view. Not reported. Fixed
  `waitForTimeout` sleeps are already ratcheted by the `e2e-wait-timeout`
  rule in `ui-guards`.

## Areas NOT covered yet (next iteration)

- `otto-sessions/src/ws.rs` (3,887 lines) beyond the input-queue tests, and
  `otto-sessions/src/mcp.rs` (2,025 lines).
- `otto-telemetry-collector`: the recovery semantics.
- The `swarm_ingest` product, mockup and discovery-report handler bodies,
  beyond the token gate in 6-01 and `verified_swarm_scope`.
- `workflow_engine/nodes/{swarm_task, self_improve, product}.rs`.
- The AWS and Kubernetes consoles, Insights, skills-eval, otto-vault and
  otto-design.
- The shooter and royale sims, deferred while the game modules are split.
- The `apps/desktop/src-tauri` supervisor (55 tests, not read).

## Findings

### test-quality-6-01: major. The credential and sender gates on the unauthenticated entry points never run in a test with a missing or wrong credential

These routes sit in `public_routes()` (`routes/mod.rs:83-157`), outside the
bearer chokepoint. Each one validates its own credential in the handler. Every
helper is unit-tested, but no test sends one of these requests with a bad or
missing credential, so deleting the check passes every test.

| Entry point | Gate (file:line) | What is tested | What breaks silently |
|---|---|---|---|
| `POST /webhooks/{ws}` (channel webhook → agent session as root) | `channel_webhook.rs:146-170`: integration enabled, key configured, `extract_key` + `ct_eq` | `ct_eq_matches_only_identical_keys`, `extract_key_prefers_custom_header_then_bearer`, `repo_in_workspace`, `admit_webhook` (`:428-496`) | Deleting the `match extract_key` block, or the enabled check: anyone who can reach the daemon starts an agent session as root. |
| `POST /webhooks/{ws}/run` (Run with Otto as root) | `channel_webhook.rs:297-337` (a second copy of the same guard) | the same helpers | Same: the guard is pasted, not shared, so each copy can drift on its own. |
| `POST /webhooks/swarm/{ws}/{sid}` | `swarm_webhook.rs:126-155`: enabled, key, swarm in this workspace; `:165` `verify_cmd` refusal | `verify_cmd_from_the_webhook_is_detected`, `repo_path_must_be_a_registered_repo` (pure) | Deleting the key check, or the `s.workspace_id == ws` scope, which launches another workspace's swarm. |
| `POST /db-assist/{aid}/query` | `db_assist.rs:490-498` (`x-assist-key`), `:516` (`statement_is_write` read-only guard) | `statement_is_write` (otto-dbviewer), and that the `q` script includes the key (`:879-890`) | Deleting the key compare, or the read-only guard: anyone can run SQL (writes included) on the user's connection. |
| `/ingest/claude`, `/ingest/codex`, `/ingest/usage`, `/ingest/swarm/{board,product,mockup,discovery-report}` | `activity.rs:240, 421`; `usage.rs:533`; `swarm_ingest.rs:141, 242, 561, 696`, all calling `SessionManager::verify_ingest_token` (`otto-sessions/src/manager.rs:2260-2266`) | **Nothing**: `grep verify_ingest_token` finds only the definition and the seven call sites. | `verify_ingest_token` returning `true`, or dropping its `!token.is_empty()` guard: forged swarm-board posts, Product drafts, mockups, usage rows and activity events. |
| `/plugin-host/{repos,jira/accounts,jira/credentials,agents/run}` | `plugins.rs:907-927` `auth_plugin` (Bearer, non-empty, `find_enabled_by_token`) | the repo lookup, only for a rotated token (`plugins.rs:1239-1240`) | Dropping the `token.is_empty()` refusal, or the `auth_plugin` call in `host_jira_credentials` (`:1004-1012`), which hands out Jira `email:token` pairs. |
| Slack/Telegram sender allow-list | `otto-channels/src/bridge.rs:681` (`integration_admits` in `Bridge::handle`); Slack also at `slack.rs:1165` | `allowed_users_gate`, `a_blank_allow_list_admits_nobody_unless_opened` (`bridge.rs:1694-1760`, pure) | For a Telegram text message, `bridge.rs:681` is the only gate. Deleting it lets any stranger who can message the bot drive an agent in the user's repo. The one `Bridge` harness test (`telegram.rs:1017-1066`) uses the admitted user `"7"` only. |

Iteration 5 found the same pattern twice (5-04 and 5-05: the helper is tested
but the gate that wires it in is not). These are the pre-auth versions of it.

- **Fix:**
  1. In one new `it` suite (`tests/public_entry_gates.rs`), on a
     `ServerCtx::for_tests` daemon with a root user and an enabled webhook
     integration whose key is set in the test keychain, check:
     - all three webhook routes: no key, a wrong key, the right key on a
       disabled integration → 401/401/404, and no session, run or swarm run
       row is created;
     - the right key → 202. On the swarm route, the right key with another
       workspace's swarm id → 404.
     - `/db-assist/{aid}/query`: a registered assist with no key → 401, a
       wrong key → 401, and `UPDATE …` with the right key → the `read-only`
       error, and the driver is never called. A stub connection kind
       returns "not browsable" before any network.
     - `/ingest/swarm/board`: no `x-otto-token` → refused, a token minted
       for another session → refused, and no board row is written.
  2. Add a unit test in `otto-sessions` for `verify_ingest_token`: an empty
     token, an unknown session, another session's token and the right token.
  3. Plugin host: no `Authorization` → 401, `Bearer ` (empty) → 401, a
     disabled plugin's token → 401, on `/plugin-host/jira/credentials`.
  4. Allow-list: reuse the `telegram.rs:1017` harness. Send a text update
     from user `"8"` to an integration with `allowed_users: "7"`, then assert
     that `manager.list(...)` has no session and that `health` lists `"8"` as
     rejected.
  5. While there, make one `webhook_key_guard(ctx, ws, headers)` shared by
     all three webhook handlers, so the test pins one function.

### test-quality-6-02: major. The database-change review routes' per-step authority checks are never driven over HTTP; deleting the `change_approve` check on Approve passes every test

- Files:
  - `crates/otto-server/src/routes/database_changes.rs:209-229` (`approve`:
    `authorize(…, "change_approve")` at `:217`);
  - `:231-254` (`reject`, `:239`);
  - `:255-285` (`execute`: `change_execute` at `:264`, and the approver
    re-check `authorize(&approver, …, "change_approve")` at `:273`);
  - `:286-306` (`cancel`: running → `change_execute`, otherwise the author
    plus `change_submit`);
  - `:307-330` (`reconcile`);
  - `:100-130` (`detail`'s per-revision history filter, `:112-119`).
- Evidence:
  - `grep database-changes` in `crates/otto-server/tests` finds only
    `route_inventory.rs` and the policy snapshot. Nothing mounts
    `database_changes::api_router()` in a test.
  - `ui/e2e/desktop-database-changes.spec.ts` stubs every `/database-changes`
    call with `route.fulfill` (`:20-30`), so it checks only the UI.
  - What is tested:
    - the repo (`otto-state/src/database_changes_tests.rs`: independent
      approval vs impersonation, revision invalidation, atomic claims,
      recovery);
    - `authorize` (`otto-server/src/database_changes.rs:374-488`);
    - the executor gate (`otto-dbviewer/tests/resource_access.rs:337-`).
  - The repo's `transition(…, "approved", …)` enforces only approver ≠
    author. It has no resource policy, so it cannot know who holds
    `change_approve`.
  - `visible` (`database_changes.rs:152-168`) admits any holder of
    `change_submit`.
  - So each of these edits passes every test, and in each case a user
    granted only `change_submit` on a production connection can do more:
    - deleting `:217`: approve a colleague's DDL;
    - deleting `:273`: execute a change whose approver has since lost
      approval authority;
    - changing `:300`'s `op` to `change_submit`: cancel a running rollout;
    - deleting the `historical` check at `:117`: read SQL from earlier
      revisions that targeted connections they can't see.
- Fix: add an `it` suite `database_changes_api.rs` with a `ServerCtx` and
  two users on one `Prod` connection policy:
  - `author` holds `discover` and `change_submit`;
  - `reviewer` also holds `change_approve`.

  Flow: author creates, validates (executor = reviewer) and submits. Then:
  1. A third user `peer` with `change_submit` only: `POST …/approve` → 403,
     and the status stays `awaiting_review`. `…/reject` with a note → 403.
  2. `reviewer`: approve → 200.
  3. Remove `change_approve` from `reviewer`'s rule, then
     `author`/`executor` `…/execute` → 403 "approve", with no attempt row.
  4. `peer`: `GET …/{id}` on a change whose revision 1 targeted a
     connection `peer` can't discover → `history` holds no revision-1
     targets.

  All of this runs before any driver call; the executor gate already
  refuses before network.

### test-quality-6-03: major. Budget admission's measured "cap exceeded" path is never reached by a test; no caller is tested with a blocked verdict

- Files:
  - `crates/otto-server/src/budget/mod.rs:197-216` (`make_row`:
    `exceeded: used >= 1.0`, `warning: used >= 0.8`);
  - `:141-195` (`fold_budget_status`);
  - `:286-301` (`relevant && row.exceeded` → `blocked: cfg.block_on_exceed`).
  - Callers: `routes/goal_loops.rs:69-77, 88, 237, 424, 481`;
    `workflow_engine/nodes/budget_gate.rs:23-30`;
    `modules/review/routes.rs:367`; `swarm_host.rs:106`;
    `product_host.rs:480`.
- Evidence:
  - Both budget tests (`budget/mod.rs:320-415`) run with no usage engine,
    so `session_totals_for_budget` always errors.
  - They cover enforce off, an irrelevant cap, a zero cap, config
    unavailable and spend unavailable. No test feeds a non-empty
    `SessionTotals` slice.
  - `grep fold_budget_status|make_row|used_fraction` finds no other caller
    in tests.
  - `grep "budget exceeded|BUDGETS_KEY"` finds no caller-side test: no goal
    loop, review, swarm turn, product run or `budget_gate` node is ever
    started with a blocked verdict.
  - Each of these mutants passes every test:
    - `>= 1.0` → `> 1.0`: a cap is only enforced past 100%;
    - `relevant` → `true`: another workspace's overspend blocks this one;
    - `row.scope == "provider" && row.key == provider` → `false`: provider
      caps are never enforced;
    - deleting `goal_loops.rs:424`: a loop resumes over budget.

  The budget cap is the user's opt-in protection against runaway paid agent
  spend.
- Fix:
  1. Make the verdict pure:
     `fn verdict_from(cfg: &UsageBudgetConfig, totals: &[SessionTotals], ws: &str, provider: &str) -> BudgetVerdict`,
     with labels resolved later. Unit-test it:
     - spend exactly at the cap → `exceeded`, and `blocked` iff
       `block_on_exceed`;
     - 79% → no warning, 80% → warning;
     - workspace A's overspend → A blocked, B not;
     - a provider cap reached on `codex` → blocked for `codex`, not for
       `grok`;
     - `enforce: false` → never blocked.
  2. For one caller, store an unparsable budget config (the
     `config_unavailable_verdict` path already blocks with no ClickHouse).
     Assert that `POST /goal-loops/{id}/start` gets 400 "Budget exceeded",
     that the loop status is unchanged, and that a workflow run with a
     `budget_gate` node ends in `Error` with that reason.

### test-quality-6-04: major. The `review_run` gate's pass/fail logic is executed by a test only on an empty diff

- File: `crates/otto-server/src/workflow_engine/nodes/review_run.rs`:
  - `:378-384`: the score is 100 − 20·bugs − 5·warns − 5·infos, clamped,
    with per-node `scoring` overrides (`:167-172`);
  - `:450-453`: the goals blend `(review + goals) / 2`;
  - `:477`: `passed = complete && !no_changes && score >= threshold`;
  - `:478-490`: the reason text;
  - `:564-596`: the multi-repo aggregate (min score;
    `outs.len() == targets.len()` and every repo passed);
  - `:606-621`: `require_pass` → `_workflow_error`.
- Evidence:
  - The only test that runs the node is
    `workflow_recovery_tests.rs:146-215`. It runs on an empty diff and
    asserts `passed == false`, `no_changes == true` and "empty diff".
  - `review_coverage_complete` is unit-tested (`:227-237`). The mode
    precedence is tested (`unit/workflow_engine.rs:1782-1830`).
  - `grep '"score"|threshold'` in the tests finds no assertion on a scored
    review.
  - Each of these mutants passes every test:
    - `>=` → `>`: a review that scores exactly at the threshold fails;
    - `.all(passed)` → `.any(passed)`: one passing repo opens PRs for every
      repo;
    - dropping `outs.len() == targets.len()`: a target that never reviewed
      counts as a pass;
    - a bug weight of 2 instead of 20;
    - swapping min to max for `agg_score`.

  `passed` is what `loop` `until`, conditions and a following `git_pr` gate
  on. This is the quality gate between agent-written code and an opened PR.
- Fix: extract three pure functions next to `review_coverage_complete` and
  unit-test them:
  - `fn review_score(bugs, warns, infos, weights, no_changes) -> i64`:
    3 bugs → 40, 1 warn and 1 info → 90, weights from `scoring`, clamped
    at 0, and an empty diff → 0;
  - `fn review_verdict(status, coverage_complete, no_changes, score, threshold) -> (bool, String)`:
    a score at the threshold passes, threshold − 1 fails with
    "below threshold (79 < 80)", `status = "error"` → "did not complete",
    and coverage incomplete with status done → the coverage reason;
  - `fn aggregate(outs: &[Map], n_targets) -> (i64, bool)`: the worst score
    wins, one failing repo fails all, and fewer outputs than targets fails.

  `execute` then calls them, so the recovery test still pins the wiring.

### test-quality-6-05: minor. Rest of 5-06: the swarm turn's session reuse and create-as identity are not pinned

- File: `crates/otto-swarm/src/runtime/run.rs:225-235` (`find_agent_session`
  passes `user` to `reusable_for`), `:765` (called with `acting_user`),
  `:812` (`let creator: Id = acting_user.to_string()`).
- Evidence:
  - `reusable_for` is now unit-tested (`:1251-1285`).
  - `grep find_agent_session` finds no test caller, and no test records the
    `creator` passed to `manager().create`.
  - FIXES P06 lists this as not done.
  - Each of these still passes every test, and each runs Alice's task with
    Bob's MCP token, provider account and git identity:
    - passing `swarm.created_by` at `:812`;
    - calling `find_agent_session(…, &swarm.created_by)` at `:765`.
- Fix: in `tests/swarm_identity.rs` (which already builds the world), seed
  one live agent session that bob created for `agent_id = dev`. Assert that
  `find_agent_session(ctx, ws, "dev", "alice")` is `None` and that bob's
  lookup returns it (make it `pub(crate)` behind `cfg(test)` or move it to a
  `SwarmRt` method). For `:812`, a `SwarmHost` test double whose `manager()`
  records `create(.., creator, ..)` is enough: assert that the recorded
  creator equals `acting_user(&task)`.

### test-quality-6-06: minor. Rest of 5-05: the agent-run secret-binding test builds its own binding, so `start`'s choice to bind is untested

- Files: `crates/otto-server/src/routes/api_automation_runs.rs:108-121`
  (`if is_agent_caller(..) { Some(StepSecretBinding { .. }) } else { None }`);
  `crates/otto-server/src/routes/api_client/automations.rs:740-760` (the
  test: "the binding exactly as `start` builds it").
- Evidence:
  - The test duplicates the construction field by field, then calls
    `run_step` directly.
  - `is_agent_caller` is unit-tested (`tests/unit/api_client.rs:1001-1007`).
  - Changing `start` to bind `None` for agents, or building `bound_hosts`
    from an empty map, passes every test, because nothing calls `start` as
    an agent. Its only test is `validate`'s bounds (`:388-`).
- Fix: move the construction into
  `StepSecretBinding::for_caller(agent: bool, api, wid, vars, env, blob) -> Result<Option<Self>>`,
  call it from `start` and from the test, and add a one-line assertion that
  `for_caller(false, …)` is `None` and `for_caller(true, …)` binds the
  secret's host. That puts both halves of the decision under test without
  an `it` suite.
