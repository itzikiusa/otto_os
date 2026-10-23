# Test-quality review: iteration 6, wave 1 (2026-10-23)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (grep, rg, sed, git diff). No cargo, npm or e2e runs. Branch
`fix/quality-iter6-20261023` at `37738d908`. The scope is the same as wave 0:
the areas listed below, plus the lens-relevant parts of `git diff origin/main`.
Every finding below was confirmed by reading the cited lines. For each new
test I checked one question: would deleting the code it covers make it fail?

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 0 major, 6 minor, 0 nit.

10.0 − (6 × 0.1) = 10.0 − 0.6 = **9.4 / 10** (wave 0: 8.6)

All four wave-0 majors are fixed or reduced to a minor remainder:

- **6-03 and 6-06:** fully fixed.
- **6-01, 6-02 and 6-04:** fixed for every mutant the wave-0 fix list
  named. Some listed locations are still unpinned. These remainders are
  minors **6w1-01**, **6w1-03** and **6w1-04**.
- **6-05:** half done. The remainder is **6w1-05**.

The fixes introduced two problems:

- **6w1-02:** the new swarm-board ingest test cannot fail.
- **6w1-06:** several new UI tests check the component source with
  regular expressions, so they pass even when the behaviour breaks.

## Previous findings (wave 0)

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-6-01 | **Partly fixed** (rest is **6w1-01**; the new board test is **6w1-02**) | **Fixed:** <ul><li>One `webhook_key_guard` (`channel_webhook.rs:134-176`) is used by the channel, run and swarm handlers (`:189`, `:319`, `swarm_webhook.rs:97`).</li><li>`tests/public_entry_gates.rs:92-233`: no key, a wrong key, an empty key and a wrong bearer get 401. A disabled integration gets 404. The right key reaches the next check (400/503). Another workspace's swarm gets 404. No session, run or swarm project is created. Deleting the guard, or its `enabled` check, fails this test.</li><li>DB-Assist (`:235-303`): no key, an empty key and a wrong key get 401. An unknown assist gets 404. `UPDATE` with the right key gets the `read-only` error. Deleting the guard sends the call to the driver on port 1, which gives a different error.</li><li>`/plugin-host/jira/credentials` (`:362-410`): bearer cases.</li><li>Telegram: a stranger is listed as rejected (`otto-channels/src/telegram.rs:1067-1135`). Deleting `bridge.rs:681` leaves the stranger unlisted, so the test fails.</li><li>`verify_ingest_token` has a unit test (`otto-sessions/src/manager/tests/mod.rs:4127-4155`).</li></ul> **Still open:** <ul><li>The seven `verify_ingest_token` call sites.</li><li>`auth_plugin` in three of the four plugin-host handlers.</li></ul> |
| test-quality-6-02 | **Partly fixed** (rest is **6w1-03**) | `tests/database_changes_api.rs` pins the approve check (`:217`) and the reject check (`:239`): `peer` gets 403 and the status stays `awaiting_review`. It pins the executor's approver re-check (`:273`): after the approver's grant is revoked, execute gets 403 and no attempt row is written. `fresh_approval` (`database_changes.rs:221-243`) does not re-check the approver, so the 403 can only come from `:273`. It also pins the revision-1 history filter (`:112-119`). Still unpinned: `cancel` (`:286-306`, `op` at `:294`) and `reconcile` (`:307-330`, `:315`). Both were listed in 6-02. |
| test-quality-6-03 | **Fixed** | `verdict_from` and `budget_rows` (`budget/mod.rs:162-328`) are pure, with five tests (`:438-505`): <ul><li>exactly at the cap: exceeded, and blocked only when `block_on_exceed` is set (`>=`→`>` fails);</li><li>79% vs 80% warning;</li><li>another workspace's overspend does not block this one (`relevant`→`true` fails);</li><li>a provider cap blocks only that provider (`→false` fails);</li><li>enforcement off never blocks.</li></ul> Caller tests: <ul><li>`goal_loops.rs:746-815` drives `start` (the `:424` gate wave 0 named) and gets 400 "Budget exceeded", with the loop still a Draft;</li><li>`tests/unit/workflow_engine.rs:1972-2040`: a `budget_gate` run ends in Error, and the next node is Skipped.</li></ul> |
| test-quality-6-04 | **Partly fixed** (rest is **6w1-04**) | `review_score`, `review_verdict`, `goals_from_reply` and `aggregate` are pure (`review_run.rs:54-196`) and covered by nine `review_gate` tests (`tests/unit/workflow_engine.rs:1836-1970`). They kill every mutant wave 0 named: at the threshold, `.any`, the `outs.len()` drop, the weights, and min→max. `require_pass` → `_workflow_error` is pinned by `workflow_recovery_tests.rs:147-205`. Still unpinned: <ul><li>the goals blend `(review_score + gs) / 2` (`:830`, the old `:450-453`);</li><li>the per-node `scoring` parse `ScoreWeights::from_params` (`:34-47`, the old `:167-172`).</li></ul> |
| test-quality-6-05 | **Partly fixed** (rest is **6w1-05**) | `tests/swarm_identity.rs:637-664` `a_turn_reuses_only_its_acting_users_session` drives the real `find_agent_session`: alice gets `None`, bob gets his session, and another agent gets `None`. Still unpinned: the call site passing `acting_user` (`run.rs:805`) and the creator (`:852`). FIXES P08 lists "creator double" as not done. |
| test-quality-6-06 | **Fixed** | `StepSecretBinding::for_caller` (`api_client/automations.rs:138-158`) is the one constructor. The route calls it at `api_automation_runs.rs:108-116`, and the test at `:778-800`, asserting `None` for a person and a bound secret host for an agent. The only unpinned part is the one-argument `is_agent_caller(..)` at `:109`. That is plain wiring, not a decision, so it is not reported. |

