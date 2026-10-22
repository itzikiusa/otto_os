# Maintainability review: iteration 5 (2026-10-22)

Lens: dead code, unused exports, routes and settings, duplicated helpers,
misleading names and comments, stale TODOs that point at finished work,
idioms that differ from the surrounding code, and code that is more clever
than it needs to be.

Worktree: `fix/quality-iter5-20261022` at `8177c4d85`. The review was
read-only. It used grep, rg, sed, git and small read-only Python scanners in
the session scratchpad. Nothing was built and no tests were run.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per
minor and 0.03 per nit.

| Severity | Count | Deduction |
|----------|------:|----------:|
| blocker  | 0 | 0.00 |
| major    | 0 | 0.00 |
| minor    | 6 | 0.60 |
| nit      | 2 | 0.06 |
| **total**|   | **0.66** |

**Score: 10.0 − 0.66 = 9.34 / 10.** Earlier scores: iteration 1 7.25,
iteration 2 8.28, iteration 3 8.65, iteration 4 9.39.

The score is a little lower than iteration 4 even though 10 of the 11
iteration-4 findings are fixed. Two things explain it. First, this pass
covered areas that earlier passes excluded or did not reach: the room games,
the 24-file keyed-lock allow-list and the Svelte store methods. Second, the
iteration-4 fix pass added nine new "moved to lib/, re-exported" shims and four
new Rust compatibility re-exports. That is the same pattern that 3-08 and 4-03
asked to be removed.

## Status of the iteration-4 findings

| Id | Iter-4 severity | Status now | Evidence |
|----|-----------------|------------|----------|
| 4-01 | minor | Fixed | `SessionManager::relay`, `RelayOutcome` and `resolve_absent` are gone. `ImprovementsRepo::set_edit_status` is now the CAS function (`improvements.rs:370-402`). It takes `from`/`to` and its four callers are in `otto-improve/src/engine.rs:726-807`. |
| 4-02 | minor | Fixed | None of the 17 named functions are still defined. The vault store's `fts_search` is now a test-only helper (`store.rs:1522`). |
| 4-03 | minor | **Partly open** (folded into 5-01) | The `renderQueue` and `excalidraw-build` shims are deleted. `canvas/mermaid.ts` still has 6 importers, and `vault/mdRender.ts` is still imported by `unit/chatV2.test.ts:40`. |
| 4-04 | minor | Fixed | `bridge.rs:20` imports `otto_core::keyed_lock::KeyedLocks`, `:55` declares `type ConvLocks = KeyedLocks<ConvKey>` and `:62` defines a free `enqueue`. `bridge.rs` is off `KEYED_LOCK_ALLOW`. |
| 4-05 | nit | Fixed | `otto-review/src/engine.rs:5` names `modules/review/`, and `proof-packs.md:300` names `modules/`. |
| 4-06 | nit | Fixed | The `linear.rs` `truncate` is gone. `design-assist` `cap_chars` (`lib.rs:592`) delegates to `clip_chars`. |
| 4-07 | nit | Fixed | `sessionUsage.ts` `formatTokens` calls `formatCount`. |
| 4-08 | nit | Fixed | `codeBlocks.ts:13` imports `escapeHtml, unescapeHtml`. |
| 4-09 | nit | Fixed | `__issuePathForTest`, `MockupFormat` and `GlobalRole` have no hits. |
| 4-10 | nit | Fixed | `fields_ready` has no hits. |
| 4-11 | nit | Fixed | The `canvasGeometry.ts` pair is now `LoopStepLabel` / `loopStepLabels` (`:12-17`). |

Totals: 10 fixed, 1 partly open and carried forward, and 0 found to be
invalid.

## Areas covered in this iteration

- **Re-checked every iteration-4 finding** against the current code (see the
  table above).
- **The iteration-4 fix diff** (`6cff08270..HEAD`, 584 files): the database
  store split, the k8s and database moves into `lib/`, `otto_sessions::submit`
  (the prompt-submit move), the `budget/` service, the otto-issues
  Markdown/ADF/storage emitters, MCP profiles and the kart/shooter rebuild.
