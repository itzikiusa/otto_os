# Application review and repair — 2026-10-04

Goal: review existing application correctness, performance, visual design and user experience; fix verified findings in up to three iterations; deliver one PR and merge after checks pass.

Worktree: `/Users/itziklavon/claude_ade-review`, branch `fix/app-review-20261004`.
Baseline: `a16f4c71`. Other active effort: `/Users/itziklavon/claude_ade-design` (Claude-owned design work). Main PRs 75, 76 and 77 are integrated into this branch. Final design PR77 (`01f0c290`) was integrated as `65e13b4f`; overlap hunks were reviewed, Rust inputs are unchanged, and the combined UI gates passed. Coordinate before overlapping fixes and before resource-intensive commands. Never modify that worktree.

## Reviewer matrix

Twenty reviewers: five each for correctness, performance, visual design, and user experience. Five reviewers for each lens, with the same five feature partitions:
1. App shell, navigation, agents, sessions, terminals and conversation.
2. Git, workbench, DB Explorer, connections, brokers and API client.
3. Vault, canvas, Design Hall, product, browser and snip.
4. Workflows, loops, swarm, scheduled tasks, mission control and MCP.
5. Settings, plugins, usage/insights, home, share, auth, AWS/Kubernetes, rooms, assistant/personal agents, history/proof and shared components.

Run reviewers in resource-limited waves (at most two shell-active reviewers; one heavy command at a time). Reviewers initially read only. Findings need exact locations, user impact, evidence/trace and concrete repair. Performance findings need a sized cost model; UI design findings need an existing guideline or broken user flow, not personal taste. Record coverage gaps honestly.

UX reviews cover task completion, discoverability, feedback, error recovery and workflow friction. Visual design reviews cover hierarchy, consistency, readability, responsive layout and accessibility. Deduplicate cross-lens findings.

## Implementation team

Five implementation agents total, assigned deduplicated findings by feature partition and explicit file ownership. At most two shell-active agents and one heavy build/test command at a time. Reviewers remain independent from implementers. All changes stay on this branch and one PR.

## Iterations (maximum three)

- [x] Iteration 1: complete 20 specialist reviews and consolidate/deduplicate findings.
- [x] Implement the accepted partition 1–5 repairs and author behavioral regressions; record executed red/green evidence separately from source traces.
- [x] Iteration 2: independently recheck all five partitions and adjacent failure paths; record integration findings and implement follow-up repairs.
- [x] Iteration 3: complete bounded final source rechecks for partitions 2–5 and the original design findings. Partition 5's final recheck covers the explicit-resume follow-up; its broader source disposition remains in iteration 2. Partition 1's iteration-2 recheck remains its latest independent report.
- [x] Track every original and follow-up finding, including unresolved external work and verification limits, in [TRACKER.md](TRACKER.md).
- [x] Independently recheck the final Assistant explicit-resume repair: [focused source approval](iteration-3-partition-5.md), zero remaining findings.
- [x] Execute the final Assistant resume regressions and Insights route-policy correction through the passing closing Rust gate.
- [x] Accept the imported Composer repairs at the recorded source/runtime scope: deferred-upload remount and nine short tiles with long drafts pass in light/dark; screenshots inspected. Final PR77 main integration and combined checks are complete below.
- [x] Close the recorded source/runtime gates for the current combined source: Rust closing gate, UI check and 1,081 units, rebuilt daemon, and 63 browser cases verified as 61 initial passes plus two repaired reruns. Post-PR77 checks are recorded below; native acceptance limits remain explicit.

Implementation source is frozen during closing load measurement. The role-5 explicit-resume repair and foreign-thread cancellation regressions passed the closing Rust gate. The recorded source/runtime acceptance includes the final Assistant history-acquisition and History initial-load repairs; it does not establish native assistive-technology or physical-device acceptance. See [iteration-2-implementation-5.md](iteration-2-implementation-5.md) and [VERIFICATION.md](VERIFICATION.md).

## Verification and integration

[VERIFICATION.md](VERIFICATION.md) records test commands/results; [PERFORMANCE.md](PERFORMANCE.md) records resource measurements and limits. The following completed runs certify the source present at their execution, not later changes or external work.

