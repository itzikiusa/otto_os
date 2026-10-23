# Maintainability review: iteration 6, wave 0 (2026-10-23)

Lens: dead code, unused exports, routes and settings, duplicated helpers,
misleading names and comments, stale TODOs that point at finished work,
idioms that differ from the surrounding code, and code that is more clever
than it needs to be.

Worktree: `fix/quality-iter6-20261023` at `02a64019b` (the iteration-5 merge).
The review was read-only. It used grep, rg, sed, perl, git and small shell
scanners. Nothing was built and no tests were run.

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
iteration 2 8.28, iteration 3 8.65, iteration 4 9.39, iteration 5 9.34.

The iteration-5 fixes closed 5 of the 8 findings and most of the other 3.
The score does not move for two reasons. First, three findings are only
partly fixed. Second, this pass found new problems in places earlier passes
could not see. Two Svelte files contain a raw NUL byte, so git and `rg`
treat them as binary. That hid at least one caller from every earlier scan,
including this one's first run. The iteration-5 fixes also added a new
type-only shim that the new shim ratchet does not count.

## Status of the iteration-5 findings

| Id | Iter-5 severity | Status now | Evidence |
|----|-----------------|------------|----------|
| 5-01 | minor | **Fixed** | All 11 shims are gone: `modules/database/{sql-util,…}.ts`, `modules/kubernetes/{resourcePoll,viewState}.ts`, `monitor/tickCoalescer.ts`, `canvas/mermaid.ts` and `vault/mdRender.ts`. `database.svelte.ts` no longer re-exports the filter helpers. `condLabel` is gone from `filter-chips.ts`. `statement.ts:52-55` now says the store does not re-export them. `lib/api/k8s.ts:105` points at `lib/k8s/resourcePoll.ts`. The `UI_LIB_SHIM_MAX=3` ratchet is at `scripts/check.sh:248-267`. A new shim slipped past that ratchet: see **6-05**. |
| 5-02 | minor | **Partly open** (folded into 6-03) | 7 of the 8 sites use `KeyedLocks`: `tasks.rs:42`, `api_secrets.rs:22`, `eval_score.rs:30`, `workflow_progress.rs:253`, `backup_git.rs:618`, `design/service.rs:82` and `goal_loop.rs:109`. `agent_session_lifecycle.rs:19-35` `acquire_lease` is still a hand-rolled copy. |
| 5-03 | minor | **Partly open** (6-01) | The budget re-export, the `sha256_hex` alias and the `confluence.rs` `markdown_to_storage` re-export are gone. The `otto_sessions::submit` re-export block in `otto-review/src/session.rs:860-864` is still there, and five otto-server callers still use it through `crate::review_session::submit_prompt`. FIXES.md:255 says "submit callers repointed", which overstates it. |
| 5-04 | minor | **Fixed** | `rg "injectInput\|injections\b\|lastInjN"` over `src`, `unit` and `e2e` finds nothing. |
| 5-05 | minor | **Fixed** | `shooter/net.ts:30,57,92,202` use `SNAPSHOT_VERSION`. `kart/net.ts:26,28,43,74` use `KART_SNAPSHOT_VERSION`. |
| 5-06 | minor | **Fixed** (one nit left, 6-08) | All 16 dead exports are gone. `TEAM_SIZES` is used and `SQUADS` is gone. `kart/render/surfaces.ts:13-15` imports `canvas` and `finish`. The kart pair is now `TerrainSurfaceKind` / `terrainSurfaceTexture` (`:239,:247`). `nums` lives in `common/math.ts:21`. `teamPlay` was deleted, which the fix allowed. The residual is the shooter's private `canvas()`. |
| 5-07 | nit | **Partly open** (6-02) | `router.lastRouteFor` is deleted. `mongoIdFilter`, `Kind::k8s_kind` and `wait_for_tui_strict` are still defined and still have no caller. |
| 5-08 | nit | **Fixed** | `docs/features` has no `injection_for` hit and no `superpowers/specs` hit. |

Totals: 5 fixed, 3 partly open and carried forward, and 0 found to be
invalid.

## Areas covered in this iteration

- **Every iteration-5 finding**, re-checked against the current code (see
  the table above).
