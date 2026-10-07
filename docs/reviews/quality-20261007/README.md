# Quality review — 2026-10-07

Status: the initial bounded review is complete, with all five category scores at 9.8/10. Publication follow-up is tracked in [PR #94](https://github.com/itzikiusa/otto_os/pull/94): broader CI exposed existing functional-suite failures plus new static-analysis findings, and final closure is tracked in [the publication follow-up](post-ci-closure.md). The scorecard below describes its original reviewed scope, not certification of every later repair. Baseline `196048df` was pulled from main; every change remains on `fix/quality-20261007` in one PR.

## Final scorecard — 11:21 UTC

| Category | Final bounded score | Independent evidence |
| --- | ---: | --- |
| Performance, including OTel | 9.8/10 | [Performance assessment](final-performance-assessment.md): actual CPU/RAM, concurrent-agent transport/rendering, telemetry off/on and query-scale measurements |
| Design | 9.8/10 | [Design/UX assessment](round-2-design-ux.md): repaired states, full populated report hierarchy, light/dark and native evidence |
| UX | 9.8/10 | [Design/UX assessment](round-2-design-ux.md): real task/draft/recovery journeys, corrected report navigation and native accessibility exposure |
| Correctness | 9.8/10 | [Correctness assessment](round-2-correctness.md): recovery, ownership, contracts, races and latest report repair independently closed |
| Test coverage/quality | 9.8/10 | [Test assessment](round-2-tests.md): meaningful regressions, runtime boundaries and recurring plugin enforcement; latest independent closure audit complete |

All five reviewers' category scores now meet the requested target in the explicitly reviewed scope. These are qualitative assessments, not coverage percentages or guarantees for every application path. Historical checkpoints below deliberately retain superseded scores and failed executions.

Final verification: **4,616 affected Rust tests passed**, followed by **27 focused WebSocket/auth/recovery tests** and strict server Clippy after the final private-helper change. **1,698 UI unit tests**, type/style checks (zero errors/warnings), production build and bundle budget passed. The latest required-browser plugin suite passed **491 tests**, with zero failures and one intentional skip. Eight final populated-report/transient cases passed. Shared-modal breakpoint and nested return cases passed. Seven telemetry browser cases verified stored parents and opt-out; actual four-boot scheduled crash/recovery passed. Current-source native pane/zoom/focus and scoped public AX checks passed; final native example strict Clippy passed.

Performance evidence includes OTel off/on, concurrent N=1/3/5 rendered terminals, fresh-browser matched comparison, real installed-app read-only sampling and production queries at100k/1M spans. Consult the assessment for exact measurements and attribution limits.

Remaining limits: VoiceOver speech/rotor, complete native background AX exclusion, packaged release/install, physical multi-display transitions, paid-provider workloads and universal long-duration/scale behavior were not certified. One earlier native run lost whole-window foreground activation for an unknown reason; subsequent strict runs passed without refocusing or weakening assertions. These observations remain in the evidence.

The review completed before publication. The user subsequently authorized one PR for the complete repair branch and merger after green checks; that publication is tracked by the PR and its checks. No deployment or production-data mutation is part of this work. [Completion audit](completion-audit.md) maps the requested work to its evidence. Logs and selected artifacts are retained under [evidence/](evidence/).

The requested categories are performance (including OpenTelemetry), design, UX, correctness/bugs, and test coverage. Initial evidence, fixes, test logs, and independent re-review will be kept distinct. A score of 9.8+ requires appropriate fresh validation; no score is promised in advance.

## Coordination

Two review workers at a time; one coordinator owns builds, test suites and browser execution. Cargo uses `CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`, with this worktree's own `target/`. Browser runs use one worker, unique ports and disposable data. No changes are made to production data or other worktrees.

## Verification ledger

| Check | Result | Evidence |
| --- | --- | --- |
| Isolated worktree | Created from clean main | `/Users/itziklavon/claude_ade-quality-20261007` |
| UI dependencies | Installed | `/tmp/otto-quality-20261007-npm-ci.log` |
| Baseline UI check | Pass: 0 errors, 0 warnings | `/tmp/otto-quality-20261007-ui-check-baseline.log` |
| Initial daemon build | Interrupted to update baseline; dependencies cached | `/tmp/otto-quality-20261007-build.log` |

Each review lists its concrete scope and unverified areas. Prior reviews provide leads only. Source inspection does not replace visual review, tests, or runtime resource measurements.

## Main synchronization

Pulled main with `git pull --ff-only origin main`, then fast-forwarded the review branch to `196048df`. No reviewed correctness finding source files changed in the update. The fresh build will use this baseline. New School home and team-performance plugin surfaces are included in subsequent reviews. The initial UI check predates this update and will be repeated.

## Fresh baseline runtime evidence

- Updated daemon build: PASS, `CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build -p ottod --timings`, 5m08s after the interrupted build cached dependencies. macOS linker emitted a compact-unwind-size warning; this is an unoptimized reduced-debug build, not release performance evidence. Log `/tmp/otto-quality-20261007-build-current.log`.
- Existing browser → server → collector → ClickHouse telemetry regression: FAIL on the complete parent-chain assertion (`desktop-telemetry.spec.ts:110`); network trace contains one `/telemetry/ingest` HTTP 400. This is an existing regression test catching C4, not an absent integration test. Fresh isolated data, slot quality107, daemon7897/UI5397, one browser worker. Log `/tmp/otto-quality-20261007-telemetry-baseline.log`; trace/screenshots `/tmp/otto-quality-20261007-e2e-baseline/`.

## Initial scorecard

| Category | Provisional score | Source report |
| --- | ---: | --- |
| Performance | 8.0/10 | round-1-performance.md |
| New-surface performance | 7.8/10 | round-1-new-surfaces.md |
| Design | 7.7/10 | round-1-design.md |
| UX | 7.5/10 | round-1-ux.md |
| Correctness | 7.8/10 | round-1-correctness.md |
| Test coverage/quality | 7.2/10 | round-1-tests.md |

These are bounded source-review judgments, not measured app-wide percentages. Fresh updated-baseline UI type/style checks passed, and the unit suite executed 1,686 tests: 1,686 passed, zero failed or skipped. Logs `/tmp/otto-quality-20261007-ui-check-current.log` and `/tmp/otto-quality-20261007-unit-baseline.log`.

## First repair checkpoint

- Focused Rust nextest selection: **101 passed** (1,332 other/ignored tests excluded); scheduled state/engine/recovery plus normal telemetry tests, two workers, reduced-debug profile. Log `/tmp/otto-quality-20261007-rust-focused-green.log`. Includes missing/malformed snapshot handling, nonzero generations, legacy insert/read/finish, retained report ownership and independent sampling/consent races. Migration compatibility and ignored collector startup fixture remain queued.
- Telemetry producer + Redis safety/feedback: **22 passed**, zero skips. Log `/tmp/otto-quality-20261007-contract-redis-green-2.log`. An interim run found a cross-VM assertion-prototype issue; normalization preserved the exact name/count/order assertions.
- UI repair browser journeys: **7 passed** (new environment navigation/save races, prior A-B-A save cases, trace empty/retry/close). Used the fresh baseline daemon because these flows modify UI only; full telemetry/runtime confirmation requires the repaired binary later. Log `/tmp/otto-quality-20261007-ui-repair-browser-green.log`. Coordinator inspected light/dark trace screenshots retained in `evidence/`.
- School membership, cloned actor lifecycle and visibility polling units: **23 passed**; browser component repairs remain in progress. Log `/tmp/otto-quality-20261007-school-unit-green.log`.
- Plugin new baseline cases: six expected failures (four browser, worker tag cache, backoff abort); additional cancellation unit cases fail as expected; Stop endpoint baseline returns404. All fixtures isolated, no external Jira or remote Git.

Coordinator review found the retained-report lookup added by C3 had no `report_path` index. A query-plan regression failed with `SCAN scheduled_task_runs`; append-only migration 0179 adds the partial ownership index. Green verification is queued. Log `/tmp/otto-quality-20261007-report-index-red.log`.

## Second repair checkpoint

- Migration compatibility before 0179: **2 passed**. Log `/tmp/otto-quality-20261007-migration-green.log`; repeat after the new index.
- Ignored collector startup/lifetime fixture: **1 passed**. Log `/tmp/otto-quality-20261007-collector-lifecycle-green.log`. This uses an owned fixture process and is not a real collector performance benchmark.
- Team Performance repair regressions: **10 passed, zero skips**, 9.08 seconds, one test worker. Log `/tmp/otto-quality-20261007-plugin-focused-green.log`. Covers overlay ownership, draft/save races, request generations, stale-status Retry, cancellation, restart and deployment-tag cache across worker processes. Additional neighboring edge checks are queued before the full plugin suite.
- School browser acceptance is running; final scorecard remains the initial independent assessment until re-review and resource measurements complete.


## Independent second review and final verification checkpoint

- Correctness: **9.6/10**, no remaining source findings in the reviewed fixes after closing the post-save draft race; integration evidence pending. See `round-2-correctness.md`.
- Design **9.5/10**, UX **9.5/10** after closing loaded School refresh/Retry, plugin draft preservation and repeated degraded-card warnings. Independent inspection of the final eight screenshots and four passing browser cases found no remaining issue in the sampled scope. See `round-2-design-ux.md`.
- Test quality **9.3/10**, meaningful original regressions and plugin gate enforcement confirmed; final integration/runtime evidence pending. See `round-2-tests.md`.
- Performance remains provisional **8.0/10** until the prepared isolated workloads execute. Source re-review found no new material performance defect; `round-2-performance-preparation.md` distinguishes structural repair from measurements.

New verification: archive/auth **6 passed**, indexed report lookup **1 passed**; plugin full suite **470 passed, 0 failed, 2 known skips**; School resource/disposal and stale refresh **2 passed**, strengthened animated Enter/Space actions **2 passed**. Logs respectively `archive-auth-green`, `report-index-green`, `plugin-full-final`, `school-final`, `school-actions-verified` under `/tmp/otto-quality-20261007-*.log`. Action test iterations exposed harness assumptions (Check on intentionally does not animate with reduced motion, and the inspecting headmaster can occlude a canvas hit); the final test uses normal motion and accessible focus to reselect the kid.

The full affected-consumer gate is running. It includes strict linting, consumer tests/doc-tests, UI type/unit/build/bundle checks, and the newly enforced serial plugin suite. One formatting issue was fixed before restarting. To preserve the existing source-size ratchet, scheduled engine unit fixtures moved from `otto-server/src/scheduled_tasks_engine_tests.rs` to `otto-server/tests/unit/scheduled_tasks_engine.rs` (still wired into the lib test target); archive tests moved to `otto-sessions/src/manager/tests/archive.rs`. No ratchet limit was raised. Temporary `ui/SwiftShader.ini` was removed after the functional browser runs. The two-thread SwiftShader run is functional evidence, not native renderer performance; sampled resource evidence is in `evidence/school-functional-cpu.json`.


## Broad gate checkpoint

Strict Clippy passed. The affected-consumer nextest run executed **4,615 tests: 4,613 passed, 2 failed, 91 skipped** (`/tmp/otto-quality-20261007-gates.log`). The failures are being resolved before the remaining doc/UI/build gates:

- Workflow handoff recovery used a legacy `create_run` fixture without an admission snapshot. The success-path fixture now uses production `admit_run`; explicit legacy recovery regressions still require conservative failure.
- Cached-auth WebSocket logout timed out. Independent tracing identified a handshake race: the socket captured the revocation generation after initial authentication, potentially after logout. A deterministic regression and handshake generation repair are in progress.

Final plugin visual check: **4 passed, zero skipped**, mobile390/desktop1280 light/dark, including collapsed input-limit explanations and keyboard expansion. Fresh screenshots `/tmp/otto-quality-20261007-plugin-visuals-final/`; log `/tmp/otto-quality-20261007-plugin-visual-final.log`. Independent visual closure is pending. Scores above remain provisional.

Handshake race reproduction is deterministic: the new integration test revokes the real cached token after successful lookup but before handshake authentication returns. It fails on the old handler with the expected2500ms timeout (`ws-handshake-red.log`). The existing logout test also failed again while the corrected workflow recovery fixture passed (`gate-failures.log`). Repair carries the revocation generation captured before authentication into the socket loop; the timeout is unchanged. Fresh verification is queued.


## Current integration checkpoint

- Broad affected-consumer rerun: **4,616 passed, 0 failed, 91 skipped**. Strict Clippy, doc-tests and source-size ratchet passed. Log `gates-final.log`. This checkpoint includes the handshake-generation repair but predates the additional transient-auth-retry test/fix.
- UI type/style gates: **0 errors, 0 warnings**. Unit suite then had **1,686 passed, 9 failed** because its component VM lacked CommonJS exports/new private pending state after the environment component began exporting guard methods. A focused harness-only correction keeps all behavior assertions; **24/24** tests in that file pass. Fresh full unit/build/plugin continuation remains queued.
- Transient revalidation: deterministic real-auth regression failed before the change (`ws-transient-red.log`). An Internal store failure now leaves generation and periodic deadline pending for next-tick retry; definitive verdicts alone acknowledge them. Targeted green tests and repaired daemon build are running.
- Self-time capacity: exact production query and 256 MiB/10-second limits, bounded two-query-thread disposable ClickHouse. After batching fixture inserts,100k spans33–41ms and1M spans458–467ms, both repeats successful. The first attempt's900k-row fixture insert exceeded its limit; that was fixture construction, not a measured query failure. Final artifacts and mathematical result validation are being recorded.


Final WebSocket follow-up: strict server Clippy passed; **27/27 targeted event-stream/auth/recovery tests passed**, including deterministic handshake and one-transient-error revocation regressions. Log `/tmp/otto-quality-20261007-ws-final-build.log`. The same sequential run is building the repaired daemon; the broad4,616-test pass remains the preceding source checkpoint, not an invented all-suite rerun after this private-helper change. Migration compatibility including0178–0180 passed in that broad run.


UI continuation completed (`/tmp/otto-quality-20261007-ui-gates-final.log`): type/style **0 errors/0 warnings**, unit **1,695 passed/0 failed**, production build and bundle budget passed. Required-browser serial plugin suite **471 passed/0 failed/2 known skips** (473 total). The repaired daemon build completed with the previously noted compact-unwind linker warning, no error. Full telemetry browser suite is now running against that repaired binary.


Runtime closure: full telemetry suite plus strengthened saved-preview case **7/7 passed** on the repaired binary (`telemetry-export-window.log`). Browser batches were accepted and the complete navigation/client/server parent chain persisted through collector and ClickHouse, then opt-out succeeded. The earlier30s runtime check was too short for the intentional120s read-refresh throttle; production cadence is unchanged. All15 selected representative browser journeys also passed. Latest bounded design/UX9.6/9.6.

Rendered5s smoke completed1/3/5real WebGL terminals successfully after aligning its ACK expectation with the real64KiB batch threshold (full90s workload still requires ACKs). The first full rendered resource run was intentionally interrupted: ancestry-only process sampling omitted macOS WebKit XPC renderer/GPU/network children parented by launchd. The actual owned root and three XPC helpers share coalition431541. That incomplete resource run is not scored; ownership-based attribution is being repaired before full rerun.
