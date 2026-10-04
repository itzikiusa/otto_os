# Application review and repair — 2026-10-04

Goal: review existing application correctness, performance, visual design and user experience; fix verified findings in up to three iterations; deliver one PR and merge after checks pass.

Worktree: `/Users/itziklavon/claude_ade-review`, branch `fix/app-review-20261004`.
Baseline: `a16f4c71`. Other active effort: `/Users/itziklavon/claude_ade-design` (`fix/design-iter-1`). Coordinate before overlapping fixes and before resource-intensive commands. Never modify that worktree.

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
- [ ] Reproduce and repair accepted findings; add behavioral regressions where warranted.
- [ ] Iteration 2: have reviewers recheck repairs and adjacent failure paths; repair confirmed remaining issues.
- [ ] Iteration 3 if needed: final review and repair, then verify all changed paths.
- [ ] Track every finding as fixed, rejected with evidence, or explicitly unresolved.

## Verification and integration

- [x] Baseline UI checks/unit tests: `npm run check` and 989 unit tests pass on a16f4c71.
- [ ] Focused Rust and UI tests for fixes; scoped `scripts/check.sh --check` gate.
- [ ] Isolated E2E on dedicated ports; verify changed UI in light/dark/phone and capture screenshots.
- [ ] Synthetic isolated concurrent-session CPU/RAM measurements and memory over time; read-only sample real app CPU/RAM.
- [ ] Coordinate other worktree; merge updated origin/main into this branch preserving both efforts.
- [ ] Inspect combined diff, run required checks, create one PR, wait for green CI and merge main.

No deployment requested. No manipulation of real user sessions/data for testing. All real-app observations are read-only.

## Coordination agreement

Claude owns visual/a11y/copy fixes and the nightly ClickHouse k8s test. Our design reviewers remain independent and forward confirmed findings to Claude. Behavioral UI changes are reserved by exact path in `/tmp/otto-app-review-coordination-20261004.txt`. Verify fixes from the other effort after integrating main. E2E slot `review04`, daemon `7814`, Vite `5314`, orphan sweeping disabled.
