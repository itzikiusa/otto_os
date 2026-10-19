# Current verification matrix

## Latest integration status (supersedes historical pending items below)

Nextest filter guard **passed** using Python3.13, exit0:
`PATH=/tmp/otto-nextest-0.9.146:$PATH /opt/homebrew/bin/python3.13 scripts/check-nextest-filters.py`.
Log /tmp/otto-root-nextest-filter-py313.log. This closes the missing-tomllib
invocation failure and compiles/lists current tests; it does not execute newly
changed reservation tests. Root Insights phone follow-up browser suite3/3 and
UI guards also passed (see iteration-02-insights-ux.md).

Fresh combined UI gate: **npm run check passed, 2,056 unit tests passed (zero
failures/skips), and production build passed**. Bundle gate still fails only
page:plugin at 8.5KB vs 8.3KB (+3.3%); Ronaldo owns the reduction. Budget unchanged.
Logs: /tmp/otto-root-ui-final-{check,unit,build,bundle}.log.

Pele's reservation batch exited101: assistant74/automation78 passed; state472
passed, one failed, two ignored. The failed proactive restoration fixture used
an agent snapshot from before save_autonomy advanced admission_generation.
Pele retains Cargo for the fixture correction and focused state/swarm checks;
swarm library did not start in that failed batch. Independent refund findings
remain open and are distinct from this fixture issue.

Latest combined root UI browser run: **14/14 passed in 27.3s**, all four named
Proof/Design/API/Insights specs against the rebuilt debug daemon (slot22).
The subsequent lazy Credentials dialog browser check also passed. Current
UI guards pass (1,086 files), and all six Product chat ownership regressions
pass after the earlier full unit run reported those six failures. A fresh combined
UI check/unit/build/bundle gate is running; the earlier full unit failure is not
being treated as a full-suite pass. The latest prior build passed, with only the
Plugin bundle over its existing tolerance; Ronaldo has since edited that module.

Rust workspace r3 passed formatting, all-target clippy, **5,978 nextest cases**
with 124 skipped, and doctests. Its final nextest-filter guard failed on system
Python missing tomllib; a Python3.13 rerun remains pending. Pele's reservation
changes postdate the workspace test compilation and are being tested separately:
assistant74 and automation78 passed, state475 is still running, swarm has not
started. Pele retains Cargo until the entire command exits. Independent refund
findings remain assigned to him; see iteration-02-automation-independent.md.

The rebuilt debug daemon's isolated active held-shell restart smoke passed with
the same shell PID and working post-restart input. Fixture cleanup completed.
This proves held-session restart/reattachment, not global detached-job drain.
No merge or deployment has occurred.

Root UI closures now have named browser evidence:

- Proof/review: four cases, including actual light/dark phone layouts, saved
  launcher draft navigation, finding-reason cancellation and proof load Retry.
- Design/Canvas: five cases covering editable phone scenes, assistant controls,
  Site panels/toolbar, Brand navigation and tablet 3D controls. The toolbar's
  corrected width additionally passed its focused rerun without the observed
  ResizeObserver loop. Pointer pan/pinch math has two unit regressions.
- API/browser: three cases covering phone request tabs/results, reachable site
  credentials/summary provenance and failed Git pull Retry. Existing browser
  send-to-session confirmation coverage also passed.
- Insights: two cases cover elapsed wall-clock catch-up and retained reports
  with Retry after a failed refresh.

See iteration-02-{proof-review,design-canvas,api-browser,insights}-ux.md for exact
commands and boundaries. These use isolated daemons and focused route fixtures;
they are not evidence of every external integration. The full UI unit run of
2,049 passing tests predates the latest API/browser and Insights edits. Final
combined UI checks remain pending while Ronaldo and Pele finish their assigned
UI groups. Full-product 9.8, merge and reinstall are not yet established.

The earlier Canvas guard/bundle and API compiler-banner pending entries below
are historical: those repairs have passed their focused checks. Pele reports
swarm library 64/64 and all-target clippy, plus design-assist 13 and state Canvas
12/1 ignored. Repeating unchanged package batches is not required.

This supplements the historical iteration reports; no full-product 9.8 sign-off
has been established. One PR111 remains open at c009da20, while expanded changes
are uncommitted. No new reviewer sessions may be created. Ronaldo/Pele/root own
7/7/6 engineering areas respectively.