- **`pub(crate) fn` with no reference** anywhere in the workspace. A new
  tokenizer-based scanner ignores comments and string literals. It found 0
  hits, which is expected because rustc warns on these. The `pub fn` re-scan
  found two hits: `k8s_kind` (reported) and the serde default (not an issue).
- **Svelte store methods with no caller.** A scanner checked class methods in
  every `*.svelte.ts` file. Each hit was confirmed with `rg -w` across `src`,
  `unit` and `e2e`, including `page.evaluate` strings. The hits on the
  Appearance setters were false positives from markup and were dropped.
- **UI exports with no importer**, now including `rooms/games/**`,
  `home/school/**` and `ClassroomsBox.svelte`. Contract DTO `*Req`/`*Resp`
  types are left out under the iteration-4 policy note.
- **All UI re-export shims**: every `export * from` and "re-exported for"
  comment in `ui/src`, with their importers counted by path.
- **Rust compatibility re-exports** ("re-exported for existing callers") and
  their callers.
- **The 24 files on the keyed-lock allow-list.** Each one still matches the
  guard regex, so none of the entries is stale. Each body was compared with
  `otto_core::keyed_lock`.
- **The MCP catalog parity** that iteration 4 deferred:
  `DEFERRED_SCHEMA_DRIFT` and `INTENTIONALLY_ABSENT` are both empty
  (`ottod/src/mcp_tools/tests/catalog_tests.rs:13,18`). Closed.
- **`docs/features/*.md`**:
  - every cited repo path was checked for existence;
  - every backticked `snake_case` identifier was checked for presence in the
    code;
  - the remaining hits were triaged by hand (deliberate "no such toggle" notes
    and external names were dropped).
- **TODO/FIXME/`allow(dead_code)`/`@ts-ignore`** in the iteration-4 diff. None
  of them are stale.

## Areas not covered yet (for iteration 6)

- **UI exports used only inside their own file.** There are about 936 of
  these: the `export` keyword is not needed, but the item is used locally.
  Whether that counts as a finding is a policy call.
- **Contract-only TS `*Req`/`*Resp` types** with no UI importer (about 70).
  This still needs the policy decision noted in iteration 4.
- **Rust struct fields that are read only by tests.** This needs a typed
  scanner, because rustc treats a test read as a use.
- **The single-flight `Arc`-strong lock maps on the allow-list.** These are
  `otto-dbviewer` `gates`/`building`, `otto-brokers` `*_locks`,
  `otto-k8s` `FLIGHTS`/`MINT`, `otto-vault` `scans` and `proof.rs`
  `ProofLocks`. Their semantics differ from `KeyedLocks`, so each needs a
  per-site migration review.
- **The rest of the behaviour text in `docs/features/*.md`.** This pass
  checked only paths and identifiers, not prose claims.
- **The `kart`/`shooter` render internals**, apart from their exports.
  Another agent is still rewriting them in parallel.

---

## Findings

### maintainability-5-01: Nine new "moved to lib/, re-exported for existing importers" UI shims, and the two from 4-03 are still in use (minor)

- **Where:**
  - `ui/src/modules/database/{sql-util,sql-dialect,filter-chips,error-normalize,mongo-format,grid-tab-state}.ts`
    are each three lines long: `export * from '../../lib/database/<x>.ts'`.
    They were added in `185b542f8` (2026-10-22).
  - `ui/src/modules/kubernetes/{resourcePoll,viewState}.ts` and
    `modules/kubernetes/monitor/tickCoalescer.ts` follow the same pattern
    over `lib/k8s/*`.
  - Left over from 4-03: `modules/canvas/mermaid.ts` and
    `modules/vault/mdRender.ts`.
