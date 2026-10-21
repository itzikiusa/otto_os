# Quality iteration 1: fix plan

There are 146 findings from 15 lenses. 131 are assigned to 10 packages (P1–P10). 15 are deferred.
Every package's `owned_paths` set is disjoint from every other package's set.

## Rules for every fixer

- **Edit only your owned paths.** You may read anything.
  - If a fix needs a change in a file you don't own, don't make it. Write it up as a handoff in your final report.
  - If removing an export would break an importer you don't own, keep a one-line re-export or alias instead.
- **Never touch `ui/src/modules/rooms/games/**`.** Other agents are working on the games.
- **No new crate or npm dependencies** (P10 only removes one). Never edit `Cargo.lock` or the root `Cargo.toml`.
- **Integration tests:** don't create new `crates/otto-server/tests/<suite>.rs` files. P4 owns `tests/it.rs`. Add tests to suite files you own, or as in-crate `#[cfg(test)]` tests.
- **Docs:** only P5 edits `docs/contracts/**` and `docs/features/**`. P5 already carries the contract edits other packages need (listed under P5).
- **UI guards:** only P10 edits `ui/scripts/ui-guards*.{mjs,json}`, `ui/package.json` and `ui/package-lock.json`.
- **Verify cheaply.** Use `cargo check -p <crate>`, `cargo test -p <crate> <filter>` and `cargo clippy -p <crate> --all-targets -- -D warnings` on your crates. For UI work, run `cd ui && npm run check` and `npm run test:unit`.
  - Run one heavy command at a time. Never run `cargo clean`.
- **Security changes ("who can access what") must not lock out legitimate users.** Recent regressions came from over-tightening. Every authz change ships with a test that the intended user still gets in.
- **Git:** don't commit, don't stash, and don't reset other agents' changes.

## Packages

| Pkg | Theme | Findings |
|-----|-------|----------|
| P1 | Git, cadence, launcher artifacts, workbench a11y | backend-correctness-1-01..04, backend-performance-1-09, integrations-1-01, integrations-1-02, ux-design-1-06, error-handling-1-08, maintainability-1-12, accessibility-responsive-1-01, -1-05, -1-06, ui-performance-1-06 |
| P2 | Sessions, MCP and the bridge | integrations-1-03..07, concurrency-async-1-02, error-handling-1-06, -1-07, -1-10, maintainability-1-23, test-quality-1-03, -1-04, architecture-1-05 (interim parity test) |
| P3 | Server hot paths, cancel registry, AWS env | backend-performance-1-05, -06, -07, -08, -10, architecture-1-03, maintainability-1-01, -1-02, -1-05, -1-15, backend-correctness-1-05 |
| P4 | Access control and security-adjacent tests | access-control-1-01..03, maintainability-1-14, -1-21, test-quality-1-01, -1-07, -1-08, -1-09, error-handling-1-05 |
| P5 | Contracts, types.ts, docs, issue UI, swarm graph removal | api-contracts-1-02..13, ux-design-1-01, -1-02, maintainability-1-03, -1-16, test-quality-1-05, -1-06, accessibility-responsive-1-09 |
| P6 | Data integrity, Rust dead code, dedupe | data-integrity-1-01..07, maintainability-1-11, -1-22, backend-performance-1-02, ui-correctness-1-03, -1-08 |
| P7 | Concurrency, sockets, plugins, usage tailer | concurrency-async-1-01, -03, -04, -05, -06, error-handling-1-14, backend-performance-1-01, -03, -04, integrations-1-08, maintainability-1-17, -1-19, -1-20 |
| P8 | Database dashboards and core UI stores | ui-correctness-1-01, -02, -04, -05, -06, -07, error-handling-1-02, -1-09, -1-11, -1-15, accessibility-responsive-1-03, -1-07, ui-performance-1-03 |
| P9 | Rooms UI, toggle names, product pollers | ui-performance-1-01, -02, -04, -05, ux-design-1-03, -04, -05, -07, accessibility-responsive-1-02, -1-04, error-handling-1-01, -1-03, -1-13, api-contracts-1-01 (client-side fix) |
| P10 | UI shared helpers and dead code | maintainability-1-04, -06, -08, -09, -10, -13, -18, architecture-1-14, error-handling-1-04, -1-12, test-quality-1-02, accessibility-responsive-1-08 |

