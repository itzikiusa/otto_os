# Maintainability review: iteration 6, final (2026-10-23)

Lens: dead code, unused exports, routes and settings, duplicated helpers,
misleading names and comments, stale TODOs that point at finished work,
idioms that differ from the surrounding code, and code that is more clever
than it needs to be.

Worktree: `fix/quality-iter6-20261023` at `2065f00e9` (all fix waves of
iteration 6; 22 commits, 864 files vs `origin/main`). The review was
read-only. It used git, grep, rg, perl, awk and small shell/Python scanners.
Nothing was built and no tests were run.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per
minor and 0.03 per nit, for each confirmed open finding.

| Severity | Count | Deduction |
|----------|------:|----------:|
| blocker  | 0 | 0.00 |
| major    | 0 | 0.00 |
| minor    | 6 | 0.60 |
| nit      | 3 | 0.09 |
| **total**|   | **0.69** |

**Score: 10.0 − 0.69 = 9.31 / 10.** Earlier scores: iteration 1 7.25,
iteration 2 8.28, iteration 3 8.65, iteration 4 9.39, iteration 5 9.34,
iteration 6 wave 0 9.34.

All eight wave-0 findings are fixed. The score still does not move up,
because this iteration's large refactors (the otto-rooms and session-turn
extractions, the transcript-roots consolidation, and the splits of
GraphView, WorkflowsPage, OverviewTab, the database store and the API-client
store) left new residue of the same kinds: comments pointing at the old
homes, named re-export shims, compatibility aliases, and helpers copied into
both halves of a split. The pass also ran the workspace-wide stale-comment
scan that wave 0 listed as not covered; it found three older stale doc
comments, which are folded into 6f-01.

## Status of the wave-0 findings

| Id | Wave-0 severity | Status now | Evidence |
|----|-----------------|------------|----------|
| 6-01 | minor | **Fixed** | The `pub use otto_sessions::submit::{…}` block is gone from `otto-review/src/session.rs`, and `pub use otto_review::session as review_session` is gone from `otto-server/src/lib.rs` (it was line 71 on `origin/main`). `git grep "review_session::"` finds no code path. `session_input.rs:232,305` and `otto-assistant/src/ctx.rs:91` name `otto_sessions::submit::submit_prompt`. `submit.rs:10-17` no longer claims a re-export. Five comments still name the deleted alias; they are carried in **6f-01** as a new stale-pointer item, not as residue of this finding, because 6-01 listed only the callers and the two docs. |
| 6-02 | nit | **Fixed** | `mongoIdFilter`, `Kind::k8s_kind` and `wait_for_tui_strict` have no definition left (`rg -a` over `ui/` and `crates/`). `query-filter.ts:128` names `mongoIdFilterFor`. `resources.rs:117-119` documents `from_k8s_kind` as mapping the Argo `status.resources[].kind` workload kinds. |
| 6-03 | minor | **Fixed** | All five registries use `otto_core::keyed_lock::KeyedLocks`: `otto-agent-run/src/session_turn/lifecycle.rs:20` (`SESSION_LEASES`), `proof.rs:27-30` (`ProofLocks = Arc<KeyedLocks<Id>>`), `routes/mission.rs:114` (`BUILDING`), `routes/personal_agents.rs:867` (`CHAT_LOCKS`) and `telegram.rs:516` (`POLLER_LOCKS`, which now prunes). None of the five paths is on `KEYED_LOCK_ALLOW` (`scripts/check.sh:220`). |
| 6-04 | minor | **Fixed** | A perl scan of every tracked `.ts/.svelte/.rs/.js/.mjs/.sh` file finds no raw C0 byte. `check.sh:246-266` now fails on them. |
| 6-05 | minor | **Fixed** | `modules/canvas/types.ts` is deleted, and all importers use `lib/canvas/types` (`CanvasPanel.svelte:21`, `LinkedCanvases.svelte:13` and the canvas module files). `orchestrate.ts:20` imports `plural` privately and no longer re-exports it. The ratchet regex is `export (type )?\* from` (`check.sh:298`), and the count is 3 (`swarm/types.ts:5`, `product/types.ts:5-6`). The named re-exports this iteration added are a new finding, **6f-02**. |
| 6-06 | minor | **Fixed** | `watchVisibility` is used by `studio3d/embed.ts:430`, `home/school/scene.ts:599` and `Scene3DViewport.svelte:898`, and tested in `ui/unit/p10-three-loop.test.ts:85-104`. |
| 6-07 | minor | **Fixed** | All ten pointers are fixed. `api.md` no longer cites `routes::usage::check_budget`. `multirun/params.rs:4` and `errors.rs:2` cite `ui/src/lib/database/…`. There is no `lib/fuzzy`, `cache_exempt_kind` survives only in the explicit removal note at `workflow_engine/run/schedule.rs:163`, and `process_event`, `expect_request_id`, `likeLiteral` and `swarm_runtime` are gone. |
| 6-08 | nit | **Fixed** | `shooter/meshes.ts:10` imports `canvas` from `../common/textures`, and `materialTexture` calls `canvas(128, 128)` (`:189`). The private copy is gone. |