- **Evidence:**
  - The comment in each shim says it exists *"so module importers keep
    working"*. None of the importers were moved:
    - **The database shims:** about 19 `./sql-util`-style imports inside
      `modules/database`. There are also 15 test imports, for example
      `unit/dbSqlDialect.test.ts:14-22`, `unit/dbClipGrid.test.ts:5`,
      `unit/dbErrorNormalize.test.ts:12` and
      `e2e/desktop-db-query-filter.spec.ts:48`.
    - **Cross-module imports through a database shim:**
      `modules/workbench/lib/format.ts:11` and `placeholders.ts:13` import
      `../../database/sql-util.ts`.
    - **`kubernetes/viewState`:** imported by `lib/uiCommands/k8s.ts:24`. That
      is a `lib/` file that imports the module shim of a file that already
      sits in `lib/k8s/`. It is also imported by `vault/LiveContext.svelte:79`
      and five kubernetes components.
    - **The other kubernetes shims:**
      `unit/k8sResourcesPerf.test.ts:5`, `unit/k8sViewState.test.ts:3` and
      `unit/k8sMonitorTicks.test.ts:3`. The doc at `lib/api/k8s.ts:105` still
      points readers at `modules/kubernetes/resourcePoll.ts`.
    - **`canvas/mermaid`:** `product/MockupLivePreview.svelte:10`,
      `MockupViewer.svelte:15`, `ActionCard.svelte:25`,
      `product/design/exporters.ts:5`,
      `agents/conversation/Markdown.svelte:86` (a dynamic import) and
      `panels/CanvasPanel.svelte:98`.
    - **`vault/mdRender`:** `unit/chatV2.test.ts:40`.
  - The split also left these stale items behind:
    - `lib/stores/database.svelte.ts:61-62` re-exports `condLabel`,
      `condToSql`, `filterValMatches`, `parseFilterValText`, `toFilterVal`,
      `FilterCond` and `FilterVal`. No file imports any of them from the
      store.
    - `lib/database/filter-chips.ts:87` `condLabel` has no caller at all. It
      also has the same name as the unrelated
      `workflows/canvasGeometry.ts:54` `condLabel`.
    - `lib/stores/database/statement.ts:52-54` says these helpers *"are
      re-exported at the top"*, but `statement.ts` does not re-export them.
  - This is the third iteration in a row that has raised this pattern (3-08,
    4-03). Each new shim brings its own set of importers that someone has to
    retarget later.
- **Fix:**
  - Retarget every importer (src, unit and e2e) to `lib/database/*`,
    `lib/k8s/*`, `lib/render/mermaid.ts` and `lib/render/markdown.ts`, then
    delete the 11 shims.
  - Delete `database.svelte.ts:61-62` and the unused `condLabel`.
  - Fix the `statement.ts` comment and the `lib/api/k8s.ts:105` pointer.
  - If a move cannot retarget its importers in the same change, add a
    `check.sh` ratchet for `export \* from '\.\./\.\./lib/` under
    `ui/src/modules`, so the number of shims can only go down.

### maintainability-5-02: Eight allow-listed copies of `otto_core::keyed_lock::KeyedLocks`, four of which keep the panic-on-poison policy it retired (minor)

- **Where:** each of these is a `Weak`-entry map with "retain the live
  entries, then upgrade or insert" logic, which is line for line what
  `KeyedLocks::handle` does (`otto-core/src/keyed_lock.rs:45-57`):
  - `otto-assistant/src/assistant/tasks.rs:38-53` (`task_lock`)
  - `otto-server/src/api_secrets.rs:20-40` (`request_guard`, which uses
    `.lock().unwrap()`)
  - `otto-server/src/eval_score.rs:29-48` (`update_guard`)
  - `otto-state/src/workflow_progress.rs:253-266` (`repair_gate`)
  - `otto-server/src/routes/backup_git.rs:617-635` (`repo_lock`, which uses
    `.expect("repository locks")`)
  - `otto-server/src/agent_session_lifecycle.rs:19-35` (`acquire_lease`)
  - `otto-design/src/service.rs:75-99` (`CommitLocks`, which uses `.unwrap()`)
  - `otto-automation/src/goal_loop.rs:109-127` (`operations`, which uses
    `.unwrap()`)
