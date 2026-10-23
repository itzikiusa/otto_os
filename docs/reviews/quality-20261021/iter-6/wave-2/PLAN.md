# Quality iteration 6, wave 2: fix plan

Worktree: `/Users/itziklavon/claude_ade-iter6` (branch `fix/quality-iter6-20261023`, at `37738d908`).
Inputs: the four re-score reports in `../wave-1/` (`architecture.md` 7.78, `test-quality.md` 9.4,
`accessibility-responsive.md` 9.6, `ux-design.md` 9.81) and `../wave-1/FIXES.md` (what wave 1 left open).
Output: 8 work packages (P1 to P8) for 8 parallel fixers in one shared worktree.

**Goal of the round.** Architecture, test-quality, accessibility-responsive and ux-design all end at 9.0 or
higher, with margin. Architecture is the hard one: 7.78 now, and the four majors alone are worth 1.2. So every
architecture finding is planned to be **fully** closed, not narrowed. A "partly fixed" verdict on a major costs
0.3 again. Every finding in the digest is planned. Nothing is deferred.

Status check before planning:

- `ClassroomsBox.svelte` is 781 lines and `ShooterGame.svelte` is 792. Both were split under 800 in wave 1. The
  owners (P5 for home, P3 for rooms) must keep them under 800. The `games-splits` and `kart-stuck-fix` agents may
  still be editing those directories (see rule 9).
- `run_workflow_with_budget` is already split (93 lines; the architecture report marks 1-01r fixed). No package
  touches it.

## Rules every package follows