| Area | Owner | Evidence available | Required closure still pending |
|---|---|---|---|
| E01 sessions | root | Root:270 lib; server lifecycle6/full1087; independent follow-up review | Ronaldo fixing newly confirmed cross-session late claim window; fresh affected checks required |
| E02 workflows | root | Root:62 lib; server DB/API actor/auth, loop replay, schedule cancellation passed | Final workspace gate |
| E03 git | root | Root:408 lib pass,1ignored; clippy;10 native stash regressions | Ronaldo independent review passed; compile after test extraction; shared consumers |
| E04 schedules | Pele | Pele:78 automation lib; all-target clippy | Never-started admission/cursor residual closure; final workspace gate |
| E05 goal loops | Pele | Pele:78 automation lib; all-target clippy | Final workspace gate and remaining UX findings |
| E06 assistant | Pele | Pele:74 assistant lib; all-target clippy | Never-started proactive reservation residual closure; final workspace gate |
| E07 swarm | Pele | Pele:61 swarm lib plus earlier8 service; all-target clippy | Pause/reservation/cancel residuals remain with Pele; final workspace gate |
| E08 API | Ronaldo | Lead:25 apiclient lib; root server API/gRPC tests passed; UI83/full2037 | Compiler-backed banner regression with Pele; final gate |
| E09 connections | Ronaldo | Lead:SSH37,connections82+isolation/global grants/export,brokers69; clippy | Shared consumers and final snapshot |
| E10 Vault/memory | Ronaldo | Lead:Vault91+32engine,memory8+7integration; clippy | Shared consumers and final snapshot |
| E11 Design/Canvas | Pele | Pele:145 design/1ignored,13 design-assist,30 canvas,12 state-canvas/1ignored; clippy | Cross-scene save-guard leak remains red; Canvas bundle budget; final gate |
| E12 Product | Pele | Pele:157 product/1ignored,125 issues; clippy; root server lifecycle tests pass | Old-revision retention boundary; final gate |
| E13 proof/review | root | Root:25 core proof+16 state reviews; server proof/publication/retry/provisioning pass | Final workspace gate |
| E14 state | Ronaldo | Prior state462pass2ignored; rootpre13:6+db12; clippy; atomic/settings11 | Full final schema/consumer gate |
| E15 runtime | root | Root:3 background/2 schedule cancellation tests; independent source review9.7 | Daemon binary tests in active workspace gate; detached work not globally drained |
| E16 credentials | root | Real-Git challenge fixtures; full Git tests+clippy pass | Server/review callers compiled; final independent closure |
| E17 channels | Pele | Pele:139 channels lib; all-target clippy; root server compiles/tests | Final workspace gate |
| E18 accounting | Ronaldo | Lead:usage56/transcript80; new strict unit1/live rebuilt CH replay1 zero skip; clippy; root budget1 | Final workspace gate; journal/legacy operational boundaries documented |
| E19 extensions | Ronaldo | Lead:MCP110,skills20,context64,browser133/clippy; root server plugin6 passed | Final workspace gate |
| E20 UI contracts | Ronaldo | UI94/full2037; state7/server atomic1; lead mounted watcher browser3 passed | Final workspace gate; browser evidence uses older daemon with route/WS fixtures |

Lead-reported counts are attributed, not independently rerun results. Many package
results precede other lanes' latest changes; final coherent workspace validation
remains mandatory. The 16 state-review filter tests include five skill_reviews.

## UI and design evidence

- Ronaldo reports D01-D10 iteration2 at9.8, source citations and owned-file guards/
  Svelte checks. These are lane assessments pending independent browser coverage;
  they are not a whole-application passing type check.
- Root current UI unit gate:2037pass,0fail,0skip (20.683s). Palette recovery
  harness repaired. Log: `/tmp/otto-product-wide-ui-unit.log`.
- RequestBuilder syntax repair landed; whole UI type check now passes. The
  banner regression still needs compiler-backed coverage, allocated to Pele;
  source-string assertions alone cannot prove rendering.
- Root NewSession named browser cases:4pass,0skip/flaky (8.6s), desktop/phone light/dark.
  Latest run additionally asserts every visible phone counter is at least36×36px.
  Log: `/tmp/otto-product-wide-sizing-browser-touch.log`; screenshots refreshed.
  Four screenshots retained in screenshots/. Current UI used an older isolated
  daemon binary: this is layout evidence only. Phone counter hitboxes passed the explicit36px assertions.
- Changed/new Rust formatting:147 files pass at latest recorded sweep; LOC including
  untracked sources passes; disk and macOS CI coverage guards pass.

## Final release gates

Use the existing PR, not separate PRs. Complete current workspace clippy/tests/
doc-tests, UI check/unit/build/bundle budget, relevant named browser specs and
required repository gates. Review actual failures; advisory-only CI is excluded
per user instruction. Update the PR around the complete final scope, push and
wait for the new head's checks. Only then merge and push main, rebuild/install,
replace the running app and verify deployment receipt/health/source identity.
No current evidence establishes completion of these steps.

