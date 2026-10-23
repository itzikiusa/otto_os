# Quality iteration 6, wave 1: fix plan

Worktree: `/Users/itziklavon/claude_ade-iter6` (branch `fix/quality-iter6-20261023`, base `main` 02a64019b).
Inputs: the 15 lens reports in `../wave-0/` and the coordinator's extra items (split `ClassroomsBox.svelte` and
`ShooterGame.svelte`, which are both 832 lines and over the 800-line cap; keep the plugin-protocol change
concurrency-async-1-05 deferred).
Output: 10 work packages (P01 to P10) for 10 parallel fixers in one shared worktree.

**Goal of the round.** Architecture (7.11), test-quality (8.6), accessibility-responsive (8.87) and ux-design (9.31)
must reach 9.0 or higher. Nine of the ten packages carry work for those four lenses, and each package lists its
target-lens majors first. Every finding in the digest is planned. Only two are deferred (see the end).

## Rules every package follows

1. **Owned paths are exclusive.** Edit only the paths your package owns. When a finding also touches another
   package's file, that package's instructions carry a "part" for it; do not edit it yourself.
   - If a fix turns out to need a file you do not own, stop. Put it in your final report as a hand-off (path and
     exact change). Do not edit the file.
   - Files nobody owns are read-only. That includes `scripts/loc-baseline.json`, `crates/otto-server/src/routes/mod.rs`,
     `crates/otto-server/src/policy.rs`, `crates/otto-server/src/state.rs`, `crates/otto-server/src/boot/**`,
     `ui/src/lib/api/` files other than the ones named below, and `ui/src/lib/components/{LoadState,EmptyState,Skeleton,LazyMount}.svelte`
     (P03 owns `lib/components/**` but must keep those four APIs unchanged).
   - Pick fixes that need no new route. A new route needs `routes/mod.rs` and `policy.rs`, which nobody owns this
     round. Extend an existing response instead (each package's instructions say how).
2. **Splits are behaviour-preserving moves.** Keep the old path compiling with a re-export (`pub use`, or a TS
   re-export) so other packages keep building. Do not change behaviour in the same edit as a move.
   - Every function you create stays under the `function_max: 400` cap. No new file over 800 lines (UI) or the
     Rust file cap.
   - Ratchet baselines are only lowered, and only by the integrator after all packages land
     (`python3 scripts/loc-ratchet.py --update-baseline`, `node scripts/ui-guards.mjs --update-baseline`). P04 is
     the one exception: it adds the baseline entries for the new guard rules it creates. If a move needs a
     baseline entry renamed, report the exact entry.
3. **Contracts.** Only P08 edits `docs/contracts/**`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/types/**` and
   `crates/otto-core/src/api/**`. Every other package keeps the wire shape unchanged, except the one field P08 adds
   for P01 in its step 0 (`mcp_skipped`, below).
4. **Migrations.** Only P08 adds a migration: `crates/otto-state/migrations/0196_*.sql`. It follows the AGENTS.md
   rollback-compat rules (`migration_compat.rs`).
5. **Integration tests.** `crates/otto-server/tests/it.rs` belongs to P05. Only P05 creates a new
   `tests/<suite>.rs` (it creates two). Everyone else adds tests to suite files they own, to `tests/unit/<area>.rs`
   files for code they own, or to `#[cfg(test)]` modules in their own sources.
6. **Cargo.**
   - Only P02 edits `crates/otto-agent-run/Cargo.toml`. No package edits the root `Cargo.toml` or runs
     `cargo update`. A package may edit its own crate's `Cargo.toml`. The lockfile then changes implicitly; never
     edit it by hand.
   - Use scoped commands: `cargo check -p <crate>`, `cargo test -p <crate> --lib <filter>`,
     `cargo test -p otto-server --test it <suite>::`, `cargo clippy -p <crate> --all-targets -- -D warnings`.
   - **Run at most one heavy cargo or npm command at a time.** This Mac froze under ~10 parallel builds. Never run
     `cargo clean`. Do not run the full workspace suite or e2e; the coordinator runs the gates.
7. **UI checks.** Run `cd ui && npm run check` plus the unit tests you touched (`npx vitest run unit/<file>`).
   - New unit tests go in new files under `ui/unit/` with a package-specific prefix (`p06-…test.ts`).
   - Edit an existing `ui/unit` or `ui/e2e` file only when it tests code you own.
   - Follow `docs/design/guidelines/`: tokens only, `LoadState` for load errors with Retry, `EmptyState` for empty,
     `confirmer`/`toasts`/`ctxMenu`, real `<button>`s with `aria-label` + `title` when icon-only, logical CSS
     properties, no horizontal scroll at 390 px.
8. **Behaviour changes ship with a test that fails without the change.** Name it in your report.
9. **Git.** No commits, stashes, resets or checkouts; the integrator commits. Other agents (games-splits,
   kart-stuck-fix, mcp-db-execute, mcp-jira-create, mcp-profiles, e2e-regressions, round-integrator) are active in
   this session. Never overwrite a change you did not make. If a file you own has uncommitted changes you did not
   make when you start, report it before editing.
10. **Final report.** For each finding id: fixed, partial or skipped, with the reason; the tests you added; the
    commands you ran and their results; and any hand-offs.

## Cross-package contracts (step 0 items)

These are small and must land first, because other packages build on them. Each owner does its step 0 before
anything else and keeps the exact names below.

| Owner | Step 0 | Used by |
|-------|--------|---------|
| P01 | `crates/otto-core/src/e2e.rs`: `pub fn isolated() -> bool` (true when `OTTO_E2E` is `1` or `true`), plus `pub mod e2e;` in `otto-core/src/lib.rs`. `otto_transcript::roots::isolated` re-exports it. | P02, P03, P06 replace their own `env::var("OTTO_E2E")` checks |
| P02 | Add `otto-transcript.workspace = true` to `crates/otto-agent-run/Cargo.toml`. Delete the `pub use otto_sessions::submit::{…}` block in `crates/otto-review/src/session.rs:860-864` (import `submit_prompt` privately) and repoint its five callers (maint-6-01). | P01 (turn fold, `wait_for_tui_strict` removal) |
| P03 | `ui/src/lib/announce.svelte.ts`: `export function announce(text: string, opts?: { assertive?: boolean }): void`, driving one always-mounted sr-only polite region and one assertive region, mounted once in the shell next to `Toasts`. Repeating the same text must re-announce (clear, then set on the next frame). | every UI package (a11y-6-01 sites) |
| P06 | `ui/src/lib/listWindow.svelte.ts`: a verbatim move of `ListWindow`/`listRange` from `modules/git/list-window.svelte.ts` (and `list-range.ts`), with the old files re-exporting it. | P04 (k8s table window), P07 (SchemaTree) |
| P08 | In the session meta type in `ui/src/lib/api/types*` and `docs/contracts/api.md`: optional `mcp_skipped?: { server: string; reason: string }[]`. | P01 (EH-6-06) |

If your step-0 dependency is not there yet when you reach the dependent item, do the rest of your package first
and come back. Do not create the other package's file yourself.

## Package overview

| Pkg | Theme | Target-lens majors and splits first | Other lenses |
|-----|-------|-------------------------------------|--------------|
| P01 | Transcripts, sessions, agents/vault/usage UI, rooms and games | arch-6-01 (major), arch-6-02, ShooterGame split, a11y/ux sweeps | integ-6-02, CA-6-01/02, EH-6-06, BC-6-05 |
| P02 | Agent-run consolidation and assist engines | arch-5-02, arch-1-04 (major), arch-4-02 (major), maint-6-01 | BC-6-04 (canvas) |
| P03 | Workflow engine, shell and shared components, check.sh ratchets | arch-1-01r, arch-4-04r, TQ-6-03/6-04 (majors), a11y-6-01 announcer | BC-6-04 (review_run), EH-6-04 |
| P04 | API client, Workflows UI, AWS/K8s/connections, UI guards | arch-1-07r, arch-6-03, arch-1-08r, a11y-6-03, TQ-6-06, guard rules | UC-6-04/05, integ-6-04 |
| P05 | MCP governance, public entry gates, channels, state archive | TQ-6-01, TQ-6-02 (majors), arch-5-03, arch-1-05 | integ-6-01 (major), DI-6-03, integ-6-03, BP-6-03 |
| P06 | Git, panels, mission control, personal agents | arch-1-09 (major), arch-6-04, arch-6-05, a11y-6-06, a11y-6-04 | EH-6-07 |
| P07 | Database Explorer and brokers | arch-1-10 (major), ux-6-01, a11y-6-03 (Aggregate) | DI-6-01 (major), UC-6-01, BC-6-03, BP-6-01/02 |
| P08 | Swarm access control, contracts, proof, insights, settings | TQ-6-05, a11y-6-01 (settings/proof) | AC-6-01/02 (majors), AC-6-03/04, CA-6-03, DI-6-02/04, EH-6-02/03, BP-6-05, all api-contracts |
| P09 | Product and issues | arch-5-04, ux-6-03, ux-6-04, a11y-6-04, a11y-6-10 | BC-6-01/02 (majors), CA-6-04, BP-6-04, UC-6-03 |
| P10 | Home, 3D, Design Hall, browser, workbench | ClassroomsBox split, a11y-6-05/6-08, arch-5-05r, maint-6-06 | UP-6-01/02, UC-6-02, EH-6-01/05 |

---

## P01: Transcripts, sessions, agents/vault/usage UI, rooms and games

**Owns:**
- `crates/otto-transcript/**`, `crates/otto-sessions/**`, `crates/otto-orchestrator/**`
- `crates/otto-usage/src/tailer/**`, `crates/otto-improve/src/evidence.rs`, `crates/ottod/src/main.rs`
- `crates/otto-agent-run/src/turn_oracle.rs` (only this file in that crate)
- `crates/otto-core/src/e2e.rs` (new) and `crates/otto-core/src/lib.rs`
- `crates/otto-vault/**`
- `crates/otto-server/src/history_index.rs`, `crates/otto-server/src/transcript/**`, `crates/otto-server/src/transcript_cache.rs`, `crates/otto-server/src/transcript_tail.rs`
- `crates/otto-server/src/routes/transcript.rs`, `crates/otto-server/src/routes/session_files.rs`
- `crates/otto-server/src/rooms/**`, `crates/otto-server/src/game_rooms/**`
- `crates/otto-server/tests/{session_files,session_screen,rooms_api,resume_missing_cwd}.rs`
- `ui/src/modules/agents/**`, `ui/src/modules/usage/**`, `ui/src/modules/vault/**`, `ui/src/modules/rooms/**`
- `ui/src/lib/stores/transcript.svelte.ts`, `ui/src/lib/stores/transcriptLifecycle.ts`, `ui/src/lib/stores/sessionUsage.svelte.ts`
- `ui/src/lib/router.svelte.ts`, `ui/src/lib/rooms/**` (new), `ui/src/lib/uiCommands/vault.ts`
- `docs/features/{agent-sessions,conversation-view,usage-and-cost,vault,session-rooms,room-games,self-improvement,subscription-accounts}.md`

**Findings:** architecture-6-01, architecture-6-02, architecture-4-01r (sessions part), architecture-1-13 (router
and vault parts), integrations-6-02, concurrency-async-6-01, concurrency-async-6-02, error-handling-6-06,
error-handling-6-05 (agents, vault, transcript-store parts), maintainability-6-01 (submit.rs part),
maintainability-6-02 (submit.rs part), maintainability-6-07 (vault part), maintainability-6-08,
test-quality-6-01 (verify_ingest_token part), backend-correctness-6-05, accessibility-responsive-6-01 (usage, vault,
agents, rooms parts), accessibility-responsive-6-02 (ConversationView, PreviewBody), accessibility-responsive-6-07,
accessibility-responsive-6-09 (GameMatch), ux-design-6-04 (RecoveryView), ux-design-6-06, ux-design-6-07 (agents,
usage, GameMatch), ux-design-6-08 (games, recap-capture), ux-design-6-09, api-contracts-6-02 (session_files comment),
coordinator: ShooterGame split.

0. **Step 0.** Add `otto_core::e2e::isolated()` as in the step-0 table, and use it everywhere in your owned files.
1. **arch-6-01 (major).** Add `otto_transcript::roots`:
   - `isolated()` (re-export of the otto-core predicate).
   - `TranscriptRoots::discover(home, data_dir)`: `OTTO_TRANSCRIPT_ROOTS`, else the isolated fixture dir, else the
     defaults: `~/.claude`, `~/.claude-*`, `~/.config/claude`, `$CODEX_HOME` else `~/.codex`, plus every
     `provider-accounts/*/{projects,sessions}` under `data_dir`. Keep provider and kind on each root.
   - `TranscriptRoots::for_session(meta, data_dir)`.
   - Make these call it and delete their own copies: the usage tailer (`otto-usage/src/tailer/driver.rs:1386`
     and `ottod/src/main.rs:524-540`), `history_index.rs`, `transcript/resolve.rs` (`:174`), `otto-improve/src/evidence.rs`
     (`:158`) and `SessionManager::codex_root_for`.
   - One fixture layout for the isolated mode. Read the e2e harness (`ui/e2e/`, read-only) for the paths it seeds and
     keep them working.
   - Tests: a Codex rollout under `provider-accounts/<id>/sessions` and one under `$CODEX_HOME` are both found by
     `discover` and by `for_session`; `OTTO_TRANSCRIPT_ROOTS` wins; isolated mode never reads the real home.
   - Replace the `OTTO_E2E` checks in your files (`otto-orchestrator/src/lib.rs:147,166,196`,
     `otto-sessions/src/trust.rs:156`, `ottod/src/main.rs`, `transcript/resolve.rs`, `otto-improve`). P02, P03 and
     P06 replace theirs. P03 adds the check.sh ratchet.
2. **arch-6-02.** Move the pure folds (`ClaudeScanState`/`ClaudeTail`, `scan_codex`/`CodexTail`, the
   task-notification parsing) into `otto_transcript::turn`, with `tail_open() -> Option<bool>`.
   - `otto-agent-run/src/turn_oracle.rs` keeps the clocks and verdicts and calls the shared fold (P02 adds the
     Cargo dependency in its step 0).
   - `SessionManager::agent_turn_open` calls `tail_open`. Delete `claude_turn_open`, `codex_turn_open` and the
     manager's `TASK_NOTIFICATION`. Point the existing manager tests at the shared fold.
   - Where the two notification rules differ today, keep the oracle's rule and add a test that pins it.
3. **integ-6-02.** `user_prompt_text` (turn_oracle) and `claude_pty::last_user_text` return `None` for `isMeta` or
   `isCompactSummary` lines (mirror `titles.rs:77`). Test: a prompt followed by an `isMeta` skill-body line;
   `last_user_text` is the prompt and `user_turns == 1`.
4. **arch-4-01r, sessions part.** Add a run-active guard to `otto_sessions::submit` additively: a new optional
   field on `SubmitOpts` (default `None`) holding `Arc<dyn Fn() -> bool + Send + Sync>`, checked before the paste and
   before each re-inject. No signature change. P08 converts the swarm host to it. Add a unit test.
5. **maint-6-01/6-02, submit.rs part.** Delete the sentence at `submit.rs:17-18` and reword `:122`. Delete
   `wait_for_tui_strict` only once `rg wait_for_tui_strict crates` shows no use outside `submit.rs` (P02 removes the
   re-export in its step 0).
6. **TQ-6-01 part (4).** Unit-test `verify_ingest_token`: empty token, unknown session, another session's token,
   right token.
7. **EH-6-06.** Collect `{server, reason}` for each skipped user MCP server into session meta `mcp_skipped`
   (`otto-sessions/src/mcp.rs:384`) and show it in the session's MCP menu in `modules/agents/` ("2 MCP servers
   weren't attached" with each reason). Test the backend collection. The type comes from P08's step 0.
8. **CA-6-02.** In `routes/transcript.rs:703` `get_artifact`: run `is_file`/`canonicalize` and
   `routes::fs::read_at_most(&canonical, ARTIFACT_CAP as usize + 1)` inside one `offload::blocking` closure, and map
   an over-cap length to `PayloadTooLarge`. Add a symlink-swap-after-canonicalize test (copy the `session_files`
   pattern). Also fix the `session_files.rs:172` comment to say 502 (api-contracts-6-02).
9. **CA-6-01.** Vault rename (`otto-vault/src/engine.rs:1743`): run everything from the move on (the backlink rewrite
   loop and `rescan_after_mutation`) in a `tokio::spawn` task that owns the write-lock and publication guards; the
   handler awaits the `JoinHandle`. Test: drop the future after the move; every backlink is rewritten and the index
   shows the new path. Also fix the stale pointer at `engine.rs:2708` (maint-6-07: the UI matcher is `fuzzyMatch` in
   `ui/src/lib/commandSearch.ts`).
10. **Rooms and games.**
    - **Split `games/shooter/ShooterGame.svelte`** (832 lines) below 800: extract the pause/menu overlay and the
      HUD/srStatus wiring into `shooter/hud/` components. Behaviour-identical.
    - **BC-6-05.** `rooms/actions.rs:138`: `self.members.get(&self.host).is_none_or(|h| !h.view.audio_joined)`
      refuses instead of panicking. Add a test with the host pruned.
    - **maint-6-08.** `shooter/meshes.ts`: import `canvas` from `../common/textures`, delete the local copy.
    - **ux-6-06.** `RoomRecap.svelte`: store `{ what, detail }` ("Couldn’t prepare the recap." / "Couldn’t check the
      local recap engines." plus dim `loadErrorText`). Show "Retry checks" only for a failed check; render the
      banner error only when `!prepare`.
    - **ux-6-07 / a11y-6-09.** `GameMatch.svelte`: use `copyText()` and the copy "Select the invitation link and
      copy it."; when the match closes mid-game, `await tick()` and focus "Back to games".
    - **ux-6-08.** "The race couldn’t start.", "The match couldn’t start.", and the two `recap-capture.ts` notes.
    - **arch-1-13, router part.** Move the room/game invite capture out of `lib/router.svelte.ts:18-19` into
      `lib/rooms/invites.ts`, so `lib/` no longer imports `modules/rooms`. Also `uiCommands/vault.ts:13`.
11. **a11y-6-01 sites** (use P03's `announce()` at the state edge and drop `role` from the visible line, or keep a
    local region mounted and empty): `usage/OttoUsage.svelte:160`, `usage/UsagePage.svelte:1017`,
    `vault/VaultPage.svelte:572`, `agents/conversation/Composer.svelte:449`, `rooms/RoomMedia.svelte:46`,
    `rooms/RoomPage.svelte:162,166`, `rooms/RoomRecap.svelte:81,90`, `rooms/RecapPanel.svelte:111`,
    `rooms/games/GameMatch.svelte:147`, `rooms/games/shooter/ShooterGame.svelte:601`. Sweep your directories for
    more `{#if …}<x role="status">` sites and fix them.
12. **a11y-6-02.** `ConversationView.svelte:868` and `PreviewBody.svelte:201`: replace the hand-rolled busy blocks
    with `<Skeleton label=…>` or `LoadState`.
13. **a11y-6-07.** `vault/LinkPreview.svelte`: an Escape keydown listener while `linkPreview.current` is set calls
    `linkPreview.hide()` without moving focus; give the card an id and set `aria-describedby` on the anchor.
14. **ux-6-04 / 6-09 / EH-6-05 (vault, agents).** `RecoveryView.svelte:137` gates the empty line on `!error`.
    `OpenApiView.svelte` gets an `EmptyState` "No operations in this spec". `loadErrorText(e)` in
    `vault/Switcher.svelte:37`, `agents/conversation/PreviewBody.svelte:145` and `lib/stores/transcript.svelte.ts:516`;
    `providerErrorText(e)` in `agents/JiraIssuePicker.svelte:159`.
15. **ux-6-07 (agents, usage).** Give the clipboard-failure bodies a next step ("Select the text and copy it
    manually.") or route them through `copyWithToast`: `ToolStep.svelte:219`, `PreviewPanel.svelte:80`,
    `TurnItem.svelte:81,89`, `ShareModal.svelte:179`, `SessionView.svelte:594`, `usage/AttributionDrilldown.svelte:110`.
16. **Sweep** `agents/`, `usage/`, `vault/` and `rooms/` for the a11y/ux patterns listed in rule 7.

**Verify:** `cargo test -p otto-transcript -p otto-sessions -p otto-orchestrator -p otto-vault -p otto-improve`
(one at a time), `cargo test -p otto-usage --lib tailer`, `cargo test -p otto-server --lib transcript`,
`cargo test -p otto-server --test it session_files:: rooms_api::`, clippy on those crates, `npm run check`.

## P02: Agent-run consolidation and the assist engines

**Owns:**
- `crates/otto-agent-run/**` except `src/turn_oracle.rs` (P01)
- `crates/otto-server/src/{agent_session,agent_session_lifecycle,agent_session_reply}.rs`, `crates/otto-server/src/lib.rs`
- `crates/otto-review/**`, `crates/otto-server/src/modules/review/**`, `crates/otto-server/src/modules/review.rs`,
  `crates/otto-server/src/modules/session_input.rs`, `crates/otto-server/src/modules/ctx_impls.rs`,
  `crates/otto-server/src/review_launch.rs`
- `crates/otto-server/src/{skill_review,skill_review_static,skill_eval,skill_eval_output_tests,skill_eval_recovery_tests,finding_agent,finding_context,vault_docs_agent,mockup_assist,design_scene3d,design_hall,automation_ctx,product_host,assistant_host}.rs`
- `crates/otto-server/src/routes/findings.rs`
- `crates/otto-canvas/**`, `crates/otto-design-assist/**`, `crates/otto-design/**`, `crates/otto-product/src/design_scene3d.rs`
- `crates/otto-server/tests/{review_agent_retry,review_comment_states,review_forge_post,agent_run_recovery,canvas_refs_api}.rs`
- `docs/features/{code-review,skills-evaluator,skills-library,review-findings,canvas}.md`

**Findings:** architecture-5-02, architecture-1-04 (major), architecture-4-02 (major), maintainability-6-01,
maintainability-6-03 (agent_session_lifecycle part), backend-correctness-6-04 (canvas part), architecture-6-01
(OTTO_E2E part).

0. **Step 0** (see table): add `otto-transcript` to `otto-agent-run/Cargo.toml`. **maint-6-01:** delete the
   `pub use otto_sessions::submit::{…}` block at `otto-review/src/session.rs:860-864` (import `submit_prompt`
   privately for `:538`). Repoint `agent_session.rs:707`, `skill_eval.rs:1025`, `skill_review.rs:43`,
   `finding_agent.rs:369` and `modules/session_input.rs:305` (and the doc at `:232`) to
   `otto_sessions::submit::submit_prompt`. Remove `pub use otto_review::session as review_session;` from
   `otto-server/src/lib.rs:71` if nothing else uses it (`rg review_session crates`). Hand off the
   `otto-assistant/src/ctx.rs:91` doc wording to P06.
1. **arch-5-02.** Move the resumable session-turn runner (`agent_session.rs`, `_lifecycle.rs`, `_reply.rs`) into
   `otto-agent-run::session_turn`, taking `&Arc<SessionManager>` and a small host trait for the few ServerCtx needs.
   Leave `otto-server/src/agent_session*.rs` as thin re-exports so other packages keep compiling. Drop the
   canvas/design-assist/automation/product forwarders that live in files you own (`ctx_impls.rs`,
   `automation_ctx.rs`, `product_host.rs`); keep any forwarder whose trait lives in a crate you do not own.
   - **maint-6-03 part:** `acquire_lease` (`agent_session_lifecycle.rs:19-35`) becomes
     `static LOCKS: KeyedLocks<String>` + `.lock(sid).await`; adapt its `Arc::ptr_eq` test to `handle()`.
   - **OTTO_E2E part:** use `otto_core::e2e::isolated()` at `agent_session.rs:232,328`, `routes/findings.rs:995`,
     `finding_agent.rs:167`.
2. **arch-1-04 (major).** Add a shared fan-out + summarizer runner in `otto-agent-run` (on top of
   `session_turn`): N reviewer turns, a managed summarizer turn with the retry index, the deterministic fallback, and
   per-agent state callbacks. Port onto it, in this order, each port behaviour-identical with its existing tests
   passing: `vault_docs_agent.rs` (`:356` `SUM_RETRY_IDX`), `skill_review.rs` (`:400`), and the code-review
   summarizer (`modules/review/summarize.rs` `summarize_and_persist`). Add runner unit tests: all reviewers ok,
   one reviewer fails, summarizer fails then falls back, retry of the summarizer only.
3. **arch-4-02 (major).**
   - Interim: replace `SceneBusy` (`otto-canvas/src/assist.rs:479`) and `BusyGuard`/`try_busy`
     (`otto-design-assist/src/lib.rs:1029-1051`) with `otto_core::cancel_signal::InFlightGuard`. Move `AgentTurn`
     (`otto-design-assist/src/ctx.rs:33`) into `otto-agent-run` and drop `otto-canvas`'s `otto-design-assist`
     dependency (repoint `assist.rs:42` to `otto_design::format::excalidraw`). Move scene3d from
     `otto-product/src/design_scene3d.rs` into `otto_design::format::scene3d`; leave the product file and
     `otto-server/src/design_scene3d.rs` as one-line re-exports.
   - Phase 2: an `AssistTarget` trait (load document, build prompt, extract and validate the reply, apply, live-poll
     path) with one shared turn engine (claim via `InFlightGuard`, `live_file::StatGate` poll, fenced extraction).
     Make canvas, the Design Hall assist and `mockup_assist.rs` implementations of it. Pin each engine's current
     behaviour with a test before porting it.
4. **BC-6-04, canvas part.** `otto-canvas/src/assist.rs:900`: `extract_json_where` with non-empty `elements` or
   `nodes`. Test with a decoy object first.

**Verify:** `cargo test -p otto-agent-run -p otto-review -p otto-canvas -p otto-design -p otto-design-assist`
(one at a time), `cargo test -p otto-server --lib agent_session skill_review vault_docs`,
`cargo test -p otto-server --test it review_agent_retry:: agent_run_recovery::`, clippy on those crates.

## P03: Workflow engine, shell and shared components, check.sh ratchets

**Owns:**
- `crates/otto-server/src/workflow_engine.rs`, `crates/otto-server/src/workflow_engine/**`,
  `crates/otto-server/src/{run_engine,workflow_budget,workflow_node_driver,workflow_trigger_scheduler,workflow_product_publish}.rs`
- `crates/otto-server/src/budget/**`, `crates/otto-server/src/routes/{goal_loops,workflows,workflow_progress,workflow_run_files,snips}.rs`
- `crates/otto-workflows/**`, `crates/otto-automation/**`
- `crates/otto-server/tests/{workflow_run_files,run_repo_scope,snips}.rs`, `crates/otto-server/tests/unit/workflow_engine.rs`
- `scripts/check.sh`
- `ui/src/shell/**`, `ui/src/App.svelte`, `ui/src/lib/components/**` except `MetricChart.svelte` (P04)
- `ui/src/lib/announce.svelte.ts` (new), `ui/src/lib/clipboard.ts`, `ui/src/lib/ask.ts`, `ui/src/lib/orchestrate.ts`
- `ui/src/modules/canvas/**`, `ui/src/lib/stores/canvas.svelte.ts`, `ui/src/lib/canvas/**`, `ui/src/modules/snip/**`
- `ui/src/modules/{scheduled-tasks,loops}/**`, `ui/src/lib/stores/{scheduledTasks,loops}.svelte.ts`
- `docs/design/guidelines/**`, `docs/features/{workflows,scheduled-tasks,goal-loops,snipping-tool}.md`

**Findings:** test-quality-6-03 (major), test-quality-6-04 (major), architecture-1-01r, architecture-4-04r,
accessibility-responsive-6-01 (announcer and lib/components parts), accessibility-responsive-6-02 (NotificationBell),
accessibility-responsive-6-09 (JsonTree, CapsBanner, ConfluenceSpaceSelect), ux-design-6-07 (clipboard.ts, snip),
maintainability-6-04, maintainability-6-05, maintainability-6-03 (allow-list trim), maintainability-6-07 (workflow
part), architecture-6-01 (OTTO_E2E part and ratchet), architecture-1-13 (canvas store part),
backend-correctness-6-04 (review_run part), error-handling-6-04, test-quality-6-01 (workflow-webhook part).

0. **Step 0: the announcer (a11y-6-01).** Write `lib/announce.svelte.ts` as in the step-0 table and mount its
   regions once in the shell beside `Toasts`. Unit-test that the same text twice is announced twice and that the
   assertive channel is separate. Add to `docs/design/guidelines/accessibility.md`: status text is announced with
   `announce()` at the state edge, never by inserting a `role="status"` element together with its text. Add to
   `components.md`: new long lists use `VirtualList` or `lib/listWindow` (arch-6-05, for P06).
1. **TQ-6-04 (major) with arch-4-04r.** In `workflow_engine/nodes/review_run.rs` extract
   `review_score(bugs, warns, infos, weights, no_changes)`, `review_verdict(status, coverage, no_changes, score,
   threshold)` and `aggregate(outs, n_targets)`, and split `execute` (615 lines) into a per-target context struct
   plus launch and score/verdict phases. Unit-test: 3 bugs give 40; a score at the threshold passes and
   threshold − 1 fails with "below threshold"; status error gives "did not complete"; the worst score wins; one
   failing repo fails all; fewer outputs than targets fails. The recovery test keeps pinning the wiring.
   - **BC-6-04, review_run part:** use `extract_json_where` (a `score` or a `goals` array). No match is the
     not-parseable branch with a note, never a silent default to `review_score`. Decoy-first test.
2. **TQ-6-03 (major).** Extract a pure `verdict_from(cfg, totals, ws, provider)` in `budget/mod.rs` and unit-test:
   spend exactly at the cap is exceeded and blocked iff `block_on_exceed`; 79 % vs 80 % for the warning; workspace A
   over its cap leaves B unblocked; a provider cap blocks codex but not grok; enforce off never blocks. Then store an
   unparsable config and assert a goal-loop start gives 400 "Budget exceeded" with the status unchanged, and that a
   `budget_gate` workflow node ends in Error.
3. **arch-1-01r.** Introduce `RunState` (the run's mutable bookkeeping) in `workflow_engine/run/state.rs`, a
   loop-control enum, and split `run_workflow_with_budget` (`:1850`, 1,250 lines) into `run/admission.rs`,
   `run/schedule.rs` and a thin driver, each function under 400 lines. Moves first, then extraction. Keep the
   `pub(crate)` names reachable from `workflow_engine.rs`. Iteration 5 planned this and did not land it: do it
   before the smaller items below, and keep `cargo test -p otto-server --lib workflow_engine` green after each step.
4. **EH-6-04.** `otto-workflows/src/chat.rs:1138`: reply per outcome. `Ok(Some)` keeps the text; `Ok(None)` →
   "Run `x` already finished — nothing to abort."; `Err(e)` → "⚠️ Couldn’t abort run `x`: {e}. Use Cancel in
   Otto." Three unit tests.
5. **OTTO_E2E part.** `workflow_engine.rs:276`, `run_engine.rs:131`, `routes/snips.rs:373`,
   `otto-automation/src/scheduled_tasks_engine.rs:889` → `otto_core::e2e::isolated()`. Fix the stale note at
   `workflow_engine.rs:2547-2548` ("no cross-run node-result cache; removed in 344315ea7").
6. **TQ-6-01, workflow-webhook part.** When P05's `crate::routes::channel_webhook::webhook_key_guard` exists, use it
   in `routes/workflows.rs` `webhook_trigger`; otherwise report a hand-off.
7. **a11y-6-02 / 6-09 / 6-01 (shell, lib/components).**
   - `shell/NotificationBell.svelte:368`: drop `aria-busy` from the wrapper.
   - `lib/components/JsonTree.svelte:155`: after "show N more" unmounts, focus the last revealed row.
   - `shell/CapsBanner.svelte:31`: rename to "Dismiss permissions notice"; after dismiss, focus the main landmark.
   - `lib/components/ConfluenceSpaceSelect.svelte:64-68`: after Retry resolves, focus the select.
   - `lib/components/ResourceAccess.svelte:197`, `lib/components/AccountPicker.svelte:97`: `announce()` instead of an
     inserted status.
   - Sweep `shell/` and `lib/components/` for the rule-7 patterns.
8. **ux-6-07.** `lib/clipboard.ts:59`: the thrown message becomes "The clipboard isn’t available here. Select the
   text and copy it manually." Fix `snip/SnipEditor.svelte:284`.
9. **maint-6-04.** Write the NUL separators as `\u0000` escapes in `lib/components/ModelPicker.svelte:81` (P07 does
   `DiagramView.svelte`). Add a check.sh guard that fails on raw C0 bytes (other than tab/LF/CR) in tracked
   `*.ts`, `*.svelte`, `*.rs` and `*.js`. Turn it on last, after P07 reports DiagramView done.
10. **maint-6-05.** Delete `modules/canvas/types.ts` after repointing its importers inside `modules/canvas/`,
    `lib/ask.ts:14`, `shell/Palette.svelte:20` and `assistDoc.ts` to `lib/canvas/types`; import `plural` from
    `lib/plural` and drop the `orchestrate.ts:383-385` re-export. P06 repoints `panels/CanvasPanel.svelte`, P09
    repoints `product/LinkedCanvases.svelte` and `product/types.ts`. Delete the shim only when `rg` shows no
    importer left. Widen the check.sh shim ratchet to `export (type )?\* from '(\.\./)+lib/`.
    **arch-1-13 part:** remove the `modules/` import from `lib/stores/canvas.svelte.ts:16`.
11. **check.sh ratchets, done last.**
    - OTTO_E2E: count `env::var("OTTO_E2E")` in `crates/*/src` outside `otto-core/src/e2e.rs`; seed with the count
      left after P01, P02, P03 and P06 finish (target 0).
    - maint-6-03: remove from `KEYED_LOCK_ALLOW` every path that no longer matches the registry grep (expected:
      `agent_session_lifecycle.rs`, `proof.rs`, `routes/mission.rs`, `routes/personal_agents.rs`,
      `otto-channels/src/telegram.rs`). Re-run the grep to decide.
12. **Sweep** `scheduled-tasks/`, `loops/`, `canvas/` and `snip/` for the rule-7 patterns.

**Verify:** `cargo test -p otto-server --lib workflow_engine budget`, `cargo test -p otto-workflows`,
`cargo test -p otto-automation`, `cargo test -p otto-server --test it workflow_run_files::`, clippy, `npm run check`,
`bash scripts/check.sh --dry-run`.

## P04: API client, Workflows UI, AWS/Kubernetes/connections, UI guards

**Owns:**
- `crates/otto-server/src/routes/api_client.rs`, `crates/otto-server/src/routes/api_client/**`,
  `crates/otto-server/src/routes/api_automation_runs.rs`, `crates/otto-server/src/workflow_api_request.rs`,
  `crates/otto-server/src/api_secrets.rs`, `crates/otto-apiclient/**`, `crates/otto-server/tests/unit/api_client.rs`
- `crates/otto-k8s/**`
- `ui/src/modules/api/**`, `ui/src/lib/stores/apiClient.svelte.ts`, `ui/src/lib/stores/apiClient/**` (new),
  `ui/src/lib/stores/{apiStream.svelte.ts,apiHistory.ts}`, `ui/src/lib/api/graphqlVars.ts` (new), `ui/src/lib/uiCommands/api.ts`
- `ui/src/modules/workflows/**`, `ui/src/lib/stores/workflowRuns.svelte.ts`, `ui/src/lib/uiCommands/workflows.ts`
- `ui/src/modules/aws/**`, `ui/src/lib/stores/aws.svelte.ts`, `ui/src/lib/uiCommands/aws.ts`
- `ui/src/modules/kubernetes/**`, `ui/src/lib/stores/k8s.svelte.ts`, `ui/src/lib/k8s/**`, `ui/src/lib/uiCommands/k8s.ts`
- `ui/src/modules/connections/**`, `ui/src/lib/stores/sftp.svelte.ts`
- `ui/src/lib/components/MetricChart.svelte`
- `ui/scripts/ui-guards.mjs`, `ui/scripts/ui-guards-baseline.json` (new rule entries only; see rule 2)
- `docs/features/{api-client,aws-console,kubernetes-console,kubernetes-monitoring,connections-ssh-sftp,session-network-profiles}.md`

**Findings:** architecture-1-07r, architecture-6-03, architecture-1-08r, accessibility-responsive-6-03
(AutomationEditor), accessibility-responsive-6-01 (api part and the guard), test-quality-6-06, ux-design-6-02 (aws),
ux-design-6-07 (api, aws), ux-design-6-08 (apiStream), architecture-6-05 (k8s part), architecture-1-13 (apiClient,
k8s parts), ui-correctness-6-04, ui-correctness-6-05, integrations-6-04, maintainability-6-02 (k8s part),
maintainability-6-07 (api_secrets part), error-handling-6-05 (connections parts).

1. **a11y-6-03.** `api/AutomationEditor.svelte`: key steps, assertions and extracts by a stable client id (not
   `(i)`). After a move, `await tick()` and focus the moved item's same arrow (or its other arrow at the edge). After
   a removal, use `focusAfterRowRemoval` from `api/rowFocus.ts`. Name each Remove after its row ("Remove check on
   {path} in step {n}"). Unit-test the reorder focus target. Then the a11y-6-01 api sites: `AutomationEditor.svelte:409`,
   `HistoryList.svelte:198`, `ApiPanel.svelte:44`, `ResponseViewer.svelte:392`, `ApiPage.svelte:416` → `announce()`.
2. **arch-6-03.** Split `lib/stores/apiClient.svelte.ts` (2,308 lines) into `lib/stores/apiClient/{tabs,automations,collectionsSync,cookies}.svelte.ts`
   behind the unchanged `apiClient` facade. Start with automations (`:1808-2008`) and collection sync
   (`:1059-1265`). Move `graphqlVars` into `lib/api/graphqlVars.ts`. Remove the `modules/` import at `:26` (1-13).
3. **arch-1-08r.** Move the live run sync (`refetchRun` `:285`, `waitRunTerminal` `:1003`), the final output and the
   approvals out of `workflows/WorkflowsPage.svelte` into a new `workflows/RunMonitor.svelte` (plus a
   `runMonitor.svelte.ts` store if needed). Behaviour-identical.
4. **arch-1-07r.** Move `execute` (`routes/api_client.rs:1803`), `execute_service` (`:1821`) and
   `resolve_environment` (`:2012`) into `otto-apiclient` behind a ctx trait (secrets, environments, history). The
   route file keeps thin handlers and `pub use` re-exports; `workflow_api_request.rs` calls the crate. Fix the stale
   `api_secrets.rs:127` doc (`allowed_request_ids`, maint-6-07).
5. **TQ-6-06.** Move the binding construction into `StepSecretBinding::for_caller(agent, api, wid, vars, env, blob)
   -> Result<Option<Self>>`, used by `start` and by the test. Assert `for_caller(false, ..)` is `None` and
   `for_caller(true, ..)` binds the secret's host.
6. **UC-6-04.** `MetricChart.svelte:325`: key the series blocks by index, or carry a unique key in
   `MetricChartSeries`. Test with two series of the same label.
7. **UC-6-05.** `kubernetes/monitor/MonitorSettings.svelte:241`: after each remove, `errors = validate(cfg)` when
   errors exist (or key errors by a stable row id). Test.
8. **arch-6-05, k8s part.** After P06's step 0, re-express `kubernetes/monitor/tableWindow.ts` over the shared
   `lib/listWindow` range with an extra-height hook.
9. **integ-6-04.** `otto-k8s/src/monitor/gateway.rs:26`: on a gateway 401, set `state.gateway = None` so the next
   ensure restarts it, and word it "credentials rejected" (not "cluster RBAC"). Test. Delete the unused
   `Kind::k8s_kind` (`resources.rs:118`) and reword `from_k8s_kind`'s doc (maint-6-02).
10. **ux-6-02 (aws).** `aws/LogsView.svelte:543,625,720` and `aws/MetricsPanel.svelte:144,152` →
    `<LoadState what=… error=… empty onretry=…/>`, keeping Sign in through `errorActions`.
11. **ux-6-07 / 6-08.** AWS clipboard sites (`SqsView:331`, `S3Browser:396`, `RdsView:133`, `AthenaView:415`,
    `Ec2View:183`, `EksView:127`, `LogsView:342`) and API ones (`RequestBuilder:684`, `ResponseViewer:256`,
    `api/JsonTree.svelte:45`) get the next step. `apiStream.svelte.ts:92` → "Couldn’t open the relay socket".
    1-13: remove the `modules/` imports from `lib/stores/k8s.svelte.ts:16` and `uiCommands/k8s.ts:22-23`.
12. **EH-6-05 (connections).** `loadErrorText(e)` in `connections/NetworkProfilePicker.svelte:18`,
    `connections/SessionNetworkStatus.svelte:40` and `lib/stores/sftp.svelte.ts:98`.
13. **UI guards (do after 1-12; seed each new rule's baseline with the count at the time).** Add cheap rules to
    `ui-guards.mjs`, each with a unit test in `ui/unit/p04-uiGuards.test.ts`:
    - `inserted-live-region`: `{#if …}` followed (same or next line) by an element with `role="status"`,
      `role="alert"` without `tone`, or `aria-live` (a11y-6-01).
    - `busy-empty-status`: `role="status"` together with `aria-busy="true"` (a11y-6-02).
    - `bare-lazy-import`: `{#await import(` with no `{:catch` before its `{/await}` (ux-6-01).
    - `empty-state-error`: `<EmptyState` whose title starts with "Couldn’t"/"Couldn't" or that carries a Retry
      action, without `tone="error"` (ux-6-02).
    - `could-not-copy`: "Could not", "Cannot open" or lower-case "failed to" inside a user-visible string literal
      or markup text (ux-6-08). Skip comments and `console.*`.
    - `index-keyed-reorder`: an `{#each … (i)}` block in a file that also defines a move/reorder function (a11y-6-03).
14. **Sweep** `api/`, `workflows/`, `aws/`, `kubernetes/` and `connections/` for the rule-7 patterns.

**Verify:** `cargo test -p otto-apiclient`, `cargo test -p otto-server --lib api_client`, `cargo test -p otto-k8s`,
clippy, `npm run check`, `npx vitest run unit/p04-*`, `node scripts/ui-guards.mjs`.

## P05: MCP governance, public entry gates, channels, state archive

**Owns:**
- `crates/otto-mcp/**`, `crates/otto-server/src/{mcp_outward,mcp_http,mcp_auto_approve,mcp_capabilities,self_call}.rs`,
  `crates/otto-server/src/routes/{mcp_cp,mcp_servers,resource_access,resource_access_tests,channel_webhook,swarm_webhook,swarm_ingest,db_assist}.rs`,
  `crates/ottod/src/mcp_tools.rs`
- `crates/otto-state/src/{mcp_audit,mcp_auto_approve,mcp_control,mcp_profiles,mcp_servers}.rs`
- `crates/otto-server/src/state_archive.rs`, `crates/otto-server/src/state_archive/**`
- `crates/otto-channels/**`
- `crates/otto-server/tests/it.rs`, `crates/otto-server/tests/public_entry_gates.rs` (new),
  `crates/otto-server/tests/database_changes_api.rs` (new), `crates/otto-server/tests/{mcp_auto_approve,mcp_profiles_edges,foundation_integrity}.rs`
- `ui/src/modules/mcp/**`, `ui/src/lib/stores/mcp-profiles.svelte.ts`, `ui/src/lib/mcpProfiles.ts`
- `docs/features/{mcp-control-plane,mcp-control-plane-design,mcp-control-plane-plan,channels-slack-telegram,state-archive,backup-restore,resource-access-and-database-changes,plugins}.md`

**Findings:** test-quality-6-01 (major), test-quality-6-02 (major), architecture-5-03, architecture-1-05,
integrations-6-01 (major), data-integrity-6-03, integrations-6-03, backend-performance-6-03, maintainability-6-03
(telegram part), maintainability-6-07 (slack part), ux-design-6-05, error-handling-6-05 (OttoServerHome part),
accessibility-responsive-6-09 (ProfilesTab).

1. **TQ-6-01 (major).** New it suite `public_entry_gates.rs` (add its `#[path]` line in `it.rs`):
   - All three webhooks (channel, swarm, workflow): no key, a wrong key and a disabled integration give
     401/401/404 with no session, run or swarm row written; the right key gives 202; another workspace's swarm id
     gives 404.
   - db-assist: no or wrong key gives 401; an UPDATE with the right key gives the read-only error.
   - `/ingest/swarm/board`: no token or another session's token is refused and no board row is written.
   - Plugin host: no or empty Bearer, or a disabled plugin's token, gives 401 on jira/credentials.
   - Telegram (reuse the `telegram.rs:1017` harness in `otto-channels`): a stranger's text creates no session and is
     listed as rejected.
   - Share one `pub(crate) fn webhook_key_guard` in `routes/channel_webhook.rs`, used by `channel_webhook.rs` and
     `swarm_webhook.rs`. P03 adopts it in `routes/workflows.rs`.
   - (P01 adds the `verify_ingest_token` unit test.)
2. **TQ-6-02 (major).** New it suite `database_changes_api.rs`: users author (submit), reviewer (submit+approve)
   and peer (submit) on one Prod connection policy. (1) peer approve → 403, status stays `awaiting_review`; peer
   reject with a note → 403. (2) reviewer approve → 200. (3) after revoking reviewer's `change_approve`, execute →
   403 with no attempt row. (4) peer GET detail shows no history entry targeting a connection peer cannot discover.
   All before any driver call. Tests only; production code in `routes/database_changes.rs` is read-only to you.
3. **integ-6-01 (major).** `otto-mcp/src/outward/exec.rs` `call_timeout`: `merge_pr` 90 s, `create_issue` 120 s,
   create/update/comment Confluence page 90 s (`create_pr` 60 s), each at or below `GOVERNED_RUN_BUDGET`. Extend the
   `call_timeout` test to assert each budget covers the provider poll constants. Put the created key/URL in the
   timeout error so a late verify cannot hide a completed write.
4. **arch-5-03.** Extract the shared governance stages into `otto_mcp::governance` and make both
   `McpService::invoke` and `governed_invoke` (`mcp_outward.rs:446`) callers of one pipeline. **Keep today's
   behaviour exactly:** `policy::evaluate` applies only where it applies today (registered servers via
   `service.rs:841,1106`), and auto-approve rules apply only to `otto.*` outward calls as today. Encode that as
   per-caller stage selection, not as new rule coverage. First add tests that pin the current allow/deny/approval
   outcomes for both callers (a policy-denied registered tool, an auto-approved `otto.*` tool, an `otto.*` tool a
   server policy would deny, a registered tool an auto-approve rule would match), then refactor; the tests must pass
   unchanged. Correct `docs/features/mcp-control-plane.md:45`.
5. **arch-1-05.** Route the native twins in `ottod/src/mcp_tools.rs` (e.g. `otto_list_prs` `:2635`) through
   `route_for`, stripping `/api/v1` in one place. The tools/list byte-budget test must stay green; do not grow the
   catalog text.
6. **DI-6-03.** State archives keep MCP profiles: archive non-builtin rows plus each builtin's editable fields and
   `is_default`; on restore merge a builtin whose id exists instead of reporting a conflict, and restore the default
   through the transactional default switch. Restore test with a custom default profile and a session naming it.
   Run `foundation_integrity` (seeded tables break fresh-DB restore if handled wrongly).
7. **integ-6-03.** `otto-channels/src/mirror.rs:1178`: on a non-rate-limit upload error, still send the head with
   "the full reply could not be attached ({error}); open the session in Otto for the rest." Keep the rate-limit
   retry. Stub-adapter test: upload fails with `missing_scope`; exactly one `send_formatted` carries head and note.
   Fix `slack.rs:401`'s pointer (`handle_event`/`drop_reason`, maint-6-07).
8. **maint-6-03, telegram.** `poller_lock` (`telegram.rs:514-521`) becomes a `static KeyedLocks<String>`.
9. **BP-6-03.** `routes/resource_access.rs:144` `retire_mcp`: collect the roots in async code, run the whole
   `retire_user_server` loop in one `spawn_blocking`, return its first error.
10. **ux-6-05 / EH-6-05 / a11y-6-09 (mcp).** `mcp/OttoServerHome.svelte`: while `attachError` is set render
    `<LoadState what="the session-attach setting" variant="compact" … onretry={() => loadAttach(wsId)}/>` instead of
    the checkbox; same for the gateway tools; `loadErrorText` for both. `mcp/ProfilesTab.svelte:187`: after "Make
    default", `await tick()` and focus the profile's ⋯ button. Sweep `mcp/` for the rule-7 patterns.

**Verify:** `cargo test -p otto-mcp`, `cargo test -p otto-channels`, `cargo test -p ottod --lib mcp_tools`,
`cargo test -p otto-server --test it public_entry_gates:: database_changes_api:: mcp_auto_approve:: mcp_profiles_edges:: foundation_integrity::`,
clippy, `npm run check`.

## P06: Git, panels, mission control, personal agents

**Owns:**
- `ui/src/modules/git/**`, `ui/src/lib/listWindow.svelte.ts` (new), `ui/src/lib/tableWindow.svelte.ts`,
  `ui/src/lib/api/git.ts` (new), `ui/src/lib/stores/git.svelte.ts`, `ui/src/lib/uiCommands/git.ts`, `ui/src/lib/gitLivePaths.ts`
- `ui/src/modules/panels/**`, `ui/src/modules/mission-control/**`
- `ui/src/modules/personal-agents/**`, `ui/src/modules/assistant/**`,
  `ui/src/lib/stores/{personalAgents,assistant}.svelte.ts`
- `crates/otto-git/**`, `crates/otto-assistant/**`
- `crates/otto-server/src/routes/{mission,personal_agents}.rs`, `crates/otto-server/src/routes/personal_agents/**`
- `crates/otto-server/tests/{personal_agent_policy,personal_agents_owner}.rs`
- `docs/features/{git,mission-control,personal-agents}.md`

**Findings:** architecture-1-09 (major), architecture-6-04, architecture-6-05, accessibility-responsive-6-06,
accessibility-responsive-6-04 (GitTabs), accessibility-responsive-6-01 (git part), ux-design-6-02 (PrList,
MissionControl), ux-design-6-04 (AgentPage), ux-design-6-07 (git, WorkItemDetail), ux-design-6-08 (git, FileTree,
AgentEditSheet), architecture-1-13 (git, assistant parts), error-handling-6-07, error-handling-6-05 (CanvasPanel),
maintainability-6-03 (mission, personal_agents), maintainability-6-05 (CanvasPanel importer), architecture-6-01
(personal_agents_engine part).

0. **Step 0 (arch-6-05).** Move `ListWindow`/`listRange` verbatim to `lib/listWindow.svelte.ts`; the old
   `git/list-window.svelte.ts` and `git/list-range.ts` re-export it.
1. **arch-6-04 then arch-1-09 (major).** Add `lib/api/git.ts` with typed `refs/status/push/pushTarget/checkout/stage/prs`
   functions. Put the post-mutation contract in the git store: `git.push`/`git.checkout` always `setStatus` and bump
   `refsRev`. Make `pushFlow.runPush` the UX wrapper over `git.push` and route the agent command and `CreatePr.svelte`
   through it. One `git.refs(repoId)` loader with one error policy. Then split `GraphView.svelte` (4,887 lines) into
   `git/graph/RefSidebar.svelte`, `git/graph/actions.svelte.ts` (checkout, merge, rebase, tag, revert, push,
   delete…) and further pieces until every file is under 800 lines and the ratchet entry can drop. Behaviour-identical;
   no keyboard-model redesign of the commit list (that needs a design decision).
2. **arch-6-05.** Make `TableWindow` the same class as `ListWindow` with a `selfScroll` option (shared range and
   measure). Give it the `findRows` provider and wire it into `WipPanel`, `panels/FileTree`, `BlamePanel` and the
   DiffViewer nav. Leave `GridView` (2-D) and `DiffViewer` (variable height) as documented exceptions using the
   shared helpers. (P04 does the k8s table window, P07 the SchemaTree, P03 the guideline sentence.)
3. **a11y-6-06.** `panels/FileTree.svelte`: compute `tabRow` among the mounted rows (fall back to the first row of
   `treeRange`), or scroll `activeRow` back into range when the tree gets focus. Port `Hierarchy.svelte:165-190`'s
   ←/→ parent/child and Home/End. Unit-test the key model. `Cannot open ${path}` → `Couldn’t open ${path}`.
4. **a11y-6-04 (GitTabs).** Copy TabBar's `moveTab` plus the ⌥⇧←/→ handler and its Move left/right menu items into
   `git/GitTabs.svelte`.
5. **ux / a11y git sites.** `PrList.svelte:194` → `LoadState` (keep `errorTitle` as `what`), `:90` "Couldn’t load
   pull requests"; `GraphView.svelte:1736` and `pushFlow.ts:29` "couldn’t"; clipboard next step at
   `GraphView.svelte:942`, `RepoView.svelte:225`; `RecoveryTools.svelte:133` and `PrMergeModal.svelte:233,253` →
   `announce()`.
6. **Mission control.** `MissionControlPage.svelte:383` → `LoadState`; `WorkItemDetail.svelte:317` gets the missing
   `else` (toast with the next step). **maint-6-03:** `routes/mission.rs:112-123` `build_gate` and
   `routes/personal_agents.rs:868-879` `chat_lock` → `static KeyedLocks<String>`.
7. **Personal agents.** `AgentPage.svelte:451`: a `LoadState` branch on `personalAgents.agentsError` with Retry
   before "Agent not found"; clear `lookedUp` on failure. `AgentEditSheet.svelte:220` "couldn’t". The OTTO_E2E check
   at `otto-assistant/src/personal_agents_engine.rs:1230` → `otto_core::e2e::isolated()`; the
   `otto-assistant/src/ctx.rs:91` doc → `otto_sessions::submit::submit_prompt` (P02 hand-off). 1-13: remove the
   `modules/` import from `lib/stores/assistant.svelte.ts:19` and `uiCommands/git.ts:19`.
8. **EH-6-07.** `otto-git/src/local.rs:2832`: fall back only when `head_err` shows an unborn HEAD ("ambiguous
   argument 'HEAD'" / "unknown revision"); otherwise return `upstream_err`; propagate fallback failures with `?`. Test.
9. **Panels.** `panels/CanvasPanel.svelte:186` → `loadErrorText`; repoint its canvas types import to
   `lib/canvas/types` (maint-6-05).
10. **Sweep** `git/`, `panels/`, `mission-control/`, `personal-agents/` and `assistant/` for the rule-7 patterns.

**Verify:** `cargo test -p otto-git`, `cargo test -p otto-assistant`, `cargo test -p otto-server --lib mission personal_agents`,
clippy, `npm run check`, `npx vitest run unit/git* unit/p06-*`.

## P07: Database Explorer and brokers

**Owns:**
- `ui/src/modules/database/**`, `ui/src/lib/stores/database.svelte.ts`, `ui/src/lib/stores/database/**`,
  `ui/src/lib/database/**`, `ui/src/lib/stores/{dbPrefs.svelte.ts,db-result-budget.ts}`, `ui/src/lib/db-view-prefs.ts`,
  `ui/src/lib/sql-dialects.ts`, `ui/src/lib/uiCommands/database.ts`
- `ui/src/modules/brokers/**`, `ui/src/lib/stores/brokers.svelte.ts`, `ui/src/lib/uiCommands/brokers.ts`
- `crates/otto-dbviewer/**`, `crates/otto-brokers/**`, `crates/otto-server/src/{database_changes,db_assist,db_drafter}.rs`,
  `crates/otto-server/src/routes/database_changes.rs`, `crates/otto-state/src/{database_changes,database_changes_tests,db_explorer}.rs`
- `docs/features/{database-explorer,message-brokers}.md`

**Findings:** architecture-1-10 (major), data-integrity-6-01 (major), ux-design-6-01, ux-design-6-02
(StructureView), accessibility-responsive-6-03 (AggregateBuilder), accessibility-responsive-6-01 (database, brokers
parts), ux-design-6-07 (brokers), ui-correctness-6-01, backend-correctness-6-03, backend-performance-6-01,
backend-performance-6-02, maintainability-6-04 (DiagramView), maintainability-6-02 (EditFlow),
maintainability-6-07 (dbviewer, sql-builder), architecture-6-05 (SchemaTree part), architecture-1-13 (brokers part).

1. **arch-1-10 (major).** Continue the `DatabaseStore` split behind the unchanged facade, with `connEpoch` as a
   shared token: connections/keep-alive/SSH tabs into `lib/stores/database/connections.svelte.ts`, the schema tree
   into `schemaTree.svelte.ts`, then object ops, query runs/plans and filters. The class keeps delegating getters so
   no caller changes. Target: `database.svelte.ts` well under the 800-line cap.
2. **DI-6-01 (major).** `database/EditFlow.svelte.ts:737` `runReview`: treat `res.errored || res.more_results?.some(r => r.errored)`
   as a failure; show the failing statement and message, clear only the drafts whose statements completed, keep the
   modal open. On SQL engines with transactional DML, make the review atomic by wrapping the batch in
   `BEGIN … COMMIT`; first verify the runner stops at the first error and rolls back (BEGIN marks the connection
   `close_on_drop`, `types.rs:1459`). If it does not, add stop-on-error rollback for a BEGIN-wrapped batch inside
   `otto-dbviewer` with no wire change. Test: three statements, the middle fails, nothing is committed and the
   modal stays open.
3. **ux-6-01.** `ResultsGrid.svelte:1798-1849`: hoist the 7 imports into module-level `lazyComponent()` and render
   each through `<LazyMount quiet what=… onerror={…}/>`, where `onerror` resets the flag that opened it.
4. **UC-6-01.** `lib/stores/database/assist.svelte.ts:229`: a turn generation bumped by `closeAssist`/`openAssist`,
   abort `assistAbort` in both, discard stale responses with `dbAssistClose(id, resp.assist_id)`, guard `finally`,
   and a discard-on-sight set for `db_assist_session_started`. Unit test: close mid-POST.
5. **a11y-6-03 (Aggregate).** `AggregateBuilder.svelte:185`: stable stage ids, focus follows the moved stage,
   `focusAfterRowRemoval` after remove.
6. **BC-6-03.** `drivers/mongodb.rs:3063`: keyset only when the cursor is an ObjectId (or use the BSON type-order
   `$or` filter); otherwise skip-paging. Test with mixed string and ObjectId ids across a page boundary.
7. **BP-6-01.** `export.rs:315`: bounded async hand-off (`mpsc::channel(8)`, 64 KiB chunks); `export_to_path` owns the
   File in one `spawn_blocking`; `/db/export` streams the bounded receiver (replaces `ChannelWriter` and the
   unbounded channel). Test that a slow consumer bounds memory.
8. **BP-6-02.** `MongoDriver::evict_idle` sweeps `self.completions` and invalidates evicted keys.
9. **Small items.** `StructureView.svelte:658` → `LoadState` (ux-6-02). `DiagramView.svelte:141` NUL → `\u0000`
   (maint-6-04; tell P03 when done). Delete `mongoIdFilter` (`EditFlow.svelte.ts:833`) and point
   `query-filter.ts:128` at `mongoIdFilterFor` (maint-6-02). Fix `sql-builder.ts:7` (`escapeLike` + `stringLiteral`),
   `multirun/params.rs:4` and `errors.rs:2` paths (maint-6-07). `MultiRunDialog.svelte:385`, `DatabasePage.svelte:1426`,
   `brokers/TopicDetail.svelte:748`, `brokers/GroupsTab.svelte:497`, `brokers/ResetWarning.svelte:18-19` →
   `announce()` or always-mounted regions (a11y-6-01). `TopicDetail.svelte:653` clipboard next step. 1-13: remove the
   `modules/` import from `uiCommands/brokers.ts:28`.
10. **arch-6-05, SchemaTree.** After P06's step 0, re-express the `SchemaTree.svelte:277-314` window over the
    shared range.
11. **Sweep** `database/` and `brokers/` for the rule-7 patterns.

**Verify:** `cargo test -p otto-dbviewer`, clippy, `npm run check`, `npx vitest run unit/database* unit/p07-*`.

## P08: Swarm access control, contracts, proof, insights, settings

**Owns:**
- `crates/otto-swarm/**`, `crates/otto-server/src/swarm_host.rs`, `crates/otto-state/src/swarm.rs`, `crates/otto-state/src/swarm/**`
- `crates/otto-state/migrations/0196_*.sql` (new)
- `crates/otto-server/tests/{swarm_identity,swarm_router,swarm_scope_api,proof_assemble_scope,admin_sessions}.rs`, `crates/otto-server/tests/unit/proof_assembly.rs`
- `ui/src/modules/swarm/**`, `ui/src/lib/stores/swarm.svelte.ts`, `ui/src/lib/uiCommands/swarm.ts`
- `crates/otto-server/src/proof.rs`, `crates/otto-server/src/routes/{proof,proof_pack}.rs`, `ui/src/modules/proof/**`, `ui/src/lib/stores/proof.svelte.ts`
- `crates/otto-insights/**`, `ui/src/modules/insights/**`
- `ui/src/modules/settings/**`, `crates/otto-server/src/routes/admin_sessions.rs`, `crates/otto-state/src/sessions.rs`
- `docs/contracts/**`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/types/**`, `crates/otto-core/src/api/**`
- `docs/features/{agent-swarm,proof-packs,insights,rbac-multiuser-sharing,daemon-http-api}.md`

**Findings:** access-control-6-01 (major), access-control-6-02 (major), access-control-6-03, access-control-6-04,
concurrency-async-6-03, data-integrity-6-02, data-integrity-6-04, test-quality-6-05, architecture-4-01r (swarm
part), architecture-1-13 (swarm parts), api-contracts-6-01 to 6-06, error-handling-6-02, error-handling-6-03,
error-handling-6-06 (contract part), backend-performance-6-05, maintainability-6-03 (proof part),
maintainability-6-07 (api.md, swarm parts), accessibility-responsive-6-01 (settings, proof parts), ux-design-6-08
(GoalsPanel).

0. **Step 0.** Add `mcp_skipped?: {server: string; reason: string}[]` to the session meta type and its api.md row.
1. **TQ-6-05.** In `tests/swarm_identity.rs`: seed a live agent session bob created for agent dev; assert
   `find_agent_session(ws,'dev','alice')` is `None` and bob's lookup returns it. Add a `SwarmHost` double recording
   `manager().create`'s creator and assert it equals `acting_user(&task)`.
2. **AC-6-01 (major).** In `create_agent` (`otto-swarm/src/http.rs:239`) apply `update_agent`'s creator / ws Admin /
   root predicate to a non-empty `scope_md`, `specialization`, `soul_md`, schedule directive or `cwd_mode`, and to a
   provider or model other than the swarm default. A plain hire stays Editor. Tests: non-creator hire with scope or
   directive → 403, plain hire → 200, Admin and creator → 200. **AC-6-03:** `AgentEditor.svelte` (and the hire form)
   derive `canEditIdentity`; when false, identity fields and the directive are read-only with an owner note. Test.
3. **AC-6-02 (major) with CA-6-03 and DI-6-02.** Migration `0196`: nullable `prompt_updated_by`/`prompt_updated_at`
   on `swarm_projects` and `swarms` (rollback-safe). Stamp them from the project, swarm, project-goal and
   standing-goal writes **in the same UPDATE/transaction as the content** (optional field on `TaskPatch`/`GoalPatch`
   with COALESCE); `create_goal`/`update_goal` set `swarm_goals.updated_by` and the task's `prompt_updated_by` in one
   transaction; copied goals carry the template author. Take `operation_guard(&goal.swarm_id)` in goal create, update
   and delete. `acting_user` uses the most recent person-editor across task, project and swarm, before the
   originator. Tests: both turn kinds act as the latest editor; a failed editor write leaves no content change.
   Update `agent-swarm.md` §5.3/§9 and api.md.
4. **AC-6-04.** `RunsList.svelte:147` and the Run inspector show "Open session" only when `ws.getSession(r.session_id)`
   resolves or the caller is root or ws Admin; else a disabled button titled with the acting user. Unit-test the
   predicate.
5. **arch-4-01r, swarm part.** Once P01's run-active guard exists on `SubmitOpts`, replace the hand-built wait → paste
   → re-inject in `runtime/host.rs:105-107`, `agent_run.rs:355-401` and `run.rs:854-898` with `submit_prompt` plus
   the guard. Fix the stale `swarm_runtime` pointers in `runtime/run.rs:6` and `wake.rs:13` (maint-6-07).
   `GoalsPanel.svelte:74` "Verification couldn’t start." 1-13: remove the `modules/` imports from
   `lib/stores/swarm.svelte.ts:8,31` and `uiCommands/swarm.ts:12`.
6. **EH-6-02.** `assemble_diff` (`proof.rs`) returns `Assembled | NotARepo | DiffFailed{commands}`; propagate the
   Conflict as the response error; add an `assembled` summary to the assemble response (api.md + types). `ProofPage`
   says "No git repository at that folder — nothing to assemble" or lists the failed commands. **maint-6-03:**
   `ProofLocks` becomes `KeyedLocks<String>` behind the same `new_locks()`/alias (no `state.rs`/`boot` edit). Tests.
7. **EH-6-03 and DI-6-04.** Persist the last scheduled attempt per cadence (`{period, attempts, last_error,
   gave_up}`) in the insights data dir and serve it inside the existing config response (no new route). `InsightsPage`
   renders "Scheduled daily reports are on, but the last 3 runs didn’t produce a report: …" with Run now, and "off"
   only when the config says off. `write_config` and the manifest writers use a unique temp name
   (`<name>.<pid>-<seq>.tmp`). Tests for both.
8. **BP-6-05.** Admin session overview defaults to live sessions plus recent exited ones with keyset pagination on
   `(created_at, id)` and an "include archived" query flag; `AdminSessions.svelte` gets the toggle and "Load more".
   Document in api.md and types. Test.
9. **api-contracts 6-01 to 6-06.** Apply each row correction exactly as the digest lists (improvement runs, room
   members, session create, env-key rule, busy/timeout statuses, scheduled-task pause, shutdown 409 and `{killed}`,
   swarm row 79 identity and 409/400 split, CP46 400s and 409, `ws.md:967` → `GET /workflow-runs/{id}/progress`,
   `ApiAutomation[]`), plus `api.md:200` → `crate::budget::check_budget` (maint-6-07).
10. **a11y-6-01 sites.** `settings/InsightsSettings.svelte:188,195`, `settings/Users.svelte:525,660`,
    `settings/Onboarding.svelte:262`, `settings/GitBackup.svelte:172-173`, `settings/Daemon.svelte:230`,
    `settings/ContextSoul.svelte:265`, `proof/ProofPage.svelte:1263` → `announce()`. Sweep `settings/`, `swarm/`,
    `proof/` and `insights/` for the rule-7 patterns.

**Verify:** `cargo test -p otto-swarm`, `cargo test -p otto-insights`, `cargo test -p otto-state --test migration_compat`,
`cargo test -p otto-server --test it swarm_identity:: swarm_router:: proof_assemble_scope:: admin_sessions::`,
`cargo test -p otto-server --lib proof`, clippy, `npm run check`.

## P09: Product and issues

**Owns:**
- `ui/src/modules/product/**`, `ui/src/lib/stores/{product,mockup-assist}.svelte.ts`
- `crates/otto-product/**` except `src/design_scene3d.rs` (P02), `crates/otto-issues/**`
- `crates/otto-state/src/{product,product_attachments,product_chat,product_discovery,product_mockup,product_refinement,product_watch}.rs`
- `docs/features/{product,discovery-chat,jira-confluence,linear}.md`

**Findings:** architecture-5-04, ux-design-6-03, ux-design-6-04 (MockupAnnotations, LinkedCanvases),
accessibility-responsive-6-04 (TestCasesTab), accessibility-responsive-6-10, ux-design-6-07 (InjectTab),
maintainability-6-05 (product importers), architecture-1-13 (product, mockup-assist stores),
backend-correctness-6-01 (major), backend-correctness-6-02 (major), backend-correctness-6-04 (refine part),
concurrency-async-6-04, backend-performance-6-04, ui-correctness-6-03.

1. **arch-5-04.** Split `product/OverviewTab.svelte` (2,971 lines) into `JiraIssueSection.svelte` (issue load and
   dev status, `:89`, `:95`, `:253-286`), `DraftEditor.svelte` and `StoryTranscripts.svelte` (`:175`, `:312-322`),
   until every file is under 800 lines. Behaviour-identical.
2. **ux-6-03.** `AnalysisTab.svelte`: `detailLoading`/`detailError`, a `LoadState` with Retry before the `!running`
   branch, allow a re-select when `activeDetail` is null, drop the toast. Test.
3. **ux-6-04.** `MockupAnnotations.svelte` and `LinkedCanvases.svelte`: a `loadError`, a compact `LoadState` with
   Retry, and a Skeleton while the picker loads. Repoint `LinkedCanvases.svelte:13` and `product/types.ts:5-6` to
   `lib/canvas/types` (maint-6-05).
4. **a11y-6-04 / 6-10.** `TestCasesTab.svelte`: "Move up"/"Move down" in each case's actions (and ⌥↑/⌥↓ on the focused
   card) that call `persistOrder()` and keep focus on the moved card; `aria-hidden` the handle glyph.
   `jira/IssueEditor.svelte`: `role="group" aria-label={ef.name}` on the multiselect; re-run `focusFirstField` when
   `assignablesLoading` turns false.
5. **UC-6-03.** `MockupViewer.svelte:110`: keep the last good SVG while re-rendering, swap only if `myGen === gen`,
   pass `isStale` to `renderMermaid`, guard `finally`/`catch`, debounce the live preview ~200 ms. Test.
6. **BC-6-01 (major).** `otto-product/src/chat.rs:668`: find actions with `extract_json_where(raw, |v| actions is a
   non-empty array)` and splice out only that span. Test: an example fence before the actions fence, with prose after
   both, keeps the example and the prose and still finds the actions.
7. **BC-6-02 (major).** `otto-issues/src/confluence.rs:1054`: a private `storage_to_markdown_depth`; past ~16 nested
   panels emit `strip_tags` + `decode_entities` of the body; reuse the caller's lowercase slice. Test: 5,000 nested
   info macros return a string.
8. **BC-6-04, refine part.** `refine.rs:572-593` `parse_turn`: `extract_json_where` with a string `reply`. Decoy test.
9. **CA-6-04.** Serialise publishes per story with a `KeyedLocks<String>` and re-read the binding under the lock,
   refusing a second publish of an already-rebound draft; run the remote call and its local record in a spawned task
   the handler awaits. Test: drop the future after the remote create; the binding is recorded.
10. **BP-6-04.** `ProductRepo::list_testcases_for_runs(&[Id])`: one `WHERE run_id IN (…)` query chunked at 500,
    grouped in memory. Test.
11. **Small items.** `InjectTab.svelte:62` clipboard next step. 1-13: remove the `modules/` imports from
    `lib/stores/product.svelte.ts:72` and `mockup-assist.svelte.ts:21`. Sweep `product/` for the rule-7 patterns.

**Verify:** `cargo test -p otto-product`, `cargo test -p otto-issues`, clippy, `npm run check`,
`npx vitest run unit/product* unit/p09-*`.

## P10: Home, 3D, Design Hall, browser, workbench

**Owns:**
- `ui/src/modules/home/**`, `ui/src/lib/uiCommands/home.ts`
- `ui/src/lib/three/**`, `ui/src/lib/design/**`
- `ui/src/modules/design-hall/**`
- `ui/src/modules/browser/**`, `ui/src/lib/stores/{browser,browserLive}.svelte.ts`, `ui/src/lib/uiCommands/browser.ts`
- `ui/src/modules/workbench/**`
- `docs/features/{design-hall,home,browser,workbench}.md`

**Findings:** coordinator: ClassroomsBox split, accessibility-responsive-6-05, accessibility-responsive-6-08,
accessibility-responsive-6-09 (SchoolBar), accessibility-responsive-6-02 (WorkbenchPage),
accessibility-responsive-6-01 (browser part), architecture-5-05r, maintainability-6-06, ux-design-6-07 (design-hall),
ux-design-6-08 (browser, workbench), architecture-1-13 (home, browser parts), ui-performance-6-01,
ui-performance-6-02, ui-correctness-6-02, error-handling-6-01, error-handling-6-05 (design-hall parts).

1. **Split `home/boxes/ClassroomsBox.svelte`** (832 lines) below 800: move the load/refresh logic into
   `boxes/classrooms/load.svelte.ts` and one view piece into `classrooms/`. Behaviour-identical, then fix
   **UC-6-02** there: in `load`'s catch `if (signal?.aborted) return false;` and skip `loaded = true` for an aborted
   run, as K8sBox does. Test.
2. **a11y-6-05.** `design-hall/site/Inspector.svelte`: focus the new view's heading (`tabindex="-1"`) or its first
   field (`focusFirstField`) after an inspector-driven selection change; on back, focus the row of the block just
   left; link-list rows keyed stably with per-row names and `rowFocus.ts` removal focus.
3. **a11y-6-08.** `lib/design/scene3d/Scene3DViewport.svelte:818`: gate shortcuts on focus only
   (`host.contains(e.target)`), or bind on `host`. Test that hover alone does not delete.
4. **a11y-6-09 (SchoolBar).** After a crumb navigates up, `refocusStage()` or focus the new last crumb.
5. **UP-6-01 with maint-6-06 and arch-5-05r.** Re-arm `requestAnimationFrame` only while there is work, and let
   `kick()`/`invalidate()` restart the loop:
   - `home/school/scene.ts`: reschedule only if `cam.tween || keys.size > 0 || (openRoom && !reduced) || kicks.size > 0 || doorsMoving`;
     `openDoor` (`:358-364`) and the other `dirty = true` paths call `kick()`.
   - `design-hall/studio3d/embed.ts`: re-arm at the end of `frame` only on `needsFrame`, a tween, autoRotate, tilt
     converging or controls damping; start a frame from the IntersectionObserver and pointer handlers. Adopt
     `onContextLost` (with `lossBudget`) from `lib/three/stage.ts`.
   - `Scene3DViewport`: `invalidate` requests a frame; re-arm only during a tween, camera tween or turntable.
   - All three use `lib/three/stage.ts` `watchVisibility` for the off-screen/hidden pause (replace `scene.ts:589-599`,
     `:762` and `embed.ts:166`, `:366-370`). Add a unit test for `watchVisibility`'s flip semantics and one per loop
     that an idle loop stops re-arming.
6. **UP-6-02.** `design-hall/studio3d/sources.ts:32`: an LRU of 8-16 refs that revokes an evicted URL once no live
   scene holds it (acquire/release from `embed.destroy()`, `ArtifactStage` and Studio3D teardown); key `@latest` by
   the resolved `versionId`; add `releaseModelCache()` called from the Design Hall teardown. Test.
7. **EH-6-01.** `design-hall/assist/model.ts:146` `mergeTurns`: on an authoritative resync, mark a local non-terminal
   turn the server no longer knows as failed ("Otto restarted while this turn was running; the turn was lost. Ask
   again."). `Studio3D.poll`: `!t` after a successful poll clears the agent and shows the error. Tests.
8. **EH-6-05 (design hall).** `loadErrorText(e)` in `CollectionView.svelte:98`, `LearnedCard.svelte:39`,
   `CompareModal.svelte:82`, `assist/OttoPanel.svelte:149`. `site/siteApi.ts:119` `resolveBrand` returns `{ error }`
   apart from `note`; the Inspector shows "Couldn’t load the brand kit — the preview uses the default palette" with
   Retry (`loadBrand()`), and the contrast audit is skipped while the kit failed.
9. **Small items.** `design-hall/brand/edit.ts:15` clipboard next step. `browser/live/connection.ts:108` and
   `workbench/preview/ImagePreview.svelte:99` "couldn’t". `browser/ReaderView.svelte:231` → `announce()`.
   `workbench/WorkbenchPage.svelte:572` → `<Skeleton label="the file">`. 1-13: remove the `modules/` imports from
   `uiCommands/home.ts:11-12` and `uiCommands/browser.ts:17`.
10. **Sweep** `home/`, `design-hall/`, `browser/` and `workbench/` for the rule-7 patterns.

**Verify:** `npm run check`, `npx vitest run unit/p10-* unit/school*`.

---

## Ordering and contention

- Step 0 items first (P01 e2e predicate, P02 Cargo dep + re-export removal, P03 announcer, P06 listWindow, P08
  `mcp_skipped`). Each takes minutes.
- Late items that wait on others: P03 check.sh ratchets and canvas-shim deletion (after P01/P02/P06/P07/P09 report
  their parts), P04 guard rules (after its own sweeps), P08 swarm submit conversion (after P01's guard),
  P03 webhook-guard adoption (after P05), P04/P07 listWindow re-expressions (after P06 step 0).
- Builds: P01, P02, P03 and P05 are cargo-heavy and all touch otto-server. Keep to one heavy command at a time per
  agent, and prefer `cargo check -p <crate>` while iterating.

## Deferred

- **concurrency-async-1-05** (plugin `free_port` TOCTOU): the fix changes the plugin protocol (the plugin binds `:0`
  and reports its port, or `/health` echoes the token). The coordinator keeps it deferred.
- **architecture-2-06** (extract an `otto-rooms` crate): a multi-day crate extraction (rooms, recap, recap_engines and
  game_rooms behind a new ctx seam) whose router assembly and `lib.rs` wiring sit in files other packages own this
  round. Step 1 (shared capability tokens) landed in iteration 5; the crate move goes to the next iteration.
