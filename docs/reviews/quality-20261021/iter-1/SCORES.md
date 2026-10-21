# Quality iteration 1: lens scores

The 15 lens reviews in this folder each gave a score out of 10 for the code as it stood at the start of
iteration 1 (base `main` @ `d447516c4`). The target is 9.8 or higher on every lens. Each score is the
reviewer's own number, before any fixes, so it does not reflect the work listed in [FIXES.md](./FIXES.md).
The iteration 2 review re-scores the code after those fixes.

| Lens | Score | Gap to 9.8 |
|------|------:|-----------:|
| backend-correctness | 9.37 | 0.43 |
| ui-correctness | 8.20 | 1.60 |
| access-control | 9.30 | 0.50 |
| backend-performance | 8.67 | 1.13 |
| ui-performance | 9.34 | 0.46 |
| concurrency-async | 9.34 | 0.46 |
| data-integrity | 9.10 | 0.70 |
| error-handling | 8.17 | 1.63 |
| architecture | 6.30 | 3.50 |
| maintainability | 7.25 | 2.55 |
| test-quality | 8.70 | 1.10 |
| api-contracts | 8.71 | 1.09 |
| ux-design | 9.17 | 0.63 |
| accessibility-responsive | 8.70 | 1.10 |
| integrations | 8.60 | 1.20 |

Mean: 8.58. Lowest: architecture (6.30), maintainability (7.25), error-handling (8.17) and ui-correctness (8.20).

Access-control was reviewed for who can reach what, not for theoretical hardening. Recent regressions
came from rules that were too tight. Two of the three access-control fixes this iteration made access
less strict: room reads and game hosting moved to Agents:View, and session owners can now revoke share
links that someone else minted. Each fix comes with a test showing that the intended user still gets in.

## Deferred findings

These 15 findings are not assigned to any package in iteration 1. Most of them are on the architecture
lens, which is why that score will stay low until they are scheduled.

- **architecture-1-01:** `execute_node` is about 2,800 lines. Splitting it into one module per node
  takes several hours, and P3 is editing `workflow_engine.rs` this iteration. Schedule it later, with
  that pass as the only writer of the file.
- **architecture-1-02:** splitting `modules.rs` (7.6k lines) conflicts with P3's edits to the same file
  and needs a dedicated pass.
- **architecture-1-04:** a generic fan-out runner in otto-agent-run is new infrastructure. Moving
  `skill_review` and `vault_docs` onto it takes more than one 45-minute session.
- **architecture-1-06:** splitting otto-telemetry into two crates changes the workspace and its
  dependency graph, which touches `Cargo.toml` and `Cargo.lock` in several crates.
- **architecture-1-07:** extracting the REST engine from the 4.4k-line `workflow_api_request` and
  `api_client` into otto-apiclient is a refactor of several hours, and it overlaps P7's file.
- **architecture-1-08:** splitting the 5k-line WorkflowsPage component conflicts with P9's edits and is
  too large for one session.
- **architecture-1-09:** splitting the 5.3k-line GraphView (lanes, menus, sidebar) is too large for one
  session.
- **architecture-1-10:** breaking the 4.3k-line DatabaseStore into sub-stores conflicts with P8's store
  fixes and is too large for one session.
- **architecture-1-11:** consolidating the markdown renderers into `lib/render` crosses vault, canvas and
  lib ownership, and it changes how markdown renders. It needs a dedicated pass.
- **architecture-1-12:** moving the 3D engine and design board into `lib/design` restructures product,
  design-hall and swarm, which three different packages own this iteration.
- **architecture-1-13:** removing every `lib` → `modules` import is a broad move across many modules. It
  needs its own pass, followed by a ratchet.
- **architecture-1-15:** the review-lifecycle `transition()` helper belongs with the `modules.rs` review
  split (architecture-1-02). Adding it now would mean churn in P3's file.
- **architecture-1-16:** moving the usage tailer driver into otto-usage is a crate move, and P7 is
  changing the same tailer code this iteration.
- **architecture-1-17:** splitting the 12k-line `types.ts` into one file per area conflicts with P5's
  `types.ts` edits. Do it later, with that pass as the only writer of the file.
- **maintainability-1-07:** moving the 12 relative-time helpers onto `rel()` is a product-copy decision.
  The helpers use four output styles ('5m', '5 min ago', 'just now' and a date), so unifying them would
  change the visible labels on 12 surfaces.