Totals: 8 fixed, 0 partly open, 0 found to be invalid.

## Areas covered

- **Every wave-0 finding**, re-checked at each listed location (see the table
  above).
- **Moved and deleted files in the iteration diff** (35 renames/deletes:
  `otto-server/src/{rooms,game_rooms}/*` → `otto-rooms`,
  `agent_session_{lifecycle,reply}.rs` → `otto-agent-run/src/session_turn/`,
  the UI `modules → lib` moves, deleted shims and tests). Each old path was
  searched in `crates/`, `ui/`, `docs/` (except reviews), `AGENTS.md`,
  `README.md` and `scripts/`.
- **Removed definitions** (1,076 names deleted in the diff; 82 have no
  definition left). Every remaining mention was checked.
- **Repo-path citations** (`crates/…`, `ui/…`, `docs/…`, `scripts/…`) across
  all tracked sources and docs: 1,995 citations, 87 unresolved. Each was
  triaged; most are examples, `ui/`-relative script paths or `.json` read as
  `.js`.
- **The stale-comment scan, workspace-wide** (this was "not covered" in wave
  0):
  - Rust: backticked `crate::…`/`otto_*::…` paths and backticked `fn()`
    calls in comments, against every definition in `crates/`.
  - UI: backticked camelCase identifiers in comments in every
    `ui/src/**/*.{ts,svelte}`, against every definition.
- **New exports and new pub fns.**
  - 230 new UI exports, checked for an importer outside their own file.
  - 410 new Rust `pub`/`pub(crate)` fns, checked for references and for
    test-only references.
  - Unused class/store members in every split store
    (`apiClient{,/*}`, `database{,/*}`, `git/graph/*`, `runSync`,
    `listWindow`).
  - Unused local functions in all 417 changed UI files.
- **Duplicated helpers.**
  - Every new UI function name was compared with every other definition in
    `ui/src` (300 names); every new Rust fn name was compared across
    `crates/` (740 names). Trait/impl pairs and generic names were triaged by
    reading the bodies.
  - Same-named helpers on both sides of each split: GraphView and
    `git/graph/*`, WorkflowsPage and RunMonitor, OverviewTab and its four
    extracted parts, and the database and API-client stores and their
    sub-stores.
- **All module → lib re-exports** (`export *`, `export type *` and named
  `export { … } from`), and the Rust `pub use` aliases added by the
  extractions.
- **New routes** (10), each checked for a UI caller and a contract row.
- **Ratchet allow-lists and baselines** (`KEYED_LOCK_ALLOW`, the shim ratchet,
  `ui-guards-baseline.json` with 437 files, `loc-baseline.json`), checked for
  entries that point at deleted files. None do.
- **`AGENTS.md` crate and UI-module maps** against `crates/` and
  `ui/src/modules/`. Both match, including the new `otto-rooms` crate.
- **TODO/FIXME/HACK markers and "for now/interim/not migrated yet" notes**
  added in the diff. There are none.
- **Identifiers cited in the 31 changed `docs/features`, `docs/contracts` and
  `docs/design` files.** All resolve. The two that do not
  (`submit_run_prompt`, `send_run_input`) are cited on purpose as removed.
- **Unit-test file naming** in the 46 test files this iteration added.

## Areas not covered

- **Contract-only TS `*Req`/`*Resp`/`*Query` types with no UI importer**, for
  example `AdminSessionsQuery` (`lib/api/types/auth.ts:113`). This is still a
  policy decision, not a finding.
- **Host/trait-only exported types** (`*Host` interfaces of the split stores)
  that are used only in their own signatures. They are not dead.
- **Rust struct fields read only by tests.** This needs a typed scanner.
- **The behaviour prose in `docs/features/*.md`.** Only paths and identifiers
  were checked.