## Areas covered

These are the same as wave 0.

- **All ten iteration-5 findings:** re-verified in wave 0. No regressions
  were found in this pass: `mcp_auto_approve.rs`, `session_files.rs`,
  `mcp_profiles_edges.rs`, `nodes/git_pr_tests.rs` and `channel_notify.rs`
  are unchanged by the diff, or changed only in ways outside this lens.
- **Iteration 5's "not covered" list:**
  - `workflow_engine/nodes/review_run.rs`: 6-04 (above).
  - `loop_node`.
  - `swarm_task`, `self_improve`, `product`: thin dispatchers.
  - The classroom unit suites.
  - Shooter and royale sims: not deepened.
  - `otto-channels/src/bridge.rs`: allow-list gate (above).
  - `otto-issues/src/atlassian.rs` retry.
  - Broker consumer-group reset.
  - `patch_session` server-owned meta keys.
- **New code from the iteration-5 fixes:**
  - `rooms/capability.rs`.
  - Game-room relay.
  - Budget fail-closed and the measured path (6-03, now fixed).
  - Swarm prompt-editor identity, the directive gate and the
    channel-trigger actor: the new `swarm_identity.rs` tests
    (`:18-230` of the diff) assert the 403s, the "no agent left behind"
    check, editor stamps and rollback on a failed stamp. No finding.
  - The workflow `approve_run` agent-token refusal.
  - The UI helper suites.
- **Unauthenticated routes:** every route in `public_routes()` was
  mapped to its tests again (6-01 above; **6w1-01**, **6w1-02**).
- **The database-change review workflow:** the routes are now driven over
  HTTP (6-02 above; **6w1-03**).
- **Git PR merge:** `merge_pinned` wire tests. No change, no finding.
- **e2e:**
  - Seven specs changed. Each change moves a `getByRole('status')`
    locator to visible text, because status lines became plain text and
    the shell announcer now speaks them (a11y-6-01). Every assertion still
    checks the same visible outcome, and none was weakened.
    `desktop-api-history-performance.spec.ts:91-93` excludes the
    `aria-live` region on purpose.
  - No new spec has a `test(` without an assertion.
