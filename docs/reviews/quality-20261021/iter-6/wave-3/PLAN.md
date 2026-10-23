# Quality iteration 6, wave 3 (last fix wave): fix plan

Worktree: `/Users/itziklavon/claude_ade-iter6` (branch `fix/quality-iter6-20261023`, at `4f6d6ab30`).
Inputs: the four re-score reports in `../wave-2/` (`architecture.md` 9.11, `test-quality.md` 9.77,
`accessibility-responsive.md` 9.87, `ux-design.md` 9.97) and `../wave-2/FIXES.md` (what wave 2 left open).
Output: 8 work packages (P1 to P8) for 8 parallel fixers in one shared worktree.

**Goal.** This is the last fix wave, and the score that follows it is the final one. Every open finding in the four
target lenses is planned to be **closed completely**. Nothing is deferred and nothing is narrowed to an interim
step. Architecture comes first: each of its nine open findings (1 major, 5 minors, 3 nits) has its own package or
shares one with findings on the same files. If every planned fix lands, architecture scores 10.0, test-quality 10.0,
accessibility-responsive 10.0 and ux-design 10.0. A "partly fixed" verdict costs the full weight again, so each
package's **Done means** is the reviewer's complete fix.

## Status check before planning (items the task lists as "known open")

These were checked against `4f6d6ab30` and need no package:

| Item | Status now |
|------|------------|
| `run_workflow_with_budget` split | Done in wave 1 (`workflow_engine.rs:1856`, 93 lines; not in `loc-baseline.json` `functions`). |
| WorkflowsPage RunMonitor move (1-08r) | Done in wave 2 (WorkflowsPage 3,710 → 2,688). Only the name collision 6w2-02 is left (P6). |
| `ClassroomsBox.svelte` / `ShooterGame.svelte` over 800 | 783 and 792 lines: under 800. Owners must keep them under 800 (P8 does not touch home; P5 owns rooms). |
| McpService::invoke vs governed_invoke (5-03) | Fixed in wave 2 (`otto_mcp::governance::run`, two named stage sets, table test of 15 cases). The decision "unify the path, keep today's allow/deny outcomes" is what landed. P3 must not change either stage set. |
| Shared fan-out/summarizer runner (1-04) | Vault docs and code review are on `otto_agent_run::fanout`. Product is the rest (P2). |
| lib → modules imports (1-13) | Fixed. `lib-imports-modules` is a hard guard (max 0). No package may add one. |

## Rules every package follows

1. **Owned paths are exclusive.** Edit only the paths your package owns. If a fix needs a file you do not own, stop
   and put a hand-off in your final report (path and exact change). Do not edit the file.
   - Read-only for everyone: `scripts/loc-baseline.json`, `crates/otto-server/src/routes/mod.rs`,
     `crates/otto-server/src/policy.rs`, `crates/otto-server/src/modules/**`, `docs/contracts/**`,
     `ui/src/lib/api/types.ts`, `ui/src/lib/api/types/**`, `crates/otto-core/src/api/**`,
     `crates/otto-state/migrations/**`, `Cargo.lock` (never hand-edit; cargo may update it), and any file no
     package lists.
   - **No new routes, no wire changes, no migrations.** Request and response shapes, status codes, error texts and
     WS events stay byte-for-byte. Tool names and the session-visible tool set stay the same (P3).
2. **Splits and moves preserve behaviour.**
   - A move and a behaviour change are never in the same edit.
   - **Rust:** keep the old path compiling with `pub use` so other packages build. Moved unit tests move with
     their code.
   - **UI:** update every importer inside your own paths. No module → lib re-export shim (`scripts/check.sh` caps
     them at 3). A same-module façade (the old store or component delegating to its new siblings) is fine.
   - New functions stay under `function_max: 400` (aim for under 150). No new or moved UI file reaches 800 lines.
     Each split states its size target. Meet it; it is part of **Done means**.
   - Ratchet baselines: the integrator runs `python3 scripts/loc-ratchet.py --update-baseline` and
     `node scripts/ui-guards.mjs --update-baseline` once, after every package lands. Report every entry your
     package lowers or removes (old → new), and every renamed key. P8's step 0 is the one exception (rule 8).
3. **Behaviour changes ship with a test that fails without the change.** Name it, and say how you showed it fails
   (revert the line, run it, see it fail, restore). For a pure move, write the **characterization test first,
   against the old code**, then move, then re-run it unchanged.
   - **Rust:** prefer existing suites and `#[cfg(test)]` modules in your own files. Only P7 edits
     `crates/otto-server/tests/it.rs`. A new otto-server suite needs a `#[path]` line there: hand it to P7.
   - **UI:** new unit tests go in new files `ui/unit/w3pN-<topic>.test.ts`. Drive behaviour through
     `componentFunctions`, the real store/class and fakes. No source regexes standing in for behaviour
     (`unit-source-regex` ratchet); regexes are fine only for structural rules and must be labelled "(structure)".
