# Test-quality review: iteration 4 (2026-10-22)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (grep, sed, git log, git show). No cargo, npm or e2e runs.
Every finding was confirmed by reading the code at the cited lines.
Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`,
`docs/reviews/`.

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 3 major, 6 minor, 0 nit.

10.0 − (3 × 0.3) − (6 × 0.1) = 10.0 − 0.9 − 0.6 = **8.5 / 10**

Iteration 3 scored 8.8. Four of its six findings are fixed, and the new tests
are real (checked for vacuity below). One is partly fixed, and one was not
attempted. The lower score comes from going one level deeper into areas the
last pass listed as not covered: the MCP approval decide route, the review
comment post path, the swarm verdict parser, and the recap media upload. Each
has an untested gate.

## Previous findings

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-3-01 | **Fixed** | `otto-swarm/src/runtime/scheduler.rs`: the rebalance is now `idle_agents` (`:501-512`), `plan_rebalance` (`:518-551`) and `take_escalation_slot` (`:557-567`), and `check_utilization` calls them (`:340-354`, `:394`). Four tests (`:689-737`) kill each named mutant: busy, verifying and inactive agents are never targets; a task on an idle agent stays and uses up a slot; a board at its cap moves nothing; and an escalation within 30 minutes is refused per swarm. The reserve → post → spawn tail is still covered only through the helpers, which is acceptable. |
| test-quality-3-02 | **Partly fixed** (rest is **4-05**) | `otto-dbviewer/tests/resource_access.rs:343-469` is not ignored. It runs six refusals on the port-1 fixture. Case 5 expects `Conflict`, which only the revision/fingerprint branch returns (`changes.rs:101-111`). That proves the SQL guard passes with the original values, so the other five refusals are not vacuous. Still open: the native cases never run in CI (`db-drivers.yml:12-14, 157`); the fingerprint half of the `:101` condition is untested; and the in-flight recheck (`:216-226`, `:248-252`) is untested. |
| test-quality-3-03 | **Fixed** | `otto-brokers/src/http.rs:838-930`. Each of the six mutating routes answers 403 with "production" or "read-only" on a prod cluster and on a read-only cluster, and the `&&` mutant fails because the prod cluster is not read-only. A confirmed replay passes the guard. A dev cluster passes. A prod dry-run reset is not refused; if the guard sat above the dry-run branch, the 403 would come back at once, before any broker call, so the test catches it. A global cluster needs Database Edit (View gets 403, Edit deletes). |
| test-quality-3-04 | **Open** (now **4-04**) | FIXES.md P02 lists it as not done. The stubs still return empty roots (`perf_budget_tests.rs:297-300`), `ObjectSearchResult::default()` (`:320-327`) and no graph (`:362-369`). No test calls `search_objects`. |
| test-quality-3-05 | **Fixed** | `resource_sessions.rs:357-411` runs two k8s exec rows on one cluster, in namespaces `a` and `b`, under an allow on `namespace:a` only. It asserts `["in-a"]` in both orders, which kills the memo key without `child`. `tests/admin_sessions.rs:438+` goes through `build_router`: a ws Admin without the `shell` grant does not see the managed bound row, and does see the unbound and legacy rows. |
| test-quality-3-06 | **Fixed** | `scheduled_tasks_engine.rs:1767-1850`. A real `BEFORE UPDATE OF last_run_at` trigger fails the settle write, with no seam. In test 1 the trigger is dropped at 0.3 s and the cursor is asserted at 1.3 s. A loop that gave up would settle only at 2.2 s (`SETTLE_HOLD` is 2 s under `cfg(test)`, `:669-670`), so test 1 fails for that mutant. Test 2 asserts that the post-hold final attempt settles. The timing margins are about 0.7 s and 0.5 s on relative sleeps, so flake risk is low. FIXES.md calls these "usage settle retry" tests, but they are the scheduled-task ones. |

## Areas covered

- All six iteration-3 findings were re-verified (table above), and the new
  tests were read for vacuity.
- From iteration 3's "not covered" list:
  - `otto-swarm` `SwarmVerifyOps`: `verify_goal` → `Verdict::parse` has no test (**4-03**).
  - `merge_back`, `record` and `escalate` are thin I/O around `MockOps`-tested control flow. No finding.
  - `scheduler.rs` `tick()`: every admission branch goes through tested helpers (`scheduled_turn_open`, `schedule_slot_open`, `is_due`, `launch_after_await`). The reserve/release guards are tested in `otto-state/src/swarm/tests.rs:205-360`. No finding.
  - `otto-mcp/src/http.rs`: traced which routes `mcp_auto_approve.rs` and `pipeline_e2e.rs` reach. The decide route's agent-credential refusal is tested. Its workspace (IDOR), resource and impersonation refusals are not (**4-01**).
  - Rooms:
    - `recap/consent.rs` is fully state-machine tested (3 tests).
    - `rooms_api.rs` drives consent, start, capture, stop and finalize over a real WebSocket. It also checks a stale-epoch `gap` → 403 and a gap during finalization → 403.
    - The audio upload's refusals are untested (**4-09**).
    - The `registry.rs` admission limits and invitation reuse are covered by `rooms/tests.rs` (11 tests).
  - `modules/review/routes.rs` `approve_comment`: only the local (PR #0) path is tested. The forge path is not (**4-02**).
  - `personal_agents_engine.rs` `advance_cursor`: untested, together with the new goal-loop `settle_status` (**4-06**).
  - UI stores: `git.svelte.ts` has three behavioural suites (`gitAutoFetch`, `gitLiveStatus`, `gitPrimaryOwnership`). `clipHistory` is a 171-line ring with a pure exported `pushClip`. No finding.
- New code from the iteration-3 fixes, checked for tests and for vacuity:
  - `routes/workflow_run_files.rs`: path confinement is well tested (unit plus `tests/workflow_run_files.rs`, including a symlink escape). The non-root credential-file refusal and the size and binary caps are not (**4-07**).
  - Bitbucket 202 merge polling: five wiremock cases, including a cross-origin `Location`.
  - `KeyedLocks`: serialisation, pruning, the shared waiter mutex and a poisoned map.
  - `memory.rs` `prev_state` undo (`:1692`).
  - The assistant orphan-inbox sweep (`tasks.rs:1582-1618`: referenced, fresh and nested files kept) and the `in_inbox` confinement.
  - The launcher env sweep (`reconcile_and_sweep_remove_stale_launcher_env_files`).
  - The issue client's encoding (`productJiraWritesConfirm.test.ts:53-80`).
  - `paletteSources` and `termGridSync` unit suites.

  All of these assert behaviour.
- Recent feature commits:
  - `8a9897823` (agy MCP config): the pure reconcile is tested, but the provider dispatch that delivers the fix is not (**4-08**).
  - `eb295dcc2` (merged-into marks): 73 lines of `local/tests.rs`.
- e2e: grepped every spec for conditional assertions (`if (await … isVisible())`, `.catch(() => {})`, `test.skip`). The hits are environment skips for live DB fixtures, or branches where both arms assert (`canvas.spec.ts:93-102`). `canvas.spec.ts:497` asserts only when more than one agent provider is registered; built-in providers make that the normal case. Not reported.

## Areas NOT covered yet (next iteration)

- `otto-server/src/workflow_engine/nodes/{git_pr,human_approval,channel_notify}.rs` and `run/finalize.rs`: no inline tests since the split. `tests/unit/workflow_engine.rs` references these node kinds, but nobody has traced whether the outward paths (PR open, channel send, approval timeout and reject) are exercised beyond graph shape.
- `otto-server/src/modules/session_input.rs` (777 lines, 3 tests) and `browser_proxy.rs` (7 tests): check the paste and submit edge cases and the proxy header and host filtering.
- `otto-sessions/src/manager/mcp_sync.rs` grok and codex delivery arms (the same shape as 4-08).
- `otto-server/src/routes/api_client/automations.rs` (671 lines, new file).
- The `otto-telemetry-collector` crate (1,235 new lines).
- The MCP `list_approvals`, `list_audit` and `stats` visibility filters across workspaces: only the budget and paging tests were read.
- `otto-dbviewer` `multirun/params.rs` (+164).

## Findings

### test-quality-4-01: major. The MCP approval decide route's workspace, resource and impersonation refusals have no test

- File: `crates/otto-mcp/src/http.rs:1015-1080` (`decide_approval`).
- Evidence: after it refuses agent credentials (`:1028-1041`), the handler has
  three more gates before `approvals().decide`:
  1. Impersonation self-approval. If the real user behind an impersonating
     token raised the request, they may not decide it (`:1047-1055`).
  2. IDOR. For an approval with no `server_id`, the decider needs ws Editor on
     `appr.workspace_id` (`:1057-1061`).
  3. Resource authority. For a server-bound approval, the decider needs
     `require_resource(…, "approve", tool, Editor)` (`:1062-1073`).

  The route-level policy is only `Require(Mcp, Admin)`
  (`tests/snapshots/policy_decisions.txt:1939`). That is a feature grant and
  carries no workspace. Every HTTP decide call in the repo is in
  `tests/mcp_auto_approve.rs` (`:536, 545, 558, 729, 1530, 1627, 1748`). All of
  them use the agent token, which is refused, or `d.human`, the admin of the
  approval's own workspace. `otto-mcp/src/http.rs` has no `cfg(test)`.
  `grep "through impersonation"` finds only the handler.

  Each of the following passes every test, and lets an `Mcp:Admin` user from
  workspace B approve workspace A's pending `create_pr` or merge (the call
  then runs on that approval), or lets an impersonator approve their own
  request:
  - deleting the `require_ws` call;
  - deleting the `require_resource` call;
  - deleting the impersonation check.
- Fix: in `mcp_auto_approve.rs`, reuse `boot()` and `agent_invoke("create_pr", …)` to get a pending card. Then:
  1. Seed a second user who has an `Mcp` Admin feature grant and is an Admin
     of another workspace only. Assert that their decide returns 403 or 404,
     and that `pending_approvals()` still holds the card.
  2. For a server-bound card, use a user who is a member but has no `approve`
     rule under an enforced resource policy, and assert 403.
  3. Raise a card as user U (a non-agent requester). Decide it with a token
     where `real_user = U` impersonates V. Assert 403 "requester cannot
     approve through impersonation", and that the card is still pending.

### test-quality-4-02: major. Approving a PR review comment, the outward post to the forge, is tested only for local reviews that never post

- File: `crates/otto-server/src/modules/review/routes.rs:615-641` (claim, re-read, post, release) and `:663-700+` (`post_review_comment`: inline, then the anchor fallback).
- Evidence: the only HTTP test of `/pr-review-comments/{id}/approve` is
  `tests/review_comment_states.rs`. It creates `create_review(&repo.id, 0)`,
  a LOCAL review that "never calls a forge" (`:58-59`), and asserts
  `posted == false` (`:121`). The e2e `desktop-ux-r2-git.spec.ts:348`
  replaces the approve route with `route.fulfill`, so the daemon is never
  reached.

  The pure helpers `inline_anchor_rejected` and `general_comment_body` are
  unit-tested (`otto-review/src/engine.rs:1024-1040`), and so is the repo-level
  claim. The handler's composition is not. That composition is what makes
  posting at most once, and what makes a rejected anchor still post.

  Each of the following passes every test:
  - posting without `claim_comment_post` (a double click or a retried request
    posts twice to GitHub, GitLab or Bitbucket);
  - setting `pr_posted = true` on a failed post (the comment shows as posted
    but is not on the PR);
  - dropping the fallback call after an anchor 422 (an approved finding is
    silently not posted);
  - dropping the re-read under the claim (a decline that races the approve is
    posted anyway).
- Fix: add a case to `review_comment_states.rs` (or a new `it` suite) using a
  GitLab git account whose `api_base_url` points at a wiremock server, as
  `otto-git/src/http_tests.rs:464` does. Use a review with `pr_number = 7`.
  1. A 201 on the notes endpoint: assert `posted == true` and exactly one
     request (`.expect(1)`). Approve again and assert that no second request
     is sent.
  2. A 500: assert `posted == false`, and that a later approve posts again
     (the claim was released).
  3. A 400 on the positioned discussion, then a 201 on the general note:
     assert two requests, and that the second body cites `a.rs:3`.

### test-quality-4-03: major. `Verdict::parse`, which turns the leader's reply into the swarm merge gate, has no test

- File: `crates/otto-swarm/src/runtime/verify.rs:45-73`, called from `verify_goal` (`:584-614`).
- Evidence: `run_verification` and `classify` are well tested, but always with
  hand-built `Verdict`s (`vd(target_met, blocker)`, `:1593`, through
  `MockOps`). The function that builds a `Verdict` from the agent's text is
  never called in a test: `grep -rn 'Verdict::parse'` finds only `:613`.

  Its defaults are the fail-closed choices for a merge gate:
  - a missing `blocker` reads as `true` (`:52`);
  - a missing or non-bool `target_met` reads as `false` (`:48-51`).

  `classify` lets a non-blocker past its retries through as `Warned`, so
  `run_verification` merges the agent branch (`:107-111`). Flipping the
  `blocker` default to `false` therefore merges work whose verdict omitted
  the field, and no test fails.

  The parser also uses `extract_json` (first balanced span) rather than the
  verify-time acceptor `extract_json(t).is_some()`. A fenceless reply such as
  ``Checked `fn x() {}` … {"target_met": true}`` parses `{}` as the verdict
  (`otto-core/src/text.rs:142-167`, `:82-127`). That is the case
  backend-correctness-3-03 fixed for PR drafts with `extract_json_where`. A
  parser test would have pinned which object wins.
- Fix: add a unit test in `verify.rs` `mod tests` with these cases:
  1. A fenced, complete verdict: every field is read.
  2. `{"target_met": false}` with no `blocker`: `blocker == true`, and
     `classify` at the retry ceiling returns `Unmet`.
  3. `{"target_met": "true"}`: `target_met == false`.
  4. Prose with no JSON: `None`.
  5. A fenceless reply with a `{}` before the verdict: assert the intended
     object wins. Switch `parse` to `extract_json_where(reply, |v| v.get("target_met").is_some())`
     so this case passes.

### test-quality-4-04: minor. The DB Explorer tree-root, object-search and ERD-edge reader filters are still not exercised (3-04, not attempted)

- File: `crates/otto-dbviewer/src/service.rs:1700-1722` (`schema_root`), `:1765-1800` (`search_objects` and the `scanned` recount), `:1960-2040` (`schema_graph` edge filter).
- Evidence: FIXES.md P02 "Not done" lists 3-04. The stubs are unchanged:
  - `perf_budget_tests.rs:297-300` returns `Vec::new()` roots;
  - `:320-327` returns `ObjectSearchResult::default()`;
  - `:362-369` returns `Ok(None)` for the graph.

  No service-level test calls `search_objects`. The only restricted-reader
  `schema_root` assertion is inside the ignored `native_fixture`. Returning
  `nodes` unfiltered, keeping the driver's `scanned`, or skipping the edge
  `zip` still passes every test, and would show hidden database names to a
  scoped reader.
- Fix: as in 3-04. Reuse the `shop`-only reader from
  `object_detail_filters_mixed_fk_scopes_without_changing_order` (`:484`),
  and make a stub return:
  - roots `shop` and `hidden`;
  - search hits in `shop`, `hidden` and `shop`;
  - one edge into `shop` and one into `hidden`.

  Assert that only `shop` is returned, that the two `shop` hits come back in
  order with `scanned == 1`, and that only the `shop` edge is kept.

### test-quality-4-05: minor. Rest of 3-02: the native resource-access and governed-root cases never run, and the executor gate's fingerprint and in-flight rechecks are untested

- Files:
  - `.github/workflows/db-drivers.yml:12-14, 157`;
  - `crates/otto-dbviewer/tests/resource_access.rs:327-333, 656-668, 748-754`;
  - `crates/otto-dbviewer/src/service/changes.rs:101-111, 216-226, 241-252`.
- Evidence:
  - Seven `#[ignore = "requires explicitly provisioned disposable … fixture"]`
    tests are still excluded by `-E 'not binary(resource_access)'`. They
    include `governed_root_reads_cannot_call_a_mutating_builtin_overload`,
    the only test of the native read-only gate for root reads on prod.
  - The new non-ignored gate test changes the policy revision (case 5).
    It never changes the connection or credential, so the
    `change_target_fingerprint(..) != attempt.connection_fingerprint` half
    of `:101-108` can be deleted without a failure. No test outside the
    ignored ones reaches the "approved artifact changed during preflight"
    recheck (`:216-226`) or the 500 ms in-flight revocation (`:248-252`).
  - FIXES.md P02 records the fixture part as not done.
