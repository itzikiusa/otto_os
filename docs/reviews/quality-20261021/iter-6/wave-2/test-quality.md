# Test-quality review: iteration 6, wave 2 (2026-10-23)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (grep, rg, sed, git diff). No cargo, npm or e2e runs. Branch
`fix/quality-iter6-20261023` at `4f6d6ab30`. The scope is the same as wave 1
(the areas listed below), plus the lens-relevant parts of `git diff origin/main`.
That diff now includes the wave-2 fixes (`37738d908..HEAD`) and the merged
`otto.query_db` feature. Every finding below was confirmed by reading the cited
lines. For each new or changed test I asked one question: would deleting the
code it covers make it fail?

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 0 major, 2 minor, 1 nit.

10.0 − (2 × 0.1) − (1 × 0.03) = 10.0 − 0.2 − 0.03 = **9.77 / 10** (wave 1: 9.4)

- Five of the six wave-1 minors are fully fixed: 6w1-02, 6w1-03, 6w1-04,
  6w1-05 and 6w1-06.
- 6w1-01 is fixed except for one of its seven listed ingest sites,
  `/ingest/codex`. That remainder is the nit **6w2-01**.
- The fixes and the merged `otto.query_db` feature introduced two new minors:
  - **6w2-02:** the new `mcp_db_execute.rs` suite says a write or read
    "reaches the driver", but only checks that certain strings are absent.
    The read case checks for the wrong string, so it cannot fail.
  - **6w2-03:** the new assist-job tests need a 5 ms poll to fire inside a
    60 ms wall-clock window. The poll and the turn race in an unbiased
    `select!`.

## Previous findings (wave 1)

| ID | Status | Evidence |
|----|--------|----------|
| test-quality-6w1-01 | **Partly fixed** (rest is **6w2-01**, nit) | **Fixed:** <ul><li>`public_entry_gates.rs:378-568` `ingest_routes_write_only_with_the_sessions_own_token` builds a real swarm agent session and mints its token and another session's token (`:486-491`). It drives `swarm/board`, `swarm/product`, `swarm/mockup`, `swarm/discovery-report`, `usage` and `claude` (`:493-512`).</li><li>For no token, an empty token and the other session's token, every route returns 204 and `written(..)` equals `before`. The fixture reads board rows, stories, attachments, the report, the trail and the usage stamp straight from the stores (`:536-550`).</li><li>Then the session's own token writes each of them (`:552-566`). Deleting any of the six `verify_ingest_token` calls makes the matching negative case write, so the test fails.</li><li>`plugin_host_routes_need_an_enabled_plugins_bearer_token` (`:576-645`) drives all four `/plugin-host/*` routes. No bearer, an empty bearer, a disabled plugin's token and an unknown token each get 401, and no session is created. The enabled token passes the gate on the three read routes. `/agents/run` gets a missing `cwd`, so a call that slipped past the gate would return 500, not 401.</li></ul> **Still open:** `/ingest/codex`. `activity.rs:385-410` `codex_ingest` calls `ingest_session` (`:412-424`), which calls `verify_ingest_token` at `:421`. It is not in the test's route list (`:493-512`), and `rg "ingest/codex" crates/otto-server/tests` finds only the `router_mount.rs` inventory and the policy snapshot. |
| test-quality-6w1-02 | **Fixed** | The board case now runs against a real session with a minted token (`:475-491`). The positive case (`after.board == 1`, `:561`) is what makes the negative cases count, as wave 1 asked. The suite header (`:12-14`) now describes this correctly. |
| test-quality-6w1-03 | **Fixed** | `database_changes_api.rs`:<ul><li>`:210-221`: a peer cancelling the author's `awaiting_review` change gets 403, and the status is unchanged. This kills the `service::author` deletion.</li><li>`:293-330`: the status is set to `running`, and both `peer` and `reviewer` (who has `change_submit` but not `change_execute`) cancelling gets 403. `cancellation_requested` stays false. This kills `:295` → `"change_submit"`.</li><li>`:332-360`: with an `outcome_unknown` attempt row, a peer reconcile gets 403 and the attempt state is unchanged. This kills the deletion of `:315`.</li></ul> A 403 can only come from `author`/`authorize`, because the peer can see the change (`:219-221`). |
| test-quality-6w1-04 | **Fixed** | `review_run.rs:70` `blended_score` is used at `:839`. `tests/unit/workflow_engine.rs:1865-1874` checks `(40,Some(100))→70`, `(90,None)→90`, `(0,Some(0))→0` and `(81,Some(80))→80`, which kills both `→gs` and `→review_score`. `:1877-1899` checks that the template's `{"scoring":{"bug":10,"warn":5,"info":1}}` gives those weights, that `{}` and a non-integer value give the defaults, and that a partial override changes only `warn`. This kills `p.get("scoring")` → any other key. |
| test-quality-6w1-05 | **Fixed** | `TurnSession` (`otto-swarm/src/runtime/run.rs:256-282`) is resolved once at `:461` and passed to `run_attempt` (`:655-659`). Inside, the reuse lookup reads `owner.reuse_user` (`:841`) and the create reads `owner.creator` (`:890`). `run_attempt` has no `swarm` in scope, so the old mutants (`&swarm.created_by`) no longer type-check. `swarm_identity.rs:671-688` resolves the owner for Bob's task in Alice's swarm and asserts that both fields are Bob, not `swarm.created_by`. `run.rs:1350-1355` pins `for_acting_user`. |
| test-quality-6w1-06 | **Fixed** | Each case wave 1 listed is now driven through the real code:<ul><li>Mermaid race (`p09-product.test.ts:80-106`): the second render resolves first, `mermaidSvg` keeps B, and `rendering` ends false.</li><li>Debounce (`:124-143`): a fake clock, a cleanup between the edits, and exactly one render after `LIVE_DEBOUNCE_MS`.</li><li>Reorder (`:185-220`): it runs on the real `caseOrder.svelte.ts` and checks the persisted orders, the queue during a save and the focus targets.</li><li>Focus (`p10-focus-a11y.test.ts:17-89`): Inspector `go()`, `LinkListField.remove()` and ClassroomsBox `upTo()` run against stub elements.</li><li>Tab move (`p06-loadStatesAndTabs.test.ts:30-92`): `onRepoTabKey`/`moveTab` run over a fake store, in LTR and RTL.</li></ul> The source regexes that remain in these files are labelled "(structure)". |

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