- **The lens over `git diff origin/main`:** this covers the new Rust
  suites (`public_entry_gates.rs`, `database_changes_api.rs`, the
  `swarm_identity.rs` additions, `review_gate`, the budget tests) and the
  24 new `ui/unit` files. Eight of those 24 read component source with
  `readFileSync` (**6w1-06**).

## Areas NOT covered yet (next iteration)

These are unchanged from wave 0.

- `otto-sessions/src/ws.rs` beyond the input queue, and
  `otto-sessions/src/mcp.rs`.
- `otto-telemetry-collector` recovery semantics.
- The `swarm_ingest` product, mockup and discovery-report handler bodies.
- `workflow_engine/nodes/{swarm_task, self_improve, product}.rs`.
- The AWS and Kubernetes consoles, Insights, skills-eval, otto-vault and
  otto-design.
- The shooter and royale sims.
- The `apps/desktop/src-tauri` supervisor.

## Findings

### test-quality-6w1-01: minor. Rest of 6-01: the ingest-token calls and three plugin-host `auth_plugin` calls still aren't covered by any test

- **Ingest routes.** `verify_ingest_token` is called at seven sites:
  - `routes/activity.rs:240` (`/ingest/claude`) and `:421`
    (`/ingest/codex`);
  - `routes/usage.rs:533` (`/ingest/usage`);
  - `routes/swarm_ingest.rs:141` (board), `:242` (product), `:561`
    (mockup) and `:696` (discovery-report).

  The function itself is now unit-tested. No test sends a request to any of
  these routes in a way that fails if the call is removed. The board test
  is the only HTTP case, and it cannot fail either (**6w1-02**).
  `rg "/ingest/" crates/otto-server/tests` finds only that test and the
  `router_mount.rs` inventory.
- **Plugin-host routes.** `plugins.rs:938` (`host_repos`), `:967`
  (`host_jira_accounts`) and `:1060` (`host_agents_run`) each call
  `auth_plugin`. Only `host_jira_credentials` (`:1009`) is driven, by
  `public_entry_gates.rs:362-410`. The rotated-token test (`plugins.rs:1193-1240`)
  asserts on `find_enabled_by_token` directly, not on a route.
- **What breaks silently:**
  - Deleting `:1060` lets anyone who can reach the daemon start a
    Claude/Codex agent with any prompt in any `cwd`.
  - Deleting a `swarm_ingest.rs` call lets a forged request post Product
    drafts, mockups or discovery reports into a swarm.
- **Fix:**
  1. In `public_entry_gates.rs`, loop the plugin case over all four
     host routes:
     - no `Authorization` and a disabled plugin's token → 401;
     - `/agents/run` with `{"prompt":"x"}` → 401 and no orchestrator
       call. The 401 comes before any agent is spawned.
  2. For ingest, build the real swarm-session fixture from **6w1-02** once
     and drive `/ingest/swarm/{board,product,mockup,discovery-report}` with
     another live session's token. Assert that nothing is written.
  3. Do the same for `/ingest/usage` (no usage row) and `/ingest/claude`
     (no activity event).

### test-quality-6w1-02: minor. The new swarm-board ingest test passes with the token check deleted

- File: `crates/otto-server/tests/public_entry_gates.rs:306-360`.
- Evidence:
  1. Every case uses `x-otto-session: sess-1`, but no session with that
     id exists in the fixture.
  2. In `board_ingest` (`swarm_ingest.rs:128-200`), the line after the
     token check is `ctx.manager.get(&sid)` (`:144-147`). For `sess-1`
     that fails, and the handler returns 204.
  3. So with `swarm_ingest.rs:141-143` deleted, all four cases still
     return 204 and write no board row. Steps 1 and 2 decide the result,
     not the token check.
  4. The status assertion `assert_eq!(status, 204, "{label}: refused silently")`
     (`:349`) also pins nothing, because an accepted post returns 204 too
     (`:200`).
  5. The suite's header (`:1-13`) says it pins the ingest board. It only
     shows that an unknown session is refused.
