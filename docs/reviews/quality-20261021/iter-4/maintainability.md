# Maintainability review: iteration 4 (2026-10-22)

Lens: dead code, unused exports, routes and settings, duplicated helpers,
misleading names and comments, stale TODOs that point at finished work,
idioms that differ from the surrounding code, and code that is more clever
than it needs to be.

Worktree: `fix/quality-loop-20261021` at `6cff08270`. The review was
read-only. It used grep, rg, sed, git and small read-only Python scanners in
the session scratchpad. Nothing was built and no tests were run.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per
minor and 0.03 per nit.

| Severity | Count | Deduction |
|----------|------:|----------:|
| blocker  | 0 | 0.00 |
| major    | 0 | 0.00 |
| minor    | 4 | 0.40 |
| nit      | 7 | 0.21 |
| **total**|   | **0.61** |

**Score: 10.0 − 0.61 = 9.39 / 10.** Earlier scores: iteration 1 7.25,
iteration 2 8.28, iteration 3 8.65.

## Status of the iteration-3 findings

| Id | Iter-3 severity | Status now | Evidence |
|----|-----------------|------------|----------|
| 3-01 | major | Fixed | `git.svelte.ts` has no `prs`, `prsLoading`, `prError`, `prsGen` or `loadPrs`. `unit/gitPrimaryOwnership.test.ts:74` now asserts that no `/prs` call is made. |
| 3-02 | minor | Fixed | All 12 fields have 0 hits. |
| 3-03 | minor | Fixed | `AttachIssue.svelte:27` returns early on `if (attaching) return;`. `mintedToken`, `qreason` and `findingsLoading` are gone. |
| 3-04 | minor | Fixed | `McpServerProvider`, `transcript_paths` and `NOTIFY_APPROVAL_KEY` are gone. `FINDINGS_POLL` is private and used (`session.rs:55`, `:938`). |
| 3-05 | minor | Fixed | `repo_directory.rs:256-268` returns `None` for `~user` and expands through `otto_core::paths::expand_tilde`. |
| 3-06 | minor | Fixed for every listed site | **New evidence:** four other `modules.rs` pointers are still wrong. See 4-05. |
| 3-07 | minor | **Partly open** (now 4-07) | Every cost goes through `formatUsd`, and the conversation tokens go through `formatCount`. `lib/sessionUsage.ts:13` `formatTokens` is still a separate formatter ("12.3K"). |
| 3-08 | minor | Fixed | The scene3d shims and `modules/database/JsonTree.svelte` are deleted, and `lib/components/JsonTree.svelte` imports only from `lib/jsonTree/`. **New evidence:** four more shims of the same kind exist. See 4-03. |
| 3-09 | minor | Fixed | Every `crates/*` and `ui/src/modules/*` name now appears in AGENTS.md (checked by script). |
| 3-10 | minor | Fixed | `connections-ssh-sftp.md` now says that there is no **Pinned** group. |
| 3-11 | minor | Fixed | All four links are retargeted, and `scripts/check.sh:233` runs rustdoc with `-D rustdoc::broken_intra_doc_links`. |
| 3-12 | nit | Fixed | `GlobalRole`, `DbResult` and `NsQuery` are gone from Rust. `governance.rs` validates against `const STATES`. The TS mirror of `GlobalRole` was left behind (see 4-09). |
| 3-13 | nit | Fixed | |
| 3-14 | nit | **Partly open** (now 4-08) | `OverviewTab` and Excalidraw use `unescapeHtml`. `agents/conversation/codeBlocks.ts:49` still has its own `unesc`. |
| 3-15 | nit | Fixed | Both banners sit directly above `#[cfg(test)]` (`activity.rs:746-750`, `local.rs:5548-5549`). |

Totals: 13 fixed, 2 partly open and carried forward, and 0 found to be
invalid.

## Areas covered in this iteration

- **Re-checked every iteration-3 finding** against the current code (see the
  table above).
- **The iteration-3 fix diff** (`3dd55d9f2..HEAD`):
  - `otto-core/src/api.rs` was split into `api/*.rs`.
  - `api_helpers.rs` moved to `otto-apiclient::rest`.
  - `KeyedLocks` and the `check.sh` guard for it, including the guard's
    allow-list.
  - The workflows inspector split, `paletteSources` and the App shell split.
  - The new issue client.
  - New `pub` items were scanned for single references. Only contract DTOs
    turned up, and those are valid.
