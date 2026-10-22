# Test-quality review: iteration 5 (2026-10-22)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (grep, sed, git log, git show). No cargo, npm or e2e runs.
Every finding was confirmed by reading the code at the cited lines.
Excluded: `target/`, `node_modules/`, `docs/reviews/`. The room games and the
classroom were in scope this time.

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 5 major, 5 minor, 0 nit.

10.0 − (5 × 0.3) − (5 × 0.1) = 10.0 − 1.5 − 0.5 = **8.0 / 10**

Six of the nine iteration-4 findings are fixed, and the new tests are real
(each was read for vacuity, see the table). Three are partly fixed; their
remaining parts are carried as minors (5-01, 5-02, 5-03).

The score is lower than iteration 4's 8.5 because this pass went one level
deeper into areas earlier passes had not read. It found five untested
compositions, each of which carries a security or user-data property:

- the work-area exception in the iteration-4 session-file fix;
- the secret-to-host binding for agent-started API automations;
- the per-user session reuse in swarm turns;
- the `git_pr` workflow node;
- the `channel_notify` workflow node's workspace filter.

In each case the pure helper is tested, but the code that wires it in is not.

## Previous findings

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-4-01 | **Partly fixed** (rest is **5-01**) | `mcp_auto_approve.rs:595-620` adds a test where an `Mcp` Admin of another workspace tries `approved: true` and `approved: false`, gets 403 or 404, and the card stays pending. That kills the `require_ws` mutant. `:626-680` adds a test where alice, impersonating carol (a ws1 admin), gets 403 "requester cannot approve through impersonation". That is not vacuous: without the gate, the repo's approver ≠ requester check compares against carol and would allow the decision. Case (2) is still open. No test refuses a server-bound card under an enforced resource policy (`grep '"approve"'` finds no `approve` rule in any test). FIXES.md P06 says so. |
| test-quality-4-02 | **Fixed** | New `it` suite `tests/review_forge_post.rs` (registered at `it.rs:63`) runs against a wiremock GitLab. A 201 response posts once (`.expect(1)`), and a second approve sends nothing (`:176-199`). A 500 leaves the comment unposted, the claim is released, and a later approve posts (`:202-233`). A 400 on the anchor falls back to `/discussions` then `/notes`, and the fallback cites the line (`:237-284`). |
| test-quality-4-03 | **Fixed** | `Verdict::parse` now selects JSON by shape (`extract_json_where(reply, is_verdict)`, `verify.rs:58-59`). `verdict_parse_reads_fields_and_fails_closed` (`:1810-1851`) checks the full read, the missing `blocker` → blocks → `Unmet` default, a non-bool `target_met`, prose → `None`, and a `{}` preamble or an earlier JSON example being skipped. |
| test-quality-4-04 | **Fixed** | `perf_budget_tests.rs:562-692` adds a `TwoDbs` stub with roots `shop` and `hidden`, hits in shop, hidden and shop with the driver's `scanned = 2`, and edges into both databases. A `shop`-only reader gets `["db:shop"]`, `["orders","order_items"]` with `scanned == 1`, and only the `shop` edge. Root still sees both roots. |
| test-quality-4-05 | **Partly fixed** (rest is **5-02**) | Case 4b (`resource_access.rs:447-473`) edits `params_json` after the claim with the same policy revision, which pins the fingerprint half of the `changes.rs:101` condition. Still open: `db-drivers.yml:157` keeps `-E 'not binary(resource_access)'`, and the in-flight recheck has no test. |
| test-quality-4-06 | **Partly fixed** (rest is **5-03**) | `automation_ctx.rs:284-366` tests the goal-loop `settle_status` with a real `BEFORE UPDATE OF status ON goal_loops` trigger. In the transient case the trigger is dropped after 300 ms and the status lands. In the persistent case the result is `Err`, exactly one error `Notice` is sent, and the stored status is unchanged. The personal-agent `advance_cursor` (`personal_agents_engine.rs:1114`) is still untested; its only references are its four callers. |
| test-quality-4-07 | **Fixed** | `tests/workflow_run_files.rs:126-131` seeds `id_rsa`, a 500 KiB `big.log` and a NUL-bearing `blob.bin`. A Viewer gets 403 on `id_rsa` and root gets 200 (`:272-276`). The truncation and binary caps are asserted. |
| test-quality-4-08 | **Fixed** | `manager/tests/submit.rs:101-145` calls `sync_workspace_mcp` for an agy session and a claude session in temp cwds. For agy it asserts that `.agents/mcp_config.json` lists `otto` and that `meta.mcp_servers` holds it. For claude it asserts no agy file is written. |
| test-quality-4-09 | **Fixed** | `rooms_api.rs:714-731` checks three refusals: a replayed `sequence: 1` gets 409 and the body must say "Duplicate", so a busy queue can't satisfy it; a `member_generation + 1` gets 403; and `capture_epoch: 1` gets 403. FIXES.md P02 adds a pure `sequence()` unit test. |

