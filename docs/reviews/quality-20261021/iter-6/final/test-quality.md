# Test-quality review: iteration 6, final (2026-10-23)

Lens: tests that would not fail if the code broke, happy-path-only coverage of
risky logic, over-mocking, flaky timing-based tests, important untested
behaviour, and e2e specs that assert nothing meaningful.

Method: read-only (git, grep, rg, sed, awk). No cargo, npm or e2e runs. Branch
`fix/quality-iter6-20261023` at `2065f00e9`, the same commit the wave-3 review
read; no code has changed since. Scope: the wave-0 areas, plus the
lens-relevant parts of `git diff origin/main` (22 commits, 835 files, 147 test
files). Excluded: `target/`, `node_modules/`, `docs/reviews/`. For each test I
read, I asked one question: would breaking the code it covers make it fail?

## Score

Start at 10.0. Confirmed open findings: 0 blocker, 0 major, 0 minor, 0 nit.

10.0 − 0 = **10.0 / 10**

Score history: wave 0 8.6, wave 1 9.4, wave 2 9.77, wave 3 10.0, final 10.0.

- All three wave-2 findings are still fixed. The wave-0 and wave-1 findings
  that waves 1–3 marked fixed are still in place.
- The fresh pass found no new finding. Two candidates came close, and both are
  explained below under "Candidates checked and rejected".

## Previous findings

### The latest report (wave 3)

Wave 3 had no open findings, so it adds no rows. Wave 3 rated the three
wave-2 findings below as fixed. I checked each one again against the current
code.

| ID | Status | Evidence (current code) |
|----|--------|-------------------------|
| test-quality-6w2-01 (nit) | **Fixed** | `crates/otto-server/tests/public_entry_gates.rs:512` puts `("codex", json!({"type": "agent-turn-complete"}))` in `routes`. <ul><li>The negative loop (`:537-551`) covers no token, an empty token and another session's token. Each must return 204 and leave `written(..) == before`.</li><li>The positive loop (`:557-576`) requires each `claude` and `codex` post to add exactly one trail line.</li><li>`after.trail == 2` is asserted at `:577`.</li></ul> |
| test-quality-6w2-02 (minor) | **Fixed** | `crates/otto-server/tests/mcp_db_execute.rs:226-242`: `assert_reached_driver` requires `DRIVER_REACHED = "Connection refused"` and the absence of all five gate refusals. It is used for the approved write (`:282`) and for the `GET` on the read-only connection (`:329`), so a `GET` classified as a write fails the test. |
| test-quality-6w2-03 (minor) | **Fixed** | `crates/otto-agent-run/src/assist/tests.rs`: <ul><li>`Previews` (`:36-79`) registers interest before it checks (`enable()` at `:60`), and `wait_first` panics after 5 s.</li><li>`FakeRunner::run_turn` (`:230-256`) waits on `wait_first()` instead of a sleep.</li><li>`a_dropped_job_stops_its_live_poll` (`:417-433`) drops the job when the first preview arrives.</li><li>`SETTLE` (`:228`) is used only for negative checks.</li></ul> |

### Earlier findings, spot-checked for regressions

- **6-01 / 6w1-01:** the webhook, DB-Assist, plugin-host, ingest and Telegram
  allow-list gates.
- **6-02 / 6w1-03:** `database_changes_api.rs`.
- **6-03:** budget `verdict_from`.
- **6-04 / 6w1-04:** the `review_run` pure gate functions.
- **6-05 / 6w1-05:** `TurnSession`.
- **6-06:** `StepSecretBinding::for_caller`.

All are still present and unchanged since wave 3. Every
`crates/otto-server/tests/*.rs` suite except `snips.rs` still has its
`#[path]` line in `tests/it.rs`, so no suite has silently stopped running.

## Areas covered

- **The wave-0 areas:**
  - the iteration-5 findings;
  - `loop_node`, `review_run`, the classroom suites, the `bridge.rs`
    allow-list, Atlassian retry, broker group reset and `patch_session` meta
    keys;
  - `rooms/capability.rs` and the game-room relay, now in `otto-rooms`;
  - budget, swarm identity and workflow `approve_run`;
  - unauthenticated routes, the database-change routes, `merge_pinned` and
    e2e.

  I re-read them where this iteration touched them; the others are unchanged.
- **The governance stage runner (`otto-mcp/src/governance.rs`):**
  - `the_stage_table_for_both_entry_points` (`:257-436`) checks the exact
    stage sequence and outcome for each case: policy deny, require-dry-run,
    require-approval, auto-approve, a forced approval, pending, dry-run and
    the fail-closed audit, for both stage sets.
  - The real registered-server stages are also driven end to end by
    `otto-mcp/tests/pipeline_e2e.rs`: policy deny (`:312-376`), single-use
    re-gate (`:241-249`), and a resource denial that blocks a dry-run too
    (`:381-`).