- **Rust `pub fn` with no call-like reference anywhere in the workspace.**
  This used a new scanner that ignores comments and strings. Iteration 3's
  scanner counted doc mentions and error-label strings as uses, so it missed
  these. Each hit was confirmed with `rg -w`. Serde `default_*` functions
  and test-only helpers were dropped as false positives.
- **Write-only Rust struct fields** (deferred from iteration 3). A scanner
  checked the fields of non-serde `pub struct`s for any read. Confirmed:
  `ObjectSnap::fields_ready`. Rejected: `PendingTask::since_line`, which is
  debug data and is compared by the derived `PartialEq`.
- **Hand-rolled helpers that copy `otto_core::text`** (the 125
  `chars().take` sites that iteration 3 deferred). The local
  `clip`/`truncate`/`cap_chars` functions were compared one by one. Most
  either delegate or differ on purpose: whitespace flattening, `n-1` plus
  `…`, byte budgets, or `otto-transcript`, which deliberately has no
  `otto-core` dependency.
- **UI exports with no importer** (a fast word-count scanner) and
  **write-only component `$state`** across `ui/src`. The excluded areas were
  skipped.
- **Re-export shims across `ui/src` and `otto-server`**, with their
  importers counted.
- **Docs and comments that name deleted or moved items** (`docs/features`,
  `docs/design` and crate docs).
- TODO/FIXME sweep of `crates/` and `ui/src`: none are stale.

## Areas not covered yet (for iteration 5)

- `ottod/src/mcp_tools.rs` compared with `otto-mcp/src/outward/catalog.rs`.
  The parity test's `DEFERRED_SCHEMA_DRIFT` still tracks this.
- **Write-only fields of non-`pub` Rust structs**, and fields that are read
  only by tests.
- **`pub(crate)` functions and Svelte-store methods that nothing calls.** The
  dead-fn scanner looked only at `pub fn`.
- **Behaviour text in `docs/features/*.md`** apart from the pointers checked
  here, for example `review-findings.md`, `goal-loops`, `vault` and `browser`.
- **The 24 files on the keyed-lock guard's allow-list.** Each is a
  candidate to migrate to `KeyedLocks`.
- **Contract-only TS request types** in `lib/api/types/*.ts` that no UI code
  imports (about 70 `*Req`/`*Resp` types). They mirror Rust DTOs, so they
  need a policy decision before anyone reports them.
- `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**` and
  `ClassroomsBox.svelte` stay excluded because another team is rewriting
  them.

---

## Findings

### maintainability-4-01: Dead "unsafe predecessor" APIs keep the simpler name and reintroduce bugs that have already been fixed (minor)

Each of these public functions has no caller. A safer sibling replaced it,
yet the dead one keeps the shorter, canonical-looking name. Its behaviour is
exactly what the live path was changed to avoid.

- **`SessionManager::relay` and `RelayOutcome`**
  (`crates/otto-sessions/src/manager.rs:2270-2340`, `:1619-1632`)
  - `rg '\.relay\('` finds nothing.
  - `POST /workspaces/{id}/relay` is served by `workspace_relay`
    (`otto-server/src/modules.rs:403-459`). That handler resolves addresses
    only against `input_agents(&ctx, who, &ws_id)`, the candidates the caller
    is allowed to reach, and sends through `submit_session_text`.
  - The dead `relay` instead addresses every live agent session in the
    workspace through `list_by_workspace` and calls `self.submit_text`, with
    no caller identity.
  - Its `Addressable` mapping is a line-for-line copy of the handler's
    mapping.
  - `RelayOutcome`'s rustdoc also begins with a stray line,
    `/// Owns live sessions: PTY handles keyed by session id plus persistence.`
    (`:1619`). That line was meant for `SessionManager`.
- **`ImprovementsRepo::set_edit_status`**
  (`crates/otto-state/src/improvements.rs:371-394`)
  - It is an unconditional `UPDATE … SET status`.
  - The doc of the CAS sibling right below it (`:396-400`) explains that this
    shape let *"two decisions on one edit … both 'win', leaving a file
    applied under a Rejected edit that could then never be rolled back"*.
  - It has no caller.
- **`ReviewFindingsRepo::resolve_absent`**
  (`crates/otto-state/src/review_findings.rs:439-456`)
  - It calls `resolve_absent_scoped(…, None, None)`.
  - The scoped variant's doc (`:459-465`) says the unscoped call is *"WRONG
    for local reviews … resolved every open local finding in the repo"*.
  - Production calls only `resolve_absent_scoped`
    (`modules/review/summarize.rs:483`).
  - Even so, `summarize.rs:17`, `:276` and `:427` and
    `docs/features/review-findings.md:152` and `:380` still name
    `resolve_absent` as the resolution step.