- Fix:
  1. Create a real agent session for a swarm agent: `SessionsRepo::create`
     with meta `{swarm_id, agent_id, workspace_id}`, as
     `a_turn_reuses_only_its_acting_users_session` does.
  2. Mint its token through the manager's ingest env
     (`OTTO_INGEST_TOKEN`), and mint a second token for another session.
  3. Assert:
     - no token, an empty token, and the other session's token → still no
       row in `list_board`;
     - the session's own token → exactly one row.

  The positive case is what makes the negative cases count. Without it,
  the test cannot tell whether a post is ever accepted.

### test-quality-6w1-03: minor. Rest of 6-02: the cancel and reconcile authority checks aren't covered by any test

- File: `crates/otto-server/src/routes/database_changes.rs`:
  - `:286-306` (`cancel`): a running change needs `change_execute`
    (`:294-295`); otherwise only the author with `change_submit` may
    cancel;
  - `:307-330` (`reconcile`): `change_execute` at `:315`.
- Evidence: `tests/database_changes_api.rs` never calls `…/cancel` or
  `…/reconcile` (`rg "cancel|reconcile"` on it finds nothing). The repo
  tests do not know the resource policy. So each of these edits passes
  every test:
  - changing `:295` to `"change_submit"`: any submitter can stop a running
    production rollout;
  - deleting `service::author` from the non-running branch: a peer can
    cancel someone else's draft;
  - deleting `:315`: a submitter can mark an unknown attempt as applied
    or not applied, which settles the ledger.
- Fix: extend `each_review_step_is_refused_to_a_user_without_its_authority`:
  - `peer` `POST …/{id}/cancel` on the author's `awaiting_review` change →
    403, and the status is unchanged;
  - put a change into `running` through
    `DatabaseChangesRepo::claim(...)`, then `peer` cancel → 403 and the
    `reviewer` (no `change_execute`) cancel → 403;
  - insert an `unknown` attempt row, then `peer` `…/reconcile` → 403, and
    the attempt outcome is unchanged.

  None of these steps reaches a driver.

### test-quality-6w1-04: minor. Rest of 6-04: the goals blend and the `scoring` parameter parse are untested

- File: `crates/otto-server/src/workflow_engine/nodes/review_run.rs`:
  - `:827-832`: `score = (review_score + gs) / 2` when a goals score
    exists;
  - `:34-47`: `ScoreWeights::from_params` reads `scoring.{bug,warn,info}`.
- Evidence:
  - The `review_gate` tests build `ScoreWeights { bug: 50, .. }` directly
    (`tests/unit/workflow_engine.rs:1856-1861`) and test `goals_from_reply`
    on its own. No test feeds a node's `params` to `from_params`.
  - The blend is inline in `score_target`, which needs a live review.
  - Each of these passes every test:
    - `(review_score + gs) / 2` → `gs`: the goals agent alone decides the
      gate, and a review with 3 bugs (40) plus goals 100 passes at 80;
    - `→ review_score`: goals are ignored;
    - `p.get("scoring")` → `p.get("weights")`: every workflow's
      configured weights are silently replaced by the defaults.
  - `routes/workflows.rs:1128` ships a template with
    `"scoring": {"bug": 10, "warn": 5, "info": 1}`. That template's gate
    depends on this parse.
- Fix:
  1. Extract `fn blended_score(review: i64, goals: Option<i64>) -> i64`
     and test it:
     - `(40, Some(100))` → 70;
     - `(90, None)` → 90;
     - `(0, Some(0))` → 0.
  2. Add one `from_params` test: the template's
     `{"scoring":{"bug":10,"warn":5,"info":1}}` gives those weights, and
     `{}` or `{"scoring":{"bug":"x"}}` gives the defaults.