1. **Owned paths are exclusive.** Edit only the paths your package owns. If a fix needs a file you do not own,
   stop. Put a hand-off in your final report (path and exact change). Do not edit the file.
   - Unowned files are read-only: `scripts/loc-baseline.json`, `crates/otto-server/src/routes/mod.rs`,
     `crates/otto-server/src/policy.rs`, `docs/contracts/**`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/types/**`,
     `crates/otto-core/src/api/**`, `crates/otto-state/migrations/**`, `ui/src/lib/uiCommands.ts`,
     `ui/src/lib/uiCommands/index.ts`, and any file no package lists.
   - **No new routes, no wire changes, no migrations** this wave. Every package keeps today's request and response
     shapes byte-for-byte. If a fix seems to need one, stop and hand it off.
2. **Splits and moves preserve behaviour.**
   - Do not change behaviour in the same edit as a move.
   - **Rust:** keep the old path compiling with `pub use` so other packages build.
   - **UI:** update every importer inside your own paths. Do **not** add a module → lib re-export shim
     (`export * from '../../lib/…'`): `scripts/check.sh` caps those at 3. A same-module façade (the old store class
     delegating to its new sibling files) is fine.
   - New functions stay under the `function_max: 400` cap. No new or moved file reaches 800 lines (UI
     `file-loc`) or the Rust file cap. The size target for each split is stated in its package; meet it.
   - Ratchet baselines are lowered only by the integrator, after all packages land
     (`python3 scripts/loc-ratchet.py --update-baseline`, `node scripts/ui-guards.mjs --update-baseline`). P8 is the
     one exception: it adds the entries for the guard rules it creates. If a move renames a baseline key, report the
     exact old and new entry.
3. **Behaviour changes ship with a test that fails without the change.** Name the test, and say how you checked
   that it fails (revert the line, run it, restore it). This is the test-quality lens's main check this round.
   - **Rust:** put tests in suite files or `tests/unit/*.rs` files you own, or in `#[cfg(test)]` modules in your own
     sources. Only P4 may edit `crates/otto-server/tests/it.rs`. Prefer an existing suite.
   - **UI:** put new unit tests in new files `ui/unit/w2pN-<topic>.test.ts` (N is your package number). Drive
     behaviour through `componentFunctions` and fakes. Do not regex-match component source to stand in for a race,
     a debounce, a reorder or focus logic (test-quality-6w1-06). Source regexes are fine only for structural rules.
4. **UI rules** (`docs/design/guidelines/`). Each UI package fixes every target-lens finding in its directories
   **and sweeps those directories** for more of the same:
   - **Live regions:** no status or alert inserted together with its text. Call `announce()` (from
     `lib/announce.svelte.ts`, assertive for failures and connection loss) at the state edge, keep the visible line
     plain, mark label-only spinners `aria-hidden="true"` (or `role="img"` with a name), and drop the role from
     static notes. Run `node scripts/ui-guards.mjs` and work the `inserted-live-region` hits in your paths down. A
     hit that is genuinely fine (for example, a region that stays mounted and only its text changes) stays. List it
     in your report with the reason.
   - **Controls:** icon-only buttons are real `<button>`s with `aria-label` and `title`. Dialogs and menus move focus
     in, trap it, close on Escape and return focus. Use logical CSS properties. Nothing scrolls horizontally at
     390 px.
   - **States:** use `PageHeader`/`PageBody`. Loading, empty, error-with-Retry and loaded each render. Use
     `LoadState` for load errors and `EmptyState` for empty. Use tokens only. Use `confirmer`/`toasts`/`ctxMenu`.
     Write "Couldn’t …" (never "could not" / "Failed to …"). Clipboard failures say what to do next (use
     `copyWithToast` from P8's step 0).
   - **e2e specs:** if you remove a `role` that a spec asserts (`getByRole('status'|'alert')`), update the spec when
     its file name is your module's (`ui/e2e/*<module>*.spec.ts`). Otherwise report the spec as a hand-off. Do not
     run Playwright.
5. **Cargo.** No package edits the root `Cargo.toml` except P3 (one workspace member and one workspace dependency
   line for `otto-rooms`). Only P3 edits `crates/otto-server/Cargo.toml`. A package may edit the `Cargo.toml` of a
   crate it owns. Never edit `Cargo.lock` by hand, and never run `cargo update`. A dependency missing from
   `[workspace.dependencies]` is a hand-off to P3.
6. **Commands and load.** This Mac froze under about 10 parallel builds.
   - Run **at most one heavy command at a time** (cargo build/test/clippy, `npm run check`, a full vitest run),
     through the coordinator's lock if one is provided. Budget: about 4 cargo commands and 2 `npm run check` runs per
     fixer.
   - Iterate with `cargo check -p <crate>`. Then run `cargo test -p <crate> --lib <filter>` /
     `cargo test -p otto-server --test it <suite>::` and `cargo clippy -p <crate> --all-targets -- -D warnings` on your
     crates. For the UI, run `cd ui && npx vitest run unit/<your files>` and one `npm run check` at the end.
   - Never run `cargo clean`, the full workspace suite, e2e, or `scripts/check.sh --all`. The integrator runs the
     gates.
7. **Docs.** Update the `docs/features/*.md` page your package lists when a moved path or a behaviour it cites
   changes. Leave `docs/contracts/**` alone (rule 1).
8. **Comments and style.** Match the file you edit. A moved block keeps its comments. A new module gets a short
   header saying what it owns and who calls it.
9. **Git and other agents.**
   - Make no commits, stashes, resets or checkouts. The integrator commits.
   - Other agents are active in this session: `games-splits`, `kart-stuck-fix`, `mcp-db-execute`,
     `mcp-jira-create`, `mcp-profiles`, `e2e-regressions` and `round-integrator`.
   - Before editing a file, run `git status --short <file>` and `git diff <file>`. If it has uncommitted changes
     you did not make, report it and leave it alone.
   - The `mcp-*` agents work in other worktrees on the same MCP catalogs. P4: keep the `ottod/src/mcp_tools.rs` diff
     focused, so their later merge stays tractable.
10. **Final report.** For each finding id, say fixed, partial or skipped, with the reason. List the tests you added
    (and how each was shown to fail without the change), the line counts before and after for every split, the
    commands you ran and their results, and any hand-offs.

## Cross-package contracts (step 0)

These are small, and other packages build on them. Each owner does its step 0 first and keeps the exact names
below. If a dependency is not there when you reach the dependent item, do the rest of your package first and come
back. Do not create the other package's file yourself.

| Owner | Step 0 | Used by |
|-------|--------|---------|
| P3 | In `crates/otto-transcript/src/roots.rs`: `pub fn encode_cwd(cwd: &str) -> String` (the one Claude project-dir encoder; it must give byte-identical output to `otto_sessions::lifecycle::claude_project_dir_name` today), and `impl TranscriptRoots { pub fn claude_session_file(&self, cwd: &str, psid: &str) -> Option<PathBuf> }` (the first existing `<root>/projects/<encode_cwd(cwd)>/<psid>.jsonl`, else the first Claude root's expected path). | P2 (`session_turn::transcript_path`) |
| P3 | In `crates/otto-sessions/src/submit.rs`: an async-capable `RunGuard` (see P3 item 2). Keep `submit_prompt_while`'s name. | none outside P3 (P2 must not call the old sync form) |
| P8 | In `ui/src/lib/clipboard.ts`: `export async function copyWithToast(text: string, what: string): Promise<boolean>`. Move the design-hall version: the success toast is `Copied {what}` (body = text when ≤ 80 chars), and the failure toast is `Couldn’t copy` / `The clipboard isn’t available here. Select the text and copy it manually.`. Return whether it copied. | P5 (design-hall), P6 (database `results-format.ts` may delegate), P7, P3, P4, P8 sites |
| P7 | `ui/src/lib/rowFocus.ts`: a verbatim move of `modules/api/rowFocus.ts`, with all five importers updated in the same step (P7 owns all five). | none |
| P5 | `ui/src/lib/listWindow.svelte.ts`: an optional "extra height after index i" input to `listRange` (additive; every existing call is unchanged). | P5 itself (k8s `tableWindow.ts`), P6 (unchanged `SchemaTree` use) |

## Package overview

| Pkg | Theme | Findings |
|-----|-------|----------|
| P1 | Fan-out runner adoption, REST engine move, review-gate tests (Rust) | arch-1-04 (major), arch-1-07r, TQ-6w1-04 |
| P2 | One assist turn engine, resume authority in the turn runner (Rust) | arch-4-02 (major), arch-5-02r, arch-6w1-01 (session_turn part) |
| P3 | Swarm submit guard, transcript paths, `otto-rooms` crate, rooms UI | arch-4-01r, arch-6w1-01, TQ-6w1-05, arch-2-06, ux-6w1-01, a11y-6w1-01 (rooms) |
| P4 | MCP governance and catalog unification, public entry gate tests, MCP UI | arch-5-03, arch-1-05, TQ-6w1-01, TQ-6w1-02, TQ-6w1-03, ux-6w1-04, a11y-6w1-01 (mcp) |
| P5 | Git graph split, list windows, home, assistant, design hall, 3D (UI) | arch-1-09 (major), arch-6-05r, arch-1-13 (assistant, home), TQ-6w1-06 (p06, p10), ux-6w1-02 (design-hall), ux-6w1-03 (Scene3D), a11y-6w1-01 (these dirs) |
| P6 | Database store split, brokers, connections, AWS, k8s, skills, insights, proof, browser, workbench (UI) | arch-1-10 (major), ux-6w1-03 (AWS, browser), a11y-6w1-01 (these dirs) |
| P7 | API client store, Workflows run monitor, Product, swarm, loops, tasks, canvas, snip (UI) | arch-6-03, arch-1-08r, arch-5-04, arch-6w1-02, arch-1-13 (swarm), a11y-6w1-02, TQ-6w1-06 (p09), ux-6w1-03 (snip), a11y-6w1-01 (these dirs) |
| P8 | Shared components and shell, LoadState, clipboard, UI guards, agents, vault, usage, settings and small modules (UI) | a11y-6w1-01 (LoadState + these dirs), ux-6w1-02, arch-1-13 (vault), guard rules |

---

## P1: Fan-out runner adoption, REST engine move, review-gate tests

**Owns:**
- `crates/otto-agent-run/src/fanout.rs` and a new `crates/otto-agent-run/src/fanout/**` (declared from `fanout.rs`;
  P2 owns the rest of the crate, including `lib.rs` and `Cargo.toml`)
- `crates/otto-server/src/vault_docs_agent.rs` and a new `crates/otto-server/src/vault_docs_agent/**`,
  `crates/otto-server/tests/unit/vault_docs_agent.rs`
- `crates/otto-server/src/modules/review.rs`, `crates/otto-server/src/modules/review/**`, `crates/otto-review/**`,
  `crates/otto-server/tests/{review_agent_retry,review_comment_states,review_forge_post}.rs`,
  `crates/otto-server/tests/unit/{review_adoption,review_launch_recovery}.rs`
- `crates/otto-product/src/run.rs`, a new `crates/otto-product/src/run/**`, `crates/otto-server/src/product_host.rs`
- `crates/otto-server/src/routes/api_client.rs`, `crates/otto-server/src/routes/api_client/**`,
  `crates/otto-server/src/routes/{api_automation_runs,api_stream,grpc,api_oauth}.rs`,
  `crates/otto-server/src/workflow_api_request.rs`, `crates/otto-server/tests/unit/api_client.rs`, `crates/otto-apiclient/**`
- `crates/otto-server/src/workflow_engine/nodes/review_run.rs`, `crates/otto-server/tests/unit/workflow_engine.rs`
- `docs/features/{code-review,vault,product,api-client,workflows}.md`

**Findings:** architecture-1-04, architecture-1-07r, test-quality-6w1-04.

1. **arch-1-04 (major): every fan-out engine runs on `otto_agent_run::fanout`.** Today only skill review does.
   - **Vault-docs writers + summarizer** (`vault_docs_agent.rs:14`, `:356`, `:1287-1375`, `:1950` `run_docs`,
     `:2389-2469`). Drive the writer pass through `fanout::run` and the summarizer (and its summarizer-only retry)
     through `fanout::summarize`.
     - If the runner lacks something vault docs needs (a user retry of one writer mid-run, which kills and re-runs
       that slot), add it to `fanout.rs` generically. Use `Slot::retry_index` and a retry hook, and add a `fanout`
       test for it.
     - Delete `SUM_RETRY_IDX`, the hand-written writer/summarizer turn loops and `close_retry` bookkeeping that the
       runner replaces. Update the header comment at `:14`.
     - Move what remains of the engine's phases into `vault_docs_agent/` siblings, so the file drops well below its
       4,093 lines.
   - **Code-review summarizer** (`modules/review/summarize.rs::summarize_and_persist`, 467 lines). Make it a thin
     adapter: build the managed summarizer turn and the deterministic fallback, call `fanout::summarize`, and
     persist the result.
     - Port the reviewer retry path in `modules/review/retry.rs` to `Slot`/`retry_index`.
     - No function over 150 lines remains in `summarize.rs`.
   - **Product analysis** (`otto-product/src/run.rs:6`, `:214-320`). Port its lens fan-out and summarizer to
     `fanout::run`/`summarize` through the `ProductRunHost` it already has, and delete its own summarizer retry
     handling.
   - **Done means** `rg -n "SUMMARIZER_RETRY_IDX|SUM_RETRY_IDX|summariz" crates/otto-server/src/vault_docs_agent*
     crates/otto-product/src/run* crates/otto-server/src/modules/review` shows only calls into `fanout` (or
     comments). The `vault_docs_agent.rs:14` "Mirrors the product_run fan-out" comment is gone. Existing suites stay
     green: `review_agent_retry`, `review_launch_recovery`, `tests/unit/vault_docs_agent.rs` and the otto-product run
     tests.
   - **Tests:** a vault-docs test where one writer fails and the summarizer still runs over the others' results; a
     summarizer failure that yields the deterministic fallback (review and product); a user retry of one writer that
     re-runs only that slot.
2. **arch-1-07r: the REST execute engine moves into `otto-apiclient`.**
   - Move `execute_service`, `resolve_environment`, `run_step`, `prepare_stream`, `effective_timeout`,
     `workspace_allows_local`, `substitute`, `history_retention` and the engine helpers they need from
     `routes/api_client.rs` into `otto-apiclient` (for example `src/service.rs`, `src/env.rs`). They go behind an
     `ApiClientCtx` trait for what they need from the server (pool, secrets, net guard, settings, audit).
   - Implement the trait for `ServerCtx` in `routes/api_client/ctx.rs`; do not touch `modules/ctx_impls.rs` (P2).
   - The axum handlers stay in the route file. Keep `pub use` re-exports in `routes/api_client.rs` for every moved
     name, so `workflow_api_request.rs`, `api_automation_runs.rs`, `api_stream.rs`, `grpc.rs`, `api_oauth.rs`,
     `mcp_outward.rs` and `state_archive.rs` compile unchanged. Repoint the callers you own to the crate path.
   - Target: `routes/api_client.rs` under 2,000 lines, with no engine function left in it.
   - The existing `tests/unit/api_client.rs` must pass unchanged in behaviour. Move the tests that exercise the
     moved engine into `otto-apiclient`.
3. **TQ-6w1-04: review-gate scoring.**
   - Extract `fn blended_score(review: i64, goals: Option<i64>) -> i64` in `review_run.rs` (used at `:827-832`).
     Test that `(40, Some(100))` gives 70, `(90, None)` gives 90 and `(0, Some(0))` gives 0.
   - Add a `ScoreWeights::from_params` test. The template's `{"scoring":{"bug":10,"warn":5,"info":1}}` gives those
     weights; `{}` and `{"scoring":{"bug":"x"}}` give the defaults.
   - Put both tests in `tests/unit/workflow_engine.rs` next to the `review_gate` tests.

Order: 3 (small), then 1 (vault docs, then review, then product), then 2.

---

## P2: One assist turn engine and resume authority in the turn runner

**Owns:**
- `crates/otto-agent-run/**` except `src/fanout.rs` and `src/fanout/**` (P1). This includes `lib.rs`,
  `Cargo.toml`, `session_turn.rs`, `session_turn/**`, `turn_oracle.rs`, `live_file.rs` and a new `src/assist/**`.
- `crates/otto-canvas/**`, `crates/otto-design-assist/**`, `crates/otto-design/**`
- `crates/otto-server/src/{mockup_assist,agent_session,design_hall,automation_ctx,assistant_host}.rs`,
  `crates/otto-server/src/modules/ctx_impls.rs`
- `crates/otto-server/tests/{canvas_refs_api,agent_run_recovery}.rs`,
  `crates/otto-server/tests/unit/{assist_resume_ownership,product_studio_stop}.rs`
- `docs/features/{canvas,design-hall}.md`

**Findings:** architecture-4-02, architecture-5-02r, architecture-6w1-01 (session_turn part).

1. **arch-4-02 (major), phase 2: one assist engine.** Today there are three design-document assist engines, each
   with its own turn job and live poll: `otto-canvas/src/assist.rs` (1,310, poll `:740`), `otto-design-assist/src/lib.rs`
   (3,307, `spawn_live_poll` `:1696-1707`) and `otto-server/src/mockup_assist.rs` (775, `:457`).
   - Add `otto_agent_run::assist`, with a trait `AssistTarget`: `load` (current document), `prompt` (build the
     turn prompt), `parse` (reply → proposal), `apply` (persist the proposal), `live_path` (file the agent writes
     while working), plus the busy key.
   - Add one shared `run_assist_job` over it: claim through `InFlightSet`, run `session_turn` (`AgentTurn`), start
     and stop the live poll (reuse `live_file.rs`), honour cancel, parse, apply, and report each state.
   - Make the three engines thin adapters that implement `AssistTarget`, and delete their own job/poll code. Keep
     every public function and route-facing type they expose today (re-export when you move them), so the routes
     and the UI see no change.
   - **Tests** (in `otto-agent-run`, with a fake `AssistTarget` and a fake turn runner): a success applies once and
     stops the live poll; a cancel mid-turn applies nothing and stops the poll; a parse failure applies nothing and
     reports the error; a second claim while busy is refused. Existing suites stay green (`canvas_refs_api`,
     the canvas `parse_assist` tests, the design-assist tests and `unit/product_studio_stop.rs`).
   - **Done means** `rg -n "spawn_live_poll|live_poll" crates/otto-canvas crates/otto-design-assist
     crates/otto-server/src/mockup_assist.rs` finds no own implementation, and `otto-design-assist/src/lib.rs` shrinks
     by its job code.
2. **arch-5-02r: forwarders and `require_owned_resume`.**
   - Give `session_turn` a `ResumeAuthority` hook (a trait implemented over `otto_state::SessionsRepo`, or the repo
     passed in), so `require_owned_resume` (`agent_session.rs:25`) moves into `otto_agent_run::session_turn`.
   - Then let `CanvasAssistCtx` and `DesignAssistCtx` expose `fn sessions(&self) -> &Arc<SessionManager>` (plus the
     authority), and run the turn in the crate instead of forwarding through `run_agent_turn`. Delete the four
     forwarders: `otto-canvas/src/assist_ctx.rs:56`, `otto-design-assist/src/ctx.rs:53`, and
     `modules/ctx_impls.rs:545` and `:641`.
   - Keep every `pub` function in `agent_session.rs` with its current signature. `vault_docs_agent`, `skill_review`,
     `automation_ctx` and `product_host` call them, and P1 edits two of those.
   - Move `tests/unit/assist_resume_ownership.rs`'s cases to the new location, and keep a canvas test showing that
     another user's session is not resumed (403).
3. **arch-6w1-01, session_turn part.** After P3's step 0, `session_turn::transcript_path` (`:1097-1112`) calls
   `TranscriptRoots::claude_session_file` (through `for_session` for an account) and stops calling
   `claude_pty::session_jsonl_path`. Add a test: under `OTTO_TRANSCRIPT_ROOTS` (inject it through `RootsEnv`, not
   the process env), the watcher path is inside that root.

Order: 2, then 1, then 3 (when P3's step 0 lands).

---

## P3: Swarm submit guard, transcript paths, `otto-rooms` crate, rooms UI

**Owns:**
- `crates/otto-transcript/**`
- `crates/otto-sessions/src/{submit,manager,lifecycle}.rs`, `crates/otto-sessions/src/manager/**`
- `crates/otto-orchestrator/src/claude_pty.rs`, `crates/otto-orchestrator/Cargo.toml`
- `crates/otto-improve/src/pathsafe.rs`, `crates/otto-improve/Cargo.toml`
- `crates/otto-swarm/**`, `crates/otto-server/src/swarm_host.rs`, `crates/otto-server/tests/swarm_identity.rs`
- a new `crates/otto-rooms/**`, `crates/otto-server/src/rooms/**`, `crates/otto-server/src/game_rooms/**`,
  `crates/otto-server/tests/rooms_api.rs`
- root `Cargo.toml` (only the `otto-rooms` member and workspace-dependency lines), `crates/otto-server/Cargo.toml`,
  `crates/otto-server/src/lib.rs`, `crates/otto-server/src/state.rs`, `crates/otto-server/src/boot/**` (only the
  rooms wiring)
- `AGENTS.md` (only the crate-table rows)
- `ui/src/modules/rooms/**`, `ui/src/lib/rooms/**`, `ui/unit/p01-room-invites.test.ts`
- `docs/features/{agent-swarm,session-rooms,room-games,agent-sessions,conversation-view}.md`

**Findings:** architecture-4-01r, architecture-6w1-01, test-quality-6w1-05, architecture-2-06, ux-design-6w1-01,
accessibility-responsive-6w1-01 (rooms part).

0. **Step 0** (for P2): `otto_transcript::roots::encode_cwd` and `TranscriptRoots::claude_session_file`, as in the
   contracts table. Test that `encode_cwd` matches the three current encoders on a path with `/`, `.`, `_` and
   spaces.
1. **arch-6w1-01.** Make the per-session path builders call the step-0 helpers and delete their own layout code:
   - `persist_transcript_path` (`manager.rs:471-492`) stops reading `HOME` itself;
   - `lifecycle::transcript_path` / `claude_transcript_path` (`lifecycle.rs:40-50`, `:124-135`) and
     `claude_project_dir_name`;
   - `claude_pty::project_dir` / `session_jsonl_path` (`claude_pty.rs:469-485`) delegate to them;
   - `otto-improve/src/pathsafe.rs:17` uses `encode_cwd`.

   After this, `rg -n '"projects"' crates/otto-sessions crates/otto-orchestrator crates/otto-agent-run` shows no
   hand-built Claude layout. Test: with an injected `OTTO_TRANSCRIPT_ROOTS`, the persisted `transcript_path` is
   inside that root.
2. **arch-4-01r: one run-guarded submit path, and the swarm uses it.**
   - Make `RunGuard` async-capable: hold a `tokio::sync::watch::Receiver<bool>` (or a boxed-future closure).
     `submit_prompt_while` checks it under the submit lock before the paste, the re-paste, Enter and the re-Enter.
   - In the swarm engine, create the watch where `while_run_active` polls `run_is_active` (`engine.rs:49-74`), and
     flip it on stop.
   - Convert `runtime/run.rs:894-938` and `runtime/agent_run.rs:355-401` to
     `submit_prompt_while(.., SubmitOpts::VERIFIED, guard)`. Delete `bracketed_paste` and `wait_for_tui` from the
     `SwarmHost` trait (`host.rs:105-107`) and from `swarm_host.rs`, keeping only `dispatched`. If `SwarmCtx` in
     `modules/ctx_impls.rs` must change, hand that off to P2.
   - Tests: the existing `ended_run_guard_blocks_the_paste`, ported to the async guard. A swarm test in which the
     run stops between the wait and the paste, so nothing is pasted.
3. **TQ-6w1-05.** Extract `TurnSession { reuse_user, creator }` (or `fn session_owner_for_turn(acting_user) -> Id`),
   built once from `acting_user`. Use it at `run.rs:805` (`find_agent_session`) and `:852` (`creator`). Unit-test
   that both fields equal the acting user and not `swarm.created_by`.
4. **Rooms UI** (do this before item 5).
   - **ux-6w1-01.** In `RecapPanel.svelte`, `generate()` sets
     `actionError = \`Couldn’t start the summary. ${loadErrorText(e)}\`` (or adopts RoomRecap's `{ what, detail }`).
     The failed refresh at `:112` renders `<strong>Couldn’t refresh this recap.</strong> <span>{error}</span>` +
     Retry, or `LoadState` with the stale bar. Add a `w2p3-recap.test.ts` on the error text.
   - **a11y-6w1-01, rooms:** `rooms/games/GameMatch.svelte:218` (keep the lobby region mounted, or announce on the
     return to the lobby), `rooms/games/kart/KartGame.svelte:383,437` (only if `kart-stuck-fix` has no uncommitted
     change there; otherwise report it), the `Lobby:460` static note (drop the role), and every other
     `inserted-live-region` hit under `modules/rooms/**`. Also sweep the rooms module for the rule-4 patterns.
5. **arch-2-06: an `otto-rooms` crate.**
   - Create `crates/otto-rooms` and move in the domain: `rooms/{registry,capability,ice,annotations,presentation,
     terminal,auth}`, `rooms/recap/{archive,consent,media,mod}`, `rooms/recap_engines/**`, and
     `game_rooms/{registry,types}`.
   - What they need from the server goes behind a `RoomsCtx` trait (pool, session manager, secrets, base URL, data
     dir, audit). Implement it for `ServerCtx` in `otto-server/src/rooms/ctx.rs`.
   - The axum handlers and sockets (`rooms/{http,socket}.rs`, `rooms/recap/http.rs`, `game_rooms/{http,socket}.rs`)
     stay in `otto-server`, and `otto-server/src/rooms/mod.rs` and `game_rooms/mod.rs` re-export the moved names.
     `lib.rs` (routes, `run_ocr_stdio`), `state.rs` (`Arc<crate::rooms::RoomRegistry>`) and `policy.rs` then compile
     unchanged. `policy.rs` is read-only.
   - Move the unit tests with their code (`rooms/tests.rs`, `game_rooms/tests.rs`). `api_tests.rs` and
     `tests/rooms_api.rs` stay in the server.
   - Add the crate row to `AGENTS.md`. Done means `rg -n "ServerCtx" crates/otto-rooms` is empty, and the crate
     builds and tests on its own (`cargo test -p otto-rooms`).

Order: 0, 1, 3, 2, 4, 5.

---

## P4: MCP governance and catalog unification, public entry gate tests, MCP UI

**Owns:**
- `crates/otto-mcp/**`, `crates/otto-server/src/{mcp_outward,mcp_http,mcp_auto_approve,mcp_capabilities,self_call}.rs`,
  `crates/otto-state/src/{mcp_audit,mcp_auto_approve,mcp_control,mcp_profiles,mcp_servers}.rs`
- `crates/ottod/**`
- `crates/otto-server/tests/{it,public_entry_gates,database_changes_api,mcp_auto_approve,mcp_profiles_edges}.rs`
- `crates/otto-server/src/plugins.rs` (tests only), `crates/otto-server/src/routes/{swarm_ingest,activity,usage,database_changes,plugins}.rs`
  (read-only unless a test seam is unavoidable; then report it)
- `ui/src/modules/mcp/**`, `ui/src/lib/stores/mcp-profiles.svelte.ts`, `ui/src/lib/mcpProfiles.ts`
- `docs/features/{mcp-control-plane,mcp-control-plane-design,plugins,resource-access-and-database-changes}.md`

**Findings:** architecture-5-03, architecture-1-05, test-quality-6w1-01, test-quality-6w1-02, test-quality-6w1-03,
ux-design-6w1-04, accessibility-responsive-6w1-01 (mcp part).

1. **Pin the current governance outcomes first** (a new section in `tests/mcp_auto_approve.rs` or an `otto-mcp`
   test module). Write a table-driven test over both entry points, `McpService::invoke` (registered servers) and
   `governed_invoke` (`otto.*` outward tools). For each case, assert the decision, the audit `decision`/`reason` and
   whether the executor ran:
   - policy-as-code deny / require-dry-run / allow;
   - auto-approve rule match / no match;
   - dangerous with approval required or not;
   - dry-run request;
   - an audit-insert failure.

   This must reflect today's behaviour exactly, including what does **not** apply: policy-as-code rules do not
   apply to `otto.*` tools, and auto-approve rules do not apply to registered servers.
2. **arch-5-03: one stage runner.** Add `otto_mcp::governance`, a stage runner for approval → dry-run → audit (and
   the policy and auto-approve stages as optional members of a `StageSet`).
   - `McpService::invoke` (`service.rs:698-1070`) and `governed_invoke` (`mcp_outward.rs:207-470`) both call it with
     their own stage sets and executors. Delete the duplicated stage order in both.
   - Behaviour stays exactly as today; the decision is made explicit in code. Each entry point's `StageSet` is a
     named constant with a doc comment saying which stages apply and why. For example, `otto.*`: auto-approve yes,
     policy-as-code no, today's behaviour, deliberately unchanged.
   - Item 1's table passes unchanged before and after. Update the `mcp-control-plane.md:45` paragraph to point at
     the runner.
3. **arch-1-05: one catalog.**
   - Extend `ROUTED_TWINS` (`ottod/src/mcp_tools.rs:2538-2560`) to every near-twin (`otto_get_pr`, `:2652`, and the
     others) whose outward route can serve the session result. Where output differs, adapt with a projection, so
     the session-visible output stays identical; pin that with a test per tool.
   - Generate the session catalog entries for every tool present in both catalogs from the control-plane spec:
     name, description and input schema. The control-plane schema wins; this was decided on 2026-10-22. Session-only
     tools stay hand-written.
   - Add a parity test: every tool in both catalogs has the identical schema. Mind the tools/list byte-budget gate.
     Trim text rather than raise the budget. Raise both budgets only for a genuinely new tool, and say so in the
     report.
   - Report `route_for` / `run_tool` / `base_tool_catalog` line counts before and after.
4. **TQ-6w1-02 and TQ-6w1-01: public entry gates** (`public_entry_gates.rs`).
   - **Real swarm-session fixture.** Create a swarm agent session with `SessionsRepo::create` and meta
     `{swarm_id, agent_id, workspace_id}` (as `a_turn_reuses_only_its_acting_users_session` does), plus a second
     live session. Mint both ingest tokens through the manager's ingest env.
   - **Board** (replace `:306-360`). No token, an empty token and the other session's token write no `list_board`
     row. The session's own token writes exactly one row. The positive case is what makes the negatives count.
   - **Other ingests:** `/ingest/swarm/{product,mockup,discovery-report}`, `/ingest/usage` (no usage row) and
     `/ingest/claude` (no activity event) with the other session's token write nothing. Where a positive case is
     cheap, add one.
   - **Plugin host.** Loop over all four host routes (`host_repos`, `host_jira_accounts`, `host_jira_credentials`
     and `host_agents_run`). No `Authorization` and a disabled plugin's token give 401. `/agents/run` with
     `{"prompt":"x"}` gives 401 and creates no session.
   - Check each new case by deleting the guarded `verify_ingest_token` / `auth_plugin` call locally: the case must
     fail. Then restore the call.
5. **TQ-6w1-03: database change authority** (`database_changes_api.rs`). Extend
   `each_review_step_is_refused_to_a_user_without_its_authority`:
   - a peer's cancel on an `awaiting_review` change gives 403, and the status is unchanged;
   - after `DatabaseChangesRepo::claim` puts the change in `running`, a cancel by the peer and by the reviewer
     (who lacks `change_execute`) gives 403;
   - with an `unknown` attempt row inserted, a peer's reconcile gives 403, and the outcome is unchanged.

   No case reaches a driver.
6. **MCP UI.**
   - **ux-6w1-04:** in `OttoServerHome.svelte:343`, while `attach === null && !attachError`, render
     `<LoadState what="the session-attach setting" variant="compact" loading empty rows={1} />` instead of the
     guessed ticked box. Test it in `w2p4-ottoServerHome.test.ts`.
   - **a11y-6w1-01, mcp:** fix the `inserted-live-region` hits under `modules/mcp/**`, and sweep the module for the
     rule-4 patterns.

Order: 4, 5 (small and independent), 1, 2, 6, 3.

---

## P5: Git graph split, list windows, home, assistant, design hall, 3D (UI)

**Owns:**
- `ui/src/modules/git/**`, `ui/src/lib/listWindow.svelte.ts`, `ui/src/lib/tableWindow.svelte.ts`,
  `ui/src/lib/api/git.ts`, `ui/src/lib/stores/git.svelte.ts`, `ui/src/lib/uiCommands/git.ts`
- `ui/src/modules/kubernetes/monitor/tableWindow.ts` (only this file in the kubernetes module)
- `ui/src/modules/panels/**`, `ui/src/modules/mission-control/**`, `ui/src/modules/personal-agents/**`,
  `ui/src/lib/stores/personalAgents.svelte.ts`
- `ui/src/modules/assistant/**`, `ui/src/lib/stores/assistant.svelte.ts`, a new `ui/src/lib/assistant/**`
- `ui/src/modules/home/**`, `ui/src/lib/uiCommands/home.ts`, a new `ui/src/lib/uiCommands/homePort.ts`, a new
  `ui/src/lib/home/**`
- `ui/src/modules/design-hall/**`, `ui/src/lib/design/**`, `ui/src/lib/three/**`
- `ui/unit/p06-*.test.ts`, `ui/unit/p10-*.test.ts`, `ui/unit/w2p5-*.test.ts`
- `docs/features/{git,mission-control,personal-agents,home}.md` (P2 owns `design-hall.md`; report changes there as a hand-off)

**Findings:** architecture-1-09, architecture-6-05r, architecture-1-13 (assistant and home parts),
test-quality-6w1-06 (p06 and p10 parts), ux-design-6w1-02 (design-hall part), ux-design-6w1-03 (Scene3DViewport),
accessibility-responsive-6w1-01 (git, panels, mission-control, personal-agents, assistant, home, design-hall,
lib/design).

1. **arch-1-09 (major): split `GraphView.svelte`** (4,879 lines).
   - Extract `git/graph/RefSidebar.svelte` (branches, tags, remotes and stashes list, its filter and its ref menus)
     and `git/graph/graphActions.svelte.ts` (the checkout, branch create/rename/delete, tag, stash, push, pull and
     merge handlers over `gitApi` / the git store, with their confirmations and toasts).
   - Then extract further self-contained sections until GraphView is **≤ 3,000 lines**: the ref popover, the commit
     detail pane and the toolbar are candidates.
   - Each new file is under 800 lines. Behaviour stays identical, including focus, keyboard and ⌘F.
   - Add `w2p5-graphActions.test.ts`, which drives the actions with a fake git client (checkout success refreshes
     refs; a failed push shows the error toast and keeps the state).
2. **arch-6-05r.** Delete `modules/git/list-range.ts` and `modules/git/list-window.svelte.ts` (no importers). Add an
   "extra height after index i" option to `listRange` (step 0) and re-express
   `kubernetes/monitor/tableWindow.ts` over it, keeping its rendered rows identical. Test that the old and new range
   agree, including the expanded-row case that was off by one.
3. **arch-1-13, assistant and home.**
   - Move `modules/assistant/model.ts` (pure) to `lib/assistant/model.ts`, and update every importer (no shim).
   - Move `modules/home/kinds.ts` (pure) to `lib/home/kinds.ts`.
   - For the home store, add `lib/uiCommands/homePort.ts`, a port the home module registers when its store loads
     (`registerHomePort(home)`). `lib/uiCommands/home.ts` navigates to Home (as it does now), waits for the port and
     drives it.
   - Afterwards, `rg "modules/" ui/src/lib/stores/assistant.svelte.ts ui/src/lib/uiCommands/home.ts` is empty.
     Test the port wait in `w2p5-homePort.test.ts`.
4. **TQ-6w1-06, p06 and p10.** Replace the source regexes in `p06-loadStatesAndTabs.test.ts:27-33` (⌥⇧←/→ tab
   move) and `p10-focus-a11y.test.ts:11-32` (Inspector `go()`/`upTo()` focus, link-row removal focus, breadcrumb
   unmount focus) with behaviour tests: `componentFunctions` with stub elements, asserting which element is focused
   and the resulting tab order. Keep only the structural assertions.
5. **ux-6w1-02 (design hall).** `design-hall/brand/edit.ts` imports `copyWithToast` from `lib/clipboard` (P8 step
   0) and deletes its own copy.
6. **ux-6w1-03.** `Scene3DViewport.svelte:1037` reads "Couldn’t load {n} models".
7. **a11y-6w1-01 in your directories:**
   - `git/ReviewPanel.svelte:1183`, `git/GitAccounts.svelte:394`;
   - `design-hall/ArtifactStage.svelte:268`, `ArtifactView.svelte:779`, `brand/BrandEditor.svelte:540`,
     `site/PublishModal.svelte:140`, `assist/OttoPanel.svelte:593`, `studio3d/Studio3D.svelte:628`, the
     `DesignStorage:104` static note;
   - `lib/design/scene3d/Scene3DViewport.svelte:997` (label-only spinner);
   - `MissionControlPage:399` (static note);
   - every other `inserted-live-region` hit in your paths.

   Also sweep the directories for the rule-4 patterns. Keep `ClassroomsBox.svelte` under 800 lines.

Order: 0 (listWindow option), 3, 4, 5, 6, 1, 2, 7.

---

## P6: Database store split, brokers, connections, AWS, k8s, skills, insights, proof, browser, workbench (UI)

**Owns:**
- `ui/src/lib/stores/database.svelte.ts`, `ui/src/lib/stores/database/**`, `ui/src/lib/database/**`,
  `ui/src/modules/database/**` **except** `AggregateBuilder.svelte` (P7)
- `ui/src/modules/brokers/**`, `ui/src/lib/stores/brokers.svelte.ts`
- `ui/src/modules/connections/**`, `ui/src/lib/stores/sftp.svelte.ts`
- `ui/src/modules/aws/**`, `ui/src/lib/stores/aws.svelte.ts`
- `ui/src/modules/kubernetes/**` **except** `monitor/tableWindow.ts` (P5), `ui/src/lib/stores/k8s.svelte.ts`,
  `ui/src/lib/k8s/**`
- `ui/src/modules/{skills-eval,skills-lab,insights,proof}/**`, `ui/src/lib/stores/proof.svelte.ts`
- `ui/src/modules/{browser,workbench}/**`, `ui/src/lib/stores/{browser,browserLive}.svelte.ts`
- `ui/unit/p07-*.test.ts`, `ui/unit/p04-k8sMonitor.test.ts`, `ui/unit/p08-{insightsScheduleCopy,proofAssembleOutcome}.test.ts`,
  `ui/unit/w2p6-*.test.ts`
- `docs/features/{database-explorer,message-brokers,connections-ssh-sftp,aws-console,kubernetes-console,
  kubernetes-monitoring,skills-evaluator,insights,proof-packs,browser,workbench}.md`

**Findings:** architecture-1-10, ux-design-6w1-03 (AWS RegionErrors, browser store),
accessibility-responsive-6w1-01 (these directories).

1. **arch-1-10 (major): split `DatabaseStore`** (3,210 lines). Follow the `filters.svelte.ts` pattern:
   - `lib/stores/database/connections.svelte.ts`: connections, warm-up and keep-alive. `connEpoch` becomes a shared
     token object passed to the other parts.
   - `lib/stores/database/schemaTree.svelte.ts`: tree load, expand and refresh.
   - `lib/stores/database/objectOps.svelte.ts`: create, rename, drop and truncate object operations.
   - `lib/stores/database/queryRuns.svelte.ts`: run, cancel, results and history.

   `DatabaseStore` stays a façade with the same public API (delegating getters/methods), so no page changes, and
   it drops to **≤ 1,200 lines**. Each new file is under 800 lines. Add `w2p6-dbStoreSplit.test.ts`: a stale
   `connEpoch` response is dropped after a reconnect, and a schema-tree refresh after a drop removes the node.
2. **ux-6w1-03.** `aws/RegionErrors.svelte:18` reads "{n} regions couldn’t be listed — …";
   `lib/stores/browser.svelte.ts:368` reads "Couldn’t load the page.".
3. **Clipboard.** Make `database/results-format.ts` `copyWithToast` delegate to P8's `lib/clipboard` version
   (keep its signature for the database callers), or report why not.
4. **a11y-6w1-01 in your directories:**
   - `kubernetes/ResourceTable.svelte:153`, `brokers/OverviewTab.svelte:96`, `aws/AthenaView.svelte:536`,
     `aws/S3Browser.svelte:664,687,763,771`;
   - `database/SchemaTree.svelte:561,579`, `ImportDialog.svelte:277`, `ExportDialog.svelte:288`;
   - `connections/ConnectionImportDialog.svelte:220,279`;
   - `skills-lab/SkillsBrowser.svelte:401`, `SkillReviewPanel.svelte:430`;
   - `browser/live/RemoteLiveView.svelte:929,936,962` (assertive for reconnecting/ended);
   - `workbench/WorkbenchPage.svelte:658`;
   - static notes: `SftpBrowser:363`, `CsvPreview:30`, `TopicDetail:803,851`, `RetentionDialog:56`,
     `LearnedPage:458` and `Scorecard:158` (drop the role);
   - every other `inserted-live-region` hit in your paths.

   Also sweep the directories for the rule-4 patterns.

Order: 2, 3, 4, then 1. Do the split last, so the sweep's edits land in the moved code in one pass. Alternatively,
do the split first and sweep afterwards, but do not interleave the two.

---

## P7: API client store, Workflows run monitor, Product, swarm, loops, tasks, canvas, snip (UI)

**Owns:**
- `ui/src/modules/api/**`, `ui/src/lib/stores/apiClient.svelte.ts`, `ui/src/lib/stores/apiClient/**`,
  `ui/src/lib/stores/{apiHistory.ts,apiStream.svelte.ts}`, a new `ui/src/lib/rowFocus.ts`,
  `ui/src/modules/database/AggregateBuilder.svelte` (the import line only)
- `ui/src/modules/workflows/**`, `ui/src/lib/stores/workflowRuns.svelte.ts`, `ui/src/lib/uiCommands/workflows.ts`
- `ui/src/modules/product/**`, `ui/src/lib/stores/{product,mockup-assist}.svelte.ts`
- `ui/src/modules/swarm/**`, `ui/src/lib/stores/swarm.svelte.ts`, a new `ui/src/lib/swarm/**`
- `ui/src/modules/{loops,scheduled-tasks,canvas,snip}/**`, `ui/src/lib/stores/{loops,scheduledTasks,canvas}.svelte.ts`
- `ui/unit/p09-*.test.ts`, `ui/unit/p08-swarmAccess.test.ts`, `ui/unit/w2p7-*.test.ts`
- `docs/features/{goal-loops,scheduled-tasks,snipping-tool}.md`. P1 owns
  `docs/features/{api-client,workflows,product}.md`, P2 owns `canvas.md` and P3 owns `agent-swarm.md`. Report any change you need in those
  as a hand-off.

**Findings:** architecture-6-03, architecture-1-08r, architecture-5-04, architecture-6w1-02, architecture-1-13
(swarm part), accessibility-responsive-6w1-02, test-quality-6w1-06 (p09 part), ux-design-6w1-03 (SnipEditor),
accessibility-responsive-6w1-01 (these directories).

0. **Step 0 / arch-6w1-02.** Move `modules/api/rowFocus.ts` to `lib/rowFocus.ts`, update its five importers
   (EnvSelector, RequestBuilder, AutomationEditor, EnvironmentsView and `database/AggregateBuilder.svelte:15`) and
   delete the old file. Leave no shim.
1. **arch-6-03: finish the `apiClient.svelte.ts` split** (2,113 lines). Follow the `automations.svelte.ts` pattern:
   - `lib/stores/apiClient/collectionsSync.svelte.ts`: Postman import/export and git pull/push (`:1001-1263`).
   - `lib/stores/apiClient/tabs.svelte.ts`: tab persistence and slots (`:218-650`).
   - `lib/stores/apiClient/cookies.svelte.ts`: the cookie jar (`:951`).
   - `lib/stores/apiClient/curl.ts`: curl import/export (`:1655`).

   The class stays a façade with the same API, and drops to **≤ 900 lines**. Add `w2p7-apiTabs.test.ts`: a
   restored tab slot keeps its request after a reload, and closing the active tab selects its neighbour.
2. **arch-1-08r: the run monitor.** Move the live run sync (`refetchRun` `:285`, `waitRunTerminal` `:1003`), the
   final output and the approvals out of `WorkflowsPage.svelte` (3,710) into `workflows/RunMonitor.svelte` (plus a
   `runMonitor.svelte.ts` for the sync logic if the component would exceed 800). The page keeps the canvas and
   editor, and drops to **≤ 2,700 lines**. Add `w2p7-runMonitor.test.ts` on the sync logic: a terminal status stops
   polling, and an approval resolves into the run.
3. **arch-5-04.** Extract `product/JiraIssueSection.svelte` (the issue load, dev status and `IssueEditor` host,
   `OverviewTab.svelte:81-83` and that section) from OverviewTab (2,494). OverviewTab drops to **≤ 2,000**.
4. **a11y-6w1-02: TestCasesTab moves.**
   - Keep the Move buttons enabled during a save. Queue the save: when `persistOrder` is called while `savingOrder`
     is true, set `pendingOrder`; its `finally` calls `persistOrder` again when `pendingOrder` is set.
   - Focus stays on the pressed Move button, and falls back to the checkbox only when the list edge disables it.
   - Gate ⌥↑/⌥↓ the same way, so every move is persisted.
   - Test it behaviourally (item 5).
5. **TQ-6w1-06, p09.** Rewrite `p09-product.test.ts:63-81` with `componentFunctions` and fakes:
   - **Mermaid race:** call `renderFor` twice with a `renderMermaid` fake whose first promise resolves last. Assert
     that the second SVG is kept and that `rendering` ends `false`.
   - **Debounce:** with fake timers, two edits inside `LIVE_DEBOUNCE_MS` make exactly one render call.
   - **Reorder:** `moveCase(i, +1)` with a recording `persistOrder` gives the expected id order. Two moves during an
     in-flight save both persist, with the second save carrying the final order.
   - **Focus:** the pressed Move button keeps focus mid-list.
6. **arch-1-13, swarm.** Move `modules/swarm/runStats.ts` (pure) to `lib/swarm/runStats.ts` and update its importers
   (no shim). Afterwards `lib/stores/swarm.svelte.ts` imports no `modules/`.
7. **ux-6w1-03.** `snip/SnipEditor.svelte:119` reads "The image couldn’t be decoded. Retry loading the snip.".
8. **a11y-6w1-01 in your directories** (`run-with-otto` belongs to P8):
   - `api/ResponseViewer.svelte:375`, `api/HistoryList:180` (static note);
   - `product/MockupViewer.svelte:243`, `DiscoveryChat.svelte:388`, `RefineChat.svelte:336`;
   - the label-only spinners `AnalysisTab.svelte:549`, `HistoryTab.svelte:117` and `OverviewTab.svelte:798`;
   - every other `inserted-live-region` hit in your paths, including `StoryTranscripts.svelte`'s two.

   Also sweep the directories for the rule-4 patterns.

Order: 0, 6, 7, 4, 5, 3, 2, 1, 8.

---

## P8: Shared components and shell, LoadState, clipboard, UI guards, agents, vault, usage, settings, small modules (UI)

**Owns:**
- `ui/src/lib/components/**`, `ui/src/shell/**`, `ui/src/App.svelte`, `ui/src/lib/clipboard.ts`,
  `ui/src/lib/announce.svelte.ts`
- `ui/scripts/ui-guards.mjs`, `ui/scripts/ui-guards-baseline.json` (new rule entries only; rule 2),
  `scripts/check.sh` (only if a guard needs wiring), `ui/unit/p04-uiGuards.test.ts`, `ui/unit/p03-*.test.ts`
- `ui/src/modules/agents/**`, `ui/src/lib/stores/{transcript,sessionUsage}.svelte.ts`
- `ui/src/modules/vault/**`, `ui/src/lib/uiCommands/vault.ts`, a new `ui/src/lib/uiCommands/vaultPort.ts`,
  `ui/unit/p01-vault-link-preview-a11y.test.ts`
- `ui/src/modules/{usage,settings,help,share,desktop,run-with-otto,plugins}/**`,
  `ui/src/lib/stores/{runWithOtto,plugins,issueAccounts}.svelte.ts`
- `ui/unit/w2p8-*.test.ts`
- `docs/design/guidelines/**`, `docs/features/{agent-ui-control,usage-and-cost,help,run-with-otto,session-sharing}.md`

**Findings:** accessibility-responsive-6w1-01 (LoadState and these directories), ux-design-6w1-02,
architecture-1-13 (vault part and the guard), the guard rules.

0. **Step 0.** Add `copyWithToast` to `lib/clipboard.ts` (contracts table). Test the success and failure toasts in
   `w2p8-clipboard.test.ts`.
1. **a11y-6w1-01: LoadState first** (shared, so it fixes the stale banner on every page).
   - An `$effect` on the `error` edge, while stale data is shown, calls
     `announce(\`Couldn’t refresh ${what}. ${error}\`, { assertive: true })`. Do not re-announce the same error on
     re-render.
   - `role="status"` comes off `.ls-stale`. The component API is unchanged.
   - Test it in `w2p8-loadStateAnnounce.test.ts`: the error edge announces once, and a second render with the same
     error does not announce again.
   - Then: `lib/components/FindInPage.svelte:431` (announce the first match count), `App.svelte:196` (daemon
     unreachable, assertive), `agents/FirstRunCoach.svelte:263`, `agents/NewSession.svelte:697`,
     `run-with-otto/RunDetail.svelte:147` (assertive), `help/TourFilm.svelte:185`, `share/SharePage.svelte:422`
     (assertive), `usage/UsagePage.svelte:550,561,1051`, `vault/NoteView.svelte:412`,
     `vault/RefineDrawer.svelte:259,262`, `settings/PluginsSettings.svelte:207`, `BrowserSettings.svelte:203`,
     `IssueAccounts.svelte:281`, `SelfImprovement.svelte:415`, and every other `inserted-live-region` hit in your
     paths.
   - Also sweep the directories for the rule-4 patterns.
2. **ux-6w1-02: clipboard.** Route these through `copyWithToast` or `copyText`:
   - `settings/Users.svelte:57,66`, `Logs.svelte:140`, `TrustSafety.svelte:190`;
   - `shell/NotificationBell.svelte:252`, `Palette.svelte:150`, `Navigator.svelte:444`;
   - `agents/history/HistoryPage.svelte:223`;
   - `agents/conversation/Markdown.svelte:108-112`: `copyText(text)` with success and failure toasts, so a missing
     `navigator.clipboard` no longer makes a silent no-op.

   Each failure body says what to do next. Add a test for the Markdown copy on an origin without
   `navigator.clipboard`: the failure toast fires.
3. **arch-1-13, vault.** Add `lib/uiCommands/vaultPort.ts`, a port the vault store registers when it loads
   (`registerVaultPort(vault)`). `lib/uiCommands/vault.ts` navigates to the vault (as now), waits for the port and
   drives it. Afterwards `rg "modules/" ui/src/lib --glob '!**/*.test.ts'` is empty once P5 and P7 land. Test the
   port wait.
