# Quality iteration 3: lens scores

The 15 lens reviews in this folder each gave a score out of 10 for the code as it stood at the start of
iteration 3, after the iteration 2 fixes. The target is 9.8 or higher on every lens. Each score is the
reviewer's own number, before any iteration 3 fixes, so it does not reflect the work listed in
[FIXES.md](./FIXES.md). The next review re-scores the code after those fixes.

| Lens | Iteration 1 | Iteration 2 | Iteration 3 | Change | Gap to 9.8 |
|------|------------:|------------:|------------:|-------:|-----------:|
| backend-correctness | 9.37 | 9.77 | 9.47 | −0.30 | 0.33 |
| ui-correctness | 8.20 | 8.37 | 8.97 | +0.60 | 0.83 |
| access-control | 9.30 | 9.30 | 8.00 | −1.30 | 1.80 |
| backend-performance | 8.67 | 9.77 | 9.64 | −0.13 | 0.16 |
| ui-performance | 9.34 | 9.30 | 9.80 | +0.50 | 0.00 |
| concurrency-async | 9.34 | 9.64 | 9.68 | +0.04 | 0.12 |
| data-integrity | 9.10 | 9.27 | 9.41 | +0.14 | 0.39 |
| error-handling | 8.17 | 9.17 | 9.31 | +0.14 | 0.49 |
| architecture | 6.30 | 5.90 | 6.77 | +0.87 | 3.03 |
| maintainability | 7.25 | 8.28 | 8.65 | +0.37 | 1.15 |
| test-quality | 8.70 | 8.50 | 8.80 | +0.30 | 1.00 |
| api-contracts | 8.71 | 9.05 | 9.11 | +0.06 | 0.69 |
| ux-design | 9.17 | 9.04 | 9.37 | +0.33 | 0.43 |
| accessibility-responsive | 8.70 | 8.50 | 8.10 | −0.40 | 1.70 |
| integrations | 8.60 | 9.47 | 9.50 | +0.03 | 0.30 |

Mean: 8.97 (iteration 2: 8.89, iteration 1: 8.58). ui-performance reached the target. The lowest
lenses are architecture (6.77), access-control (8.00), accessibility-responsive (8.10),
maintainability (8.65) and test-quality (8.80).

Access-control dropped because the reviewer found a blocker: a non-root `mcp:admin` could rotate
root's MCP token and receive a working root credential (access-control-3-01). It also found that
unattended workflow runs acted as the workflow's creator after someone else edited the graph
(access-control-3-02), and that a run's own context files were unreachable to the workspace members
who can see the run (access-control-3-04). Iteration 3 fixed 3-01, 3-02, 3-04 and 3-05. It did not
start 3-03 (swarm session identity, PC-4); see FIXES.md.

Accessibility-responsive dropped because the reviewer checked keyboard behaviour more widely:
radio groups without arrow-key handling, focus lost after a delete, and nested live regions.
Iteration 3 fixed these across the app and added a `radio-key` UI guard so new radio groups can't
skip `radioKey`.

Architecture rose with iteration 2's splits. Iteration 3 split `WorkflowsPage.svelte`,
`otto-core/src/api.rs`, part of `Terminal.svelte` and `App.svelte`, broke the product ↔ design-hall
cycle, and added the shared Atlassian transport, the swarm `activate` lifecycle and `KeyedLocks`.
`run_workflow_with_budget`, `GraphView.svelte`, `DatabaseStore`, the usage tailer move and the MCP
catalog generation are still open.

## Deferred findings

These 4 findings are not assigned to any package in iteration 3.

- **architecture-1-04:** genuinely multi-day. It needs a generic fan-out and summarizer runner
  (agent-row state machine, `CancelGuard`, summarizer stage, persistence trait) across at least 3
  engines. P04 is also changing the review engine's retry and cancel paths this round, so it should
  start after those land.
- **architecture-2-06:** the otto-rooms crate extraction is blocked by ownership. The games team is
  mid-change in rooms and game rooms (`rooms/games/**` is off-limits), so this waits for their merge.
- **architecture-2-08:** the game catalog JSON is paired with 2-06 and touches the games team's map
  and capacity ids, so it waits for the same merge.
- **concurrency-async-1-05:** needs a product decision. Both fixes for the plugin `free_port` TOCTOU
  (the plugin binds `:0` and reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`) change the
  plugin launch protocol and would break existing third-party plugins, so plugin-API versioning has
  to be decided first.