### test-quality-6w1-05: minor. Rest of 6-05: the turn's call site and the create-as identity are still not pinned

- File: `crates/otto-swarm/src/runtime/run.rs`:
  - `:805`: `find_agent_session(ctx, ws, agent_id, acting_user)`;
  - `:852`: `let creator: Id = acting_user.to_string();`.
- Evidence:
  - `find_agent_session` is now tested on its own
    (`tests/swarm_identity.rs:637-664`).
  - No test runs a turn through `:805`. No test double records the
    `creator` passed to `manager().create(..)`.
  - FIXES P08 says "creator double: needs a host seam".
  - Each of these still passes every test, and each runs Alice's task
    with Bob's MCP token, provider account and git identity:
    - `:805` → `&swarm.created_by`;
    - `:852` → `swarm.created_by.clone()`.
- Fix: the smallest seam that avoids a full host double:
  - extract `fn session_owner_for_turn(acting_user: &str) -> Id` (or a
    `TurnSession { reuse_user, creator }` struct) built once from
    `acting_user`;
  - use it at both lines and unit-test that both fields equal the acting
    user and not `swarm.created_by`.

  If a `SwarmHost` test double is added later for other reasons, assert on
  the recorded `create` call instead.

### test-quality-6w1-06: minor. New fix tests check component source with regular expressions where the repo's function harness could test the behaviour

- Files:
  - `ui/unit/p09-product.test.ts:63-72`: Mermaid stale-render guard and
    live debounce;
  - `ui/unit/p09-product.test.ts:74-81`: keyboard reorder of test cases;
  - `ui/unit/p10-focus-a11y.test.ts:11-32`: focus after an Inspector
    selection, a link-row removal and a breadcrumb unmount;
  - `ui/unit/p06-loadStatesAndTabs.test.ts:27-33`: ⌥⇧←/→ tab move.
- Evidence:
  - Each test reads the `.svelte` source with `readFileSync` and runs
    `assert.match` on lines of the implementation. Logic that breaks still
    passes, as long as those lines remain. For example:
    - In `MockupViewer.svelte:102-105`, dropping
      `return () => clearTimeout(t)` passes the test. The debounce is then
      gone, and every keystroke queues a Mermaid render. The regex at
      `:71` only checks the `setTimeout(...)` text.
    - Moving `gen++` out of `revokeCreated` (`:89-93`) leaves every
      `myGen !== gen` regex matching (`:65-67`), but the stale-render
      guard never fires.
    - `p09-product.test.ts:80`
      `/async function moveCase[\s\S]*?void persistOrder\(\);[\s\S]*?\.focus\(\)/`
      passes if `persistOrder` is sent the wrong order, or if focus lands
      on the wrong row.
  - The same file already drives `AnalysisTab.svelte` behaviour through
    `componentFunctions` (`p09-product.test.ts:13-54`). The behavioural
    tool was available.
  - The repo has many source-reading tests (108 of 393 `ui/unit` files).
    Most pin structure, which is fine: "router imports no `modules/`", "no
    hand-rolled rAF". These four instead stand in for a race guard, a
    debounce, a reorder and focus management. That logic is where a
    regular expression gives false confidence.
- Fix: use `componentFunctions` with fakes:
  1. **Mermaid race:** drive `renderFor` twice with a `renderMermaid` fake
     whose first promise resolves last. Assert that `mermaidSvg` holds the
     second result and that `rendering` ends `false`.
  2. **Debounce:** use fake timers. Two source edits within
     `LIVE_DEBOUNCE_MS` should give exactly one `renderMermaid` call.
  3. **Reorder:** call `moveCase(i, +1)` with a recording `persistOrder`
     fake and assert the persisted id order.
  4. **Focus:** use stub elements to check which element `go()` and
     `upTo()` focus.

  Keep the source regular expressions only for structural rules.
