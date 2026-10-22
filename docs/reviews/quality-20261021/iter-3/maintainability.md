# Maintainability review: iteration 3 (2026-10-22)

Lens: dead code, unused exports, routes and settings, duplicated helpers,
misleading names and comments, stale TODOs that point at finished work,
idioms that differ from the surrounding code, and code that is more clever
than it needs to be.

Worktree: `fix/quality-loop-20261021` at `3dd55d9f2`. The review was
read-only. It used grep, rg, sed, git and small read-only Python scanners in
the session scratchpad. Nothing was built and no tests were run.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per
minor and 0.03 per nit.

| Severity | Count | Deduction |
|----------|------:|----------:|
| blocker  | 0  | 0.00 |
| major    | 1  | 0.30 |
| minor    | 9  | 0.90 |
| nit      | 5  | 0.15 |
| **total**|    | **1.35** |

**Score: 10.0 − 1.35 = 8.65 / 10.** Iteration 1 scored 7.25 and iteration 2
scored 8.28.

## Status of the iteration-2 findings

| Id | Iter-2 severity | Status now | Evidence |
|----|-----------------|------------|----------|
| 2-01 | major | Fixed (functional part) | `boot/build.rs:71` and `test_support.rs:115` both chain `.with_ops(BrokerOpsRepo::new(..))`. The structural part (make ops a required `new` argument) is deferred in FIXES.md. It is not re-scored. |
| 2-02 | major | Fixed | All eight copies now delegate to `otto_core::paths::expand_home_strict` or `expand_tilde` (`connections/http.rs:714`, `dbviewer/http.rs:866`, `fs.rs:111`, `skill_review_static.rs:260`, `git/http.rs:1271`, `workspaces.rs:99`, `s3_download.rs:152`, `vault/engine.rs:254`). **New evidence:** a ninth copy, which the iteration-2 scan missed, still has the `~user` bug. See 3-05. |
| 2-03 | minor | Fixed | `otto_core::aws_env::apply_aws_env` applies `AWS_ENV_REMOVE` at all 10 spawn sites. The k8s wrapper is renamed `aws_env_from_row`. |
| 2-04 | minor | Fixed | All 11 functions and `Story.flag` are gone (each name now has 0 hits). |
| 2-05 | minor | Fixed | `get_cached_output` and `set_cached_output` are gone. |
| 2-06 | minor | Fixed | k8s, aws and scene3d now delegate to `lib/metric-format.formatBytes` (k8s passes `{ iec: true }`). |
| 2-07 | minor | Fixed | Every remaining local helper is a one-line wrapper over the shared `rel()`. `RunInspector`'s absolute formatter is renamed `fmtWhen`. |
| 2-08 | minor | **Partly open** (now 3-10) | `api.md:856` now documents `{ids:[…]}`. The routes stay on purpose (see FIXES.md). The feature doc still describes a "Pinned" group that the UI does not have. |
| 2-09 | minor | Fixed | `message-brokers.md:547` points to the shared `/connection-sections` API. |
| 2-10 | minor | Fixed | All 17 members have 0 hits. |
| 2-11 | minor | Fixed | `lib/html.ts:17` is the only escaper. Some leftovers remain; see 3-14. |
| 2-12 | minor | Fixed | WorkflowsPage calls `patchWorkflow` at four sites, and `getWorkflowVersion` is gone. |
| 2-13 | nit | Fixed | |
| 2-14 | nit | **Partly open** (now 3-15) | Four of the six banners were moved. `activity.rs:707-709` and `local.rs:5562-5564` still sit above production code. |
| 2-15 | nit | Fixed | |
| 2-16 | nit | Fixed | `cancel.rs:10-16` documents when to use which registry. `state.rs` spells the type `otto_core::cancel::CancelRegistry` everywhere. |

Totals: 14 fixed, 2 partly open and carried forward, and 0 found to be
invalid.

## Areas covered in this iteration

- Re-checked every iteration-2 finding against the current code (table above).
- **Iteration-2 split commits** (`52edfdbe7` backend splits, `49ede5829` UI
  splits). Checked every `modules.rs` and `workflow_engine.rs` reference in
  code comments and docs against where the named items live now. Checked
  the compatibility shims that the moves left behind and who still imports
  them.
- **Svelte component props** (a scanner over 356 of the 423 `$props()`
  components): every declared prop is read, apart from the underscore-ignored
  box-protocol props.