- **The `kart`/`shooter` render internals**, apart from their exports and
  duplicate helpers.
- **The deliberately different keyed-lock maps still on `KEYED_LOCK_ALLOW`**
  (single-flight with a cache, or with a per-service lifetime). These were
  judged in wave 0 and were not re-judged here.

---

## Findings

### maintainability-6f-01: Comments, docs and a shipped agent skill still point at names and paths this iteration deleted, plus three older stale doc comments (minor)

Each of these was checked: the cited name or path has no definition anywhere.

- **The deleted `crate::review_session` alias** (removed with 6-01):
  - `crates/otto-sessions/src/manager.rs:34`:
    `see otto_server::review_session::review_skills_extra_dirs`.
  - `crates/otto-sessions/src/manager/tests/mod.rs:3747`: the same pointer.
  - `crates/otto-server/src/workflow_engine.rs:2908`:
    *"Same skip as `review_session::stop_review_sessions`"*.
  - `crates/otto-server/src/assistant_host.rs:4` and `:279`:
    *"session driving via `agent_run` / `review_session`"*.
  - Both functions now live only at `otto_review::session::…`
    (`otto-review/src/session.rs:421`, `:943`).
- **The deleted `ui/src/modules/canvas/types.ts`** (deleted with 6-05). It is
  still named as the source of truth in four places:
  - `docs/features/canvas.md:64` and `:407`.
  - `docs/contracts/api.md:3848`.
  - `crates/otto-skills/assets/skills/development/otto-canvas/references/scene-schema.md:4`:
    *"This mirrors the UI's single source of truth,
    `ui/src/modules/canvas/types.ts`"*. This skill reference is shipped to
    agents, so an agent that follows it looks for a file that is not there.
  - The file is now `ui/src/lib/canvas/types.ts`.
- **`crates/otto-usage/src/tailer/driver.rs:1392` and `:1406`.** Both
  `#[allow]` comments say *"list_claude_files/list_codex_files only run
  inside spawn_blocking"*.
  - `list_codex_files` was renamed to `list_codex_files_under` (`:1377`).
  - `list_claude_files` is now `#[cfg(test)]` only (`:1341`). The production
    walker is `list_claude_files_under`.
- **`ui/src/lib/k8s/actions.ts:26`** (moved here this iteration):
  *"The `restart` action isn't in this list for the drawer — see
  `restartLabel`."*
  - `restartLabel` has never existed: `git grep -w restartLabel` on
    `origin/main` finds only this comment.
  - The claim is also false: `restart` is the first entry of the list right
    below it (`:28`).
- **Older stale doc comments**, found by the workspace-wide scan:
  - `ui/src/modules/database/builder/sql-builder.ts:762`: *"`defaultSchema`
    fills a missing schema on unqualified tables"*. But
    `parseSelect(sql, dialect)` (`:767`) takes no such parameter, and an
    unqualified table keeps `schema: null` (`:805`).
  - `ui/src/lib/poll.ts:9`: *"a slower `hiddenMs` cadence"*. The option is
    `hidden?: 'pause' | number` (`:51`).
  - `ui/src/modules/product/AttachmentsPanel.svelte:27`: *"`insertIntoBody`
    is handled by the parent (OverviewTab) itself"*. No `insertIntoBody`
    exists anywhere in `ui/`.
- **Fix:**
  - Repoint the five `review_session` comments at `otto_review::session::…`.
  - Change the four canvas citations to `ui/src/lib/canvas/types.ts`. The
    skill asset matters most.
  - Change the driver comments to `list_claude_files_under` /
    `list_codex_files_under`.
  - Delete the false sentence at `actions.ts:26`.
  - Drop `defaultSchema` from the `parseSelect` doc, say `hidden: <ms>` in
    `poll.ts`, and remove the `insertIntoBody` line.

### maintainability-6f-02: The module → lib moves added five named re-export shims that the shim ratchet cannot see (minor; same pattern as 6-05, 5-01, 4-03, 3-08)