- **The iteration-5 fix diff** (`8177c4d85..HEAD`, 373 files), with these
  scans:
  - new UI `export`s that have no importer outside their own file;
  - new Rust `pub`/`pub(crate)` fns that have no reference, or only a test
    reference;
  - functions removed in the diff that are still cited in docs.
- **A raw control-byte scan of every tracked text source** (`.ts`,
  `.svelte`, `.rs`, scripts) using perl. Recursive `rg` skips a file at its
  first NUL, so this has to run first. That is how 6-04 was found.
- **Repo-path citations** (`ui/src/…`, `crates/…`) in code comments,
  `docs/contracts`, `AGENTS.md` and `README.md`, checked for existence.
- **Backticked identifiers in comments** in the files the iteration-5 diff
  touched, checked for a definition that still exists.
- **All UI re-export shims**, including `export type *` and named
  `export { x }` re-exports. The Rust "re-exported for existing callers"
  comments and the `otto-server/src/lib.rs` crate aliases were also checked.
- **The 17 files on the keyed-lock allow-list.** Each body was compared with
  `otto_core::keyed_lock::KeyedLocks`. This covers the single-flight
  `Arc`-strong maps that iteration 5 deferred.
- **Exports in `rooms/**`, `home/**`, `lib/three` and `relaySocket.ts`.**
  The room games and the classroom were included.
- **`AGENTS.md` crate and UI-module maps** against `crates/` and
  `ui/src/modules/`. Both match.
- **`OTTO_*` env vars documented in `docs/features`**, checked against the
  code. Every one is read; the `OTTO_TUNNEL_<NAME>_*` vars are generated.
- **TODO/FIXME/HACK markers and `allow(dead_code)`.** None of the 17 markers
  is stale. The `allow(dead_code)` uses are `FromRow` ordering columns and a
  test-only builder, which is fine.
- **Duplicated byte/duration formatters** in the UI. Every `formatBytes` now
  delegates to `lib/metric-format`, so this is not a finding.

## Areas not covered yet (for later waves)

- **UI exports used only inside their own file.** About 60 of these are in
  `rooms/**` and `home/school/**` alone, for example `kart/layout.ts`,
  `school/model.ts` and `shooter/builds.ts`. Whether that counts as a
  finding is still a policy call.
- **Contract-only TS `*Req`/`*Resp` types** with no UI importer. This still
  needs a policy decision.
- **Rust struct fields that are read only by tests.** This needs a typed
  scanner.
- **The `otto-server/src/lib.rs` crate-alias re-exports**
  (`review_fallback`, `report_delivery`, the `workflow_*` aliases). These
  are deliberate extraction seams. Whether each should go needs a review
  per crate.
- **The behaviour prose in `docs/features/*.md`.** This pass checked paths
  and identifiers only.
- **The `kart`/`shooter` render internals** apart from their exports and
  duplicate helpers.
- **The stale-comment scan beyond the iteration-5 diff.** The identifier
  scan in 6-07 covered only the files that diff touched. A workspace-wide
  run is likely to find more pointers like these.

---

## Findings

### maintainability-6-01: The `otto_sessions::submit` compatibility re-export is still used by five otto-server callers, and five of its six names have no user (minor, carried from 5-03)

- **Where:**
  - `crates/otto-review/src/session.rs:860-864`:
    `pub use otto_sessions::submit::{bracketed_paste, dispatched, submit_prompt, wait_for_tui, wait_for_tui_strict, PASTE_TO_ENTER};`.
    The comment says *"these re-exports keep the old
    `otto_review::session::*` paths compiling"*.
  - `crates/otto-server/src/lib.rs:71`:
    `pub use otto_review::session as review_session;`.
- **Evidence:**
  - Callers still on the old path:
    - `otto-server/src/agent_session.rs:707`
    - `skill_eval.rs:1025`
    - `skill_review.rs:43`
    - `finding_agent.rs:369`
    - `modules/session_input.rs:305`
  - Docs that cite the old path: `session_input.rs:232` and
    `otto-assistant/src/ctx.rs:91`.
  - `otto-sessions/src/submit.rs:17-18` still says *"`otto_review::session`
    re-exports these items under their old names"*.
  - `rg "session::(bracketed_paste|dispatched|wait_for_tui|PASTE_TO_ENTER)"`
    finds nothing. Five of the six re-exported names have no user through
    this path.
  - The only in-crate use is `session.rs:538` `submit_prompt(...)`.
  - FIXES.md:255 lists "budget and submit callers repointed", but only the
    swarm host and the retry test were moved.