- **Svelte write-only state**: scanned component-local `let x = $state` and
  store class fields across `ui/src`, `ui/unit` and `ui/e2e`. Each hit was
  confirmed by hand.
- **Rust `pub` items** across `crates/`, `apps/` and `examples/`: scanned
  `pub fn`, `pub const`, `pub static`, `pub struct`, `pub enum`, `pub type`
  and `pub trait` items that are referenced only at their definition.
- **Rust intra-doc links** (`[`name`]`) whose target exists nowhere in the
  workspace.
- **UI modules and exports**: files that nothing imports, and exports with a
  single reference, across `ui/src`, `ui/unit`, `ui/e2e` and `ui/scripts`.
- **The duplicate-helper families** that iteration 2 named but did not
  compare: `clip` (most now delegate to `otto_core::text`), `truncate`,
  `valid_id` (domain-specific, so valid), and `home_dir`. Also UI cost,
  token, number and date formatters.
- **The AGENTS.md crate and UI-module map** against `crates/` and
  `ui/src/modules/`.
- **Guard baselines**: `ui/scripts/ui-guards-baseline.json` and
  `scripts/loc-baseline.json` have no entries for files that no longer exist.
- **`#[allow(dead_code)]` sites**: all five remaining sites are valid.

## Areas not covered yet (for iteration 4)

- `ottod/src/mcp_tools.rs` compared with `otto-mcp/src/outward/catalog.rs`.
  The parity test (`mcp_tools/tests/catalog_tests.rs`) already tracks the
  known drift in `DEFERRED_SCHEMA_DRIFT` (architecture-1-05), so it was not
  re-reported here.
- Write-only **Rust** struct fields (`pub` fields are not covered by
  `dead_code`).
- `docs/features/*.md` behaviour text, beyond the `modules.rs` pointers and
  the connections pin section.
- Inline `chars().take(N).collect()` clipping (125 sites). Most are
  intentional caps, and they were not compared one by one.
- `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**` and
  `ClassroomsBox.svelte` were excluded on purpose because another team is
  rewriting them.

---

## Findings

### maintainability-3-01: The git store's PR list is never rendered, but it still calls the forge on every repo switch (major)

- **Where:**
  - `ui/src/lib/stores/git.svelte.ts:144-147`: the `prs`, `prsLoading`
    and `prError` fields
  - `:424-443`: `loadPrs`
  - `:421`: `selectPrimary` calls `void this.loadPrs(repo.id)`
  - `:488`: `refreshPrimary` calls it too
- **Callers in production:**
  - `ui/src/shell/App.svelte:502` (`git.loadRepos` on every workspace
    change, and again on focus retry at `:508`)
  - `ui/src/modules/git/GitPanel.svelte:25` (`git.detectFor` each time the
    side panel's folder changes)
- **Evidence:**
  - No component reads `git.prs`, `git.prsLoading` or `git.prError`. A
    search of every file that imports `stores/git.svelte` finds only
    `unit/gitPrimaryOwnership.test.ts:50-64`.
  - The PR tab renders `PrList.svelte`, which fetches its own list
    (`PrList.svelte:74`). FocusView fetches its own too (`FocusView.svelte:64`).
  - Even so, `selectPrimary` and `refreshPrimary` fire
    `GET /repos/{id}/prs?state=open`. `pr_list` (`otto-git/src/http.rs:2847-2848`)
    resolves the provider and calls `provider.list_prs` on GitHub or
    Bitbucket.
  - So every workspace load, side-panel folder change and primary refresh
    spends a forge API call (and rate limit) on a result nobody sees.
  - `prError`'s comment says *"Surface the upstream reason (e.g. a 401 bad
    token) instead of a misleading 'no pull requests'"*. Nothing surfaces it.
  - Commit `09da05a90` hardened this dead path against stale responses, and
    a unit test locks it in.
- **Fix:**
  - Delete `prs`, `prsLoading`, `prError`, `prsGen` and `loadPrs`, plus the
    two `void this.loadPrs(...)` calls and the `this.prs = []` resets at
    `:386` and `:395`.
  - Rewrite `unit/gitPrimaryOwnership.test.ts` to assert stale-response
    dropping on `primaryStatus` instead.

### maintainability-3-02: Twelve store fields are written but never read, and three have docs that describe consumers that do not exist (minor)

