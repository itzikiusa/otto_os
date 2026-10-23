# Test-quality review: iteration 6, wave 3 (2026-10-23, final)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (grep, rg, sed, git diff). No cargo, npm or e2e runs. Branch
`fix/quality-iter6-20261023` at `2065f00e9`. Scope: the same areas as wave 2
(listed below), plus the lens-relevant parts of `git diff origin/main`, with
the wave-3 fixes (`4f6d6ab30..HEAD`) read in full. For each new, changed or
moved test I asked one question: would breaking the code it covers make it
fail?

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 0 major, 0 minor, 0 nit.

10.0 − 0 = **10.0 / 10** (wave 2: 9.77, wave 1: 9.4)

- All three wave-2 findings are fixed at every listed location.
- The fresh pass over the same areas found no new finding. That includes the
  tests the wave-3 fixes added or moved.

## Previous findings (wave 2)

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-6w2-01 (nit) | **Fixed** | `crates/otto-server/tests/public_entry_gates.rs:512` adds `("codex", json!({"type": "agent-turn-complete"}))` to `routes`. That body maps to a trail entry in `normalize_codex` (`activity.rs:429-432`), and `codex_ingest` reaches `ingest_session` → `verify_ingest_token` (`activity.rs:390`, `:421`). <ul><li>Negative cases (`:537-551`): no token, an empty token and another session's token each leave `written(..) == before`, codex included.</li><li>Positive case (`:557-576`): every own-token post on `claude` or `codex` must grow the trail by exactly one, and `after.trail == 2` (`:577`). Deleting `:421` makes the codex negative cases write, so the test fails. A per-route positive also means the codex route can no longer be silently satisfied by the claude post.</li></ul> |
| test-quality-6w2-02 (minor) | **Fixed** | `crates/otto-server/tests/mcp_db_execute.rs:218-242`: `assert_reached_driver` now requires `DRIVER_REACHED = "Connection refused"` (the dead-port `127.0.0.1:1` connect error). It also requires the absence of `read-only connection`, `review_required`, `Forbidden`, `forbidden:` and `write_blocked`. <ul><li>It is used for the approved write (`:281-282`) and for the `GET` on the read-only connection (`:319-329`).</li><li>If `GET` were classified as a mutation, the enforced gate would answer "read-only connection 'cache-ro' …", which fails both halves of the helper. The read case can now fail.</li><li>The refusal cases (`:306-317`) still match the exact refusal texts.</li></ul> |
| test-quality-6w2-03 (minor) | **Fixed** | `crates/otto-agent-run/src/assist/tests.rs`: <ul><li>`Previews` (`:39-79`) pairs the recorded previews with a `Notify`. `first()` registers interest (`enable()`) before it checks, so a push in between is not lost. `wait_first()` caps the wait at 5 s with a named panic.</li><li>`FakeRunner::run_turn` (`:231-256`) writes "draft one", then waits on `wait_first()` instead of `sleep(SETTLE)`. The turn cannot finish before the poll previewed, so the unbiased `select!` in `assist/mod.rs:256-259` can no longer drop the poll first.</li><li>`a_dropped_job_stops_its_live_poll` (`:417-433`) drops the hanging job when the first preview arrives, not after `timeout(SETTLE * 2)`.</li><li>`SETTLE` (`:228`) remains only for negative checks: the post-stop write and `assert_poll_stopped`. A fixed wait is acceptable there, as wave 2 noted.</li></ul> |

## Areas covered

These are the same as wave 0 and wave 1.

- **All ten iteration-5 findings:** re-verified in wave 0. No regressions
  were found in this pass. `mcp_auto_approve.rs`, `session_files.rs`,
  `mcp_profiles_edges.rs`, `nodes/git_pr_tests.rs` and `channel_notify.rs`
  are unchanged by the wave-2 diff.
- **Iteration 5's "not covered" list:**
  - `workflow_engine/nodes/review_run.rs`: 6-04 and 6w1-04, now fixed.
  - `loop_node`.
  - `swarm_task`, `self_improve`, `product`: thin dispatchers.
  - The classroom unit suites.
  - Shooter and royale sims: not deepened.
  - `otto-channels/src/bridge.rs`: the allow-list gate.
  - `otto-issues/src/atlassian.rs` retry.
  - Broker consumer-group reset.
  - `patch_session` server-owned meta keys.