- **The `otto.query_db` approval binding:**
  - `find_usable_in` filters on `args_hash` (`otto-state/src/mcp_control.rs:1441`).
    `approval_single_use_and_args_binding` (`:1966-`) shows that swapped
    arguments are not usable.
  - At the route, `the_pending_count_matches_the_list`
    (`mcp_auto_approve.rs:1332-1356`) files three calls that differ only in
    `title` and expects three cards. If `mcp_outward.rs:407` ignored the
    arguments, that test would fail. The wiring is pinned.
- **The `RunGuard` submit tests (`otto-sessions/src/manager/tests/submit.rs`):**
  - The three tests use a real PTY and a `cat` sink.
  - `a_run_stopped_between_the_wait_and_the_paste_pastes_nothing`
    (`:146-215`) waits until the guard has been checked and the PTY has been
    quiet for more than 1 s, then waits another 300 ms.
  - The strict TUI wait is ready at 600 ms of quiet, polled every 250 ms
    (`submit.rs:28-30`, `:175-206`). So the submit is parked on the lock
    before the run ends, and only the under-lock check (`submit.rs:291`) can
    refuse it.
- **The vault-docs fan-out (`tests/unit/vault_docs_agent.rs:1446-1510`):** the
  40 ms sleeps only order events. The runner takes a pending retry before it
  checks whether the stage is still open, so the outcome does not depend on
  timing.
- **UI tests added in wave 3:**
  - `w3p6-apiClientSplit.test.ts` runs the real store and its parts against a
    fake `api`. It covers the tab-close slot, the failed Postman sync (no
    import, collections unchanged), the curl round trip and the cookie clear,
    including the failure path.
  - `w3p7-chatStop.test.ts` runs the real announcer and the component
    functions. It asserts the exact announcements and focus targets, and
    that focus the user moved elsewhere is not taken back.
- **`ui/unit/w2p5-homePort.test.ts`:** the 120 ms wait at `:69` only orders
  "navigate first, bind later". The handler has no shorter deadline.
- **Twin snapshots (`ottod/src/mcp_tools/tests/twin_tests.rs`):**
  - `assert_snapshot` (`:40-58`) fails on a changed, missing or extra entry.
  - Re-blessing needs `OTTO_BLESS_TWIN_SNAPSHOTS`, which is not set by any
    CI workflow or script.
- **Moved code:** `otto-rooms` (`authority`, `terminal`, `ice`),
  `workflow_engine/run/{admission,schedule,state}`, `otto-design-assist/job.rs`
  and `otto-transcript/roots.rs` are moves or splits. Each kept its tests:
  - `otto-design-assist` has 13 tests before and after;
  - `roots.rs` has 6;
  - `terminal.rs` has 6.

  Wave 3's name-diff check (no `#[test]` lost) still holds at the same commit.
- **e2e:** the 15 changed specs each change one or two lines (locator moves),
  and each still contains `expect` assertions. No spec changed after wave 3.

## Candidates checked and rejected

- **`submit.rs:146-215`, the precondition loop.** The loop that waits for
  "parked on the lock" (`:193-196`) can run out without failing. If the
  spawned submit were starved for more than 4 s, the test could pass with the
  under-lock check deleted. With the test's margin (more than 1.3 s of quiet
  against a 600 ms + 250 ms requirement), this would need multi-second
  starvation. That is speculative, so it is not reported.
- **Approval-to-arguments binding for otto tools.** No route test approves
  arguments A and then sends arguments B. The repo filter and the
  pending-card test above together pin both halves, so the "approve A, run B"
  mutant fails a test. Not a finding.

## Areas NOT covered

These areas have not been reviewed in depth under this lens by any wave of
iteration 6. They are deferred, not judged clean.

- `otto-sessions/src/ws.rs` beyond the auth gate and the input queue (it has
  43 tests), and `otto-sessions/src/mcp.rs` (22 tests, not read case by case).
- `otto-telemetry-collector` recovery semantics.
- `workflow_engine/nodes/{swarm_task, self_improve, product}.rs`.
- The AWS and Kubernetes consoles, Insights, skills-eval, otto-vault and
  otto-design.
- The shooter and royale sims.
- The `apps/desktop/src-tauri` supervisor.

## Findings

None open. Every previous finding is fixed, and the fresh pass found nothing
that meets the bar: confirmed in the code, inside the lens, and not a matter
of taste.
