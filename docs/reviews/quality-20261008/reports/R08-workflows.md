# R08 — workflows, automation, work graph, runs and proof

Review base: merged PR94 `a0bd718b9fbc008d72c164ce643a24ed78e6368c`; repairs are in shared branch `review/quality-20261008`. This focused review/repair pass is complete; final repository integration remains with the coordinator. No staging, commits, real user data, external delivery or production agent execution was used.

## Coverage and method

Applied correctness-review, performance-review and architecture-review. Traced source through scheduler admission → immutable task snapshot → report capture → delivery outcome → next-run suppression; command process/pipe lifetime; goal-loop terminal outcome → Run-with-Otto stages; boot recovery → background admission; proof status → publication gate; stores → selection changes → delayed mutation/confirmation continuations. Read workflow validation, checkpoint/retry/trigger helpers and selected executor admission, pinned-definition, recovery, resume and cancellation branches; the 10,122-line workflow executor was **not exhaustively line-reviewed**. Workgraph service/projector lifecycle, reconciliation and UI ownership were inspected. Coverage is focused, not a claim every path in the 26-file domain was executed.

Browsers mount actual Svelte stores/components with controlled deferred HTTP responses for ownership regressions. Existing named specs exercise an isolated daemon with deterministic E2E agent stubs. Real coding-agent quality, actual Slack/Telegram/email/webhook sends and remote PR publication remain deliberately outside these tests.

## Confirmed findings and repairs

1. **High — stale confirmation targets.** Loop delete/stop, Run cancel/open-PR, Proof delete/assemble/create and Workflow stop read mutable selection/workspace after confirmation. Navigating from A to B could execute the action on B although the sheet named A. Capture the target before awaiting and suppress old completion navigation. Eight actual continuation regressions passed; four earlier cases and four newly discovered cases each failed before their fixes. Mounted Loop Stop and Run PR cases passed with real confirmation UI.
2. **High — store scope and mutation races.** Loop close could leave a queued detail reload that reopened a closed panel; delayed lifecycle/iteration/mutation responses could publish old-workspace data. Run list GETs could overwrite newer cancellation, and late launch could switch the store back to its old workspace. Added generation/per-entity ownership. Run launcher now owns each detection controller/ticket and rejects unmounted launch completion.
3. **High — Proof identity and workspace leaks.** List-driven workspace changes did not clear summary keys, pending detail could publish across workspace navigation, and same-workspace identity reset did not invalidate old summary reads. A shared scope transition now invalidates all owned loads/caches. Three actual source-store regressions failed before repair, then passed.
4. **High — proof command timeout left descendants alive.** A shell subprocess survived a timed-out proof command and wrote a marker later. Proof now shares the cancellation/process-group-aware verification runner. Red: isolated timeout marker test failed as expected. The timeout regression and proof consumer suite are green.
5. **Medium — verification output capture was unbounded.** The goal-loop runner retained 6,291,472 bytes for a command writing 3 MiB to each pipe. It now shares the scheduled-command bounded drainer: 512 KiB raw bytes per stream plus omitted-byte marker, while both pipes continue draining. The regression also verifies the command reaches its final marker instead of blocking on a full pipe. Invalid UTF-8 can expand when rendered; raw retention stays bounded.
6. **High — failed notifications suppressed retry.** `last_ok_report_hash` accepted an `ok` execution even when outward delivery failed or partially failed. Subsequent identical reports could be suppressed indefinitely. Only the latest eligible successful execution with error-free delivery/unchanged skip establishes the baseline, and its immutable admitted destination must match. Failed, absent or legacy destination evidence causes retry. SQLite outcome and destination fixtures avoid actual sends. The analogous personal-agent defect was handed to R09 and shared red evidence supplied.
7. **High — detached startup recovery could fail fresh runs.** The supervisor reaped interrupted rows inside a detached task after startup had returned. A blocked DB scan could resume after a new request/worker entered executing. Recovery now runs as the **first awaited step** in `recover_before_serve`, before session restoration and background workers; DB errors propagate through the existing fallible boot boundary. The later scheduler only redrives resumable stages. A failed compare-and-set no longer emits a false failure event. Deterministic old-boundary regression failed before repair; final blocked-DB/fresh-run and failure-propagation cases passed. The first final fixture used a date-only timestamp and failed when full boot parsed workspaces; corrected to RFC3339 and reran both cases successfully.
8. **High — cached proof at PR publication.** `open_pr` trusts `run.proof_status`, a list snapshot, even after the linked proof fails or disappears. The SQLite regression failed at invalid-draft parsing instead of the proof gate, proving the bypass without provider access. Publication now recomputes live evidence/policy and checks run/workspace ownership. It also rejects dirty work and a HEAD different from the captured diff revision; the late auto-commit was removed, and push failure aborts before provider publication. An isolated git regression exercises actual diff capture, a clean positive control, dirty work and a later commit, while proving no staging/commit mutation. Server regressions passed.
9. **High — failed goal-loop outcome accepted as execution success.** `poll_goal_loop` returns `Ok` for failed/stopped/exhausted as well as succeeded, allowing preserved branches to proceed to proof/review. An actual persisted failed state returned Ok in the red regression. Only succeeded now returns Ok; failed/stopped/exhausted return their explicit unsuccessful outcome. Server regressions passed.