- **Fix:**
  - Point the five callers and the two docs at
    `otto_sessions::submit::submit_prompt`. Every crate involved already
    depends on otto-sessions.
  - Make `session.rs` import `submit_prompt` privately.
  - Delete the `pub use` block and the sentence in `submit.rs:17-18`.

### maintainability-6-02: Three accessors still have no caller (nit, carried from 5-07)

- **Where and evidence:**
  - **`ui/src/modules/database/EditFlow.svelte.ts:833`, `mongoIdFilter(rowIdx)`:**
    - `rg -w mongoIdFilter` over `src`, `unit` and `e2e` finds only the
      definition and the doc at `query-filter.ts:128`. That doc still
      describes `mongoValueJson` as matching *"the grid's
      `mongoIdFilter`"*.
    - The adapters call `mongoIdFilterFor` directly.
  - **`crates/otto-k8s/src/resources.rs:118`, `Kind::k8s_kind`:**
    - The only other reference is the intra-doc link at `:141`.
    - That doc says *"Reverse of [`Kind::k8s_kind`]"*, but
      `from_k8s_kind` maps only the Argo workload kinds.
  - **`crates/otto-sessions/src/submit.rs:131`, `wait_for_tui_strict`:**
    - It is referenced only by the 6-01 re-export (`session.rs:863`) and
      its own doc (`:122`).
    - The strict path is reached through `SubmitOpts { echo_wait: true }`.
- **Fix:**
  - Delete the three functions.
  - Point the `query-filter.ts:128` doc at `mongoIdFilterFor`.
  - Reword `from_k8s_kind`'s doc to say it maps the Argo
    `status.resources[].kind` workload kinds.
  - Reword `submit.rs:122` to stand on its own.

### maintainability-6-03: Five more copies of `KeyedLocks` on the allow-list, one of which never prunes (minor, extends 5-02)

- **Where:** each of these is a per-key async-mutex registry whose body
  matches `otto_core::keyed_lock::KeyedLocks::handle`
  (`otto-core/src/keyed_lock.rs:45-57`) or `::lock` (`:61`). None of them
  adds any extra semantics:
  - **`otto-server/src/agent_session_lifecycle.rs:19-35`, `acquire_lease`:**
    a `Weak` map with retain, upgrade or insert, then `lock_owned()`. This
    is the same as `LOCKS.lock(sid).await`, which already returns an
    `OwnedMutexGuard`.
  - **`otto-server/src/proof.rs:28-32` and `:441-447`, `ProofLocks` and
    `lock_in`:** an `Arc` map that prunes on `strong_count > 1`. It is held
    on `ServerCtx` (`state.rs:168`, `boot/ctx.rs:198`).
  - **`otto-server/src/routes/mission.rs:112-123`, `BUILDING` and
    `build_gate`:** the same `strong_count` prune.
  - **`otto-server/src/routes/personal_agents.rs:868-879`, `chat_lock`:**
    the same prune, with the comment *"Drop idle entries … so the map stays
    tiny"*.
  - **`otto-channels/src/telegram.rs:514-521`, `poller_lock`:** an `Arc`
    map with no pruning at all. It grows by one entry for every bot token
    the process has ever polled, and keeps those token strings in memory
    for the life of the process.
- **Evidence:**
  - All five paths are on `KEYED_LOCK_ALLOW` (`scripts/check.sh:220`).
  - The comment there says the list is *"the files not migrated yet: shrink
    it"*.
  - Each crate already depends on `otto-core`.
  - The truly different entries stay on the list:
    - `otto-sessions/src/mcp.rs:124-131` `cwd_lock` uses a sync
      `std::sync::Mutex` and holds it across fs work only.
    - The `otto-dbviewer`, `otto-k8s`, `otto-aws`, `otto-brokers` and
      `otto-vault` single-flight maps pair the lock with a cache or a
      per-service lifetime.