- **Where.** Each of these was added or turned into a shim in this
  iteration's diff:
  - `ui/src/modules/browser/selector.ts:5`: the whole file is
    `export { buildSelector, MAX_SELECTOR_LEN } from '../../lib/stores/browser.svelte'`.
    - Its comment says *"Re-exported for the module's own callers
      (ReaderView)"*.
    - The only importer is `browser/ReaderView.svelte:29`.
    - `MAX_SELECTOR_LEN` has no user through this path.
  - `ui/src/modules/rooms/games/access.ts:3`: the whole file is
    `export { captureGameInvite, consumeGameInvite } from '../../../lib/rooms/invites.ts'`.
    - Importers are `GameGuest.svelte:5` and two unit tests
      (`routerGuard.test.ts:1`, `routerNativeHistory.test.ts:1`).
    - `p01-room-invites.test.ts:7` already imports the real module.
  - `ui/src/modules/kubernetes/actions.ts:17`:
    `export { ACTIONS, actionsFor, typedConfirm, type ActionDef } from '../../lib/k8s/actions'`.
    `ACTIONS` and `typedConfirm` have no importer through this path.
  - `ui/src/modules/git/diff-virtual.ts:33`:
    `export { findScroller } from '../../lib/listWindow.svelte'`. It has one
    importer, `DiffViewer.svelte:51`.
  - `ui/src/modules/kubernetes/k8s-util.ts:10-12`: re-exports seven names
    from `lib/k8s/kinds`. `KINDS` has no importer through this path.
- **Evidence:**
  - The ratchet (`scripts/check.sh:297-298`) matches only
    `export (type )?\* from '(\.\./)+lib/`.
  - So `UI_LIB_SHIM_COUNT` stays at 3 while these five new shims exist.
  - The ratchet comment's own rule is "import from ui/src/lib directly".
- **Fix:**
  - Point `ReaderView`, `GameGuest`, the two router tests, `DiffViewer` and
    the k8s components at the `lib/` modules.
  - Delete `browser/selector.ts` and `rooms/games/access.ts`, and the
    re-export lines in `kubernetes/actions.ts` and `diff-virtual.ts`.
  - Widen the ratchet to also count `^export (type )?\{[^}]*\} from '(\.\./)+lib/`
    in `ui/src/modules`. The existing named re-exports
    (`conversation/api.ts:9`, `aws/util.ts:128`, `design-hall/tabKeys.ts:2`,
    `personal-agents/cadence.ts:8`, `database/results-format.ts:148`,
    `brokers/types.ts:33`) then become the baseline count.

### maintainability-6f-03: DiscoveryChat and RefineChat carry the same ~85-line stop-turn state machine, and this iteration added each new piece to both copies (minor)

- **Where:**
  - `ui/src/modules/product/DiscoveryChat.svelte:133-213`: `stopTurn`,
    `stopWaiting`, `reloadAfterStop`, `speakStopNotice`, `rescueStopFocus`,
    the `BACKGROUND_NOTICE` text at `:77`, and the two stop bubbles at
    `:388-430`.
  - `ui/src/modules/product/RefineChat.svelte:114-194`: the same, with the
    notice text at `:51` and the bubbles at `:335-378`.
- **Evidence:**
  - After renaming `loadChat`→`loadThread`, `cid`→`tid` and the
    `Discovery`/`Refinement` API prefix, `diff` of
    `DiscoveryChat.svelte:133-215` against `RefineChat.svelte:114-196` shows
    **0 differing lines**.
  - This iteration's diff added the same 25 lines to both files: `tick`
    focus, `announceEdge`, `speakStopNotice`, `rescueStopFocus`, `reloadBtn`,
    `composerEl` and the `BACKGROUND_NOTICE` constant.
  - Every later fix to the stop flow has to be made twice, and
    `w3p7-chatStop.test.ts` has to drive both.
- **Fix:** Lift the turn/stop state into one `.svelte.ts` class or factory,
  for example `modules/product/chatTurn.svelte.ts`. Pass it the two transport
  calls (`stop(target, turnId)` and `load(id, opts)`). Both chats then keep
  only their markup and their API bindings.

### maintainability-6f-04: "Is focus in a text field?" is hand-rolled in thirteen places, and the copies disagree (minor)

- **Where:**
  - `lib/design/scene3d/viewportLoop.ts:32`: `isTypingTarget`, exported, new
    this iteration.
  - `modules/snip/SnipEditor.svelte:649`: a private `isTypingTarget`.
  - `modules/kubernetes/ClusterWorkspace.svelte:306`: `isTyping`.
  - `lib/components/DockedDrawer.svelte:61`, `modules/home/HomePage.svelte:62`,
    `modules/aws/ViewToolbar.svelte:56`,
    `modules/design-hall/site/SiteStudio.svelte:538`,
    `lib/components/FloatingBar.svelte:578-584` and `shell/App.svelte:408-414`:
    the check written inline.
  - Four more inline copies inside `lib/keys.ts` itself: `:130-137`,
    `:186-193`, `:273-279` and `:343-348`.