4. **Guard rules** (in `ui-guards.mjs`, each with a unit-tested detector in `p04-uiGuards.test.ts` and a baseline
   entry set to the count at the end of the wave):
   - `lib-imports-modules`: becomes a hard rule (max 0, no baseline). Tell the integrator if P5 or P7 have not
     landed their parts yet.
   - `cross-module-import`: a file under `modules/<a>/` importing from `modules/<b>/` (b ≠ a). This class produced
     arch-6w1-02. Ratcheted per file.
   - `guessed-checked`: `checked={… ?? true}` (or `?? false`) on a control whose value is still loading (ux-6w1-04).
   - `clipboard-no-next-step`: a `toasts.*` call whose title or body mentions copy/clipboard on a failure path, but
     whose body lacks the "copy it manually" / "Select" next step (ux-6w1-02).
   - `icon-button-name`: a `<button>` whose only child is `<Icon …/>` and that lacks either `aria-label` or `title`.
     Check first that `unlabeled-control` does not already cover it; if it does, extend that rule instead.
   - `unit-source-regex`: a `ui/unit/*.test.ts` that `readFileSync`s a `.svelte` file and `assert.match`es function
     bodies. Ratchet per file, so source-regex tests stop growing (TQ-6w1-06). The detector counts
     `readFileSync(...'.svelte'` occurrences per test file.

   Add each rule's one-line description and document the rules in `docs/design/guidelines/review-checklist.md`.