- **Where (each name has only its declaration and its assignments across
  `ui/src`, `ui/unit` and `ui/e2e`):**
  - `lib/events.svelte.ts`:
    - `:50` `ImprovementUpdateBus.lastKind`
    - `:52` `lastId`
    - `:173` `FindingBus.findingId`
  - `lib/stores/product.svelte.ts`:
    - `:193` `loadingVersions`
    - `:194` `loadingAnalyses`
  - `lib/stores/mockup-assist.svelte.ts:53` `lastResult`. Its doc says
    *"(so the arena can select it)"*. No arena code reads it.
  - `lib/stores/transcript.svelte.ts:186` `lastAppendAt`. Its doc says
    *"the draft is hidden until the screen text moves past…"*. Nothing reads
    it.
  - `lib/stores/serviceHealth.svelte.ts:14` `lastDownAt`. Its doc says
    *"(for 'last seen' display)"*. There is no such display.
  - `lib/stores/k8s.svelte.ts`:
    - `:186` `statusLoading`
    - `:195` `clustersLoading`
    - `:700` `monitorTickCluster` (consumers read `monitorTickClusters`)
  - `lib/stores/database.svelte.ts:831` `objectSearchSupported`
- **Evidence:**
  - These are `$state` fields, so every write still runs reactive
    bookkeeping.
  - Three of the doc comments promise behaviour that does not exist, so a
    reader is misled about what drives the draft, the arena and the outage
    banner.
- **Fix:**
  - Delete the 12 fields and their writes.
  - Where the doc names a feature that was meant to exist ("last seen" or
    selecting the arena attachment), either build that feature or delete the
    sentence.

### maintainability-3-03: Four component `$state` flags are write-only, and AttachIssue's busy flag never guards anything (minor)

- **Where:**
  - `ui/src/modules/agents/AttachIssue.svelte:16`, `:26`, `:43` (`attaching`)
  - `ui/src/modules/agents/ShareModal.svelte:43`, `:137`, `:153`, `:202`,
    `:239` (`mintedToken`)
  - `ui/src/modules/aws/AthenaView.svelte:94`, `:268`, `:339` (`qreason`)
  - `ui/src/modules/git/ReviewPanel.svelte:120`, `:320`, `:336`
    (`findingsLoading`)
- **Evidence:**
  - Each variable is only ever assigned.
  - `attaching` is set around `ws.attachIssue(...)`, but nothing disables
    the picker or returns early, so a double pick attaches twice. The flag
    looks like a guard but is not one.
  - `qreason` duplicates `resultError`, which already embeds `s.reason`
    (`:340`).
  - `findingsLoading` is set beside `mergeReadinessLoading`, which is the
    flag that is actually rendered.
  - `mintedToken` keeps the raw share token in component state although only
    `mintedUrl` is displayed.
- **Fix:**
  - AttachIssue: add `if (attaching) return;` at the top of `attach`, or
    pass `disabled={attaching}` to the picker.
  - Delete the other three variables.

### maintainability-3-04: Dead `pub` items whose docs describe a contract or role they no longer have (minor)

- **Where and evidence:**
  - **`otto_core::hooks::McpServerProvider`** (`crates/otto-core/src/hooks.rs:77-88`)
    - Its doc says it is *"queried by `SessionManager` just before an agent
      spawn"* and that *"implementations handle their own errors (log,
      return empty)"*.
    - Nothing implements or uses it. The spawn path uses
      `otto_sessions::mcp::McpServerSource` (`otto-sessions/src/mcp.rs:155-166`).
      That trait is fallible *on purpose*: its doc warns that an error mapped
      to an empty list "would strip the user's servers for every session".
    - The dead trait therefore documents the exact contract the live one
      forbids.
  - **`SessionsRepo::transcript_paths`** (`crates/otto-state/src/sessions.rs:654-665`)
    - `52edfdbe7` removed its only caller when History moved to
      `history_page::legacy_entries`.
    - Its doc still says *"with `provider_session_ids` this is the 'claimed'
      set that keeps an indexed on-disk transcript from ALSO showing up as an
      `on_disk` History entry"*. History no longer works that way.
  - **`otto_channels::improve_notify::NOTIFY_APPROVAL_KEY`** (`improve_notify.rs:54-58`)
    - It is a settings key, `"channels.notify_approval_required"`, that
      nothing reads or writes, including the UI.
    - Its doc admits it exists *"only for documentation clarity"*. A reader
      will take it for a real setting.
  - **`otto_review::session::FINDINGS_POLL`** (`session.rs:55`)
    - It is never used.
    - The poll loop in the same file hard-codes
      `Duration::from_millis(1000)` (`:938`).