- **Evidence.** The copies have drifted:
  - `ViewToolbar.svelte:56`, `App.svelte:410-413`, `keys.ts:186-193` and
    `keys.ts:343-348` leave out `SELECT`. `keys.ts:273-279`, three screens
    above in the same file, includes it.
  - `ViewToolbar`, `App`, `FloatingBar` and `keys.ts:137` also treat
    `.cm-editor` (and `FloatingBar` also `.xterm`) as typing. The `scene3d`,
    `SnipEditor`, `ClusterWorkspace`, `DockedDrawer`, `HomePage` and
    `SiteStudio` copies do not.
  - So a single-key shortcut fires inside a CodeMirror editor or a `<select>`
    on some pages and not on others.
  - The one exported helper lives under `lib/design/scene3d/`, where other
    modules would not look for it.
- **Fix:**
  - Add one `isTypingTarget(el, { editors = true })` to `lib/keys.ts`. It
    should cover `INPUT`, `TEXTAREA` and `SELECT`, contenteditable, and
    optionally `.cm-editor` and `.xterm`.
  - Re-export or move the scene3d one there, and replace the copies.
  - Each site that wants a different set says so through the option.

### maintainability-6f-05: The transcript-roots consolidation (architecture-6-01) left two copies of its helpers, one of which has drifted (minor)

- **Where:**
  - **`crates/otto-server/src/transcript/resolve.rs:24-30`, `home_dir()`:**
    falls back to `dirs::home_dir()` when `$HOME` is unset or empty, then to
    `/`.
    - It feeds `TranscriptRoots::discover(&home_dir(), …)` (`:37`).
    - The canonical `otto_transcript::roots::home_dir()`
      (`otto-transcript/src/roots.rs:332-337`), which `otto-sessions` uses
      (`manager.rs:419`, `manager/transcript_roots.rs:75`), has no
      `dirs::home_dir` step.
    - So without `$HOME`, the daemon's transcript reader and the session
      manager discover roots under different homes.
  - **`crates/otto-usage/src/tailer/driver.rs:1393-1402`, `read_subdirs`:**
    the same body as `otto-transcript/src/roots.rs:341-350`. The only
    difference is `.map(…).unwrap_or(false)` against `.is_ok_and(…)`.
- **Evidence:** `roots.rs` is documented as *"the one place that lists
  transcript roots"*: FIXES wave 1, and `resolve.rs:32-33` cites it.
  `otto-server` already depends on `otto-transcript`.
- **Fix:**
  - Delete `resolve.rs::home_dir` and call `otto_transcript::roots::home_dir()`.
    If the `dirs` fallback is wanted, add it to the canonical function.
  - Make `roots::read_subdirs` `pub` (or `pub(crate)` plus a small
    `list_dir` API) and use it from the usage tailer.

### maintainability-6f-06: 46 new unit-test files are named after fix-plan work packages (`p03-`, `w2p8-`, `w3p1-`…), so one subject's tests are split across files with no subject name (minor)

- **Where:** `ui/unit/` gained 46 files this iteration named
  `p01-…` through `p10-…` and `w2p3-…` through `w3p8-…`. Examples:
  - `p03-clipboard.test.ts`, `w2p8-clipboard.test.ts` and
    `w2p8-copyFailures.test.ts` all test `lib/clipboard.ts`.
  - `p03-announce.test.ts`, `w2p8-announceShown.test.ts`,
    `w3p8-announce.test.ts` and `w2p8-loadStateAnnounce.test.ts` all test
    `lib/announce.svelte.ts`.
  - The graph split's tests: `w2p5-graphActions`, `w3p1-graphData`,
    `w3p1-graphSelection`, `w3p1-graphCompile`, `w3p1-refPopover` and
    `w3p1-gitLiveRegions`.
- **Evidence:**
  - `git ls-tree origin/main ui/unit/` has **0** files with this prefix. The
    other 369 unit files are named by subject (`helpGuide.test.ts`,
    `restContract.test.ts`, …).
  - The prefixes refer to `docs/reviews/…/PLAN.md` packages. They mean
    nothing to someone who opens `ui/unit/` without that folder.
  - A maintainer who changes `lib/clipboard.ts` has to grep to find its three
    test files.
