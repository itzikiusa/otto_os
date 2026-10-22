# Quality iteration 5: lens scores

The 15 lens reviews in this folder each gave a score out of 10 for the code as it stood at the start of
iteration 5, after the iteration 4 fixes. The target is 9.8 or higher on every lens. Each score is the
reviewer's own number, before any iteration 5 fixes, so it does not reflect the work listed in
[FIXES.md](./FIXES.md). The next review re-scores the code after those fixes.

| Lens | Iteration 1 | Iteration 2 | Iteration 3 | Iteration 4 | Iteration 5 | Change | Gap to 9.8 |
|------|------------:|------------:|------------:|------------:|------------:|-------:|-----------:|
| backend-correctness | 9.37 | 9.77 | 9.47 | 9.74 | 9.10 | −0.64 | 0.70 |
| ui-correctness | 8.20 | 8.37 | 8.97 | 9.30 | 9.37 | +0.07 | 0.43 |
| access-control | 9.30 | 9.30 | 8.00 | 9.00 | 9.10 | +0.10 | 0.70 |
| backend-performance | 8.67 | 9.77 | 9.64 | 9.61 | 9.65 | +0.04 | 0.15 |
| ui-performance | 9.34 | 9.30 | 9.80 | 9.60 | 9.47 | −0.13 | 0.33 |
| concurrency-async | 9.34 | 9.64 | 9.68 | 9.81 | 9.78 | −0.03 | 0.02 |
| data-integrity | 9.10 | 9.27 | 9.41 | 9.47 | 9.44 | −0.03 | 0.36 |
| error-handling | 8.17 | 9.17 | 9.31 | 9.41 | 8.87 | −0.54 | 0.93 |
| architecture | 6.30 | 5.90 | 6.77 | 7.31 | 7.55 | +0.24 | 2.25 |
| maintainability | 7.25 | 8.28 | 8.65 | 9.39 | 9.34 | −0.05 | 0.46 |
| test-quality | 8.70 | 8.50 | 8.80 | 8.50 | 8.00 | −0.50 | 1.80 |
| api-contracts | 8.71 | 9.05 | 9.11 | 9.34 | 9.64 | +0.30 | 0.16 |
| ux-design | 9.17 | 9.04 | 9.37 | 9.15 | 8.55 | −0.60 | 1.25 |
| accessibility-responsive | 8.70 | 8.50 | 8.10 | 8.50 | 8.51 | +0.01 | 1.29 |
| integrations | 8.60 | 9.47 | 9.50 | 8.80 | 9.50 | +0.70 | 0.30 |

Mean: 9.06 (iteration 4: 9.13, iteration 3: 8.97, iteration 2: 8.89, iteration 1: 8.58). No lens
reached the target this time; concurrency-async (9.78) and backend-performance (9.65) are closest. The
lowest lenses are architecture (7.55), test-quality (8.00), accessibility-responsive (8.51), ux-design
(8.55) and error-handling (8.87).

Backend-correctness dropped because the reviewer fed adversarial depth to every recursive parser: the
workflow expression parser, the Confluence storage checker and the Mongo shell parser each overflowed
the stack on deeply nested input (5-01 to 5-03), and the MCP server accepted an unchecked profile id
(5-05). Iteration 5 caps the depth of all three parsers and makes the profile parse fail closed.

Error-handling and ux-design dropped on failures that were silent or generic: a report that couldn't be
written still recorded a successful run (5-06), an unreadable budget config admitted work (5-07), the
review summarizer's fallback hid its cause behind a hard-coded provider (5-05), and several pages showed
"Couldn't load" toasts or empty states instead of an inline error with Retry. Iteration 5 fixes these
with the existing error fields and `LoadState`.

Test-quality dropped because the reviewer listed refusal and recovery paths no test reached: the git PR
node's dirty-tree and worktree cases, the cursor settle retry, channel-notify targeting, the
session-file persona boundary, the MCP profile edges and the native DB driver cases in CI. Iteration 5
adds tests for each; the DB driver cases run in the `db-drivers` workflow against new fixtures.

Integrations recovered (8.80 to 9.50) after iteration 4 moved every MCP budget into one place and made a
failed secret read an error. Architecture rose with the iteration 4 splits; `run_workflow_with_budget`,
`GraphView.svelte`, `WorkflowsPage`'s run monitor, the REST engine move and the
`agent_session` move into `otto-agent-run` are still open.

## Deferred findings

These findings are not assigned to any package in iteration 5.

- **architecture-5-03:** needs a product and security decision, and is multi-day. Unifying
  `McpService::invoke` and `governed_invoke` would apply existing `mcp_policies` and auto-approve
  rules to `otto.*` tools and registered servers for the first time, which changes what admins'
  current rules allow or deny. It needs an explicit decision and a rule-migration plan.
- **architecture-1-04:** multi-day, and depends on architecture-5-02 landing first (P09 was to move
  `agent_session` into `otto-agent-run` this iteration). Schedule the shared fan-out/summarizer
  runner next iteration on top of `otto-agent-run::session_turn`.
- **architecture-2-06:** extracting the otto-rooms crate (rooms, recap engines, ICE, game rooms and
  their ctx seams) is multi-day. Its step 1, the shared capability primitives, ships this iteration as
  architecture-5-01 in P10.
- **concurrency-async-1-05:** the fix changes the plugin protocol (the plugin binds `:0` and reports
  its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`). That breaks existing third-party plugins, so
  it needs a protocol-versioning decision.
- **Partial deferrals inside assigned findings (the ids stay assigned):** backend-performance-5-04
  pagination and default view are a product decision, while the column projection ships in P08.
  architecture-4-02 phase 2 (AssistTarget engine) is multi-day after 5-02, while the interim steps
  ship in P09. The architecture-5-02 step-2 forwarders for `AutomationCtx` and `ProductStudioHost`
  are follow-ups, because their crates are owned by P01 and P03 this round.