## Areas NOT covered yet (next iteration)

These are unchanged from wave 0.

- `otto-sessions/src/ws.rs` beyond the input queue, and
  `otto-sessions/src/mcp.rs`.
- `otto-telemetry-collector` recovery semantics.
- The `swarm_ingest` product, mockup and discovery-report handler bodies
  beyond the token gate.
- `workflow_engine/nodes/{swarm_task, self_improve, product}.rs`.
- The AWS and Kubernetes consoles, Insights, skills-eval, otto-vault and
  otto-design.
- The shooter and royale sims.
- The `apps/desktop/src-tauri` supervisor.

## Findings

### test-quality-6w2-01: nit. Rest of 6w1-01: `/ingest/codex` is the one ingest route whose token check no test covers

- Files:
  - `crates/otto-server/src/routes/activity.rs:385-424`: `codex_ingest`
    calls `ingest_session`, which calls `verify_ingest_token` at `:421`.
  - `crates/otto-server/tests/public_entry_gates.rs:369-378` and
    `:493-512`.
- Evidence:
  - The new test's doc comment says it covers "every `/ingest/*` route that
    takes a session's hook token". Its route list has `claude` but not
    `codex`.
  - `rg "ingest/codex" crates/otto-server/tests` finds only the
    `router_mount.rs:242` inventory and `snapshots/policy_decisions.txt`.
  - Deleting `:421`, or making `ingest_session` skip the check, passes every
    test. After that, anyone who can reach the daemon and knows a session id
    can write trail entries such as "Agent finished responding" into any
    session.
  - This is rated a nit and not a minor: it is one site out of seven, and it
    can only append a trail line.
- Fix: add
  `("codex", json!({"type": "agent-turn-complete"}))` to `routes` in
  `ingest_routes_write_only_with_the_sessions_own_token`.
  - The `trail` counter already measures what it writes.
  - To keep the positive case per-route, assert the trail grows by one after
    the own-token `claude` post, and again after the own-token `codex` post.
    The current `after.trail > 0` would be met by either route alone.