## Areas covered

- All nine iteration-4 findings were re-verified (table above), and every new
  test was read for vacuity.
- Iteration 4's "not covered" list:
  - `workflow_engine/nodes/git_pr.rs`: the body is never executed by a test (**5-07**).
  - `workflow_engine/nodes/channel_notify.rs`: never executed, and it has no
    transport seam (**5-08**).
  - `workflow_engine/nodes/human_approval.rs`: approve and reject run through
    `workflow_product_publish_tests.rs:205-240`. `is_gate_decision` and
    `wake_on_event` are unit-tested (`workflow_engine.rs:5837-5895`). The
    canceled-run `continue` and the timeout branch are untested, but approval
    marking nulls `approved_by` first, so neither can turn into a false
    approval. Not reported.
  - `run/finalize.rs`: the terminal write is a compare-and-set
    (`update_run_if`, tested at `tests/unit/workflow_engine.rs:191`).
    Driverless rows are covered by `orphan_sweep_errors_driverless_runs_after_two_passes`.
    No finding.
  - `modules/session_input.rs` and `otto-sessions/src/submit.rs`:
    `scheduled_style_submit_and_submit_text_do_not_interleave` uses a real PTY
    and `cat` sink and asserts two separate bracketed pastes. A dead session is
    reported as not submitted. Echo detection is unit-tested. No finding.
  - `modules/browser_proxy.rs`: the ticket is single-use, bound to its URL and
    expires; CSP headers are checked on every refusal, and the body cap is
    enforced. The SSRF row in `every_proxy_response_carries_sandbox_csp`
    asserts only `!is_success()`, but the guarded resolver in `otto-netguard`
    is a second, independently tested layer. Not reported.
  - `mcp_sync.rs` grok and codex arms: the codex creds file carries
    `profiles` (`manager/tests/mod.rs:195-207`). All providers share one
    `otto_tools_env` (`manager.rs:2494`). No finding.
  - `routes/api_client/automations.rs`: the agent secret binding is untested
    (**5-05**). The helpers (`unbound_secret_host`, `unresolved_placeholders`,
    `apply_extractions`, GraphQL variables) are tested.
  - `otto-telemetry-collector`: 32 tests, including `collector_recovery_tests.rs`. Not deepened.
  - MCP `list_approvals`, `list_audit`, `stats` and `count`: the `discover`
    filter is tested (`tool_39` hidden, `:1117-1127`). Workspace scoping is
    never tested with a row from another workspace (**5-09**).
  - `otto-dbviewer` `multirun/params.rs`: placeholders, Postgres dollar and
    `E''` literals, per-engine quoting, number validation and names are all
    unit-tested. No finding.