## Latest full UI gates

Root `npm run check` passed exit0: guards1085 files, zero Svelte errors/warnings,
node/e2e/unit TypeScript configs passed. Production `npm run build` passed in
9.54s. Bundle gate failed only Canvas11.2KB gzip vs10.8KB (+3.4%); assigned to
Pele. No budget baseline raised. Logs /tmp/otto-product-wide-ui-check.log and
/tmp/otto-product-wide-ui-build.log. Full UI unit rerun passed2037/2037 after the palette harness repair.
Later source changes still require affected tests and the final coherent gate.

## Shared compilation milestone

Root `cargo check -p otto-server --tests` passed (66s, exit0). All server test targets compile after fixing scheduled-task moved-flag access and the workflow PR typed credential caller. This closes compilation gaps, not runtime assertions or clippy. Log: `/tmp/otto-product-wide-server-check.log`. Cargo released to Pele.

## Focused server runtime evidence

Budget1/1 passed. Shared focused batch72/73 passed, zero ignored; startup cancellation cleanup failed for a created session. Root repaired kill registration/retirement ordering and added a manager regression; manager regression1/1 and full sessions270/270 passed; server relink/rerun pending. Workflow DB/API actor+auth, loop replay, proof/publication/provisioning/retry, API/gRPC, settings, plugin and background shutdown filters passed in that binary. Exact per-test evidence `/tmp/otto-root-server-focused-tests.log`; durable summary in verification-evidence.json. This is not a green overall server gate.

## Server cleanup failure closed by execution

After rebuilding against kill retirement, all six lifecycle tests passed, including the formerly failing startup-cancellation drain. The broader focused batch was rerun with the budget filter included: **74 passed,0failed,0ignored (7.80s)**. This supersedes the earlier72/73 result; final workspace gates remain. Logs `/tmp/otto-root-server-lifecycle-tests.log`, `/tmp/otto-root-server-focused-tests.log`; durable summaries/hashes in verification-evidence.json.

## Full server library execution

Root ran the rebuilt server library binary directly: **1087 passed, 0 failed, 1 ignored (87.10s)**. Log `/tmp/otto-root-server-lib-all.log`. This follows the lifecycle retirement repair and supersedes the focused-only runtime boundary. Ronaldo independently reviewed the retirement locking/error restoration; root subsequently moved network cleanup after successful kill so a rejected holder kill preserves tunnels. That final ordering change still needs affected verification.

Ronaldo reports mounted Activity/Outputs browser recovery **3/3 passed** (slot21); the fixture uses the older isolated daemon and verifies current UI with route/WS fixtures, not rebuilt daemon behavior. Pele reports nine owned package lib batches and all-target clippy green; exact attributed counts remain in the lane reports. Final coherent workspace gates remain pending.

## Active workspace gate

Root workspace formatting and all-target clippy now pass after replacing one synchronous recursive cleanup in an async accounting test with Tokio cleanup. `scripts/check.sh --all --check --no-ui` is running workspace nextest, then doc-tests/filter checks. Log `/tmp/otto-product-wide-workspace-gate-r2.log`; no final test result yet. Later source repairs require fresh affected checks.

## Workspace nextest result

Root `cargo nextest run --workspace --profile default --no-fail-fast` completed: **5973 passed,0failed,124skipped (246.796s)** across106 binaries. Full all-target clippy and workspace formatting passed immediately before it. Workspace doc-tests/filter guard are still running. The late-capture atomic claim repair landed after these binaries were built and needs a fresh affected test run; this result does not cover it. Log `/tmp/otto-product-wide-workspace-gate-r2.log`.

## Latest affected-consumer result

After the atomic late-capture claim repair, root sessions/server nextest passed **1586/1586,6skipped (123.668s)**, including the new claim regression. The Python3.13 nextest filter guard passed; the check.sh invocation's only final failure was default Python3.9 lacking tomllib. Doc-test gate covered42 targets with0 executable passes/0 failures/1ignored. Current consumer clippy is running.

Root isolated current daemon smoke: fresh boot and repeat boot both reached health and exited0 on SIGTERM in0.043s/0.040s. Binary identity and retained fixture logs are in `daemon-shutdown-smoke.json`. This is empty E2E-mode startup/shutdown evidence, not active-job quiescence.

## Additional blocking CI gates

Root required Vault50k scale gate1/1 passed (16.758s), telemetry ignored recovery suite5/5 passed (158.15s, including real collector), and release terminal perf budget1/1 passed (1.70s) at existing CI scale3. No thresholds changed. A further Run-with-Otto supervisor admission gap was repaired and is undergoing focused verification; prior workspace passes do not cover that later edit.
