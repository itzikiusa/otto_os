# Quality iteration 2: lens scores

The 15 lens reviews in this folder each gave a score out of 10 for the code as it stood at the start of
iteration 2, after the iteration 1 fixes. The target is 9.8 or higher on every lens. Each score is the
reviewer's own number, before any iteration 2 fixes, so it does not reflect the work listed in
[FIXES.md](./FIXES.md). The next review re-scores the code after those fixes.

| Lens | Iteration 1 | Iteration 2 | Change | Gap to 9.8 |
|------|------------:|------------:|-------:|-----------:|
| backend-correctness | 9.37 | 9.77 | +0.40 | 0.03 |
| ui-correctness | 8.20 | 8.37 | +0.17 | 1.43 |
| access-control | 9.30 | 9.30 | 0.00 | 0.50 |
| backend-performance | 8.67 | 9.77 | +1.10 | 0.03 |
| ui-performance | 9.34 | 9.30 | −0.04 | 0.50 |
| concurrency-async | 9.34 | 9.64 | +0.30 | 0.16 |
| data-integrity | 9.10 | 9.27 | +0.17 | 0.53 |
| error-handling | 8.17 | 9.17 | +1.00 | 0.63 |
| architecture | 6.30 | 5.90 | −0.40 | 3.90 |
| maintainability | 7.25 | 8.28 | +1.03 | 1.52 |
| test-quality | 8.70 | 8.50 | −0.20 | 1.30 |
| api-contracts | 8.71 | 9.05 | +0.34 | 0.75 |
| ux-design | 9.17 | 9.04 | −0.13 | 0.76 |
| accessibility-responsive | 8.70 | 8.50 | −0.20 | 1.30 |
| integrations | 8.60 | 9.47 | +0.87 | 0.33 |

Mean: 8.89 (iteration 1: 8.58). Lowest: architecture (5.90), maintainability (8.28), ui-correctness
(8.37), and test-quality and accessibility-responsive (both 8.50).

Architecture dropped because iteration 1 deferred all the large splits, and the reviewers measured the
god files at their largest. This iteration did most of those splits: `execute_node`, `modules.rs`,
`types.ts`, the otto-telemetry crate, the event route table and the API-client handlers. Some only
partly landed (see FIXES.md). The new production-line LOC ratchet and the UI `file-loc` and
`lib-imports-modules` guards stop the files from growing back.

## Deferred findings

These 4 findings are not assigned to any package in iteration 2.

- **architecture-1-04:** this is several days of work, not a move. It needs a new generic fan-out and
  summarizer runner (agent-row state machine, per-agent `CancelGuard`, an explicit summarizer stage
  and a persistence trait) shared by three engines whose summarizer rules differ (name match, last row,
  `SUM_RETRY_IDX`). It should start from the `modules/review/*` layout that P02 created this
  iteration.
- **architecture-1-05:** needs a product decision. Generating the native MCP listings from
  `otto_mcp::outward::catalog` changes the schemas that live agent sessions see for the 7
  `DEFERRED_SCHEMA_DRIFT` pairs. Under the control-plane/session parity rule, someone must choose the
  winning schema for each pair. The parity test already records the drift.
- **architecture-2-06:** another team is rewriting game rooms this iteration (kart shortcuts, boosts
  and track, the shooter, the classroom). Moving about 9.7k lines of rooms and game rooms into a new
  `otto-rooms` crate now would break their work in progress. Schedule it right after their changes
  merge.
- **architecture-2-08:** the UI half is in `ui/src/modules/rooms/games/**`, which another team owns and
  this loop must not edit. It also depends on architecture-2-06.
