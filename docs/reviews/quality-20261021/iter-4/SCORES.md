# Quality iteration 4: lens scores

The 15 lens reviews in this folder each gave a score out of 10 for the code as it stood at the start of
iteration 4, after the iteration 3 fixes. The target is 9.8 or higher on every lens. Each score is the
reviewer's own number, before any iteration 4 fixes, so it does not reflect the work listed in
[FIXES.md](./FIXES.md). The next review re-scores the code after those fixes.

| Lens | Iteration 1 | Iteration 2 | Iteration 3 | Iteration 4 | Change | Gap to 9.8 |
|------|------------:|------------:|------------:|------------:|-------:|-----------:|
| backend-correctness | 9.37 | 9.77 | 9.47 | 9.74 | +0.27 | 0.06 |
| ui-correctness | 8.20 | 8.37 | 8.97 | 9.30 | +0.33 | 0.50 |
| access-control | 9.30 | 9.30 | 8.00 | 9.00 | +1.00 | 0.80 |
| backend-performance | 8.67 | 9.77 | 9.64 | 9.61 | −0.03 | 0.19 |
| ui-performance | 9.34 | 9.30 | 9.80 | 9.60 | −0.20 | 0.20 |
| concurrency-async | 9.34 | 9.64 | 9.68 | 9.81 | +0.13 | 0.00 |
| data-integrity | 9.10 | 9.27 | 9.41 | 9.47 | +0.06 | 0.33 |
| error-handling | 8.17 | 9.17 | 9.31 | 9.41 | +0.10 | 0.39 |
| architecture | 6.30 | 5.90 | 6.77 | 7.31 | +0.54 | 2.49 |
| maintainability | 7.25 | 8.28 | 8.65 | 9.39 | +0.74 | 0.41 |
| test-quality | 8.70 | 8.50 | 8.80 | 8.50 | −0.30 | 1.30 |
| api-contracts | 8.71 | 9.05 | 9.11 | 9.34 | +0.23 | 0.46 |
| ux-design | 9.17 | 9.04 | 9.37 | 9.15 | −0.22 | 0.65 |
| accessibility-responsive | 8.70 | 8.50 | 8.10 | 8.50 | +0.40 | 1.30 |
| integrations | 8.60 | 9.47 | 9.50 | 8.80 | −0.70 | 1.00 |

Mean: 9.13 (iteration 3: 8.97, iteration 2: 8.89, iteration 1: 8.58). concurrency-async reached the
target. The lowest lenses are architecture (7.31), test-quality (8.50), accessibility-responsive
(8.50), integrations (8.80) and access-control (9.00).

Access-control recovered after iteration 3 fixed the MCP token rotate blocker and the workflow-run
identity. The reviewer still found that swarm turns acted as the swarm's creator whatever user
created the task (access-control-4-01, carried from 3-03), that a non-owner Editor could rewrite a
personal agent's standing goals, rules and memory (4-02), and that a non-root owner got 403 on the
files of their own data-dir sessions (4-03). Iteration 4 fixed all three (PC-1, PC-2, PC-3).

Integrations dropped because the reviewer measured the MCP bridge budgets end to end: the gateway
call timed out before a governed call that waits for approval could answer (4-01), Codex used its
own 60 s tool timeout below the bridge budgets (4-03), and a failed keychain read was treated as
"no secrets" and could wipe a server's secrets on the next save (4-05). Iteration 4 moved every
budget into `otto_core::mcp_budgets`, gave the gateway call its own 65 s timeout and made a failed
secret read an error.

Test-quality dropped because the reviewer looked for refusal paths that no test exercised: the
forge-post review path, run-file key and size handling, the rooms media sequence guard and the MCP
decide-route refusals. Iteration 4 added tests for each; one MCP decide case is still open (see
FIXES.md).

Architecture rose with the iteration 3 splits and the usage tailer move. `run_workflow_with_budget`,
`GraphView.svelte`, the `Terminal.svelte` resize half, `RunMonitor` and the REST engine move are
still open. The LOC ratchet now also caps every production function at 400 lines, so the 23
functions over that cap today can only shrink.

## Deferred findings

These 5 findings are not assigned to any package in iteration 4.

- **architecture-1-04:** multi-day. Review, skill review and vault docs have different summarizer
  contracts, budgets and fallbacks. A shared fan-out and summarizer runner needs a design pass and a
  port of all three engines with their tests.
- **architecture-2-06:** blocked on the games team's merge. `game_rooms` and
  `ui/src/modules/rooms/games/**` are owned by another team, so extracting otto-rooms now would
  collide with their work.
- **architecture-2-08:** depends on architecture-2-06 and on the games team's files (the shared game
  catalog JSON).
- **concurrency-async-1-05:** both fixes change the plugin protocol (the plugin binds `:0` and
  reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`) and would break existing third-party
  plugins. It needs a product decision and a protocol version bump.
- **architecture-4-02 (phase 2 only; phase 1 is in P10):** merging the Design Hall, canvas and
  product-mockup assist engines into one `AssistTarget` engine means choosing which completion,
  validation and Excalidraw rules win (a product decision), and it is multi-day work.