- Fix:
  1. In `executor_gate_refuses_tampered_or_stale_claims_before_any_network`,
     add a case that edits the connection's params, or its credential
     profile, after the claim. Assert `Conflict`.
  2. Seed `otto_reader`, `shop` and `hidden` in `dev/dbviewer` and drop the
     nextest filter, so the native and governed-root cases run nightly.
     The in-flight recheck can then be covered by a slow statement (for
     example `SELECT SLEEP(2)`) plus a `cancellation_requested=1` write
     mid-run, asserting `Forbidden`.

### test-quality-4-06: minor. The goal-loop status settle and the personal-agent cursor retry loops have no test

- Files: `crates/otto-automation/src/goal_loop.rs:1164-1200` (`settle_status`, `SETTLE_ATTEMPTS`); `crates/otto-assistant/src/personal_agents_engine.rs:1114-1155` (`advance_cursor`).
- Evidence: both are the same loop that 3-06 had to test for scheduled tasks.
  - `settle_status` (new in error-handling-3-02) retries `finalize` five
    times. On a persistent failure it sends a "Couldn’t save the goal loop’s
    result" Notice and returns the error.
  - `advance_cursor` retries `settle_captured_schedule` five times, then
    holds 60 s for a last attempt. Without it, the next tick re-launches the
    same occurrence (`:1126-1128`).

  `grep -rn 'settle_status|SETTLE_ATTEMPTS|Couldn’t save the goal loop'`
  outside their definitions finds nothing. `advance_cursor` is referenced
  only by its four callers and a comment in `scheduled_tasks_engine.rs:633`.
  The `goal_loop.rs` test list (`:2218-2478`) has no settle case.
  `settle_captured_schedule` is tested only for generation fencing
  (`otto-state/src/personal_agents.rs:1943, 2375, 2619`). Removing either
  loop, or the Notice, passes CI.