- **New code from the iteration-5 fixes:**
  - `rooms/capability.rs`.
  - Game-room relay.
  - Budget fail-closed and the measured path (6-03).
  - Swarm prompt-editor identity, the directive gate and the
    channel-trigger actor (the `swarm_identity.rs` tests, plus the new
    `TurnSession` case).
  - The workflow `approve_run` agent-token refusal.
  - The UI helper suites.
- **Unauthenticated routes:** every route in `public_routes()` was mapped
  to its tests again. `routes/mod.rs` and `policy.rs` are unchanged since
  wave 1. Only `/ingest/codex` is still unpinned (**6w2-01**).
- **The database-change review workflow:** the approve, reject, cancel,
  reconcile and execute authority checks are all driven over HTTP now
  (6w1-03 fixed).
- **Git PR merge:** `merge_pinned` wire tests. No change, no finding.
- **e2e:**
  - Seven specs changed in wave 2: `desktop-network-profiles`,
    `desktop-review4-product-publication`, `desktop-review5-scheduled-draft`,
    `desktop-room-window`, `desktop-ux-r3-automation`,
    `desktop-workflow-recovery` and `room-recap`. Each change moves a
    `getByRole('alert')` locator to the element's class or test id, because
    the role was removed (a11y-6w1-01).
  - Every assertion still checks the same visible text. `room-recap` is
    stronger: it now asserts the "Couldn’t refresh this recap" lead as well as
    the cause.
  - The spoken announcement is pinned in unit tests instead
    (`w2p8-loadStateAnnounce`, `w2p8-announceShown`).
  - No new spec has a `test(` without an assertion.
- **The lens over `git diff origin/main`:** this covers the wave-1 suites
  above and the wave-2 additions:
  - **Rust:**
    - `otto-agent-run` `fanout/retry_tests.rs`: the waits are ordering, not
      deadlines. `wait_for_retry` checks `take` before `open`, and the cancel
      wait has no upper bound. No finding.
    - `otto-agent-run` `assist/tests.rs`: **6w2-03**.
    - `otto-agent-run` `session_turn/resume.rs`: the four
      `assist_resume_ownership` cases were ported and run against both
      authorities.
    - `canvas_refs_api.rs`.
    - `tests/unit/vault_docs_agent.rs` (2 new tests).
    - `otto-sessions` `manager/tests/submit.rs` (RunGuard).
    - The `otto-mcp` governance stage table.
    - `otto-mcp` `outward/tests.rs` (`query_db` catalog, route and card).
    - `otto-dbviewer/tests/mcp_query.rs`.
    - `mcp_db_execute.rs`: **6w2-02**.
  - **UI:**
    - The 15 `ui/unit/w2p*.test.ts` files. Four read source; their regexes
      are structural and labelled.
    - The tests that were removed or ported:
      - `workflowPublicationPreview.test.ts` and the `workflowInspector`
        final-output case → `w2p7-runMonitor.test.ts:143-240`. Every case is
        kept, now against the extracted `RunMonitor` class.
      - `k8sMonitorTableWindow.test.ts` → `w2p5-tableWindow.test.ts`.
      - The two branch-delete cases from `gitUiSafety`/`gitUiSafety3` →
        `w2p5-graphActions.test.ts:106-128`, with the full request bodies.
        `splitRemoteRef` is still pinned at `gitUiSafety.test.ts:21-24`.

      No coverage was lost.


### Lens over the wave-3 diff (`4f6d6ab30..HEAD`)

This section reports what the wave-3 fixes did inside the areas above. It
does not add new areas.

- **No test was lost in the moves.**
  - Every `#[test]` / `#[tokio::test]` name under `crates/` at `4f6d6ab30`
    still exists at HEAD. The check was `git grep` over both revisions plus
    `comm -23`: 6,659 names before, 6,693 after, none missing.
  - This covers the moves out of:
    - the `otto-product` `run.rs` split. Its 37 tests are in `run/tests.rs`
      (36) and `run/retry.rs` (1). `run/engine_tests.rs` adds 8.
    - `otto-server` `api_secrets.rs`, `routes/api_client*.rs` and
      `tests/unit/api_client.rs`, now in `otto-apiclient` `engine/tests.rs`
      and `secrets/tests.rs`.
    - `ottod` `mcp_tools.rs`.
    - `otto-server` `rooms/` and `game_rooms/`, now in `otto-rooms`.
  - The same check over `ui/unit` test titles shows nothing missing.