## Verification evidence

- `node --experimental-strip-types --test` over `proofOwnership`, `platformOwnership4`, `automationConfirmation`, `loopOwnership`, `runOwnership`, `runLauncherOwnership`: **53 passed, 0 failed** (`/tmp/r08-ownership-green.log`). New cases execute production store/method code, not copied implementations.
- `npm --prefix ui run check`: **passed**, Svelte 0 errors/0 warnings, all type projects and UI guards (`/tmp/r08-ui-check-final.log`).
- Named Playwright `desktop-automation-ownership`, `desktop-goal-loop-verification`, `desktop-workflow-recovery`: **7 passed in 8.2 s** (`/tmp/r08-browser-green.log`). Actual light/dark loaded-state recapture: **2 passed in 3.2 s** (`/tmp/r08-browser-schemes.log`). No full browser suite.
- Screenshots: `../evidence/r08-loop-{1440,390}-{light,dark}.png`; actual 1440 light and 390 dark inspected. Phone horizontal-overflow assertions pass. This does not certify native VoiceOver/keyboard behavior.
- Expected reds: loop ownership 4, run ownership 4, launcher ownership 2, confirmation 4+4, Proof store 3; unbounded command output 1, proof descendant timeout 1, shared scheduled/personal delivery baseline 2, detached startup boundary 1. Logs `/tmp/r08-*-red.log` identify actual assertions, not build failures.
- State library: first full run **420 passed, 2 failed, 2 ignored** in 162.02 s. Both failures exposed my initial destination-snapshot parser reading the wrong level; corrected it to the existing versioned `AdmittedTaskSnapshot`. Focused scheduled module rerun **20 passed, 0 failed** in 9.97 s. The full run also passed R01 workspaces and R09 notification/paging regressions. No failures are hidden by a summary “green” label.
- Migration compatibility including new 0181: **2 passed, 0 failed** in 0.25 s (`/tmp/r08-migration-green.log`).
- Additional Node workflow versions/forms/publication/review agents/budgets/checkpoints/run inputs: **27 passed**, making 80 selected Node cases total.
- Additional named browser journeys: Proof v2 + scheduled draft recovery + Run-with-Otto **32 passed in 13.9 s**; Mission Control iphone portrait **9 passed in 10.7 s**. These use the earlier isolated binary; they do not validate new Rust repairs.
- Owned package libraries: **121 passed** (automation 58 / workflows 60 / workgraph 3; `/tmp/r08-domains-green.log`).
- Focused server consumers: **134 passed**: proof 11, run engine 8, run service 3, recovery 2, workflow engine 75, node driver 6, workflow trigger scheduler 17, workgraph projector 6, scheduled routes 6. Logs `/tmp/r08-server-<module>-green.log`. Run-service coverage uses actual isolated git capture, checks both dirty and committed drift, permits excluded untracked runtime files, and proves a no-remote push error aborts before provider publication.
- Scoped clippy `-D warnings`, all targets, for automation/workflows/workgraph: **passed** in 12.31 s. Rustfmt check over all R08 Rust changes: **passed**. LOC ratchet: **passed** at 125097/130735 server lines, without raising a baseline.
- Final UI check after stale-proof action fix: **passed**, 0 Svelte errors/warnings (`/tmp/r08-ui-check-final2.log`).
- Mounted stale-partial-proof action: expected red (button disabled) then **1 passed in 2.9 s**. The Run UI now labels proof as “at execution” and permits authoritative server revalidation, so adding evidence/waiving proof can recover the action. Approval/repository checks remain. Log `/tmp/r08-proof-ui-{red,green}.log`.
- Initial `cargo nextest` attempt found no command on PATH; used standard Cargo focused modules. Coordinator later supplied the existing `/tmp/otto-quality-tools/nextest-0.9.146/cargo-nextest` path. No gate was skipped or weakened because of tool discovery.
- LOC ratchet: no R08 baseline increase; root relocated unrelated existing tests to cure other domains' growth.