- New code from the iteration-4 fixes:
  - `routes/session_files.rs` (PC-1, plus the verifier's fix): **5-04**.
  - `personal_agents_owner`: goals, rules and memory get 403 for a non-owner
    Editor; toggles still get 200; an ownerless preset is adopted.
  - `swarm_identity`: resolution plus the `PATCH` gate are tested. The
    session reuse is not (**5-06**).
  - Session folder removal plus the orphan sweep (`archive.rs:173-210`):
    removed with the row, a grace window, and a live session's files kept.
  - Staged no-replace restore (`state_archive.rs:926-960`): rename, no
    replace, and a symlinked parent refused.
  - Goal-loop settle and the agy dispatch (table above).
- Features added since iteration 4:
  - MCP profiles: `otto-mcp/src/profiles/tests.rs` (11 tests);
    `ottod mcp_tools/tests/profile_tests.rs` (stdio list and refuse, with
    audit); `otto-state/mcp_profiles.rs` (4); `ui/unit/mcpProfiles.test.ts`
    (6); and `desktop-mcp-profiles.spec.ts`. The server edges (the session
    PATCH gate and `/mcp/http?profile=`) are untested (**5-10**).
  - Jira create and Confluence verify: `otto-issues/tests/router.rs:786-1300`
    covers every createmeta problem in one 400 with `posted.is_none()`, a
    verification mismatch, a 502, verified page and comment writes, and
    malformed `body_html` refused before sending. Strong.
- Room games: the kart fix `4bf5dff97` adds regression tests. The play-tests
  assert that the CPU watchdog never fires, across six nudged grids per track.
  Not deepened further.
- e2e: `workbench-typing.spec.ts` asserts that the caret stays at the end after
  re-detection. `desktop-mcp-profiles.spec.ts` asserts the API state after the
  UI acts. No vacuous assertions.

## Areas NOT covered yet (next iteration)

- `otto-server/src/workflow_engine/nodes/{review_run (878 lines), loop_node, product, swarm_task, self_improve}.rs`.
  The review gate has recovery tests; how fully they cover the review-run
  composition was not traced.
- The classroom (`ui/src/modules/home/school/**`): only the commit list was
  checked; the suites themselves were not read.
- The shooter and royale room-game sims beyond the kart fix.
- `otto-channels/src/bridge.rs` (287 lines changed in iteration 4: the shared
  submit, and the first failed paste counting as not submitted).
- The `otto-issues/src/atlassian.rs` transport retry tests (classification of
  429/503 and 401/403, never re-sending a write): read only from FIXES.md.
- The broker replay intent/checkpoint and the consumer-group reset 409 tests
  (`otto-brokers/src/service.rs`, `replay.rs`).
- `otto-sessions` `patch_session` other server-owned meta keys over HTTP (same
  shape as 5-10).
- The `otto-telemetry-collector` recovery semantics.

## Findings

### test-quality-5-04: major. The session-file and LSP work-area exception (the verifier's PC-1 fix) never runs with the data dir protected, so the gate it fixed is untested

- Files: `crates/otto-server/src/routes/session_files.rs:79-83` (`is_session_work_dir`), `:96` (the `check` clause), `:347` (the LSP root match); `crates/otto-server/tests/session_files.rs`.
- Evidence:
  - The refusal that keeps a session whose cwd is `~` (or the data dir) away
    from other users' persona folders and workflow worktrees is
    `sp::in_protected_dir(canonical) && !self.is_session_work_dir(root)`
    (`:96`). `in_protected_dir` is true only for the real Otto data dirs, or
    for a dir registered through `otto_core::secret_paths::protect_dir_for_tests`
    (`secret_paths.rs:361`).
  - No test protects one: `grep protect_dir_for_tests` finds only
    `secret_paths.rs:626` and `otto-vault/tests/engine.rs:1273`. The suite's
    own comment says "the global data-dir deny never sees this test's data
    dir".
  - As a result, every test runs with the `in_protected_dir` clause false:
    - `the_owner_lists_and_reads_their_sessions_working_folder`;
    - `restrict_opens_work_areas_but_not_state_files` (`:405-428`), which
      asserts only the `is_session_work_dir` truth table and the `otto.db`
      name rule;
    - `lsp_opens_on_the_owners_persona_dir_only`.
  - Each of the following passes every test:
    - deleting the whole `in_protected_dir` clause, so a non-root reads
      anything in the data dir;
    - reverting to the pre-fix `in_work_area(root)`;
    - dropping the `is_session_work_dir` check at `:347`.

  The fix is exactly what stops a non-root user whose session cwd is `~` from
  listing and reading other users' workflow worktrees and persona folders.
- Fix: in `tests/session_files.rs` `boot()`, call
  `otto_core::secret_paths::protect_dir_for_tests(&tmp.path().canonicalize()?)`.
  Add a session owned by carol whose cwd is the temp data dir's parent, the
  stand-in for `~`. Then assert:
  1. Carol reading `<data>/personal/agent-2/AGENTS.md` (another user's
     persona dir) through that session gets 403.
  2. Through her persona-dir session, she still gets 200 on her own
     `memory/notes.md`.
  3. `lsp_status` for carol on `<data>/personal/agent-2` gets 403.

  Assertion 1 fails for both mutants above.

### test-quality-5-05: major. The secret-to-host binding for agent-started API automations is tested only as a helper; no test runs a step with a binding

- Files: `crates/otto-server/src/routes/api_automation_runs.rs:108-121` (`is_agent_caller` → `Some(StepSecretBinding)`); `crates/otto-server/src/routes/api_client/automations.rs:274-306` (the `if let Some(binding)` block in `run_step`, placed AFTER the pre-request script and GraphQL assembly).
- Evidence:
  - `unbound_secret_host` is unit-tested (`tests/unit/api_client.rs:803-867`),
    and so is `is_agent_caller` (`:981`).
  - The only `run_step` call in a test passes `binding: None`
    (`api_client.rs:1312-1321`).
  - `grep StepSecretBinding {` finds only the production constructor.
  - `api_automation_runs.rs` has one test, `validate`'s bounds (`:386-397`).
  - No `it` suite starts `/automations/{id}/runs` as an agent.
  - Each of the following passes every test:
    - building `binding` as `None` for agents;
    - deleting the `run_step` block;
    - moving it ABOVE the pre-request script.

  In each case a prompt-injected agent can run an automation whose dataset
  row, chained extraction or `pm` pre-script retargets a secret-bearing step
  to its own host, and the stored token is sent there. The block's own
  comment names that threat (`:274-276`).
- Fix: add a test in `tests/unit/api_client.rs` with:
  - a human-saved request on `http://<wiremock A>` with a `$secret` bearer;
  - an automation step on it whose pre-request script sets the URL to
    `http://<wiremock B>`.

  Call `run_step(…, Some(&binding))`, with the binding built as
  `api_automation_runs::start` builds it. Assert that `!step.ok`, that the
  error names B's host, and that B received no request (`.expect(0)`). Repeat
  with `binding: None` (a person's run) and assert that the request is sent.

### test-quality-5-06: major. Swarm turns reuse a session per (agent, acting user), but only the user resolution is tested; the reuse filter and the create-as identity are not

- Files: `crates/otto-swarm/src/runtime/run.rs:225-237` (`find_agent_session`: `s.created_by == user`), `:743` (reuse), `:790` (`let creator: Id = acting_user.to_string()`).
- Evidence:
  - `tests/swarm_identity.rs` says in its header that it "pin[s] the
    resolution itself … (spawning a PTY turn is out of scope)" (`:6-10`).
    Every turn assertion calls `otto_swarm::runtime::run::acting_user`
    directly (`:147-155`).
  - `grep find_agent_session|run_attempt` finds no test caller.
  - PC-2 (access-control-4-01) fixed a turn running with the swarm creator's
    token and identity. Each of the following brings that bug back with every
    test green:
    - dropping `&& s.created_by == user`, so Alice's task reuses the session
      Bob's task created, which carries Bob's MCP token, provider account and
      git identity;
    - passing `swarm.created_by` instead of `acting_user` at `:790`.
- Fix: extract the predicate at `:232-237` into a pure
  `reusable_for(s: &Session, agent_id, user) -> bool`. Unit-test it: the same
  agent with another creator is false; an archived or exited session is
  false; the same agent and user is true. Then add a `manager`-backed test.
  Seed one live agent session created by bob for `agent_id = dev`, and assert
  that `find_agent_session(ctx, ws, "dev", "alice")` is `None` and
  `find_agent_session(…, "bob")` is that session. A `MockOps`-style host that
  records `manager().create(…, creator, …)` can pin `:790`.

### test-quality-5-07: major. The `git_pr` workflow node, which commits leftovers, pushes and opens PRs, is never executed by a test

- File: `crates/otto-server/src/workflow_engine/nodes/git_pr.rs`:
  - `:173-191`: the leftover commit runs ONLY when `wt != repo.path`, never
    in a user's main checkout;
  - `:283`: push before create;
  - `:304`: `create_pr_for_repo`;
  - `:346-358`: R13, `open && opened_n == 0` → error.
- Evidence:
  - `grep '"git_pr"'` in the tests finds only graph-shape uses: resume and
    retry classification (`unit/workflow_engine.rs:336, 480, 542`), the
    retry scope, and the policy fixture.
  - `collect_pr_targets` is unit-tested (`:1303-1340`).
  - `commit_all_if_dirty` is tested in otto-git.
  - Nothing calls `nodes::git_pr::execute` or `execute_node` with a `git_pr`
    node.
  - Each of the following passes every test:
    - deleting the `wt != repo.path` guard, so the node sweeps a user's
      uncommitted work in their own checkout into a "chore: commit workflow
      changes" commit;
    - removing R13, so a step asked to open a PR that opened none reports
      success with `opened: false`;
    - letting one repo's draft failure abort the other repos.
- Fix: reuse `workflow_product_publish_tests.rs`'s `Fixture`/`execute_node`
  harness (`:166-203`) with a registered temp git repo:
  1. Target the repo's own path, make the tree dirty, and use `open: false`.
     Assert that HEAD is unchanged and the file is still untracked or
     modified, whatever the node returns (the draft fails with "nothing to
     PR").
  2. Target a `git worktree add` path that is dirty. Assert that one new
     commit exists there.
  3. Use `open: true` on a repo with no git account and a wiremock forge
     that answers 422. Assert `Err` containing "asked to open a PR but none
     opened".

  The draft agent turn can be skipped in cases 1 and 2, because
  `pr_draft_prompt` fails first when the branch has no diff against its base.

### test-quality-5-08: major. `channel_notify` posts outward to Slack or Telegram for whichever integrations its in-memory filter keeps; the workspace filter and thread routing are untested

- File: `crates/otto-server/src/workflow_engine/nodes/channel_notify.rs:96-115`:
  - `list_all_enabled()` returns every workspace's integrations
    (`otto-state/src/integrations.rs:208-211`);
  - `.filter(|i| i.workspace_id == ws.id)` at `:109` keeps this workspace's
    only;
  - `:129-135` choose between the originating chat and thread, and the
    default chat.
- Evidence:
  - The only references to `channel_notify` in tests are graph shapes
    (`unit/workflow_engine.rs:416, 511, 760`).
  - The adapters post to a hard-coded `https://slack.com/api/chat.postMessage`
    (`otto-channels/src/slack.rs:439`), so the node has no seam and its
    target selection is inline.
  - Deleting `:109` passes every test, and a workflow run in workspace A
    then posts its message to every other workspace's default Slack and
    Telegram chats. The message can carry run input such as PR titles,
    tickets and paths, through `{key}` substitution.
  - Breaking the `same_channel` match, so a reply leaves its originating
    thread (the bug the comment at `:62-67` describes), also passes.
- Fix: extract `fn notify_targets(all: Vec<Integration>, ws: &Id, preferred: Option<Channel>) -> Vec<Integration>`
  and `fn reply_target(integ, origin_channel, origin_chat, origin_thread) -> (chat, thread)`.
  Unit-test them:
  - integrations in two workspaces → only `ws`'s;
  - webhook and blank `channel_id` dropped;
  - a `preferred` filter;
  - a Slack-originated run replying in-thread on Slack, but posting to the
    default chat with no thread on Telegram.

  Optionally, put the `{key}` substitution behind the same pure function and
  assert that the node input overrides the run input on a collision.

### test-quality-5-09: minor. MCP approval, audit, stats and count lists are never tested with a row in another workspace; a cross-workspace list call asserts only 200

- Files: `crates/otto-mcp/src/http.rs:225-239` (`accessible_ws`), `:962-985`, `:1107-1121`, `:1159-1166`; `crates/otto-state/src/mcp_control.rs:1034-1046` (the `workspace_id IN (…) OR null_ws_clause` filter).
- Evidence:
  - Every audit and approval fixture uses `ws1` or `None`:
    - `mcp_auto_approve.rs:630, 856, 953, 1100, 1654, 1717`;
    - `mcp_control.rs:2152-2209`, where rows are in `ws` or `None` only.
  - `workspace_less_otto_rows_are_only_the_callers_own` pins the
    caller-own rule for workspace-less rows, but no row in a second
    workspace is ever listed. A rewrite of `workspace_id IN (…)` to
    `workspace_id IS NOT NULL`, or a handler passing `None` for a non-root
    caller, passes every test.
  - `mcp_admin_elsewhere` (`mcp_auto_approve.rs:570-590`) has bob, an Admin
    of `ws-bob` only, call `GET /mcp/approvals?status=pending` while ws1's
    `create_pr` card is pending. It asserts only `st == 200`, not that the
    card is absent.
- Fix:
  1. In `mcp_admin_elsewhere`'s caller test, assert that bob's list is `[]`
     and that `/mcp/approvals/count?status=pending` returns `0`.
  2. In `workspace_less_otto_rows_are_only_the_callers_own`, insert a
     `Some("other-ws")` row and assert it is absent from both `list` and
     `stats` for the `ws`-scoped query.

### test-quality-5-10: minor. The MCP profile's server edges are untested: the session PATCH gate and `/mcp/http?profile=`

- Files:
  - `crates/otto-sessions/src/http.rs:678-684`: in `patch_session`, the
    profile is changed only when `meta.mcp_profiles` differs, and the agent is
    detected with `agent_session_of`.
  - `crates/otto-server/src/mcp_http.rs:67-82`: `q.get("profile")` →
    `ProfiledTools`.
  - `crates/otto-server/src/mcp_outward.rs:1471-1478`: `?profiles=`.
- Evidence:
  - `check_mcp_profiles_patch` is unit-tested as a pure function
    (`http.rs:935-960`).
  - `ProfiledTools` is tested against a fake `OutwardTools`
    (`profiles/tests.rs:210-280`).
  - The ottod bridge is tested against a fake daemon
    (`mcp_tools/tests/profile_tests.rs:67-110`).
  - `grep mcp_profiles|profile=` in `crates/otto-server/tests` finds
    nothing. Each of the following passes every test, and each fails open
    to every tool:
    - computing `agent` as `false`, so a session's own agent token widens
      its profile to `full`;
    - reading `q.get("profiles")` in `mcp_http_post`, so the documented
      `?profile=` is ignored.
  - Profiles only narrow what a caller sees and governance is unchanged, so
    this is minor.
- Fix: in an `it` suite:
  1. `PATCH /sessions/{id}` with `{"meta":{"mcp_profiles":["full"]}}`, using
     the session's own managed token, gets 403; the owner gets 200; an
     unchanged round-trip from the agent gets 200.
  2. `POST /mcp/http?profile=database` with `tools/list` lists no `k8s_*`
     tool, and `tools/call k8s_logs` returns the profile denial. Without
     `?profile`, the tool is listed.

### test-quality-5-01: minor. Rest of 4-01: the decide route's resource-authority refusal for a server-bound approval has no test

- File: `crates/otto-mcp/src/http.rs:1063-1074` (`require_resource(…, "approve", tool, Editor)`).
- Evidence:
  - Both new decide tests use workspace-bound cards (`server_id: None`) on
    the `otto_tool` path.
  - No test creates an approval with `server_id: Some(..)` under an enforced
    resource policy, and `grep '"approve"'` finds no `approve` rule in any
    test.
  - Deleting the `require_resource` call passes every test. FIXES.md P06
    lists it as not done.
- Fix: create a server-bound card with `McpApprovalRepo::create`
  (`server_id: Some(s.id)`, `tool: Some("t")`). Enforce a live policy on the
  server that allows `discover` but not `approve` for user U, a ws Editor.
  Assert that U's decide gets 403 and the card stays pending. Then add an
  `approve` rule for U and assert 200.

### test-quality-5-02: minor. Rest of 4-05: the native resource-access and governed-root cases still never run, and the in-flight revocation recheck is untested

- Files: `.github/workflows/db-drivers.yml:12-14, 157` (`-E 'not binary(resource_access)'`); `crates/otto-dbviewer/src/service/changes.rs:216-226, 248-252`.
- Evidence:
  - The nextest filter is unchanged.
  - The seven `#[ignore = "requires explicitly provisioned disposable … fixture"]`
    cases are still never run. They include
    `governed_root_reads_cannot_call_a_mutating_builtin_overload`, the only
    test of the native read-only gate for root reads on prod.
  - No non-ignored test reaches the preflight artifact recheck (`:216-226`)
    or the 500 ms in-flight revocation (`:248-252`).
  - FIXES.md P08 lists this as not done.
- Fix: as in 4-05.
  1. Seed `otto_reader`, `shop` and `hidden` in the `db-drivers.yml` MySQL
     service and drop the filter, so the ignored cases run in that workflow.
  2. Cover the in-flight recheck there with `SELECT SLEEP(2)`, then write
     `cancellation_requested = 1` mid-run, and assert `Forbidden`.

### test-quality-5-03: minor. Rest of 4-06: the personal-agent `advance_cursor` retry and hold loop is untested

- File: `crates/otto-assistant/src/personal_agents_engine.rs:1114-1155`.
- Evidence:
  - Its only references are its four callers (`:552, 946, 1019, 1038`).
  - `settle_captured_schedule` is tested only for generation fencing.
  - Without the retry, the next tick relaunches the same occurrence
    (`:1126-1128`).
  - The scheduled-task and goal-loop twins now have trigger-based tests
    (`scheduled_tasks_engine.rs:1767-1850`, `automation_ctx.rs:284-366`).
- Fix: make the 60 s hold a `cfg(test)` constant (as `SETTLE_HOLD` is,
  `scheduled_tasks_engine.rs:669-671`). Fail
  `UPDATE … personal_agent_schedules` with a `BEFORE UPDATE` trigger, drop it
  after 300 ms, and assert that the cursor advanced. Keep the trigger for the
  whole hold and assert that the final attempt settles once it is dropped.