Order: 0, 1 (LoadState), 2, 3, the rest of 1, 4 (last, so the baselines reflect the fixes).

---

## Ordering and contention

- **Step 0 first:** P3 (transcript helpers, async `RunGuard`), P8 (`copyWithToast`), P7 (`rowFocus` move) and P5
  (`listRange` option). Each takes minutes.
- **Waits:**
  - P2 item 3 waits on P3 step 0.
  - P5 item 5 and P6 item 3 wait on P8 step 0.
  - P8's `lib-imports-modules` hard rule waits on P5 item 3 and P7 item 6. P8 checks this with `rg` before flipping
    it, and otherwise leaves it ratcheted and tells the integrator.
- **Heavy builds:** P1, P2, P3 and P4 are cargo-heavy and all compile `otto-server`. Iterate with
  `cargo check -p <crate>` and keep to one heavy command at a time (rule 6). P3's `otto-rooms` extraction changes
  the workspace members. Run it after P3's other Rust items, and tell the coordinator before the first build that
  includes the new crate, because it invalidates more of the shared target dir than a normal edit.
- **Shared crate:** `otto-agent-run` is split by file: P1 owns `fanout.rs` (+ `fanout/`), and P2 owns the rest.
  P1 must not add a module to `lib.rs` (declare submodules from `fanout.rs`).