### Exact server consumer commands

Each row ran `cargo test -p otto-server --lib <filter>` successfully. The common compiled server test binary was `target/debug/deps/otto_server-1a101a9211bc88ec`; production changes were frozen before final consumer completion. No server-wide test selection was represented as passing.

| Filter | Passed | Log |
| --- | ---: | --- |
| `proof::` | 11 | `/tmp/r08-server-proof__-green.log` |
| `run_engine::` | 8 | `/tmp/r08-server-run_engine__-green.log` |
| `run_service::` | 3 | `/tmp/r08-server-run_service__-green.log` |
| `run_scheduler_` | 2 | `/tmp/r08-server-run_scheduler_-green.log` |
| `workflow_engine::` | 75 | `/tmp/r08-server-workflow_engine__-green.log` |
| `workflow_node_driver::` | 6 | `/tmp/r08-server-workflow_node_driver__-green.log` |
| `workflow_trigger_scheduler::` | 17 | `/tmp/r08-server-workflow_trigger_scheduler__-green.log` |
| `workgraph_projector::` | 6 | `/tmp/r08-server-workgraph_projector__-green.log` |
| `routes::scheduled_tasks::` | 6 | `/tmp/r08-server-routes__scheduled_tasks__-green.log` |
| **Total** | **134** | **0 failed** |

## Assessment and remaining evidence

No 9.8 claim is supported by this pass. No confirmed defect from the nine finding families remains open in the repaired paths, but coverage is explicitly narrower than the full domain.

| Vertical | Assessment / 10 | Confidence and remaining evidence |
| --- | ---: | --- |
| Correctness / bugs | 9.5 | High for repaired target ownership, delivery outcomes, command cleanup, startup recovery and publication gates. Full workflow executor/node families, every source/provider and cross-domain journeys need the independent integration pass. |
| Performance | 9.3 | Bounded capture exercised with 6 MiB output. Existing 5,000-pack/200-summary indexed-read test passed its <50 ms warm-query limits, along with metadata-only proof/history tests. Representative large workflow graphs, long loop histories, concurrent schedulers and UI render latency were not measured here. |
| Design | 9.2 | Repairs reuse the shared command runner, original admission snapshot and existing fallible boot phase; no new transport shape or baseline increase. The large workflow executor limits reviewability, and the full six-module visual/state matrix was not inspected in both themes. |
| UX / usability | 9.4 | Actual confirmation navigation, stale proof action recovery, scheduled draft recovery, proof truthfulness, run approval/rejection/cancel and Mission Control phone journeys passed. Native keyboard/VoiceOver and real provider/agent failure UX were not exercised. |

These are scoped engineering assessments, not calibrated measurements or an assertion the requested 9.8 target is met. R15/R16 should challenge the source/test evidence and complete the remaining workload/journey coverage. No user/prod system or real external delivery is needed to review the remaining local seams.

Cargo was handed to R11 after all R08 gates finished. No staging/commit/push was performed. The earlier phrase “staged locally” in coordination meant edited files only; the Git index was not used.

## Source anchors

- `ui/src/lib/stores/loops.svelte.ts:49`, `runWithOtto.svelte.ts:36`, `proof.svelte.ts:51`: asynchronous scope ownership.
- `ui/src/modules/proof/ProofPage.svelte:410,696,744`; `ui/src/modules/workflows/WorkflowsPage.svelte:1292`: immutable confirmation targets.
- `crates/otto-automation/src/goal_loop_commands.rs:15`, `command_output.rs:5`; `crates/otto-server/src/proof.rs:110`: bounded process output and cleanup.
- `crates/otto-state/src/scheduled_tasks.rs:645`: delivery-qualified destination-aware baseline.
- `crates/otto-server/src/boot/recovery.rs:14`, `run_scheduler.rs:30`: awaited old-life settlement before admission.
- `crates/otto-server/src/run_service.rs:367,465`, `proof.rs:239`: current proof and revision publication gates.
- `crates/otto-server/src/run_engine.rs:768`: unsuccessful terminal goal-loop outcomes.