- **Fix:** Rename each file to its subject, and merge the per-subject
  duplicates (`clipboard.test.ts`, `announce.test.ts`,
  `gitGraph*.test.ts`…). Keep the fix-plan id in a comment if traceability
  is wanted. If needed, add a check that rejects
  `ui/unit/(p|w[0-9]+p)[0-9]+-` file names.

### maintainability-6f-07: Compatibility re-exports and aliases left by the extractions; one has no user (nit)

- **Where:**
  - **`crates/otto-server/src/agent_session.rs:21`:**
    `pub use otto_agent_run::session_turn::{submit_confirm_for, TurnOpts, STUCK_IDLE};`.
    `submit_confirm_for` has no user through this path. Its only callers are
    inside `otto-agent-run/src/session_turn.rs` (`:538` and tests).
  - **`crates/otto-server/src/rooms/mod.rs:15-16`:**
    `pub(crate) use otto_rooms::auth::owner_auth;` and
    `pub use otto_rooms::{capability, recap_engines, RoomRegistry};`.
    - Each has one user through the old path: `game_rooms/http.rs:14`,
      `game_rooms/socket.rs:1`, `lib.rs:383` and `state.rs:48`.
    - The sibling `rooms/http.rs:13` imports `otto_rooms::auth::owner_auth`
      directly, so the two idioms sit side by side.
  - **`crates/otto-server/src/game_rooms/mod.rs:3`:**
    `pub use otto_rooms::games::Registry;`. Its one user is `state.rs:50`.
  - **`crates/otto-sessions/src/lifecycle.rs:39`:**
    `pub use otto_transcript::roots::encode_cwd as claude_project_dir_name;`.
    - This renames the one encoder back to an old name. The users are the
      same file (`:45`, `:186`) and one test (`manager/tests/mod.rs:3466`).
    - `roots.rs:372` itself calls it *"the three encoders this replaced
      (`lifecycle::claude_project_dir_name`, …)"*.
  - **`ui/src/lib/tableWindow.svelte.ts:28`:**
    `export type WindowRange = ListRange;` has no importer anywhere. It was
    the return type of the pre-split `range()`.
- **Fix:**
  - Drop `submit_confirm_for` from the `pub use`.
  - Import `otto_rooms::…` directly at the five otto-server sites and delete
    the re-exports.
  - Call `otto_transcript::roots::encode_cwd` in `lifecycle.rs` and the test,
    and delete the alias.
  - Delete `WindowRange`.

### maintainability-6f-08: The GraphView and WorkflowsPage splits copied private helpers into both halves (nit)

- **Where:**
  - `modules/git/GraphView.svelte:192-198` and
    `modules/git/graph/CommitDetailPane.svelte:62-68`: identical
    `fmtDate(iso)` (`rel(iso, { compact: true })`) and `fmtDateTitle(iso)`.
  - `modules/workflows/WorkflowsPage.svelte:1008-1014` and
    `modules/workflows/RunMonitor.svelte:35-41`: identical
    `runStatusLabel` / `dotKey` one-line wrappers over `runStatus(s)`,
    including the same doc comment.
- **Evidence:** A same-name scan over every split pair this iteration (GraphView
  ↔ `git/graph/*`, WorkflowsPage ↔ RunMonitor, OverviewTab ↔ its four parts,
  and the store façades ↔ sub-stores) finds exactly these four.
- **Fix:**
  - Call `runStatus(s).label` / `.key` inline, or export the two wrappers
    once from `runStatus`'s module.
  - Use the shared `RelTime` component (`lib/components/RelTime.svelte`),
    which already renders the relative label with a `toLocaleString()`
    title, or one exported pair of date helpers.

### maintainability-6f-09: Two exported `copyWithToast` functions with different contracts (nit)

- **Where:**
  - `ui/src/lib/clipboard.ts:73`: `copyWithToast(text, what: string)`. It
    always toasts `Copied {what}`. This one is new this iteration.
  - `ui/src/modules/database/results-format.ts:159`:
    `copyWithToast(s, successToast?: [title, body?])`. It is silent on
    success unless asked, and records clip history.
- **Evidence:**
  - Both are imported by name across the app: `lib/clipboard`'s by 6 files
    and `results-format`'s by 8.
  - `results-format.ts:150-158` needs a paragraph to explain *"Not
    lib/clipboard's `copyWithToast`"*.
  - Editor auto-import offers both.
- **Fix:** Rename the database one to what it does, for example
  `copyCellValue` or `copyQuietly`. Keep the "silent on purpose" note on that
  name.