### test-quality-6w2-02: minor. `mcp_db_execute.rs` claims "reaches the driver" by checking that certain strings are absent, and the read case checks for the wrong string

- File: `crates/otto-server/tests/mcp_db_execute.rs`:
  - `:250-262`: after approval, the write on the writable connection is
    asserted only as
    `!err.contains("write_blocked") && !err.contains("read-only") && !err.contains("403")`.
  - `:297-305`: "A read on the guarded connection is not a write: it reaches
    the driver" is asserted only as `!err.contains("write_blocked")`.
- Evidence:
  - The connection is access-enforced, so the read-only refusal on that path
    is
    `Error::Forbidden("read-only connection '…' cannot perform database mutations; turn off Read-only …")`
    (`otto-dbviewer/src/service.rs:909-914`). That message has no
    `write_blocked` prefix. The test's own write case (`:285-290`) gets that
    exact text.
  - So if `GET otto:test` were classified as a mutation (`access::operations`),
    or the `conn.read_only && operations.iter().any(..)` check at `:909`
    refused every statement, the read would be refused with "read-only
    connection 'cache-ro' …". `:305` would still pass. The one MCP-level case
    meant to show that a guarded connection still serves reads cannot fail.
  - At `:257-262`, any other refusal also passes. Examples: a
    `review_required:` gate, a resource-access "Forbidden" whose text has no
    "403", or a stale-approval message. The test never shows that the call got
    past the gates. (`call_error` only checks `executed && is_error`.)
  - The dbviewer-level tests do this properly: they match on the error
    *variant* (`mcp_query.rs:191-196`, `:519-525`). The server-level suite
    does not.
- Fix: assert positively that the driver was reached:
  1. In `:257-262` and `:300-305`, assert that `err` contains the
     dead-port connect error the fixture produces (for example
     "Connection refused" / "connect", whatever the Redis driver reports).
  2. Or, at the least, also assert
     `!err.contains("read-only connection") && !err.contains("review_required") && !err.contains("Forbidden")`.
  3. Better still, pass a parsed error kind through the envelope and match
     on it, as the dbviewer suite does.

### test-quality-6w2-03: minor. Assist-job tests need a 5 ms poll to fire inside a 60 ms wall-clock window, and the poll races the turn in an unbiased `select!`

- File: `crates/otto-agent-run/src/assist/tests.rs`:
  - `:179`: `SETTLE = 60ms`.
  - `:188-191`: the fake turn writes "draft one", then sleeps `SETTLE` and
    returns.
  - `:233-237` (`success_applies_once_and_stops_the_live_poll`) and `:259`
    (`stop_mid_turn_applies_nothing_and_stops_the_poll`) assert
    `doc.live() == ["draft one"]`.
  - `:370-373` (`a_dropped_job_stops_its_live_poll`) asserts the same after
    `timeout(SETTLE * 2)`.
- Evidence:
  - The live preview is recorded only if `live_poll`
    (`assist/mod.rs:307-333`) wakes from its 5 ms sleep, stats and reads the
    file (async fs, so a blocking-pool round trip), and calls `on_live`
    before the turn's 60 ms sleep finishes.
  - The two futures race in an unbiased `tokio::select!`
    (`assist/mod.rs:256-259`). Suppose the runtime thread is starved for 60 ms
    or more after the write. That happens under the nextest `ci` profile, and
    on this Mac under parallel builds. Then both timers are due together, and
    `select!` picks a branch at random. If the turn wins, the poll is dropped
    before it reads, `live()` is `[]`, and the test fails even though the code
    is correct.
  - This is the "flaky timing-based test" pattern: a correctness check that
    depends on the scheduler.
- Fix: make the fake turn wait for the preview instead of sleeping.
  1. Give `FakeDoc` a `tokio::sync::Notify` that `on_live` signals.
  2. In `FakeRunner::run_turn`, replace `sleep(SETTLE)` after the write with
     `timeout(Duration::from_secs(5), doc_seen.notified())`. Panic on timeout,
     with a message saying the preview never came.
  3. In `StoppedMidTurn`, keep the post-stop write followed by a short
     settle. That case asserts something does *not* happen, so a fixed wait
     is acceptable there.
  4. `assert_poll_stopped` can keep its sleep for the same reason.