## Integrator checklist (after all packages report)

1. Apply hand-offs.
2. `python3 scripts/loc-ratchet.py --update-baseline` and `node scripts/ui-guards.mjs --update-baseline`. Expect:
   - drops for `GraphView.svelte`, `database.svelte.ts`, `apiClient.svelte.ts`, `WorkflowsPage.svelte`,
     `OverviewTab.svelte`, `vault_docs_agent.rs`, `routes/api_client.rs`, `otto-design-assist/src/lib.rs` and the
     `otto-server` total;
   - a new `otto-rooms` crate entry;
   - `inserted-live-region` well below 236;
   - `lib-imports-modules` at 0.
3. Lower `UI_LIB_SHIM_MAX` in `scripts/check.sh` to the new count (P5 deletes the two git shims).
4. `scripts/check.sh --base origin/main`, then the e2e specs the packages listed as touched.

## Deferred

None. Every finding in the digest is planned:

- arch-2-06 (deferred in wave 1) is P3 item 5, now that P3 owns `lib.rs`, `state.rs` and `otto-server/Cargo.toml`.
- arch-4-02 phase 2 is P2 item 1.
- The product-analysis port of arch-1-04 is P1 item 1 (done last within it). If it does not fit, P1 reports it as
  partial with the remaining step named precisely, and the integrator decides.