- Fix: reuse the 3-06 pattern, a SQLite trigger with no seam.
  1. For goal loops: `CREATE TRIGGER … BEFORE UPDATE OF status ON goal_loops … RAISE(ABORT,'busy')`.
     Call `settle_status`, drop the trigger after 300 ms, and assert the
     stored status. A second case keeps the trigger: assert `Err`, and that
     one `Event::Notice` with level `error` is on `ctx.events()`.
  2. For personal agents: make the hold a `cfg(test)` constant, as
     `SETTLE_HOLD` is. Fail `UPDATE … personal_agent_schedules` the same way,
     and assert that the cursor settles after the trigger is dropped.

### test-quality-4-07: minor. The run-context file reader's credential-file refusal and its size and binary caps are untested

- File: `crates/otto-server/src/routes/workflow_run_files.rs:197-226` (`read_sync`).
- Evidence: the route is new (access-control-3-04). Its confinement is well
  tested (`resolve` unit tests, plus `tests/workflow_run_files.rs:232-251`
  for `..`, absolute paths and a symlink escape).

  Two behaviours the FIXES entry promises have no test:
  - Refusing key-like names to non-root callers
    (`if restricted && is_denied_file(&file)`, `:200-204`). The only
    `read_sync` unit call passes `restricted = true` on `final-output.md`
    (`:340`), and no fixture writes an `id_rsa`, `.env` or `*.pem` into the
    context dir.
  - The `/fs/read` cap and binary handling (`:205-226`). No file over
    400 KiB, and no file with a NUL byte, is seeded.

  Each of the following passes every test:
  - dropping the `restricted &&` check;
  - passing `user.is_root` instead of `!user.is_root`;
  - removing the truncation.

  A workflow agent writes this directory, so a credential the agent writes
  there is readable by any workspace Viewer.