The full owned paths and instructions for each package are in the structured output that this file accompanies. The cross-package notes are below.

## Cross-package notes

- **P3, architecture-1-03:** keep `otto_core::cancel::CancelRegistry` as the existing type alias. Add the operations as an extension trait plus an RAII `CancelGuard`, so files P3 does not own keep compiling.
  - `otto-swarm/src/runtime/run.rs` must keep `signal_cancel(&CancelRegistry, &str)`, because P6 calls it from the swarm delete path.
- **P5 makes the contract edits for other packages:**
  - access-control-1-02: share revoke authority, and 404 when nothing matched.
  - access-control-1-03: rooms GETs and POST /game-rooms at Agents:View.
  - maintainability-1-01: drop the node-cache clause at api.md:2331.
- **P5 also removes `getWorkflowVersion` from `lib/api/workflows.ts`** for maintainability-1-08, which P10 owns. P10 must not touch `workflows.ts`.
- **P9 fixes api-contracts-1-01 on the client.** ProofPage resolves the repo through the existing `GET /workspaces/{id}/repos`, so no route or contract change is needed.
- **P2 deletes `SessionManager::with_mcp_tools_bin` (manager.rs:2018)** for maintainability-1-11, which P6 owns.

## Deferred (15)

| Finding | Reason |
|---------|--------|
| architecture-1-01 | `execute_node` is ~2,800 lines. A per-node module split is several hours of work, and P3 is editing `workflow_engine.rs` this iteration. Schedule it for a later iteration as the only writer of the file. |
| architecture-1-02 | Splitting `modules.rs` (7.6k lines) conflicts with P3's edits to the same file. It needs a dedicated pass. |
| architecture-1-04 | A generic fan-out runner in otto-agent-run is new infrastructure, and migrating skill_review and vault_docs onto it is more than 45 minutes. |
| architecture-1-06 | Splitting `otto-telemetry` into two crates is a workspace and dependency-graph change. It touches Cargo.toml and Cargo.lock across many crates. |
| architecture-1-07 | Extracting the REST engine from the 4.4k-line `workflow_api_request` and `api_client` into `otto-apiclient` is a multi-hour refactor that touches P7's file. |
| architecture-1-08 | The 5k-line `WorkflowsPage` split conflicts with P9's edits to the same file and is too large for one session. |
| architecture-1-09 | The 5.3k-line `GraphView` split is too large for one session. |
| architecture-1-10 | Breaking the 4.3k-line `DatabaseStore` into sub-stores conflicts with P8's store fixes and is too large. |
| architecture-1-11 | Consolidating the markdown renderers into `lib/render` crosses vault, canvas and lib ownership and changes rendering behaviour. Do it in a dedicated pass. |
| architecture-1-12 | Moving the 3D engine and design-board into `lib/design` restructures modules across product, design-hall and swarm, which three packages own. |
| architecture-1-13 | Removing every lib → modules import is a broad move across many owned files. It needs its own pass and then a ratchet. |
| architecture-1-15 | The review-lifecycle `transition()` helper belongs with the `modules.rs` review split (architecture-1-02). Doing it now creates churn. |
| architecture-1-16 | Moving the tailer driver into otto-usage is a crate move. P7 is changing the same tailer code this iteration. |
| architecture-1-17 | Splitting `types.ts` (12k lines) into per-area files conflicts with P5's edits to `types.ts`. Do it later as the only writer of the file. |
| maintainability-1-07 | The 12 relative-time helpers use four different output styles ("5m", "5 min ago", "just now", and a date). Unifying them on `rel()` is a product-copy decision about which style to keep, and it changes visible labels on 12 surfaces. |
