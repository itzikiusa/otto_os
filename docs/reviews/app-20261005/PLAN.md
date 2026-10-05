# Application review iterations 4–6 implementation plan

> For agentic workers: use superpowers:subagent-driven-development for the five implementation roles. This continues the completed app-20261004 effort; do not reopen its historical pending statuses without checking final verification.

**Goal:** Three further review/fix/verification iterations, targeting at least 9.8/10 in correctness, performance, design and UX with explicit evidence and coverage limits.

**Architecture:** Existing Rust daemon/Tauri/Svelte boundaries remain. Twenty review roles use five feature partitions; five implementers own deduplicated behavioral fixes. Claude owns visual/a11y/copy work in a separate worktree; reserve exact UI paths before edits and integrate both efforts without overwriting either.

**Tech stack:** Rust/Axum/SQLite, Svelte 5/TypeScript, Tauri, Vitest/Playwright/nextest.

## Baseline and delivery

- Baseline `03f2bc3e` (PR78, all 12 checks green). Prior measured results are in `../app-20261004/VERIFICATION.md` and `PERFORMANCE.md`; they are not a whole-app numerical score.
- Worktree `/Users/itziklavon/claude_ade-review`, branch `fix/app-review-20261005`; one continuation PR after these three iterations, merge after green checks as requested in the ongoing task. No deployment.
- Other agent `/Users/itziklavon/claude_ade-design`, currently `fix/design-iter-4`. Current shared coordination `/tmp/otto-app-review-coordination-20261005.txt` (prior round used the 20261004 note). Sync at iteration boundaries and before overlapping edits/builds.
- At most two shell-active agents across our effort; one heavy command at a time coordinated with Claude. Reviews in waves, no reviewer builds. Root orchestrates tests. Reuse this worktree's independent Cargo target; no shared target or routine clean.

## Fixed scoring rubric

Scores are review judgments, not statistical reliability estimates. Correctness/performance/UX score five named dimensions from 0 to 2 in 0.1 increments, with evidence and deductions; their sum is /10. Use the same criteria in all three iterations. Publish all five partition scores, their mean, their minimum, and evidence coverage. The target requires every partition in each lens to reach 9.8; a mean cannot conceal a weak partition.

Design uses Claude's established iteration-4 rubric to keep comparisons consistent: start 10; subtract 1.0 per blocker, 0.3 per major, 0.1 per minor, 0.03 per nit; count a systemic pattern once. Ten design lenses baseline 7.9/7.8/7.0/8.1/9.3/8.4/8.2/7.7/8.6/8.0, mean 8.1. Our five independent design reviewers audit coverage/findings using that rubric, while separately reporting rendered/native acceptance. Static 9.8 is not runtime acceptance. Earlier unrubriced design averages 7.1/7.2/7.4 are not comparable.

| Lens | Five equally weighted dimensions |
|---|---|
| Correctness | contract/data integrity; state/concurrency ownership; boundary/error behavior; persistence/recovery; executed regression coverage |
| Performance | query/network work; algorithmic/serialization cost; retained memory/lifecycle; scheduling/responsiveness; representative CPU/RAM/latency measurements |
| Design | shared visual hierarchy; tokens/consistency/readability; responsive layout; keyboard/accessibility; inspected rendered states in light/dark |
| UX | task completion/discovery; feedback/state clarity; recovery/retry; draft/scope/trust preservation; executed end-to-end journeys |

Anchor each dimension: 2.0 means strong direct evidence over the stated matrix with no known material gap; 1.8–1.9 means minor residual friction or a bounded evidence gap; 1.5–1.7 means a meaningful gap; below 1.5 requires a concrete significant defect or substantial unverified coverage. An unexecuted dimension cannot receive 2.0. Blocker/major findings prevent a 9.8 acceptance regardless of arithmetic. Source-only reviewers produce provisional scores; root adds execution evidence without silently replacing the original. Do not invent issues or inflate scores to meet the target. If three iterations end below target, report the real score and remaining work.

Execution-evidence calibration (applies identically across partitions, regardless of who ran tests): 0.0 = no verified execution available; 1.0 = verified green baseline without journey mapping; 1.5 = named passes cover substantial matrix with meaningful gaps; 1.8 = current-revision affected behavior/repairs/failure paths pass with bounded gaps; 1.9 = full required matrix with one specifically identified minor evidence gap; 2.0 = entire stated matrix has strong direct evidence with no material gap. Intermediate tenths require an explicit coverage distinction. Preserve initial reviewer totals and append calibrated totals. A source-only reviewer does not erase existing verified execution, and baseline green does not certify a new repair.

## Stable feature partitions

1. Shell/navigation, agents/sessions, terminals/transcripts, conversation/history.
2. Git/workbench, databases/connections, brokers, API client.
3. Vault/canvas/Design Hall/product/browser/snip.
4. Workflows/loops/swarm/scheduling/Mission Control/MCP.
5. Settings/plugins/usage/insights/home/share/auth/cloud, rooms/assistant/personal agents/proof/shared components.

## Iteration workflow (repeat for 4, 5, 6)

- [ ] Dispatch five correctness, five performance, five design, five UX review roles in resource-limited waves. Each report names source revision, inspected surfaces, traced inputs, exact file:line findings, severity, repair, five dimension scores, and omitted runtime evidence. Review changed paths AND adjacent journeys; do not restrict to old findings.
- [ ] Deduplicate in TRACKER.md; forward visual/a11y/copy findings to Claude. Reserve behavioral UI paths before implementation. Record contracts and exact test targets for every accepted repair.
- [ ] Assign five implementation roles by partition. Before each fix, trace/reproduce the bug, write a meaningful failing regression where appropriate, implement the smallest correct repair, and execute the affected test centrally. Record red/green evidence; never call an unrun test passing.
- [ ] Independent reviewer checks intended behavior first, then quality/regressions. Resolve confirmed findings before closing the iteration; retain explicit external limits.
- [ ] Integrate current main/Claude work without overwriting. Run affected gates with `scripts/check.sh --check`; browser evidence for touched journeys, light/dark captures for UI. Avoid rerunning whole suites for report-only changes.
- [ ] Rescore using the fixed rubric and publish score + confidence + blockers. Carry remaining gaps into next iteration.

## Final acceptance

- [ ] All accepted repairs independently reviewed, checked and documented; reports distinguish confirmed bugs, questions and coverage limits.
- [ ] Complete affected integration gates and investigate full-desktop remaining failures using isolated fixtures. Native-only/manual gaps remain explicit if inaccessible.
- [ ] Performance: isolated concurrent sessions N=0/1/3/5, CPU/RAM/latency, sustained memory run, read-only real-app sample. Keep resource safety thresholds and document workload/source. Investigate prior renderer RSS trend; do not equate steady DOM with no leak.
- [ ] Record final category and partition scores honestly; 9.8 is an evidence target, not a guaranteed result.
- [ ] One continuation PR, required CI green, merge main; verify remote merge. No deployment or deletion of worktrees/branches/user data.

Implementation details and test commands are appended per confirmed finding after review; inventing fixes before identifying defects would make the plan unreviewable.