- **Fix:**
  - Replace each body with a `static LOCKS: KeyedLocks<String>`, or make
    the `ServerCtx::proof_locks` field `KeyedLocks<String>`.
  - Call `.lock(k).await` or `.handle(k)` at the call sites.
  - Adapt the `Arc::ptr_eq` tests to use `handle`.
  - Drop the five paths from `KEYED_LOCK_ALLOW`.

### maintainability-6-04: Raw NUL bytes make two Svelte sources binary to git and `rg`, which hides their diffs and their call sites (minor)

- **Where:**
  - `ui/src/modules/database/DiagramView.svelte:141`:
    ``return `${schema}<NUL>${name}`.toLowerCase();``
  - `ui/src/lib/components/ModelPicker.svelte:81`:
    `const CUSTOM = '<NUL>custom';`
  - In both lines, `<NUL>` is a literal 0x00 byte, not a `\0` escape.
- **Evidence:**
  - Running `perl -ne 'print if /[\x00-\x08\x0b\x0c\x0e-\x1f]/'` over every
    tracked text source finds exactly these two files.
  - `git log --numstat` reports `-	-` (binary) for every change to both
    files, including the iteration-5 change to DiagramView
    (`git diff --numstat 8177c4d85..HEAD`). On GitHub, every PR that
    touches them shows "Binary file not shown".
  - Recursive `rg` stops reading a file at the first NUL:
    - `rg -n fetchSchemaGraph ui/src` finds one hit, the store definition
      at `database.svelte.ts:2220`.
    - The real caller is `DiagramView.svelte:105`, below the NUL.
    - This review's dead-method scan flagged `fetchSchemaGraph` as having
      no caller for exactly that reason.
  - Every earlier dead-code and "no importer" scan that used `rg` had the
    same blind spot for anything after line 141 or line 81 of these two
    files.
- **Fix:**
  - Write the separators as escapes: `` `${schema}\u0000${name}` `` and
    `'\u0000custom'`. They behave the same.
  - Add a `check.sh` guard that fails on raw C0 control bytes (other than
    tab, LF and CR) in tracked `*.ts`, `*.svelte`, `*.rs` and `*.js`.

### maintainability-6-05: A new type-only shim (`modules/canvas/types.ts`) and an `orchestrate.ts` re-export slip past the shim ratchet (minor)

- **Where:**
  - **`ui/src/modules/canvas/types.ts:1-4`**, added in the iteration-5
    fixes:
    - The file is `export type * from '../../lib/canvas/types';`.
    - Its comment says *"Re-exported so the module importers keep
      working"*.
  - **`ui/src/lib/orchestrate.ts:383-385`:** `export { plural };`, with
    the comment *"re-exported for existing importers"*. `plural` lives in
    `lib/plural.ts`.
- **Evidence:**
  - The ratchet regex at `scripts/check.sh:256-257` is
    `export \* from '(\.\./)+lib/`.
    - It does not match `export type * from`.
    - It does not match named re-exports.
    - So the count stays at 3 (`swarm/types.ts:5` and
      `product/types.ts:5-6`), and the new shim is not counted.
  - `canvas/types` importers:
    - nine `./types` imports inside `modules/canvas`: `MermaidCanvas`,
      `scene.ts`, `CanvasPage`, `assistDoc.ts`, `ExcalidrawCanvas`,
      `D2Canvas`, `SceneList` and others;
    - `product/LinkedCanvases.svelte:13`;
    - `panels/CanvasPanel.svelte:21`.
  - `plural` is imported through `orchestrate` by
    `shell/Palette.svelte:20` and `lib/ask.ts:14`.
  - This is the fourth iteration that has raised this pattern (3-08, 4-03,
    5-01).
- **Fix:**
  - Point the canvas importers at `lib/canvas/types` and delete
    `modules/canvas/types.ts`.
  - Import `plural` from `lib/plural` in `Palette.svelte` and `ask.ts`, and
    delete the re-export.
  - Widen the ratchet to `export (type )?\* from '(\.\./)+lib/`.

### maintainability-6-06: `watchVisibility` in the shared three.js stage helper has no caller, and the surfaces it was written for still hand-roll it (minor)