- [x] Baseline UI checks/unit tests: `npm run check` and 989 unit tests pass on `a16f4c71`; isolated page-chrome browser suite passes 5/5.
- [x] Focused Rust/UI regression runs and combined library runs: latest recorded seven-package library result is 1,968 passed, 6 ignored. It predates the final Assistant resume and Insights policy changes.
- [x] Post-PR76 UI typecheck passed with zero errors/warnings; latest root-reported unit suite passes 1,081 tests, including nine Mission Control draft/refresh regressions.
- [x] Targeted isolated browser checks: API persistence 4, automation leave/save/discard 1, actual History scroll paging, and Product 2. Product's focused recovery unit group passes 36 cases. These do not cover the complete visual/native acceptance matrix.
- [x] Disposable MySQL/PostgreSQL batch fixtures pass with server-side row-counter and connection-continuity assertions; isolated 3-million-series K8s interrupted/retried migration passes under the unchanged 1 GiB server limit.
- [x] Read-only real-app CPU/RAM observation and attempted isolated baseline load run recorded. The scale run safety-aborted and is not a performance pass.
- [x] Coordinate other worktree and integrate updated main, preserving both efforts: PR75 at integration `04917dd7`, PR76 at `431ed3fd`; automatic merges inspected.
- [x] Closing Rust gate passed rustfmt, clippy, all 4,592 nextest cases (85 skipped), and the doc-test command (`/tmp/otto-review-final-gate2.log`). After completing the external imports, UI check passed with zero errors/warnings and all 1,081 unit tests passed; the final two browser repairs were followed by another passing UI check/unit run.
- [x] Rebuild the actual isolated daemon (3m53s) and verify the real local Product draft HTTP save/reopen roundtrip. The build emitted the existing macOS debug unwind-section linker warning.
- [x] Verify 63 cases across seven focused desktop specs: 61 initial passes plus two repaired reruns. Coverage includes actual Product save/reopen, Composer deferred-upload remount, nine short tiles with long drafts in light/dark, phone History full-title behavior, Assistant incremental messages without an extra history GET, and History scope controls during initial load. Light/dark tile screenshots inspected and retained in `screenshots/`.
- [x] Integrate final design PR77 (`01f0c290`) as `65e13b4f` and inspect overlap hunks. Rust/build inputs are unchanged, so the completed Rust gate remains applicable. Combined UI check passed with zero errors/warnings, all 1,081 units passed, and the production build passed. The bundle guard passed after recording only Snip's measured +451 gzip-byte feature cost; other budgets and the 3% tolerance are unchanged.
- [x] Verify all 98 distinct cases in the combined affected browser group across 97 initial passes and the repaired History fixture rerun. This was not one all-green 98-case invocation. History and terminal cases passed 33/33 across three repeats of 11 cases (28.5s); Rooms passed 3/3 (4.2s) and again in the merged group. The History fixture still requires exact wheel cursors 120 then 60. Final E2E TypeScript check passed, and merged light/dark tile screenshots were inspected.
- [x] Complete the initial bounded 0/1/3/5-session scale run (375.4s, 66 samples) and sustained three-session view switching (986.8s, 184 samples), with no page errors, swap growth or leftover processes. Across eight comparable sustained checkpoints, heap stayed at 35.0–37.5 MiB and DOM/listeners stayed at 887/375; renderer RSS still grew 32.8 MiB. This does not establish absence of long-term leaks.
- [x] Complete the final merged-UI 0/1/3/5-session scale run: 375.4 seconds, 66 samples, peak host load 6.66, at least 36.0 GiB available memory, no page errors/swap growth and zero leftovers. Measurements and limits are recorded.
- [x] Finish the review ledger, performance measurements and inspected light/dark screenshot evidence.
- [ ] Publish the single PR, wait for green CI and merge main. This document records local acceptance before publication; the PR checks and merge record establish delivery.

Acceptance limits: the external full desktop suite was not clean; its supplied failure/rerun ledger remains in [VERIFICATION.md](VERIFICATION.md). The affected browser group does not establish complete desktop, light/dark/responsive, native keyboard/focus, assistive-technology or physical-device acceptance. These unexecuted checks are limits of the evidence, not additional work promised by this plan.

No deployment requested. No manipulation of real user sessions/data for testing. All real-app observations are read-only.

## Coordination agreement

Claude owns visual/a11y/copy fixes and the nightly ClickHouse k8s test; root owns the separate runtime backfill repair and its isolated scale regression. Our design reviewers remain independent and forward confirmed findings to Claude. Behavioral UI changes are reserved by exact path in `/tmp/otto-app-review-coordination-20261004.txt`. Verify fixes from the other effort after integrating main. E2E slot `review04`, daemon `7814`, Vite `5314`, orphan sweeping disabled.