- **`gitGraphSplice.test.ts`:** its four component-scope cases were removed
  and ported to `w3p1-graphData.test.ts:44-87`. They now run against the real
  `graph/graphData.svelte.ts` (`lostRef`, `setRefs`, `applyGraph`). The
  histories are padded past the real `FIRST_PAGE = 2000`, which the test
  asserts at `:31`, so "outside the fresh page" stays meaningful.
  - The `spliceHistory: () => null` stub does not hide anything:
    `applyGraph` (`graphData.svelte.ts:581-612`) never calls it.
  - The "inside the page" case asserts no reload, the new commit list, and
    that the tip was consumed.
- **`swarm_ingest_bodies.rs`:** a new suite for the product, mockup and
  discovery-report bodies behind the gate.
  - Every bad-body case (400 / 422 / 413 / blank 204) compares a full
    `Rows` snapshot read from the stores. The mockup cases also check that
    no attachment directory was created (`:503-505`).
  - Each route has its own positive test (`:300-371`, `:412-457`,
    `:533-552`).
  - The S4-03 forged-meta cases (`:554-594`) cover the three checks in
    `verified_swarm_scope`: the swarm's workspace (`:67`), the project's
    swarm and workspace (`:71-78`), and the derived swarm. Each has at least
    one forged session that a deleted check would let write.
  - For `product`, the project-only forged session is stopped earlier, by
    the `swarm_id` presence check (`swarm_ingest.rs:251`). The other two
    forged sessions still drive the scope check, so no mutant survives.
- **`swarm_router.rs`:**
  - `swarm_turn_submit_does_not_interleave_with_submit_text`
    (`:466-502`) fires the broadcast when the swarm's paste echoes in the
    scrollback, which is inside the paste → Enter window, and polls for up to
    15 s with a named panic. With correct code the outcome does not depend on
    timing: the broadcast waits on the lock. Without the lock it merges into
    one line, so the test can fail.
  - `swarm_turn_submit_types_nothing_for_a_stopped_run` (`:508-526`) proves
    that nothing was typed: a later Enter flushes only "after".
- **`ottod` `mcp_tools/tests/twin_tests.rs` + `routing_tests.rs`:**
  - The snapshots were taken from the hand-written arms before the
    generation change. `assert_snapshot` (`:40-58`) fails on a changed,
    missing or extra entry. Blessing needs the explicit
    `OTTO_BLESS_TWIN_SNAPSHOTS`.
  - The dispatch recorder logs method, path, query and body for every twin
    case. `every_twin_has_a_dispatch_case` stops the case list from
    drifting.
- **`otto-product` `run/engine_tests.rs`:** a scripted host, no sleeps. Each
  test asserts both the stored row (status, summary, questions, plan,
  versions) and which sessions ran.
- **`otto-rooms` `ctx_tests.rs`:** invites and audit, the authority sync
  refusal, TURN only with a secret (and the secret is not stored in
  plaintext), and the maintenance sweep. Each case asserts state, not just
  return codes.
- **UI `w3p*`:** the new tests drive real functions. These are
  `componentFunctions`, or `loadSource` with the real announcer, `listWindow`,
  `ref-menus` and `sessionAttach`. Source and AST reads are labelled
  "(structure)", and each sits beside a behaviour test for the same rule.
  - `p04-uiGuards` gains positive and negative detector cases for the
    widened opener set. No old case was dropped. The one changed case moved
    its region to the third line, matching the new two-line look-ahead.
- **e2e:** no spec changed in wave 3 (`git diff 4f6d6ab30..HEAD -- ui/e2e` is
  empty).

## Areas NOT covered yet (next iteration)

These are unchanged from wave 2, except that `swarm_ingest_bodies.rs` now
covers the `swarm_ingest` product, mockup and discovery-report handler
bodies.

- `otto-sessions/src/ws.rs` beyond the input queue, and
  `otto-sessions/src/mcp.rs`.
- `otto-telemetry-collector` recovery semantics.
- `workflow_engine/nodes/{swarm_task, self_improve, product}.rs`.
- The AWS and Kubernetes consoles, Insights, skills-eval, otto-vault and
  otto-design.
- The shooter and royale sims.
- The `apps/desktop/src-tauri` supervisor.

## Findings

None open. Every previous finding is fixed, and the fresh pass over the same
areas found nothing that meets the bar: confirmed in the code, inside the
lens, and not a matter of taste.