- **Evidence:**
  - The module doc of `keyed_lock.rs:5-12` says the shared type replaces the
    hand-rolled statics that had drifted into *"two poison policies (one of
    which panicked every later caller once a holder had panicked)"*.
  - Four of the eight copies above still use that panicking policy.
  - All eight crates depend on `otto-core`.
  - Each body is 12-20 lines. With the shared type the whole body becomes
    `static LOCKS: KeyedLocks<String> = KeyedLocks::new(); LOCKS.lock(id).await`
    (or `.handle(id)` / `.try_lock(id)`, which `repair_gate`'s
    `try_lock_owned` needs).
  - `scripts/check.sh:214-217` says *"The allowlist is the files not migrated
    yet: shrink it"*. These eight are the files that are cheap to migrate,
    because none of them has any extra semantics.
- **Fix:**
  - Replace each body with a `KeyedLocks` static (or a `KeyedLocks` field for
    `goal_loop.rs` `operations`). For a tuple or path key, use
    `KeyedLocks<(PathBuf, String)>` or `KeyedLocks<PathBuf>`.
  - Drop the eight paths from `KEYED_LOCK_ALLOW`.
  - Keep the `Arc`-strong single-flight maps on the list until each one has
    had its own review.

### maintainability-5-03: The prompt-submit and budget moves left compatibility re-exports that every caller still uses, plus a renamed hash that is aliased back to its old name (minor)

- **Where and evidence:**
  - **`otto-server/src/budget/mod.rs:1-9`:**
    - The doc says the budget service was moved out of `routes/usage.rs`
      because *"every gated surface (goal loops, reviews, swarm turns, product
      runs, the workflow `budget_gate` node)"* had to import a route module.
    - `routes/usage.rs:466` then re-exports
      `pub use crate::budget::{check_budget, BudgetVerdict}`.
    - All nine gate call sites still go through the route module:
      - `routes/goal_loops.rs:69`, `:88`, `:237`, `:424` and `:481`
      - `modules/review/routes.rs:367`
      - `swarm_host.rs:105`
      - `product_host.rs:480`
      - `workflow_engine/nodes/budget_gate.rs:23`
    - Only Mission Control and the monitor use `crate::budget`. So the stated
      dependency was not removed, even though every caller is in the same
      crate.
  - **`otto-review/src/session.rs:860-864`:**
    - This block re-exports `bracketed_paste`, `dispatched`, `submit_prompt`,
      `wait_for_tui`, `wait_for_tui_strict` and `PASTE_TO_ENTER` from
      `otto_sessions::submit` *"so the old paths keep compiling"*.
    - The callers still use the old path through `crate::review_session::`:
      - `swarm_host.rs:126-139`
      - `routes/personal_agents.rs:971`
      - `agent_session.rs:707`
      - `tests/review_agent_retry.rs:253`
    - The `submit.rs:17-18` doc even advertises the re-export.
  - **`otto-state/src/workbench.rs:202-203`:**
    - The iteration-4 CodeQL fix renamed the digest to `content_digest_hex`,
      so that it is not mistaken for password hashing.
    - It then added `pub use self::content_digest_hex as sha256_hex;` for one
      importer in the same crate, `mcp_audit.rs:29`.
    - That brings back the name the rename was meant to retire.
  - **`otto-issues/src/confluence.rs:1544-1546`** re-exports
    `md_storage::markdown_to_storage` *"for existing callers"*. The callers
    are `otto-issues/src/http.rs:24` (same crate) and
    `otto-product/src/http.rs:2061`.
- **Fix:**
  - Point the callers at the new homes:
    - `crate::budget::check_budget` and `crate::budget::BudgetVerdict`
    - `otto_sessions::submit::*`
    - `content_digest_hex`
    - `otto_issues::md_storage::markdown_to_storage`
  - Delete the four re-exports and their "kept for existing callers" docs.

### maintainability-5-04: The workspace store's `injectInput` pipeline has had no caller since June, but the Terminal still runs and re-syncs it (minor)