- Fix: in `boot()`, also write `id_rsa`, a 500 KiB `big.log` and a
  `blob.bin` containing `\0` into `ctx_dir`. Then assert:
  - `vera` (a Viewer) gets 403 on `read id_rsa`, and root gets 200;
  - `big.log` returns `truncated: true` with `content.len() <= 400*1024`;
  - `blob.bin` returns `content == ""` with `truncated: true`.

### test-quality-4-08: minor. The agy MCP fix is tested only at the pure-function level; the per-provider dispatch that delivers it is not

- File: `crates/otto-sessions/src/manager/mcp_sync.rs:135-141` (`(provider == "agy").then(...)`), `:156` (`"agy" => agy_names.unwrap_or_default()`).
- Evidence: commit `8a9897823` added `reconcile_managed_servers_agy` and its
  test `agy_config_reconciles_into_agents_dir` (`mcp.rs`). That test calls
  the reconcile directly. The fix itself is the dispatch in
  `sync_workspace_mcp`: an agy spawn must write `.agents/mcp_config.json`,
  and must report those names as delivered.

  `grep sync_workspace_mcp` finds only its two production callers
  (`manager.rs:3015, 5801`). No test spawns or restarts an agy session and
  checks the file. The manager tests cover agy only for env, cwd and argv
  (`manager/tests/mod.rs:224, 435, 3080, 3621`). Changing `"agy"` to another
  provider in either place, or dropping the `delivered` arm, passes every
  test, and agy sessions lose every otto, browser and user MCP server again.
  That breaks the control-plane/session parity the user requires.