- **Fix:**
  - Delete `relay` and `RelayOutcome`, and move the stray doc line onto
    `SessionManager`.
  - Delete `set_edit_status`, and keep only the CAS function. Consider giving
    the CAS function the plain name.
  - Delete `resolve_absent`, or make it private.
  - Point the four comment and doc mentions at `resolve_absent_scoped`.

### maintainability-4-02: Fifteen more public functions that nothing calls, and two docs that describe them as live (minor)

Each name has no call, path or function-pointer reference outside its
definition (`rg -w` shows only the definition, its `dberr("…")` label and
doc mentions).

- **`crates/otto-vault/src/store.rs`:**
  - `upsert_note` (`:455`), `replace_links` (`:743`), `replace_tags`
    (`:770`), `unresolved_sources` (`:874`), `update_link_dst` (`:942`) and
    `like_search` (`:1367`).
  - The indexer uses the `*_conn`/transaction forms instead, and search goes
    through `search_filtered` (`:1159`).
  - `fts_search` (`:1340`) is reached only from a test (`:1598`).
- **`crates/otto-state/src/memory.rs`:** `graph_nodes` (`:809`),
  `ids_by_source_ref` (`:859`) and `all_links` (`:882`). These are left over
  from the graph backend that `9a8f8723d` removed ("remove embeddings,
  vector/graph backends").
- **`crates/otto-state/src/goal_loops.rs`:** `add_elapsed` (`:382`) and
  `set_config` (`:431`).
  - The doc on `finalize` (`:444-450`) is two paragraphs pasted together.
    The first says *"The controller adds the final active window via
    [`add_elapsed`] before calling this"*. The second says the window is
    banked *"in the same statement … so the time is never counted twice"*.
  - Following the first paragraph would double-count elapsed time.
- **`crates/otto-git/src/local.rs`:**
  - `staged_diff_text` (`:3098`). Only `staged_diff_text_capped` is used.
  - `pull_autostash` (`:3674`). Only `ops.rs` `pull_autostash_mode` is used.
- **`crates/otto-improve/src/engine.rs:93` `execute_run_with_autonomy`.** Its
  doc says *"The workflow `self_improve` node uses `Autonomy::Propose`"*.
  The node calls `execute_run_with_autonomy_providers`.
- **`crates/otto-usage/src/clickhouse.rs:123` `keep_awake`.** Its doc says it
  exists *"while an external local exporter uses it"*. Nothing does.
- **`crates/otto-browser/src/cdp.rs:237` `wait_for_load_event`.** Only the
  guarded variant (`:269`) is called.
- **`crates/otto-browser/src/live/session.rs:1236` `agent_input`.** It has
  never been called since `a6054781c` introduced it.
- **`crates/otto-state/src/broker_audit.rs:69` `BrokerAuditRepo::recent`.**
  Nothing reads `broker_write_audit`; no route or UI code does.
- **Fix:**
  - Delete the functions.
  - Delete the first paragraph of `finalize`'s doc.
  - Fix the doc of `execute_run_with_autonomy_providers` so it no longer
    refers to a deleted sibling.
  - For the vault store, either move `fts_search` into the test module or
    delete it together with that test.

### maintainability-4-03: Four more "moved to lib/, re-exported for existing importers" shims, which every importer still uses (minor; same pattern as 3-08)

- **Where:**
  - `ui/src/modules/vault/mdRender.ts`: re-exports `lib/render/markdown.ts`
  - `ui/src/modules/canvas/mermaid.ts`: re-exports `lib/render/mermaid.ts`
  - `ui/src/modules/canvas/renderQueue.ts`: re-exports
    `lib/render/renderQueue.ts`
  - `ui/src/modules/canvas/excalidraw-build.ts`: re-exports
    `lib/excalidraw/excalidraw-build`
- **Evidence:**
  - These shims came from `49ede5829` and `eec25c214`.
  - All in-tree importers still use them:
    - `mdRender`: 8 importers. Besides the vault files, these are
      `workbench/preview/MarkdownPreview.svelte:7`,
      `browser/ReaderView.svelte:24`, `panels/OutputsPanel.svelte:22` and
      `agents/conversation/chatMarkdown.ts:13`.
    - `canvas/mermaid`: 10 static importers plus a dynamic import at
      `panels/CanvasPanel.svelte:98`. These include
      `design-hall/ArtifactStage.svelte:26` and `ArtifactThumb.svelte:121`,
      four product files, two workbench previews and
      `vault/NoteView.svelte:46`.
    - `renderQueue`: `canvas/d2.ts:13` and `unit/renderQueue.test.ts:6`.
    - `excalidraw-build`: `canvas/ExcalidrawCanvas.svelte:30`.
  - So the cross-module imports that the moves were meant to remove are
    still there. The excalidraw shim's own comment says the Design Hall
    *"must not import a feature module"*, yet design-hall imports
    `../canvas/mermaid`.
  - The docs and comments still name the shim as the renderer:
    - `docs/features/conversation-view.md:218`
    - `docs/design/conversation-view.md:29` and `:407`
    - `agents/conversation/codeBlocks.ts:40`
    - `panels/OutputsPanel.svelte:7`
- **Fix:**
  - Point the roughly 22 importers at `lib/render/*` and
    `lib/excalidraw/*`, then delete the four shims.
  - Update the two doc tables and the two comments.

### maintainability-4-04: otto-channels defines its own `KeyedLocks<K>` that shadows `otto_core::keyed_lock::KeyedLocks` and keeps a pruning rule the shared type replaced (minor)

- **Where:** `crates/otto-channels/src/bridge.rs:52-95`
- **Evidence:**
  - `otto_core::keyed_lock` (`keyed_lock.rs:1-14`) says it *"replaces the
    hand-rolled … statics that had drifted into three pruning rules (weak
    entries, a strong-count of 1, never …)"*.
  - The bridge has a private `struct KeyedLocks<K>` with the same name and a
    different contract: a `HashMap` that is pruned by
    `Arc::strong_count(l) > 1`, and only once it reaches `PRUNE_AT = 256`.
    That is the strong-count rule the shared type was written to retire.
  - A reader who meets `KeyedLocks` in `bridge.rs` will assume it is the
    shared type, with its weak entries and prune on insert.
  - `check.sh:220` allow-lists `otto-channels/src/bridge.rs`, so the guard
    stays silent.
  - The only extra the bridge needs is `enqueue` (fixed FIFO position). The
    shared type already exposes the handle it needs: `KeyedLocks::handle<Q>`
    returns `Arc<AsyncMutex<()>>`.
- **Fix:**
  - Replace the private struct with `otto_core::keyed_lock::KeyedLocks<ConvKey>`.
  - Write `enqueue` as a free function over `locks.handle(key).lock_owned()`.
  - Drop `bridge.rs` from `KEYED_LOCK_ALLOW`.
  - If the bridge must keep its own type, at least rename it (for example
    `ConvLockMap`) so it no longer shadows the shared one.

### maintainability-4-05: Four `modules.rs` pointers that the iteration-3 sweep missed (nit; the rest of 3-06)

- **Where and evidence:**
  - `crates/otto-review/src/engine.rs:5` says *"the ctx-bound orchestration
    in otto-server (`modules.rs`) drives these"*. The orchestration lives in
    `otto-server/src/modules/review/`.
  - `crates/otto-review/src/session.rs:704` says *"`review_run_complete` in
    `modules.rs` keys on"*. `review_run_complete` is in this same crate, at
    `otto-review/src/engine.rs:659`.
  - `crates/otto-improve/src/engine.rs:78` says *"mirroring the PR-review run
    pattern in `modules.rs`"*. That pattern is in `modules/review/routes.rs`.
  - `docs/features/proof-packs.md:300` says *"`after_pr_created` in
    `crates/otto-server/src/modules.rs`"*. The impl is at
    `modules/ctx_impls.rs:298`.
- **Fix:** Retarget the four pointers. Name the directory
  (`modules/review/`) rather than a file.

### maintainability-4-06: Two verbatim copies of `otto_core::text::clip_chars` in crates that already depend on otto-core (nit)

- **Where:**
  - `crates/otto-issues/src/linear.rs:664-669` (`truncate`)
  - `crates/otto-design-assist/src/lib.rs:600-608` (`cap_chars`)
- **Evidence:**
  - Both are "unchanged if it has at most `max` chars, else the first `max`
    chars followed by `…`", which is `clip_chars` (`otto-core/src/text.rs:49`).
  - `cap_chars` counts the chars and then takes them again, so it walks the
    string twice. The shared helper walks it once.
  - Both crates list `otto-core` in `Cargo.toml`.
  - The sibling helpers in `otto-product/chat.rs:765`, `activity.rs:644`
    and `run_notices.rs:292` already delegate.
- **Fix:** Replace both bodies with `otto_core::text::clip_chars(s, max)`, or
  inline that call at the call sites.

### maintainability-4-07: `formatTokens` is still a second token formatter, so one session reads "12.3K" in the sidebar and "12.3k" in its conversation (nit; the rest of 3-07)

- **Where:** `ui/src/lib/sessionUsage.ts:13-28`, used by
  `sessionUsageLabel`, which renders in `shell/Navigator.svelte`.
- **Evidence:**
  - The conversation view's `fmtTokens` delegates to `formatCount`
    (`agents/conversation/format.ts:29-31`), which uses a lower-case `k`.
  - `formatTokens` uses `K` and is pinned by `unit/sessionUsage.test.ts:8`.
- **Fix:** Make `formatTokens` call `formatCount`, or delete it and call
  `formatCount` from `sessionUsageLabel`. Update the test's expected string.

### maintainability-4-08: `codeBlocks.ts` still hand-rolls the entity decoder and aliases the escaper (nit; the rest of 3-14)

- **Where:** `ui/src/modules/agents/conversation/codeBlocks.ts:47-49`
- **Evidence:**
  - `const esc = escapeHtml;` is an alias.
  - `const unesc = (s) => s.replace(/&#39;/g…).replace(/&amp;/g, '&')` is a
    five-step chain. `lib/html.ts:30-35` now exports `unescapeHtml` for
    exactly this purpose: it decodes once, and it also handles `&apos;`.
- **Fix:** Import `unescapeHtml` and use `escapeHtml` and `unescapeHtml`
  directly.

### maintainability-4-09: Exports that nothing imports, including a test hook no test uses and a mirror of a deleted Rust type (nit)

- **Where and evidence:**
  - `ui/src/lib/api/issues.ts:180`: `export const __issuePathForTest = issuePath;`,
    documented *"Exposed for the unit test"*. No file in `ui/unit` or
    `ui/e2e` references it. The four issue-client tests load the module and
    stub `get` instead.
  - `ui/src/lib/stores/mockup-assist.svelte.ts:24`: `MockupFormat`, a
    *"Back-compat alias"* with no users.
  - `ui/src/lib/api/types/core.ts:10`: `export type GlobalRole`. Iteration 3
    deleted the Rust `GlobalRole` (3-12), but its TS mirror stayed, against
    the "contract and types in lockstep" rule. Nothing imports it.
- **Fix:** Delete all three.

### maintainability-4-10: `ObjectSnap::fields_ready` is written at 13 sites and never read, and its doc describes a lazy-fill signal that does not exist (nit)

- **Where:** `crates/otto-dbviewer/src/complete/mod.rs:167-178`. It is
  written in the MySQL, Postgres, ClickHouse and MongoDB drivers, in
  `errors.rs`, `service/completion.rs` and `complete/sql.rs`.
- **Evidence:**
  - `rg '\.fields_ready'` finds nothing.
  - The doc on `fields` says the Mongo field paths are *"filled lazily; see
    [`ObjectSnap::fields_ready`]"*. In fact Mongo sampling is driven by the
    separate `fields` cache and the `fields_building` gates (`:335-345`).
    The flag (`false` only at `mongodb.rs:1448`) is never consulted.
- **Fix:** Delete the field and its 13 writes, and point the `fields` doc at
  the sampled-field cache.

### maintainability-4-11: The workflows module exports two different `LoopStep` types and two different `loopSteps()` functions (nit)

- **Where:**
  - `ui/src/modules/workflows/canvasGeometry.ts:11-18`:
    `interface LoopStep { name; kind }`. Its `loopSteps(n)` fills in
    `step N` and `?`.
  - `ui/src/modules/workflows/inspector/util.ts:135` and `:170-173`: a
    different `LoopStep`, the raw param object. Its `loopSteps(node)` returns
    `params.steps` unchanged.
- **Evidence:**
  - Both came out of the iteration-3 WorkflowsPage split.
  - The two files are siblings in one module. An import from the wrong one
    still compiles wherever only `.name` or `.kind` is used, but gives raw
    `undefined` names in place of the defaults.
  - `inspector/Loop.svelte:10` and `:16` already have to alias one as
    `loopStepsOf` to avoid the clash.
- **Fix:** Rename the canvas pair (for example `LoopStepLabel` and
  `loopStepLabels`), or build it on `util.ts`'s `loopSteps` plus a mapper.