- **Fix:**
  - Delete `McpServerProvider` (keep `McpServerSpec`, which is live).
  - Delete `transcript_paths` and `NOTIFY_APPROVAL_KEY`.
  - Use `FINDINGS_POLL` at `session.rs:938` and make it private.

### maintainability-3-05: A ninth local `~` expander still turns `~bob/x` into `$HOMEbob/x` (minor)

- **Where:** `crates/otto-server/src/repo_directory.rs:253-266` (`normalize_path`).
  It is used by `match_path` (`:273`, `:277`) and by the checkout-remote
  resolver (`:529`).
- **Evidence:**
  - It uses `p.strip_prefix('~')` followed by
    `format!("{}{}", home, rest)`. This is the any-`~` strip that
    `otto_core::paths::expand_home_strict` (`paths.rs:97-108`) documents as
    the bug: *"turned `~bob/notes.txt` into `$HOMEbob/notes.txt`"*.
  - `is_path_ref` (`:241-243`) treats any reference that starts with `~` as
    a local path, so `~bob/repo` reaches this code.
  - It also reads `HOME` alone, while `expand_tilde` also falls back to
    `USERPROFILE`.
  - Iteration 2 merged the other eight copies but did not catch this one,
    because it is not named `expand_*`.
- **Fix:** Expand with `otto_core::paths::expand_tilde(p)`, and treat a `~user`
  reference as no match. Keep only the trailing-slash trim here.

### maintainability-3-06: After the iteration-2 split, comments and feature docs still send readers to `modules.rs` for code that moved out (minor)

- **Where and evidence:**
  - `crates/otto-server/src/skill_review.rs:3`
    (`modules.rs::run_review_core`). It now lives in
    `modules/review/run.rs:245`.
  - `crates/otto-improve/src/http.rs:145` (`modules.rs::start_review`).
    It now lives in `modules/review/routes.rs:383`.
  - `crates/otto-server/src/routes/browser.rs:1140`
    (`modules.rs::send_input`). It now lives in
    `modules/session_input.rs:464`.
  - `crates/otto-k8s/src/lib.rs:20-21` and `crates/otto-aws/src/lib.rs:8-9`
    say the *"ctx trait impl … lives in `modules.rs`"*. The impls are in
    `modules/ctx_impls.rs:62` and `:80`.
  - `docs/features/git.md:62` (`draft_pr` and `draft_commit_message`). They
    are in `modules/pr_draft.rs:114` and `:400`.
  - `docs/features/code-review.md:15`, `:62` and `:154`
    (`run_review_core` in `modules.rs`)
  - `docs/features/insights.md:42` (the `InsightsCtx` impl). It is in
    `ctx_impls.rs:573`.
  - `docs/features/aws-console.md:41` (the `AwsCtx` impl)
  - `docs/design/product-design-arena.md:369`, `:419`
    (`resolve_skill_inline` in `otto-server/src/modules.rs`). It is in
    `otto-review/src/engine.rs:125`.
  - The doc sweep commit `11e640f0f` did not update these paths.
- **Fix:** Point each reference at the child module. For the engine docs,
  name the module (`modules/review/`) rather than the file, so the next
  split does not break them again.

### maintainability-3-07: Eight cost formatters and four token formatters disagree with each other, sometimes on the same page (minor)

- **Where:**
  - **`fmtCost`:**
    - `mission-control/lib.ts:174`
    - `agents/conversation/format.ts:22`
    - `swarm/RunInspector.svelte:80`
    - `usage/UsagePage.svelte:289`
    - `usage/ModelBreakdown.svelte:88`
    - `usage/CcusagePanel.svelte:27`
    - `usage/AttributionDrilldown.svelte:81`
    - `usage/CostForecastChip.svelte:57`
  - **Token and count formatters:**
    - `lib/sessionUsage.ts:12` `formatTokens` ("12.3K")
    - `agents/conversation/format.ts:27` `fmtTokens` ("12k", 0 decimals
      at 10k or more)
    - `usage/AttributionDrilldown.svelte:86` `fmtNum` ("12.3k")
    - `lib/metric-format.ts:59` `formatCount` (the shared one, "12.3k")