- Fix: add a `manager/tests` case that builds an agy `Session` in a temp cwd
  and calls `sync_workspace_mcp(&session, Some(otto_server), false)`. Assert
  that `<cwd>/.agents/mcp_config.json` lists `otto` under `mcpServers`, and
  that the returned delivered set contains `otto`. Assert the reverse for a
  `claude` session: no `.agents/mcp_config.json` is written.

### test-quality-4-09: minor. The recap audio upload's refusals are untested; only the happy path and the gap route's stale epoch are

- File: `crates/otto-server/src/rooms/recap/media.rs:90-150` (`audio`): member generation fence `:112`, `active()` epoch and offset `:45-56, :119-125`, audible-window check `:130-139`, `sequence()` replay guard `:58-70, :141-147`.
- Evidence: `tests/rooms_api.rs:704-705` posts one valid chunk and asserts
  200. The negative cases checked over HTTP are only `/gap` with a stale
  epoch, and `/gap` during finalization (`:630`, `:716`).

  `audio_interval` is unit-tested (`media.rs:461-468`). These are not:
  - the `member_generation` fence, which stops a stale tab from uploading a
    reconnected participant's audio;
  - `sequence()`, which refuses duplicate, reordered or too-frequent chunks.

  `grep member_generation` in the tests finds only the happy-path request.
  Each of the following passes every test:
  - dropping the generation comparison;
  - accepting `seq <= last`;
  - dropping the 15 s screen-frame floor.
- Fix:
  1. In the same `rooms_api.rs` flow, after the 200, re-post the same
     `sequence: 1` and assert 409. Post with `member_generation: generation + 1`
     and assert 403. Post with `capture_epoch: 1` and assert 403.
  2. Add a pure unit test of `sequence()`: zero, equal, lower, and a screen
     chunk under 15 s are refused; the next sequence is accepted.