4. **UI rules** (`docs/design/guidelines/`). Each UI package fixes every target-lens finding in its directories and
   **sweeps** them for more of the same:
   - **Live regions.** Nothing inserted together with its text in `{#if}`, `{:else if}` or `{:else}`. Failures:
     `use:announceShown={{ text, assertive: true }}` on the line, or `announce()` at the state edge, and drop the
     role. Notices: polite `announceShown`. Loading lines: drop the role and announce once at the edge, or use
     `LoadState` / `Skeleton`. A counter that updates: one always-mounted `sr-only` region (`NewSession.svelte:550`
     pattern). Never both a mounted region and `announceShown` for the same text (6w2-02). After P8's step 0, run
     `node scripts/ui-guards.mjs` and take every `inserted-live-region` hit in your paths to zero (a genuinely fine
     hit, such as an always-mounted region whose text changes, stays; list it with the reason).
   - **Controls.** Icon-only buttons are real `<button>`s with `aria-label` and `title`. Dialogs and menus move
     focus in, trap it, close on Escape and return focus. A control that removes itself moves focus to a named
     successor. Logical CSS properties. Nothing scrolls horizontally at 390 px.
   - **States.** `PageHeader`/`PageBody`. Loading, empty, error-with-Retry and loaded each render; `LoadState` for
     load errors, `EmptyState` for empty. Never show a guessed value (`?? true`, a `0` count while loading). Tokens
     only. `confirmer`/`toasts`/`ctxMenu`. "Couldn’t …" copy. Clipboard via `copyWithToast`.
   - **e2e specs.** If you remove a `role` a spec asserts, update the spec if it is in your owned paths. Otherwise
     report it as a hand-off. Do not run Playwright. When two packages' spec globs match one file name, the
     lower-numbered package owns it (for example `desktop-mcp-api-tools.spec.ts` is P3's).
5. **Cargo.** Only P5 edits the root `Cargo.toml` (the `otto-rooms` member and workspace-dependency lines) and
   `crates/otto-server/Cargo.toml`. A package may edit the `Cargo.toml` of a crate it owns, using
   `.workspace = true` for any dependency that is already in `[workspace.dependencies]` (reqwest,
   reqwest_cookie_store, otto-state, otto-ssh, otto-keychain, otto-transcript, axum all are). Never run
   `cargo update`.
6. **Commands and load.** This Mac froze under about 10 parallel builds.
   - At most one heavy command at a time (cargo build/test/clippy, `npm run check`, a full vitest run), through the
     coordinator's lock if one is provided. Budget: about 5 cargo commands and 2 `npm run check` runs per fixer.
   - Iterate with `cargo check -p <crate>`; then `cargo test -p <crate> --lib <filter>` /
     `cargo test -p otto-server --test it <suite>::` and `cargo clippy -p <crate> --all-targets -- -D warnings` on
     your crates. UI: `cd ui && npx vitest run unit/<your files>` and one `npm run check` at the end.
   - Never `cargo clean`, the full workspace suite, e2e, or `scripts/check.sh --all`. The integrator runs the gates.
7. **Docs.** Update the `docs/features/*.md` pages you own when a cited path or behaviour changes. AGENTS.md crate
   rows: P5 only (other crate-row changes are hand-offs to P5).
8. **Ratchet baselines during the wave.** P8 widens the `inserted-live-region` detector in its step 0 and records
   the new hits for that one rule only, so nobody's `npm run check` breaks mid-wave. Nobody else edits
   `ui/scripts/ui-guards-baseline.json` or `scripts/loc-baseline.json`.
9. **Git and other agents.** No commits, stashes, resets or checkouts; the integrator commits. Other agents in this
   session: `games-splits`, `kart-stuck-fix`, `mcp-db-execute`, `mcp-jira-create`, `mcp-profiles`,
   `e2e-regressions`, `round-integrator`. Before editing a file, run `git status --short <file>` and
   `git diff <file>`; if it has uncommitted changes you did not make, report it and leave it. The `mcp-*` agents
   work in other worktrees on the same MCP catalogs: P3 keeps its `mcp_tools.rs` change a sequence of mechanical
   moves plus deletions, so their later merge stays tractable.
10. **Final report.** For each finding id: fixed, partial or skipped, with the reason. List tests added (and how each
    was shown to fail), line counts before/after for every split, baseline entries to lower, commands run and
    their results, and hand-offs.

## Cross-package contracts (step 0)

| Owner | Step 0 | Used by |
|-------|--------|---------|
| P8 | Widen `detectInsertedLiveRegion` (see P8 item 1), add `--update-baseline --only <rule>` to `ui-guards.mjs`, and record the `inserted-live-region` counts for **that rule only**. | P1, P5, P6, P7 (each takes its own hits to zero) |
| P2 | If `otto_agent_run::fanout` needs a new generic piece for product (for example a plan summarizer that is skipped when fewer than two candidates exist), add it to `fanout.rs`/`fanout/**` with a test, keeping every existing signature. | P2 only |
| P5 | `crates/otto-rooms` exists as a workspace member before anything moves into it (an empty `lib.rs` is enough), so `cargo check -p otto-rooms` works. | P5 only |

## Package overview

| Pkg | Theme | Findings |
|-----|-------|----------|
| P1 | GraphView split to a thin shell, git module sweep (UI) | architecture-1-09 (major), accessibility-responsive-6w2-01 (git) |
| P2 | Product analysis and plan on `otto_agent_run::fanout`; `run.rs` split; assist test flake (Rust) | architecture-1-04, test-quality-6w2-03 |
| P3 | One MCP tool surface: session catalog generated from the control-plane specs, routed twins, parity test; MCP UI; DB-execute test (Rust + UI) | architecture-1-05, test-quality-6w2-02, ux-design-6w2-01 |
| P4 | REST execute engine into `otto-apiclient` behind `ApiClientCtx` (Rust) | architecture-1-07r |
| P5 | `otto-rooms` crate behind `RoomsCtx`; rooms UI sweep (Rust + UI) | architecture-2-06, accessibility-responsive-6w2-01 (rooms) |
| P6 | `apiClient` store split to a façade under 800 lines; `RunSync` rename; api/workflows/workbench sweep (UI) | architecture-6-03, architecture-6w2-02 |
| P7 | Swarm paste path removed; last Claude layout copies; ingest tests and the riskiest untested ingest bodies; product UI (Rust + UI) | architecture-4-01r, architecture-6w2-01, test-quality-6w2-01, accessibility-responsive-6w2-01 (product), accessibility-responsive-6w2-02 |
| P8 | UI guards (widened detector + new rules), design hall and the remaining modules, shared components and shell (UI) | accessibility-responsive-6w2-01 (rest + guard) |

---

## P1: GraphView split to a thin shell

**Owns:** `ui/src/modules/git/**` (including `git/graph/**`), `ui/unit/w3p1-*.test.ts`, the git unit tests that
cite GraphView (`ui/unit/gitUiSafety*.test.ts`, `ui/unit/w2p5-graphActions.test.ts`), `ui/e2e/*git*.spec.ts`,
`docs/features/git.md`.

**Findings:** architecture-1-09 (major), accessibility-responsive-6w2-01 (git part: `GitAccounts.svelte:304`).

1. **arch-1-09: extract the three inline parts, then the script state.** `GraphView.svelte` is 4,082 lines: script
   `:1-1931`, markup `:1932-2887`, style `:2888-4082`.
   - `git/graph/RefSidebar.svelte`: the `<aside>` at `:1985-2376`, its CSS and the sidebar-only state (filter,
     collapse, section expansion, row windows, drag/drop of refs). Props: the refs tree, the selected ref, the
     `GraphActions` instance and callbacks for "select commit" / "open ref menu". It renders no graph state.
   - `git/graph/RefPopover.svelte`: the popover markup at `:2822-2886`, its script (`:1723-1930`: open/close,
     clamping, row click/double-click/right-click) and its CSS (`:3636-~3700`). Keep `dialogFocus`, the backdrop
     button and the clamped `max-height` exactly (AGENTS.md floating-UI rule).
   - `git/graph/CommitDetailPane.svelte`: the `.detail-panel` at `:2743-2820` and its CSS, including the `.mobile`
     rules at `:4025-4050`.
   - Then move what remains of the script into `.svelte.ts` siblings until the component is a thin shell: for
     example `git/graph/graphSelection.svelte.ts` (selection, keyboard row navigation, scroll-to-commit),
     `git/graph/graphViewport.svelte.ts` (virtual window, lane layout wiring, resize) and the search wiring if it
     is still inline. Each takes a narrow host interface, as `GraphActionsHost` does.
   - **Size targets:** `GraphView.svelte` ≤ 1,800 lines with a script block ≤ 700 lines. Every new file < 800.
   - **Tests (characterization first):** `ui/unit/w3p1-refPopover.test.ts` (clamping into the viewport for a list
     taller than the window, Escape/backdrop close, row double-click checks out, right-click opens the ref menu),
     `ui/unit/w3p1-graphSelection.test.ts` (keyboard navigation across the window edge, RTL if it applies). Keep
     `w2p5-graphActions.test.ts` and the gitUiSafety tests green; update their imports if they referenced moved
     code.
   - **Done means** no refs-sidebar, popover or detail-pane markup is left in `GraphView.svelte`, the size target
     is met, and the e2e specs `desktop-git-graph-*`, `desktop-git-sidebar-perf`, `desktop-git-branch-*` still
     find their selectors (keep class names and `data-*` hooks; update a spec only for a renamed hook).
2. **a11y-6w2-01 (git):** `GitAccounts.svelte:304`, plus every git hit P8's widened detector reports. Sweep
   `modules/git/**` for rule-4 patterns (the new components inherit the rules: icon-only buttons named, the popover
   traps focus).

Order: RefPopover, CommitDetailPane, RefSidebar, script state, then item 2. Report the `file-loc` entry:
`src/modules/git/GraphView.svelte` 4082 → new count.

---

## P2: Product analysis and plan on the shared fan-out runner

**Owns:** `crates/otto-product/**`, `crates/otto-agent-run/**`, `crates/otto-server/src/product_host.rs`,
`crates/otto-server/tests/unit/product_studio_stop.rs`, `docs/features/product.md`.

**Findings:** architecture-1-04, test-quality-6w2-03.

1. **TQ-6w2-03 (do first, it is small).** In `otto-agent-run/src/assist/tests.rs`: give `FakeDoc` a
   `tokio::sync::Notify` that `on_live` signals. In `FakeRunner::run_turn`, replace `sleep(SETTLE)` after the
   write with `timeout(Duration::from_secs(5), doc_seen.notified())` and panic with "the live preview never
   arrived" on timeout. Keep the fixed waits only in the negative checks (the post-stop write in
   `StoppedMidTurn`, `assert_poll_stopped`). Show the old version's race is gone by running the three tests 50
   times (`cargo test -p otto-agent-run --lib assist:: -- --test-threads=1` in a loop is fine within budget, or
   `nextest --retries 0` with a repeat count).