- **Where:** `ui/src/lib/three/stage.ts:40-69`, `watchVisibility(el, change, rootMargin)`.
- **Evidence:**
  - `rg -w watchVisibility` over `src`, `unit` and `e2e` finds only the
    definition. There is no caller and no test.
  - The module doc (`:1-8`) says it is *"The WebGL lifecycle every three.js
    surface shares … Each used to hand-roll it"*.
  - The same surfaces still hand-roll the visibility half:
    - `design-hall/studio3d/embed.ts:166` and `:366-370` build their own
      `IntersectionObserver` and `visible` flag.
    - `home/school/scene.ts:589-599` (`visibilityChanged`) and `:762` add their own
      `visibilitychange` listener.
  - The other `stage.ts` helpers (`cappedPixelRatio`, `onContextLost`,
    `releaseRenderer`) are used by `games/common/view.ts`,
    `school/scene.ts` and `studio3d/embed.ts`.
- **Fix:** Use `watchVisibility` in `studio3d/embed.ts`, which needs both
  the in-view and the document-hidden states, and in the classroom scene.
  Add a unit test for the flip semantics. If neither surface should use it,
  delete the function and stop the module doc from claiming it.

### maintainability-6-07: Code comments and the API contract point at deleted functions and files (minor)

- **Where and evidence** (each checked: the cited name or path has no
  definition anywhere in the code):
  - **`docs/contracts/api.md:200`:** *"The daemon exposes a consultable
    `routes::usage::check_budget(…)`"*. The service is
    `crate::budget::check_budget`. The `routes::usage` re-export was
    deleted in iteration 5.
  - **`crates/otto-dbviewer/src/multirun/params.rs:4`** cites
    `ui/src/modules/database/sql-util.ts`. The file is now
    `ui/src/lib/database/sql-util.ts`.
  - **`crates/otto-dbviewer/src/errors.rs:2`** cites
    `ui/src/modules/database/error-normalize.ts`. It is now
    `lib/database/error-normalize.ts`.
  - **`crates/otto-vault/src/engine.rs:2708`:** *"mirrors
    ui/src/lib/fuzzy.ts"*. That file was deleted in `eec25c214`. The UI
    matcher is `fuzzyMatch` in `ui/src/lib/commandSearch.ts:15`.
  - **`crates/otto-server/src/workflow_engine.rs:2547-2548`:** *"see the
    removal note above the old `cache_exempt_kind` site"*. Neither the
    note nor the site exists. The cache was dropped in `344315ea7`.
  - **`crates/otto-channels/src/slack.rs:401`:** *"(see `process_event`)"*.
    There is no `process_event`. The flow is `handle_event` (`:1134`) and
    `drop_reason` (`:1093`).
  - **`crates/otto-server/src/api_secrets.rs:127`** documents a parameter
    called `expect_request_id`. The parameter is `allowed_request_ids`
    (`:134`).
  - **`ui/src/modules/database/builder/sql-builder.ts:7`:** the "Safety
    model" header says contains, starts-with and ends-with values go
    through `likeLiteral`.
    - No `likeLiteral` has ever existed (`git log -S` shows only the
      commit that added this comment).
    - The real path is `escapeLike` (`:266`) plus `stringLiteral`
      (`:375`).
    - Someone auditing the escaping would look for a function that is not
      there.
  - **`crates/otto-swarm/src/runtime/run.rs:6`** and **`wake.rs:13`** cite
    `swarm_runtime` and `swarm_runtime.rs`. The coordinator moved into
    `otto-swarm/src/runtime/` (`engine.rs` and siblings) in `ff135f71b`.
- **Fix:** Update each pointer to the current name or path. For the
  `workflow_engine.rs` note, say it directly ("no cross-run node-result
  cache; removed in 344315ea7"). For `sql-builder.ts:7`, name
  `escapeLike` + `stringLiteral`.

### maintainability-6-08: The shooter keeps a private copy of the shared `canvas()` helper (nit, residual of 5-06)

- **Where:** `ui/src/modules/rooms/games/shooter/meshes.ts:187-191`,
  `function canvas(size)`.
- **Evidence:**
  - Its body is the same as `common/textures.ts:6-10`
    `canvas(w, h)`, with `w = h = size`.
  - The shared helper's doc says it is *"shared by the kart's
    `render/surfaces.ts`"*.
  - `meshes.ts` already imports from `common/math`.
- **Fix:** Import `canvas` from `../common/textures` and call
  `canvas(128, 128)` in `materialTexture`. Delete the local copy.