- **Where:**
  - `ui/src/lib/stores/workspace.svelte.ts:1139-1145` (`injectInput`) and
    `:130-132` (the `injections` `$state`)
  - `ui/src/lib/components/Terminal.svelte`:
    - `:2888-2896`, the `$effect` that applies `ws.injections[sessionId]`
    - `:2425` and `:2465-2467`, where `lastInjN` is re-synced on an engine
      adopt
- **Evidence:**
  - `rg -w injectInput` over `src`, `unit` and `e2e` finds only the
    definition and two comments.
  - `git log -G'injectInput\('` shows that the last call site went away in
    `08932062f` (2026-06-20).
  - `ResultsGrid.svelte:78` says it *"replaces raw injectInput for DB
    results"*.
  - Even so, every mounted Terminal subscribes to `ws.injections`.
  - Two adopt paths keep the bump counter in step, and each carries a
    paragraph explaining why (*"An adopted socket is already OPEN: without
    this the injection effect below would re-send…"*).
  - `sendInput`'s doc (`workspace.svelte.ts:1131`) still describes itself by
    contrast with `{@link injectInput}`.
  - The Terminal comment (*"e.g. DB rows → a running agent"*) describes a
    flow that now goes through the send-to-agent dialog.
- **Fix:**
  - Delete `injectInput` and `injections`.
  - Delete the Terminal effect and both `lastInjN` re-syncs.
  - Reword the `sendInput` doc so it stands on its own.

### maintainability-5-05: The shooter's `SNAPSHOT_VERSION` constant is never read; the wire version is the literal `4` in three places (minor)

- **Where:** `ui/src/modules/rooms/games/shooter/net.ts`:
  - `:29`: `export const SNAPSHOT_VERSION = 4;`
  - `:57`: the snapshot type is `v: 4;`
  - `:92`: the encoder writes `v: 4,`
  - `:203`: the decoder rejects on `s.v !== 4`
- **Evidence:**
  - `rg -w SNAPSHOT_VERSION` finds only the definition.
  - A developer who changes the snapshot layout and bumps the named constant,
    which is the obvious move, changes nothing. Peers on mixed builds keep
    accepting each other's snapshots and mis-decode them.
  - `kart/net.ts:25` and `:40` hard-code `v: 4` the same way and have no
    constant at all.
- **Fix:** Type the field as `v: typeof SNAPSHOT_VERSION`, and use the
  constant in both the encoder and the decoder. Give `kart/net.ts` its own
  `KART_SNAPSHOT_VERSION` and use it the same way.

### maintainability-5-06: The room games keep 16 dead exports after the rebuild, and kart re-implements the shared texture helpers under the same exported names (minor)

- **Dead exports.** Each name below has exactly one `rg -w` hit, its
  definition, across `ui/src`, `unit`, `e2e` and `examples`:
  - **`common/textures.ts`:** `asphaltTexture` (`:19`), `curbTexture`
    (`:36`), `chevronTexture` (`:51`) and `groundDetailTexture` (`:78`).
    These kart textures have been unused since `c630ef667` ("kart road,
    world, effects and HUD rebuilt") moved kart onto
    `kart/render/surfaces.ts`.
  - **`common/math.ts:16`:** `towards`.
  - **`client.ts:4`:** `parseGameInvite`.
  - **`kart/layout.ts:21`:** `KART_HALF`.
  - **`kart/render/dressing.ts:70`:** `skidSide`.
  - **`shooter/weapons.ts:139`:** `ATTACH_ALL`.
  - **`shooter/meshes.ts:240` and `:250`:** `pickaxeMesh` and
    `pickaxeGeometries`.
  - **`shooter/modes/index.ts:36` and `:259`:** `teamPlay` and
    `insideArena`. The `state.squad > 1` test that `teamPlay` names is
    written inline about nine times: `sim.ts:85`, `:185`, `:477`,
    `sim/combat.ts:199`, `hud/minimap.ts:94-95` and
    `ShooterGame.svelte:200`, `:235`, `:259`, `:392`.
  - **`shooter/render/soldiers.ts:192`:** `INFECTED_SKINS`.
  - **`shooter/sim/model.ts:28`:** `TEAM_SIZES`. `net.ts:41` declares its
    own identical `SQUADS: TeamSize[] = [1, 2, 4]`.
- **Duplicated helpers and colliding names:**
  - `kart/render/surfaces.ts:14-25` copies `common/textures.ts:5-16`
    `canvas()` verbatim. Its `texture()` is `finish()` plus a colour-space
    flag.
  - It also exports `type SurfaceKind` (`:248`, sand, snow and so on) and
    `surfaceTexture(kind)` (`:256`). `common/textures.ts:93` and `:129`
    export a different `SurfaceKind` and `surfaceTexture(kind, seed)` (the
    shooter's plaster, brick and so on).
  - `shooter/render/world.ts:15` and `kart/render/road.ts:18` each import
    their own version. An auto-import picks one by luck. This is the same
    kind of collision as 4-11.
  - `nums` is defined twice: `kart/net.ts:66` and `shooter/net.ts:134`.
    The shared `finite` and `r2` already live in `common/math.ts:23-25`.
- **Fix:**
  - Delete the 16 dead exports.
  - Use `teamPlay` at the inline sites, or delete it.
  - Have `net.ts` import `TEAM_SIZES`.
  - Have `kart/render/surfaces.ts` import `canvas` and `finish` from
    `common/textures.ts` (add the colour-space option there).
  - Rename kart's pair to `TerrainSurfaceKind` and `terrainSurfaceTexture`,
    or similar.
  - Move `nums` next to `finite` in `common/math.ts`.

### maintainability-5-07: Four more accessors with no caller (nit)

- **Where and evidence:**
  - **`ui/src/lib/router.svelte.ts:530`, `lastRouteFor(id)`:** no caller.
    `openModule` (`:546`) reads `this.lastByModule` directly.
  - **`ui/src/modules/database/EditFlow.svelte.ts:831`, `mongoIdFilter(rowIdx)`:**
    no caller. The Mongo adapters call `mongoIdFilterFor` directly
    (`edit-mongo.ts:233`, `:320`). `query-filter.ts:128` still documents
    `mongoValueJson` as matching *"the grid's `mongoIdFilter`"*.
  - **`crates/otto-k8s/src/resources.rs:118`, `Kind::k8s_kind`:** no caller
    since `7e357dcf5`, apart from the intra-doc link in the doc of
    `from_k8s_kind` (`:141`). That doc calls `from_k8s_kind` the *"Reverse
    of"* `k8s_kind`, but it maps only 6 of the 18 kinds.
  - **`crates/otto-sessions/src/submit.rs:131`, `wait_for_tui_strict`:** no
    caller. The strict path is reached through
    `SubmitOpts { echo_wait: true }` (`:209`). Its only other reference is
    the re-export in `otto-review/src/session.rs:863`.
- **Fix:**
  - Delete the four functions.
  - Point the `query-filter.ts` doc at `mongoIdFilterFor`.
  - Describe `from_k8s_kind` as "the Argo workload kinds" instead of
    "reverse".

### maintainability-5-08: Feature docs cite a deleted function and two design specs that were never committed (nit)

- **Where and evidence:**
  - **`docs/features/agent-context-injection.md:128-130`:** the step is
    *"Emit the launch flags in `injection_for`"*, and the doc then says *"Add
    a match arm to `injection_for(provider, dir)`"*.
    - The wrapper was deleted in `66b55a25c`.
    - The match is now `injection_for_home(provider, dir, provider_home)`
      (`crates/otto-context/src/materialize.rs:216`).
  - **`docs/features/personal-agents.md:9`** cites
    `docs/superpowers/specs/2026-09-01-personal-agents-design.md`.
  - **`docs/features/kubernetes-monitoring.md:19-20`** cites
    `docs/superpowers/specs/2026-09-05-k8s-monitoring-dashboard-design.md`.
  - `git log --all` finds neither spec file on any ref.
- **Fix:**
  - Rename the step and the signature to `injection_for_home`.
  - Drop the two "Design spec:" lines, or commit the specs.