2. **arch-1-04: product's fan-outs run on `otto_agent_run::fanout`.**
   - **`run_analysis`** (`run.rs:495`, 575 lines). Build one future per lens spec (the existing per-lens session
     turn, unchanged) and pass them as `reviewers` to `fanout::run`. `summarizer` = `build_summarizer_prompt` plus
     the summarizer session (`:756-850`); `fallback` = the existing Rust merge (`:887-916`); `on_state` persists
     the agent rows and broadcasts exactly as today; `cancelled` reads the story cancel registry. Delete the
     hand-rolled join loop, the summarizer session block and its "stuck in running" guard (`:804-806`): the runner
     owns that edge now. Keep `SUMMARIZER_NAME_PREFIX`, `is_summarizer_agent`, `retry_analysis_agent*` and
     `reap_orphaned_agents_on_startup` behaviour identical (the reaper's summarizer case at `:1407-1416` stays).
   - **`run_generate_plan`** (`run.rs:2273`, 444 lines, also in `loc-baseline`). Same port: planners as
     reviewers; the plan summarizer runs only when more than one candidate exists (today's rule, `:2568-2569`); the
     single-candidate path returns it directly. If `fanout` lacks "skip the summarizer below N results", add it
     generically (step 0) with a test.
   - Check `run_rewrite` and `run_generate_tests`: if either runs more than one agent and merges, port it the
     same way; if it runs one agent, leave the turn alone.
   - **Split `run.rs`** (3,598 lines) into `run/{analysis,plan,rewrite,tests_gen,reap,prompts,scratch}.rs`, with
     `run.rs` a module root that re-exports every current `pub` name, so `otto-server` (`product_host.rs`,
     `workflow_engine` nodes, routes) compiles unchanged. **Size targets:** `run.rs` ≤ 400 lines; no function
     over 250; `run_analysis` and `run_generate_plan` leave `loc-baseline` `functions`.
   - **Tests** (in `otto-product`, over a fake `ProductRunHost` if one exists, else a minimal one in
     `#[cfg(test)]`): one lens fails and the summarizer still runs over the others; the summarizer fails and the
     Rust merge's questions are created, with the analysis status as today; a cancel after the lenses skips the
     summarizer and creates nothing; plan with one candidate starts no summarizer; plan with two runs it, and its
     failure keeps today's fallback. Each must fail when its branch is reverted.
   - **Done means** `rg -n "join_all|JoinSet|FuturesUnordered|summarizer_agent" crates/otto-product/src` finds
     only `fanout` adapters and comments, and the existing otto-product and `product_studio_stop` tests pass.

Order: 1, then 2 (analysis, plan, split).

---

## P3: One MCP tool surface

**Owns:** `crates/ottod/**`, `crates/otto-mcp/**`, `crates/otto-server/src/{mcp_outward,mcp_http,self_call}.rs`,
`crates/otto-server/tests/mcp_db_execute.rs`, `ui/src/modules/mcp/**`, `ui/unit/w3p3-*.test.ts`,
`ui/unit/w2p4-ottoServerHome.test.ts`, `ui/e2e/*mcp*.spec.ts`, `ui/e2e/mcp-catalog-fixture.ts`,
`docs/features/mcp-control-plane*.md`.

**Findings:** architecture-1-05, test-quality-6w2-02, ux-design-6w2-01.

**Decision (recorded 2026-10-22, no objection):** the control-plane schema wins, and the session catalog is
generated from it. **Kept exactly:** the set of tool *names* each session source sees (`tool_catalog_for_source`,
narrow sources, assistant tools, governed listing), the governance stage sets (`REGISTERED_SERVERS`, `OTTO_TOOLS`),
and every route a tool call reaches. Which surface gates the other (control plane narrower than stdio) is an open
product question, so P3 does **not** change visibility in either direction.

1. **Characterization first** (`crates/ottod/src/mcp_tools/tests/`):
   - A snapshot of the session `tools/list` names per source (default, each narrow source, assistant, vault-docs
     reviewer) taken on the current code. It must be identical after the change.
   - A table test over every one of the 46 twins (`otto_x` ⇄ `otto.x`): for representative arguments, the
     `(method, path, query, body)` the native `run_tool` dispatches today. After the change, the same table must
     hold when the call goes through `route_for`.
2. **arch-1-05: generate, route, prove parity.**
   - **Catalog.** Twins come entirely from `otto_tool_specs()` (name mapped by `governed_stdio_name`, description
     and schema from the spec). Delete their hand-written entries from `base_tool_catalog`, which then holds only
     native-only tools. `listing::generate_twin_schemas` becomes "generate twin entries".
   - **Dispatch.** Every twin whose native call maps onto `route_for` (after `resolve_native_refs` and the
     session-workspace defaults, which stay as a pre-step, and the row/text caps, which stay as a post-step)
     goes through `twin_read`/a `twin_call` sibling. Grow `ROUTED_TWINS` to the full twin list (or derive it from
     the spec list) and delete the matching arms of `run_tool`. A twin whose native behaviour cannot map without a
     visible change stays native; list each one with the reason in the report.
   - **Parity test.** A test that fails when a control-plane tool is missing from the session list for the
     sources that see governed tools today, and when a twin's session schema differs from its spec.
   - **Split `mcp_tools.rs`** (7,370 lines; ratchet entry 4,882 production) into `mcp_tools/{catalog,dispatch,
     native_refs,assistant}.rs`. **Size targets:** `run_tool` and `base_tool_catalog` under 400 lines each (they
     leave `loc-baseline` `functions`); `mcp_tools.rs` well under its 4,882 entry.
   - **Budgets.** Re-run the `tools/list` byte-budget test. Generated twin text must not raise either budget; if
     a twin's spec description is longer, the budget is not raised: report it instead.
   - **Stretch** (only after the above is green): split `route_for` (`otto-mcp/src/outward/exec.rs:522`, 1,555
     lines) into per-domain route functions it dispatches to, so it leaves `loc-baseline`.
3. **TQ-6w2-02.** In `tests/mcp_db_execute.rs`, at `:257-262` and `:300-305`, assert positively that the driver
   was reached: `err` contains the dead-port connect error the fixture's Redis driver produces (check the exact
   text once). Also assert `!err.contains("read-only connection") && !err.contains("review_required") &&
   !err.contains("Forbidden")`. Show the read case now fails if `GET otto:test` is classified as a mutation.
4. **ux-6w2-01.** `ui/src/modules/mcp/OttoServerHome.svelte`: drive `:510` from `attachState` (`:40`): ready →
   "attached"/"not attached" from `attach.attached`; loading → "Built-in tools: checking…"; error → "Built-in
   tools: unknown (see above)". Delete the `?? true` at `:58`. Add
   `{:else if gatewayLoading && !gatewayTools.length}<LoadState what="the gateway tools" variant="compact" loading empty rows={1} />`
   before the count branch. Extend `ui/unit/w2p4-ottoServerHome.test.ts` (or `w3p3-…`) to drive all three attach
   states and the gateway loading state. Sweep `modules/mcp/**` for rule-4 patterns and P8's widened hits.

Order: 3, 4, then 1 and 2.

---

## P4: REST execute engine into `otto-apiclient`

**Owns:** `crates/otto-apiclient/**`, `crates/otto-server/src/routes/api_client.rs`,
`crates/otto-server/src/routes/api_client/**`,
`crates/otto-server/src/routes/{api_automation_runs,api_stream,grpc,api_oauth}.rs`,
`crates/otto-server/src/workflow_api_request.rs`, `crates/otto-server/tests/unit/api_client.rs`,
`docs/features/api-client.md`.

**Findings:** architecture-1-07r.

1. **Characterization first.** Before moving, add tests in `tests/unit/api_client.rs` that pin what the engine's
   callers depend on: `resolve_environment` merge order, `substitute` (nested and dynamic vars),
   `effective_timeout`, `retention_from_settings`, `check_override_vars`, `new_host_conflict` /
   `unbound_secret_host` status and body, and for `execute_service` the error (status, JSON body) for a blocked
   local host, an unbound env secret and a bad URL.
2. **Move the engine.** Into `otto-apiclient` (for example `src/engine/{mod,env,send,auth,history,stream,
   multipart,tunnel,cookies}.rs`): `execute_service`, `resolve_environment`, `resolve_exec_auth`,
   `prepare_stream`, `prepare_request`, `build_and_send`, `apply_auth`, `attach_body`, `record_history`,
   `schedule_history_prune`, `cap_history_*`, `history_retention`, `retention_from_settings`,
   `workspace_allows_local`, `http_client`, `cookie_jar` and its sweep, the SSH/SOCKS tunnel cache,
   `substitute*`, `effective_timeout`, the secret-host checks, `describe_reqwest_error`, plus `run_step` from
   `routes/api_client/automations.rs`.
   - **`ApiClientCtx` trait** for what the engine needs from the server: the `ApiClientRepo` (or pool), the
     secret store, workspace settings, connection lookup for the tunnel, the broadcast/events hook for history
     appends, and the net guard. Implement it for `ServerCtx` in `routes/api_client/ctx.rs`.
   - **Errors.** The engine returns its own error enum (status + code + message + optional JSON detail). The route
     maps it to `ApiError` so every status and body is byte-identical (the step-1 tests prove it).
   - The route file keeps only axum handlers, query/body types, `repo()` and the ctx impl. Keep `pub use`/
     `pub(crate) use` re-exports for every moved name, so `routes/mod.rs` (read-only), `mcp_outward.rs`,
     `state_archive.rs` and the other callers compile unchanged. Repoint the callers you own
     (`workflow_api_request.rs`, `api_automation_runs.rs`, `api_stream.rs`, `grpc.rs`, `api_oauth.rs`) to the
     crate path.
   - `otto-apiclient/Cargo.toml` may add `reqwest`, `reqwest_cookie_store`, `otto-state`, `otto-ssh`,
     `otto-keychain`, `async-trait` and others already in `[workspace.dependencies]` (`.workspace = true`). It must
     not depend on `axum` or `otto-server`.
   - Move the unit tests that exercise moved code into `otto-apiclient`; `tests/unit/api_client.rs` keeps the
     handler-level ones.
3. **Size targets:** `routes/api_client.rs` ≤ 1,500 lines with no engine function left; no new file over 1,000
   lines; the otto-server `src` total drops by at least 1,500 lines (it was raised by 324 with this move named as
   the payback). Report the new total.

**Done means** `rg -n "fn (execute_service|resolve_environment|prepare_stream|run_step|build_and_send)"
crates/otto-server/src` finds nothing, `cargo test -p otto-apiclient` and the `api_client` unit tests pass, and
`rg -n "ServerCtx|axum" crates/otto-apiclient/src` is empty.

---

## P5: `otto-rooms` crate behind `RoomsCtx`

**Owns:** new `crates/otto-rooms/**`; `crates/otto-server/src/rooms/**`, `crates/otto-server/src/game_rooms/**`;
root `Cargo.toml` (only the `otto-rooms` member and workspace-dependency lines); `crates/otto-server/Cargo.toml`;
`crates/otto-server/src/lib.rs`, `crates/otto-server/src/state.rs`, `crates/otto-server/src/boot/**` (rooms wiring
only); `crates/otto-server/tests/rooms_api.rs`; `AGENTS.md` (crate-table rows only); `ui/src/modules/rooms/**`,
`ui/src/lib/rooms/**`, `ui/unit/w3p5-*.test.ts`, `ui/e2e/*room*.spec.ts`, `ui/e2e/rooms*.ts`;
`docs/features/{session-rooms,room-games}.md`.

**Findings:** architecture-2-06, accessibility-responsive-6w2-01 (rooms part).

1. **Step 0:** create the crate (workspace member, `[workspace.dependencies] otto-rooms = { path = … }`, empty
   `lib.rs`), and add it to `otto-server`'s dependencies.
2. **arch-2-06: move the domain.**
   - Pure domain (no `ServerCtx` today): `rooms/{registry,actions,presentation,annotations,capability,auth}.rs`,
     `rooms/recap/{archive,consent}.rs`, `rooms/recap_engines/**`, `game_rooms/{registry,types}.rs`.
   - Domain that touches the server today goes behind **`RoomsCtx`** (pool/stores, session manager, secrets,
     settings, data dir, base URL, audit/broadcast): the recap store and pipeline in `rooms/recap/mod.rs`
     (`store`, `:113-126`) and `recap/media.rs` (its non-axum parts), the ICE configuration builder in `ice.rs`
     (`configuration`, `:122`), `socket.rs`'s `sync_authority` / `end_room` / `effect` / `maintain`, and the
     terminal credit logic in `terminal.rs`. Implement `RoomsCtx` for `ServerCtx` in
     `otto-server/src/rooms/ctx.rs`.
   - What stays in otto-server: axum handlers, WS upgrade and the socket run loop (`rooms/{http,socket}.rs`,
     `recap/http.rs`, the handler parts of `media.rs`/`ice.rs`/`terminal.rs`, `game_rooms/{http,socket}.rs`), the
     route builders in `rooms/mod.rs` and `game_rooms/mod.rs`, and `api_tests.rs`. Both `mod.rs` files re-export
     every moved name, so `lib.rs`, `state.rs`, `policy.rs` (read-only) and `routes/mod.rs` (read-only) compile
     unchanged.
   - Move `rooms/tests.rs` and `game_rooms/tests.rs` with their code (into `otto-rooms`), except cases that need
     the router.
   - **Done means** `rg -n "ServerCtx|axum" crates/otto-rooms` is empty; `cargo test -p otto-rooms` passes;
     `otto-server/src/rooms` + `game_rooms` hold only glue (target ≤ 1,500 production lines together, from 8,429
     total); `tests/rooms_api.rs` passes unchanged; the AGENTS.md crate table has an `otto-rooms` row and the
     `otto-server` row says rooms routes only. Report the otto-server `src` total.
3. **a11y-6w2-01 (rooms):** `RoomRecap.svelte:73`, `RoomPage.svelte:174-175`, and every rooms hit of P8's widened
   detector. Sweep `modules/rooms/**` for rule-4 patterns. `games-splits` / `kart-stuck-fix` may be editing
   `rooms/games/**`: check `git status` first and leave any file with changes you did not make. Keep
   `ShooterGame.svelte` and `ClassroomsBox`-style files under 800.

Order: 1, 2 (pure domain, then `RoomsCtx` parts), 3.

---

## P6: `apiClient` store split; `RunSync` rename

**Owns:** `ui/src/lib/stores/apiClient.svelte.ts`, `ui/src/lib/stores/apiClient/**`,
`ui/src/lib/uiCommands/api.ts`, `ui/src/lib/api/codegen.ts`, `ui/src/lib/events/routes/apiClient.ts`,
`ui/src/modules/api/**`, `ui/src/modules/workflows/**`, `ui/src/modules/workbench/**`,
`ui/unit/{apiClientParts.ts,apiClientRunPoll.test.ts,apiWsSettings.test.ts,apiClientOwnership.test.ts,w2p7-runMonitor.test.ts}`,
`ui/unit/w3p6-*.test.ts`, `ui/e2e/desktop-api-*.spec.ts`, `ui/e2e/{desktop-review6-api-environment,desktop-ux-r4-parent-api}.spec.ts`,
`ui/e2e/*workflow*.spec.ts`, `docs/features/workflows.md`.
(`docs/features/api-client.md` belongs to P4; hand any UI file-list change for it to P4.)

**Findings:** architecture-6-03, architecture-6w2-02.

1. **arch-6w2-02.** Rename the class in `workflows/runSync.svelte.ts:77` from `RunMonitor` to `RunSync`. Update its
   importers (`RunApprovals`, `RunFinalOutput`, `RunMonitor.svelte`, `RunContextSidebar`, `WorkflowsPage`) and
   `w2p7-runMonitor.test.ts`, and drop the `RunMonitorState` alias in `WorkflowsPage.svelte:28`. Done means
   `rg -n "class RunMonitor|as RunMonitorState" ui/src` is empty.
2. **arch-6-03: split to a façade under 800 lines**, following `apiClient/automations.svelte.ts`:
   - `apiClient/model.ts`: the exported types and pure helpers (`:34-306`: `ApiDraft`, `defaultSettings`,
     `confirmNewHost`, `draftSecrets`, `draftToExtras`, `requestItem`, `TabResponse`, `historyResponse`, …).
     `apiClient.svelte.ts` re-exports them (same module, so not a lib shim).
   - `apiClient/collectionsSync.svelte.ts`: import file/parsed/environment, Postman sync, git pull/push
     (`:1028-1263`).
   - `apiClient/tabs.svelte.ts`: tabs, open-tab persistence and per-tab response slots (`:382-650`).
   - `apiClient/cookies.svelte.ts` (`:951-1000`, including `secureAll`), `apiClient/curl.ts` (`:1655-1758`).
   - `apiClient/execute.svelte.ts`: runtime vars, `execute`, `executeGrpc`, `graphqlIntrospect`, replay
     (`:1409-1654`); `apiClient/drafts.svelte.ts`: draft helpers and history-into-draft (`:1814-2055`).
   - Each part takes a narrow host (`Pick<ApiClientStore, …>`), like the database store parts. The class keeps
     every public method and field name (delegating), so importers outside your paths (`lazyModule.test.ts`,
     `shell-pages.test.ts`, `eventsPeek.test.ts`, P8's files) compile unchanged.
   - **Size targets:** `apiClient.svelte.ts` < 800 lines (it then leaves `file-loc`), every part < 800.
   - **Tests:** `ui/unit/w3p6-apiClientSplit.test.ts` drives the real store with a fake `api`: closing the
     active tab picks the neighbour and keeps its response slot; a Postman sync failure leaves collections
     unchanged and returns false; `toCurl(importCurl(x))` round-trips headers, body and method; clearing cookies
     empties the list only after the daemon call succeeds. Existing `apiClient*` tests stay green.
3. **Sweep** `modules/api/**`, `modules/workflows/**`, `modules/workbench/**` for rule-4 patterns and P8's
   widened hits.

---

## P7: Swarm paste path, last Claude layout copies, ingest tests, product UI

**Owns:** `crates/otto-swarm/**`, `crates/otto-server/tests/swarm_router.rs`,
`crates/otto-sessions/src/{nested,lib}.rs`, `crates/otto-sessions/src/manager.rs` (only the
`claude_transcript_in_window` call at `:3772`), `crates/otto-context/**`,
`crates/otto-server/tests/{public_entry_gates,it}.rs`, a new `crates/otto-server/tests/swarm_ingest_bodies.rs`,
`crates/otto-server/src/routes/{swarm_ingest,activity}.rs` (test seams only; report any), `ui/src/modules/product/**`,
`ui/unit/w3p7-*.test.ts`, `ui/e2e/*product*.spec.ts`, `docs/features/{agent-swarm,discovery-chat}.md`.

**Findings:** architecture-4-01r, architecture-6w2-01, test-quality-6w2-01, accessibility-responsive-6w2-01
(product part), accessibility-responsive-6w2-02.

1. **arch-4-01r.** Port the `swarm_router` interleave test (`swarm_router.rs:27`, `:461`) to the live path:
   `engine::submit_turn_prompt` (or `SessionManager::submit_prompt_while` with `SubmitOpts` that skip the TUI wait
   for the `cat` PTY). It must still prove the interleave rule, and it must fail if `submit_prompt_while`'s
   submit lock is removed (show it). Then delete `submit_run_prompt` and `send_run_input`, and make
   `guarded_dispatch` private to its remaining caller (`engine.rs:378`). Done means
   `rg -n "submit_run_prompt|send_run_input" crates` is empty.
2. **arch-6w2-01.**
   - `nested::claude_transcript_in_window` takes `&TranscriptRoots` (from `transcript_roots_for(session)` at
     `manager.rs:3772`) and scans each Claude root's `encode_cwd(cwd)` directory; delete its hand-built
     `.claude/projects`. Test: under an injected root (through `RootsEnv`, not the process env) a nested agent's
     transcript is found in that root and not under `$HOME`.
   - `otto-context/src/materialize.rs`: use `otto_transcript::roots::{encode_cwd, default_claude_root,
     home_dir}`; delete `enc_project` and the hand-built path in `read_memory`. Add the `otto-transcript`
     dependency. Keep the existing encoder assertions (`:1151-1152`) as tests of the shared encoder's output.
   - Drop the leftover `lifecycle::transcript_path` re-export from `otto-sessions/src/lib.rs` if nothing outside
     the crate uses it (otherwise report the callers).
   - Done means `rg -n '"\.claude"|join\("projects"\)' crates --glob '!crates/otto-transcript/**' --glob '!**/tests/**'`
     finds no Claude layout outside `otto_transcript::roots`.
3. **TQ-6w2-01.** In `public_entry_gates.rs::ingest_routes_write_only_with_the_sessions_own_token`, add
   `("codex", json!({"type": "agent-turn-complete"}))` to `routes`, and assert the trail grows by exactly one after
   the own-token `claude` post and again after the own-token `codex` post. Show it fails with `activity.rs:421`
   removed.
4. **Riskiest untested paths (test-quality "not covered").** New suite `tests/swarm_ingest_bodies.rs` (add the
   `#[path]` line in `it.rs`) for the `swarm_ingest` product, mockup and discovery-report handler bodies beyond
   the token gate: a valid post writes exactly the expected rows; an oversized or malformed body writes nothing
   and returns the current status; a post naming another workspace's story writes nothing. Pin today's
   behaviour; if a test exposes a bug, report it rather than change behaviour in this wave.
5. **Product UI.**
   - **a11y-6w2-01:** `PublishDialog.svelte:472-474` (polite `announceShown` with the visible text),
     `DiscoveryChat.svelte:394-396` and `RefineChat.svelte:342-344` (the `{:else if backgroundTurn || stopNotice}`
     bubble: polite `announceShown`, no role).
   - **a11y-6w2-02:** drop `role="status"` from the thinking bubbles (`DiscoveryChat.svelte:363-365`,
     `RefineChat.svelte:309-311`); `LiveWorkingDot` labels the state, `announceShown` speaks the notice.
   - **Focus after "Stop waiting" / "Reload chat"** (`DiscoveryChat.svelte:172-182`, `RefineChat.svelte:152-162`,
     observed by the a11y reviewer): move focus to "Reload chat" after the stop swap, and to the composer after
     the reload.
   - Test `ui/unit/w3p7-chatStop.test.ts`: the stop notice is announced exactly once (count `announce` calls),
     and focus lands on the named successor. Sweep `modules/product/**` for P8's widened hits and rule-4
     patterns; update product e2e specs that assert the removed roles.

Order: 3, 1, 2, 5, 4.

---

## P8: UI guards, design hall and the remaining modules, shared components and shell

**Owns:** `ui/scripts/ui-guards.mjs`, `ui/scripts/ui-guards-baseline.json`, `ui/unit/p04-uiGuards.test.ts`,
`ui/unit/w3p8-*.test.ts`; `ui/src/modules/{design-hall,assistant,vault,agents,aws,browser,insights,help,settings,share}/**`;
`ui/src/lib/components/**`, `ui/src/lib/announce.svelte.ts`, `ui/src/shell/**`; the e2e specs named for those modules
(`ui/e2e/*design*.spec.ts` and so on) that no lower-numbered package's glob matches; `docs/design/guidelines/**`.

**Findings:** accessibility-responsive-6w2-01 (design hall, assistant, vault, agents, aws, browser, insights,
help, settings, share, shared components; and the guard).

1. **Step 0: widen the guard.** In `detectInsertedLiveRegion` (`ui-guards.mjs:979-994`), also match `{:else if`
   and `{:else}`, and look up to two lines past the opener, skipping `<!-- … -->` and `{@const …}` lines. Keep the
   `role="alert" … tone=` exemption. Add `--only <rule>` to `--update-baseline` and record
   `inserted-live-region` alone. Unit cases in `p04-uiGuards.test.ts`: `{:else if}` + region on the next line,
   `{#if}` + comment + region, `{#if}` + `{@const}` + region (all hit); an always-mounted region (no hit). A scan
   of `4f6d6ab30` with this widening finds about 30 `role="status"`/`aria-live` hits: design-hall 9, product 5
   (P7), rooms 3 (P5), lib/components 2, aws 2, and one each in assistant, insights, settings, agents, browser,
   git (P1), help, vault and share.
2. **Further cheap guard rules** (ratchets, recorded with `--only` for the new rule names):
   - `announce-in-region`: `use:announceShown` inside an element opened with `role="status"`/`aria-live` within
     the previous 15 lines and not yet closed (the 6w2-02 double announcement).
   - `guessed-derived`: `$derived(… ?? true)` / `?? false` on a value that renders as a state word (extends
     `guessed-checked`; the 6w2-01 `?? true`).
   - `self-removing-focus` (if cheap): a `<button>` whose `onclick` sets the flag that guards its own `{#if}`
     without a `.focus()` / `tick()` in the handler. Skip it if it cannot be done without false positives.
   - Each rule gets hit and no-hit unit cases, and a line in `docs/design/guidelines/review-checklist.md`.
3. **a11y-6w2-01 sites:**
   - Failures (assertive `announceShown`, no role): `design-hall/VersionStrip.svelte:87-89` (`Couldn’t refresh
     the versions. ${error}`), `design-hall/LinksPanel.svelte:186-189`.
   - Notices (polite `announceShown`, no role): `assistant/cards/SystemLine.svelte:37-38`,
     `design-hall/brand/BrandEditor.svelte:557-558`, `design-hall/studio3d/Studio3D.svelte:630-631`,
     `design-hall/site/Inspector.svelte:489-490`.
   - Loading lines (drop the role, announce at the edge, or `LoadState`/`Skeleton`):
     `design-hall/ArtifactStage.svelte:340-341`, `design-hall/ReferencesPanel.svelte:175-176`,
     `vault/LiveContext.svelte:297-298`, `design-hall/assist/VariantsTray.svelte:48-49`,
     `agents/conversation/ToolStep.svelte:299-300`, `aws/AwsPage.svelte:203-205`,
     `browser/live/LiveEngineSetup.svelte:81-82`, `insights/CapabilitiesPage.svelte:86-88`.
   - Counter: `help/Walkthroughs.svelte:270-271` (one always-mounted `sr-only` count region; the visible count
     plain).
   - The widened hits in `lib/components/{Terminal.svelte:3024,LoadState.svelte:102}`, `settings/TrustSafety.svelte:323`,
     `share/SharePage.svelte:313`, `aws/AccountWizard.svelte:364`: fix, or keep with the reason if the region is
     always mounted (LoadState's loading region exists before its text by design).
4. **Sweep** every owned directory for rule-4 patterns, and run `ui-guards` to confirm `inserted-live-region`
   is zero in your paths. Test `ui/unit/w3p8-announce.test.ts`: VersionStrip's refresh failure announces once,
   assertively, with the error text; Walkthroughs announces the first count after typing.

Order: 1 (immediately; others depend on it), 3, 2, 4.

---

## Deferred

None. Every finding in the digest is planned. The one product question nearby (whether the MCP control plane
should also gate what stdio sessions see) is not part of any finding: P3 keeps visibility exactly as today.