- **Evidence:**
  - There are five different cost behaviours. `UsagePage` and
    `ModelBreakdown` round $100 or more to whole dollars (`toFixed(n < 100 ? 2 : 0)`).
    `AttributionDrilldown` and `CcusagePanel` always use 2 decimals.
    `CostForecastChip` shows `<$0.001` or 4 decimals. `RunInspector` and
    `conversation/format` show 4 decimals under a cent. `mission-control`
    shows `$0.00` for null.
  - `UsagePage` renders `ModelBreakdown` (`:928`) and `AttributionDrilldown`
    (`:970`) together, so one $150.25 cost reads "$150" in one panel and
    "$150.25" in the other.
  - The token formatters even disagree on the case of the suffix
    (K versus k).
  - This is the same "one helper per file" drift that iterations 1 and 2
    fixed for bytes and relative time.
- **Fix:**
  - Add `formatUsd(n, { empty })` to `lib/metric-format.ts`, beside
    `formatBytes` and `formatCount`, and use it at all eight sites.
  - Use `formatCount` for tokens and delete the three local copies.

### maintainability-3-08: The architecture moves left compatibility shims that every in-tree caller still uses (minor)

- **Where:**
  - `ui/src/modules/product/design/scene3d/*.ts`: 11 three-line files,
    each one *"Moved to lib/design/scene3d/… Re-exported for existing
    importers."*
  - `ui/src/modules/database/JsonTree.svelte`: a wrapper over
    `lib/components/JsonTree.svelte`
  - `ui/src/lib/components/JsonTree.svelte:29-31`
- **Evidence:**
  - Every importer is in this repo. Nothing outside needs the old paths.
  - Product still imports the shims: `MockupLivePreview.svelte:14`,
    `design/format.ts:8` and `DesignArena.svelte:60`. So does
    `unit/studio3d.test.ts:9-31`.
  - All six JsonTree consumers import the database wrapper:
    `aws/S3Preview`, `SqsView`, `RdsView`, `Ec2View`, `EksView` and
    `workbench/preview/JsonPreview`. The wrapper is the "aws imports
    database" coupling that the move was meant to remove.
  - The "shared" `lib/components/JsonTree.svelte` still imports three
    files from `modules/database/` (`bson`, `expansion-plan` and
    `json-find`).
  - The wrapper's comment says the tree is *"shared by the AWS, API-client
    and workbench views"*. The API client renders its own 160-line
    `modules/api/JsonTree.svelte`.
- **Fix:**
  - Point the importers at `lib/` and delete the 12 shim files.
  - Move `bson.ts`, `expansion-plan.ts` and `json-find.ts` under `lib/`, or
    pass them in as props.
  - Correct the comment about the API client.

### maintainability-3-09: The AGENTS.md crate and UI-module map leaves out three crates and seven UI modules (minor)

- **Where:** `AGENTS.md:31-81`. AGENTS.md calls itself *"the single source of
  truth for … the crate/module map"*.
- **Evidence:**
  - Comparing `ls crates` with the crate table shows that `otto-aws`,
    `otto-k8s` and `otto-transcript` are missing. Each has its own
    `lib.rs` doc and its own feature guide.
  - The UI module list leaves out `assistant`, `aws`, `desktop`,
    `kubernetes`, `personal-agents`, `rooms` and `workbench`. All seven
    exist under `ui/src/modules/`, and most are `SIDEBAR_MODULES` entries
    (`lib/sidebar.ts:90`, `:96`, `:107`, `:120`, `:139` and `:140`).
- **Fix:** Add the three crate rows and the seven module names. A small
  guard test (`ls crates` compared with the table) would stop the drift.

### maintainability-3-10: The connections guide still describes a "Pinned" sidebar group that does not exist (minor; the doc part of 2-08)

- **Where:** `docs/features/connections-ssh-sftp.md:216-221`
- **Evidence:**
  - The guide says pinning *"surfaces the connection in a **Pinned** group
    above **Recent**. The sidebar's 'Recent' group is pinned-first"*.
  - No UI code reads `Connection.pinned` (`ui/src/lib/api/types/connections.ts:38`).
    The only `pinned` state in the connections and database UI is the
    comparison pane's local variable.
  - Iteration 2 kept the routes on purpose. That is fine, but the guide
    still promises UI behaviour.
- **Fix:** Rewrite the section to say the route sets a stored flag and the
  current UI does not use it yet.

### maintainability-3-11: Four Rust doc links point to items that no longer exist (nit)

- **Where and evidence:**
  - `crates/otto-usage/src/clickhouse.rs:38`: [`Self::ensure_running`]. The
    restart path is `wake_background` (`:138`) and the lease, and no
    `ensure_running` exists.
  - `crates/otto-sessions/src/lifecycle.rs:8`: [`should_suspend`]. No such
    function exists in the workspace.
  - `crates/otto-git/src/diff_cache.rs:8`: [`PR_DIFFS`]. The cache is
    `pr_diffs()` / `pr_diffs_raw()` (`:226-233`).
  - `crates/otto-git/src/providers/github.rs:450`:
    [`Github::fetch_commit_status`]. The functions are `fetch_ci_status*`
    and `status_rows` (`:267-321`).
- **Fix:**
  - Retarget the four links.
  - Add `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps`
    to `scripts/check.sh --all` or to CI, so renames break the build
    instead of the docs.

### maintainability-3-12: Unused `pub` types and constants, including a constant set that its own validator ignores (nit)

- **Where:**
  - `crates/otto-core/src/domain.rs:12`: `GlobalRole`. It is never used;
    root is the `is_root` flag.
  - `crates/otto-dbviewer/src/types.rs:1559`: `DbResult`, documented
    *"Re-export for drivers"*. No driver uses it.
  - `crates/otto-k8s/src/http.rs:106`: `NsQuery`.
  - `crates/otto-memory/src/governance.rs:17-20`: the `STATE_*` constants.
    Three are unused, and `is_valid_state` (`:22-24`) repeats the four
    string literals instead of using them.
- **Fix:**
  - Delete `GlobalRole`, `DbResult` and `NsQuery`.
  - Write `is_valid_state` as a `matches!` over the `STATE_*` constants (or
    a `const STATES: [&str; 4]`).

### maintainability-3-13: UI files and exports that nothing imports (nit)

- **Where and evidence:**
  - `ui/src/lib/fuzzy.ts`
    - A re-export shim *"for existing callers"*.
    - There are no callers: everything imports `fuzzyMatch` from
      `lib/commandSearch`.
  - `ui/src/lib/cssColor.ts`
    - Nothing imports it.
    - Its header says *"node:test covers it, unit/cssColor.test.ts"*. That
      file does not exist.
    - It also mentions a DOM reader that the module does not export.
  - `ui/src/lib/render/mermaid.ts:116`: `parseMermaid`, exported and
    referenced nowhere.
- **Fix:** Delete `fuzzy.ts` and `parseMermaid`. Delete `cssColor.ts`, or
  confirm with the classrooms team that their rewrite will import it, and in
  that case fix the test reference.

### maintainability-3-14: Leftovers of the escaper merge: an alias and three entity decoders with different character sets (nit)

- **Where:**
  - `ui/src/modules/product/OverviewTab.svelte:419`:
    `const escapeHtmlAttr = escapeHtml;`, used once (`:473`)
  - Decoders:
    - `OverviewTab.svelte:413` `decodeEntities` decodes `& < > " '`
    - `agents/conversation/codeBlocks.ts:49` `unesc` decodes `& < > " '`
    - `canvas/excalidraw-build.ts:29-34` `clean` decodes only `& < >`
- **Evidence:**
  - 2-11 merged the escapers into `lib/html.ts`, but not their inverse.
  - `clean` leaves `&quot;` in an Excalidraw label.
  - The alias exists only because the old `escapeHtml` was not safe in
    attributes. That reason is gone.
- **Fix:**
  - Add `unescapeHtml` to `lib/html.ts` and use it at all three sites.
  - Inline `escapeHtml` in place of the alias.

### maintainability-3-15: Two `// Tests` banners still sit above production code (nit; the rest of 2-14)

- **Where:**
  - `crates/otto-state/src/activity.rs:707-709`, above
    `shown_sessions_sql` and `summary_trail_sql`
  - `crates/otto-git/src/local.rs:5562-5564`, above
    `pub const HEAD_REMOTES_CAP`
- **Fix:** Move each banner down so it sits directly above `#[cfg(test)]`.
