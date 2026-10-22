# Maintainability review: iteration 2 (2026-10-22)

Lens: dead code, unused exports, routes and settings, duplicated helpers,
misleading names and comments, stale TODOs that point at finished work,
idioms that differ from the surrounding code, and code that is more clever
than it needs to be.

Worktree: `fix/quality-loop-20261021` at `ab0a86f05`. The review was
read-only and used only grep, rg, sed, git and small read-only Python
scanners in the scratchpad. Nothing was built and no tests were run.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per
minor and 0.03 per nit.

| Severity | Count | Deduction |
|----------|------:|----------:|
| blocker  | 0  | 0.00 |
| major    | 2  | 0.60 |
| minor    | 10 | 1.00 |
| nit      | 4  | 0.12 |
| **total**|    | **1.72** |

**Score: 10.0 − 1.72 = 8.28 / 10.** Iteration 1 scored 7.25.

## Status of the iteration-1 findings

| Id | Iter-1 severity | Status now | Evidence |
|----|-----------------|------------|----------|
| 1-01 | major | **Partly open** (now 2-05, minor) | Module and `run_workflow` docs are fixed (`workflow_engine.rs:15-20`, `:1818`). `WorkflowsRepo::get_cached_output` and `set_cached_output` (`otto-state/src/workflows.rs:849`, `:872`) are still there and still have no caller. |
| 1-02 | major | **Partly open** (now 2-03, minor) | There is now one shared builder, `otto_core::aws_env`, and the blank `AWS_PROFILE` is gone. Two problems remain: `AWS_ENV_REMOVE` is never applied, and the k8s spawns never remove `AWS_PROFILE`. |
| 1-03 | major | Fixed | `RunGraph.svelte` and the store's graph pipeline are gone. |
| 1-04 | major | Fixed | Only `lib/clipboard.ts` calls `navigator.clipboard.writeText`. The database helper is renamed `copyWithToast`. |
| 1-05 | minor | Fixed | The EKS import uses `K8sClustersRepo::create`. The TODO is gone. |
| 1-06 | minor | **Partly open** (now 2-06) | 11 of the 15 copies are gone. Four remain, and the new `iec` option has no caller. |
| 1-07 | minor | **Open** (now 2-07) | Deferred in FIXES.md as "a product-copy decision". Not rejected. |
| 1-08 | minor | Fixed | The wrappers that remain (`altLoopbackState`, `getWorkflowVersion`) are referenced from `ui/unit/*`. The iteration-1 grep did not search `ui/unit`. See 2-12 for `getWorkflowVersion`. |
| 1-09 | minor | Fixed | The flow adapter and `@xyflow/svelte` are gone. |
| 1-10 | minor | Fixed | `toLegacy`, `ambientSamples`, `isPageTabId`, `prCacheSnapshot` and `scanProgress` are unit-tested exports, so they are valid. `SIDE_PANE_KEY` is now used by the store. |
| 1-11 | minor | Fixed | All 21 listed functions are gone. |
| 1-12 | minor | Fixed | The write-only converter state is gone. |
| 1-13 | minor | Fixed | `BarHost` uses a plain dynamic `import()` and shows an inline error with Retry. |
| 1-14 | minor | Fixed | The TODO in `impersonate.rs` is replaced. |
| 1-15 | minor | Fixed (for `expand_tilde`) | The k8s `expand_tilde` now delegates to `otto_core`. **The same duplication exists under another name, `expand_home`.** See 2-02. |
| 1-16 | minor | Fixed | All four paths are corrected. Two more stale paths turned up; see 2-15. |
| 1-17 | minor | Fixed | Both helpers live only in `otto-transcript/src/util.rs`. |
| 1-18 | minor | Fixed | `PanZoom`, `MIN_SCALE` and `MAX_SCALE` live in `pointerZoom.ts`. |
| 1-19 | nit | Fixed | The remaining `allow(dead_code)` attributes are valid: FromRow `ord` columns and a test-only builder. |
| 1-20 | nit | Fixed | |
| 1-21 | nit | **Open** (now 2-13) | Listed as "Not done" in FIXES.md. `policy.rs:511` still has the branch. |
| 1-22 | nit | Fixed | |
| 1-23 | nit | Fixed | `retire_user_server` (`mcp.rs:738`) now comes before `mod tests` (`:816`). |

Totals: 16 fixed, 4 partly or fully open and carried forward, and 0 found to
be invalid.

## Areas covered in this iteration

- Re-checked every iteration-1 finding against the current code (table above).
- **Every Rust crate:** scanned `pub fn` items in all 40 workspace crates by
  counting tokens across `crates/`, `apps/` and `examples/`. Iteration 1
  covered only six crates. Each candidate was confirmed by hand.
- **Desktop shell:** scanned every function in `apps/desktop/src-tauri/src`.
  The only functions with no other reference are `#[test]` functions.
- **Svelte stores:** scanned the class fields, getters and methods of
  `ui/src/lib/stores/*.svelte.ts` against `ui/src`, `ui/unit` and `ui/e2e`.
- **UI TypeScript exports:** scanned again, this time including `ui/unit`
  and `ui/scripts`.
- **Svelte components:** looked for components that nothing imports. Only
  e2e fixtures turned up, and those are loaded through the Playwright
  configs.
- **HTTP routes:** extracted all 1,059 `.route(` templates. Routes with no
  UI, `ottod` MCP, `otto-mcp` or desktop caller were checked by hand. Most
  apparent hits come from dynamically built paths (`connBase(id)`, `${base}`,
  `${action}`), and those were dropped.
- **Settings:** traced every key the UI writes through `PUT /settings` to a
  daemon reader. They all have one.
- **Docs:** checked every route row in `docs/contracts/api.md`, `ws.md` and
  `docs/features/*.md` against the route table. Checked every source path
  cited in docs, skills and `ui/src`.
- **Duplicated Rust helpers:** looked for function names defined in four or
  more places (`expand_home`, `clip`, `truncate`, `valid_id`, `home_dir`).
- **Duplicated UI helpers:** HTML escaping, `plural`, `esc` and relative time.
- **Recent commits:** the iteration-1 fix commit `66b55a25c` (new
  `aws_env.rs`, `cancel.rs`, `pointerZoom.ts`, `personal_agent_policy.rs`,
  `workflows.ts`) and `f4f4eac7d` (agy and grok usage).
- **TODO, FIXME and HACK markers:** none remain in `crates/*/src`,
  `ui/src` or `apps/desktop/src-tauri/src`.

## Areas not covered yet (for iteration 3)

- `ottod/src/mcp_tools.rs` compared with `otto-server/src/mcp_outward.rs`
  for catalog drift. `catalog_tests.rs` exists, but the parity test itself
  was not read.
- Non-`pub` (private) Rust functions. rustc already warns on these unless
  `allow` hides them, so they are lower risk. The remaining `allow` sites
  were checked.
- Duplicate helpers with different meanings that were not compared in
  detail: `clip` (7 copies), `truncate` (3), `valid_id` (6) and `home_dir`
  (4).
- Svelte component props and local state that nothing reads. Only the
  store classes were scanned.
- `docs/features/*.md` text other than route rows and source paths. Behaviour
  descriptions were checked only where a finding pointed at them.
- `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**` and
  `ClassroomsBox.svelte` were excluded on purpose because another team is
  rewriting them.

---

## Findings

### maintainability-2-01: `BrokersService::with_ops` is never called, so Lag Alerts always fail and replay evidence is never saved (major)

- **Where:**
  - `crates/otto-brokers/src/service.rs:133-138` (`with_ops`)
  - `crates/otto-brokers/src/service.rs:129` (`ops: None`)
  - `crates/otto-server/src/boot/build.rs:62-67`
  - `crates/otto-server/src/test_support.rs:109-113`
  - `crates/otto-brokers/src/replay.rs:66-82`
- **Evidence:**
  - The doc comment on `with_ops` says *"Attach the operator-workflow repo
    (lag alerts + replay evidence). Called by the server wiring after the
    pool is available."* A workspace-wide search finds only the definition,
    and has done so since `f4ff87a52`.
  - `boot/build.rs:62` builds `BrokersService::new(repo, secrets,
    Some(BrokerAuditRepo))` and never chains `.with_ops(...)`. `ops` is
    therefore always `None`.
  - Every lag-alert method begins with
    `self.ops.as_ref().ok_or_else(|| Error::Internal("ops repo not
    initialised"))` (service.rs:1090-1127). The UI calls all three:
    `LagAlertsPanel.svelte:50`, `:60` and `:84`. Both `BrokersPage` and
    `ClusterViewer` render that panel.
  - `replay::publish` skips `record_replay` when `ops` is `None`
    (replay.rs:68), so `evidence_saved` is always false.
    `ReplayPanel.svelte:126` tells the user *"An evidence record is saved
    for auditing."*
  - The test fixture also passes no ops repo, so no server-level test would
    catch this.
- **Why it matters:**
  - A builder documented as wired, but never wired, turns a whole feature
    surface into dead code that still ships routes, UI and copy.
  - This is also a functional defect. The correctness and integrations
    lenses should pick it up too.
- **Fix:**
  - In `boot/build.rs` (and in `test_support.rs`), chain
    `.with_ops(otto_state::BrokerOpsRepo::new(pool.clone()))`.
  - Better: make the ops repo a required `new` argument, so it cannot be
    forgotten, and delete `with_ops`.
  - Add a router-level test that `GET /brokers/clusters/{id}/lag-alerts`
    returns 200.

### maintainability-2-02: Eight local `~` expanders remain under other names, and two keep the `~user` bug fixed elsewhere (major)

- **Where (canonical: `otto_core::paths::expand_tilde`, `otto-core/src/paths.rs:81`):**
  - `crates/otto-connections/src/http.rs:712` `expand_home`, used by SFTP
    download and upload (:795, :835) and by `transfers.rs:197`
  - `crates/otto-dbviewer/src/http.rs:865` `expand_home`, with the doc
    comment *"(mirrors the SFTP handler)"*. Used for DB export
    destinations (:879, :1046).
  - `crates/otto-git/src/http.rs:1459` `expand_home`, for the clone
    destination
  - `crates/otto-server/src/routes/workspaces.rs:198` `expand_home`, for
    the workspace root
  - `crates/otto-server/src/skill_review_static.rs:258` `expand_home`
  - `crates/otto-aws/src/s3_download.rs:85` `expand_home`
  - `crates/otto-vault/src/engine.rs:2743` `shellexpand_home`
  - `crates/otto-server/src/routes/fs.rs:113` `expand_home`. This one is
    a correct wrapper that adds the `~user` refusal.
- **Evidence:**
  - Iteration 1 (1-15) merged the five `expand_tilde` copies. These eight
    do the same job under a different name, and all of the crates already
    depend on `otto-core`.
  - The connections and dbviewer copies use `path.strip_prefix('~')`. That
    turns `~bob/x` into `$HOMEbob/x`. `fs.rs:110-112` documents this as the
    bug it fixed: *"the old any-`~` prefix strip turned `~bob/notes.txt`
    into `$HOMEbob/notes.txt`"*. Its test `expand_home_expands_only_the_callers_own_home`
    (fs.rs:533) covers only `fs.rs`.
  - The copies also look up home three different ways:
    `env::var("HOME")`, `env::var_os("HOME")` and `dirs::home_dir()`. The
    `workspaces.rs` and `s3_download` copies also drop the bare `~` case or
    handle it differently.
- **Why it matters:** The fix for `~user` landed in one of eight copies.
  This is the same "a fix lands in one copy" drift that iteration 1 flagged
  for `expand_tilde` and the AWS env builder.
- **Fix:**
  - Add `otto_core::paths::expand_home_strict(p) -> Result<String>`
    (`expand_tilde` plus the `~user` refusal from `fs.rs`). Call it from
    the user-path sites: SFTP, DB export, clone destination and workspace
    root.
  - Call `expand_tilde` (wrapped in `PathBuf::from` where needed) from the
    rest.
  - Delete the eight local functions, and move the `~bob` test into
    `otto-core`.

### maintainability-2-03: `AWS_ENV_REMOVE` is exported and documented as required, but nothing applies it (minor)

- **Where:**
  - `crates/otto-core/src/aws_env.rs:25-31`
  - `crates/otto-k8s/src/clusters.rs:226-233` and `:275`
  - k8s spawns: `crates/otto-k8s/src/cli.rs:71`, `:131`, `logs.rs:211`,
    `:313`, `monitor/scrape.rs:691` and `monitor/gateway.rs:124`
  - `crates/otto-aws/src/cli.rs:374`, `s3.rs:637`, `:815` and
    `s3_download.rs:270`
- **Evidence:**
  - `aws_env_for` documents *"Spawns applying it must `env_remove`
    [`otto_core::aws_env::AWS_ENV_REMOVE`] first."* None of the six k8s
    spawn sites call `env_remove`. They only call
    `.envs(env.iter()...)`.
  - `AWS_ENV_REMOVE` has no code reference anywhere. otto-aws hard-codes
    `.env_remove("AWS_PROFILE")` four times instead.
  - `otto-k8s/src/clusters.rs:275` defines a `pub fn build_aws_env` with
    the same name as `otto_core::aws_env::build_aws_env` but a different
    signature (`auth_mode: &str, …, params: &Value, secret_json`). In that
    one file, the bare name and the qualified name refer to different
    functions.
  - The result is that an `AWS_PROFILE` inherited from the daemon still
    reaches kubectl's `aws eks get-token`. The shared constant was written
    to prevent exactly that.
- **Fix:**
  - Apply `for k in AWS_ENV_REMOVE { cmd.env_remove(k); }` in the k8s spawn
    helpers (`cli.rs`, `logs.rs`, `scrape.rs`, `gateway.rs`) and in the four
    otto-aws sites.
  - Rename the k8s wrapper to `aws_env_from_row`.

### maintainability-2-04: Dead `pub` functions in crates iteration 1 did not scan, several with misleading docs (minor)

- **Where (each name is referenced only at its definition):**
  - `otto-brokers/src/lib.rs:21` `librdkafka_version`
  - `otto-brokers/src/service.rs:143` `group_access_denied`. Its doc says
    *"The UI reads this to gate the Consumer Groups tab"*. Nothing reads
    it.
  - `otto-memory/src/service.rs:59` `new_keyword_only`. Its body is
    identical to `with_defaults` (:64). The doc says *"(there is no other
    kind anymore)"*.
  - `otto-transcript/src/grok_usage.rs:367` `default_grok_home`. This
    duplicates the `GROK_HOME` resolution `ottod` actually uses
    (`ottod/src/main.rs:540`, `usage_tailer.rs:212`). It was added 3 days
    ago in `f4f4eac7d`.
  - `otto-transcript/src/agy.rs:130` `discover_agy_conversation_dbs`. Also
    new in `f4f4eac7d`. `ottod` chains `default_agy_conversation_dirs`
    itself instead.
  - `otto-product/src/story_lifecycle.rs:63` `cancellation_flag`. As a
    result, `Story.flag` (:11) is write-only: `begin_delete` stores `true`
    at :54 and nothing ever loads it.
  - `otto-swarm/src/runtime/workspace.rs:232` `ensure_cwd`. Every caller
    uses `ensure_cwd_info`.
  - `otto-assistant/src/personal_agents_engine.rs:1159` `is_one_shot`
  - `otto-vault/src/store.rs:1001` `all_aliases`
  - `otto-pty/src/lib.rs:714` `wait_output_closed_blocking`
  - `otto-dbviewer/src/types.rs:1179` `unimplemented`. Its doc says *"a
    stub driver returns. Replaced by the per-engine agent"*. That
    scaffolding is gone.
- **Why it matters:** `dead_code` never warns on `pub` items. Three of these
  have doc comments that describe callers or uses that do not exist.
  `new_keyword_only` and `default_grok_home` are second copies of live
  logic that can drift.
- **Fix:**
  - Delete the 11 functions and `Story.flag` together with its store in
    `begin_delete`.
  - Consider a CI check (for example `cargo +nightly udeps`-style or
    `warnings` on `unreachable_pub`) so these do not grow back.

### maintainability-2-05: The node-cache repository methods survive the cache removal (minor; left over from 1-01)

- **Where:** `crates/otto-state/src/workflows.rs:845-900`
- **Evidence:**
  - `get_cached_output` and `set_cached_output` still sit under a
    `// --- node output cache` banner and have no caller in the workspace.
  - `workflow_engine.rs:20` calls `workflow_node_cache` *"a dormant
    table"*. The iteration-1 fix rewrote that comment but did not delete the
    methods, which was the second step of the 1-01 fix.
- **Fix:** Delete both methods. The table stays, because migrations are
  append-only.

### maintainability-2-06: Four byte formatters remain, and the shared helper has an `iec` option that nothing uses (minor; left over from 1-06)

- **Where:**
  - `ui/src/lib/metric-format.ts:36-56` (shared `formatBytes`, with
    `opts.iec`)
  - `ui/src/modules/kubernetes/k8s-util.ts:82` `formatBytes`, which prints
    IEC `Ki/Mi`. Imported by `columns.ts`, `MetricsView`, `ResourceDrawer`
    and `WorkloadPods`.
  - `ui/src/modules/aws/util.ts:6` `fmtBytes`, used by five AWS views
  - `ui/src/modules/api/storageGauge.ts:37` `formatBytes`
  - `ui/src/modules/product/design/scene3d/presets.ts:240` `formatBytes`
- **Evidence:**
  - The `iec` option's doc says *"IEC quantity suffixes the way kubectl
    prints them"*, so it was added for k8s. `rg "iec: true"` finds nothing:
    k8s still imports its own copy.
  - `aws/util.ts:6` is the same algorithm as the shared helper (1024 steps,
    one decimal under 100, `—` when empty).
  - The other two still disagree with each other and with the shared one.
    `storageGauge` uses 0 decimals for KB. `presets` stops at MB with 2
    decimals.
- **Fix:**
  - k8s: `formatBytes(n, { iec: true, empty: '' })`.
  - aws: `formatBytes(n)`.
  - `storageGauge` and `presets`: call the shared helper and accept its
    output style. Delete the four local copies.

### maintainability-2-07: Relative-time helpers are still duplicated, and one local `rel` is not relative (minor; 1-07 still open)

- **Where:**
  - Shared: `ui/src/lib/stores/now.svelte.ts:64` `rel`
  - `mission-control/lib.ts:179` `relTime`
  - `mission-control/WorkItemDetail.svelte:317` `ago`
  - `canvas/ConversationPanel.svelte:101` `ago`
  - `product/LinkedCanvases.svelte:131` `ago`
  - `git/FocusView.svelte:269` `ago`
  - `database/DatabasePage.svelte:939` `fmtAgo`
  - `kubernetes/monitor/monitor-util.ts:87` `fmtAgo`
  - `aws/util.ts:20` `fmtAgo`
  - `shell/NotificationBell.svelte:286` `relative`
  - `swarm/RunInspector.svelte:84` `rel`
- **Evidence:**
  - FIXES.md deferred this as a product-copy decision. It was not rejected.
  - `RunInspector`'s local `rel(ts)` returns
    `new Date(ts).toLocaleString()`, an absolute timestamp, under the same
    name as the shared relative `rel`. A reader, or an auto-import, will
    confuse them.
  - The non-reactive copies still freeze their labels between re-renders.
- **Fix:**
  - Rename `RunInspector.rel` to `fmtWhen` now.
  - Then make the copy decision once: give the shared `rel()` a
    `style: 'short' | 'long'` option and remove the copies.

### maintainability-2-08: Connection pinning and section reordering have routes, repository code and docs but no client; one doc gives the wrong body (minor)

- **Where:**
  - `crates/otto-connections/src/http.rs:153` (`PATCH
    /connections/{id}/pin`) and `:596-610`
  - `:175` (`POST /workspaces/{id}/connection-sections/reorder`) and
    `:1080-1091`
  - `docs/features/connections-ssh-sftp.md:216-221`, `:405` and `:434`
  - `docs/contracts/api.md:95` (row 27a) and `:847`
  - `ui/src/lib/api/types.ts:249-250` (`Connection.pinned`)
- **Evidence:**
  - No file in `ui/src`, `ottod`, `otto-mcp` or `apps/desktop` calls either
    route. No UI code reads `Connection.pinned`. The only "recent" list,
    `DatabasePage.svelte:231-236`, sorts by `last_opened_at` alone.
  - The feature doc still says pinning *"surfaces the connection in a
    **Pinned** group above **Recent**. The sidebar's 'Recent' group is
    pinned-first"*. No such group exists.
  - The UI moves sections only through `/connection-sections/{id}/move`.
  - `api.md:847` documents the reorder body as `{order:[id,…]}`. The real
    type, `otto_core::api::ReorderSectionsReq` (`api.rs:812`), is
    `{ ids }`. `connections-ssh-sftp.md:434` has it right, so the two docs
    disagree.
- **Fix:** Pick one of two options.
  - Build the pin toggle and the Pinned group the doc describes.
  - Or remove the two routes, `set_pinned` and `reorder_sections` (keeping
    the columns, because migrations are append-only), and the doc rows.

  Either way, correct `api.md:847` to `{ids:[…]}`.

### maintainability-2-09: The brokers feature doc lists four endpoints removed by migration 0095 (minor)

- **Where:** `docs/features/message-brokers.md:547-550`
- **Evidence:**
  - The doc lists `GET/POST /workspaces/{wid}/brokers/cluster-sections`,
    `PATCH/DELETE /brokers/cluster-sections/{id}` and `POST
    …/{id}/move`, all returning `BrokerClusterSection`.
  - None of these routes exists. `BrokerClusterSection` exists nowhere in
    `crates/` or `ui/src`.
  - `0095_unified_connection_sections.sql:13,64` dropped the table, and
    `docs/contracts/api.md:3524` says *"the old `/brokers/cluster-sections*`
    endpoints are gone"*.
  - The brokers store uses `/workspaces/{id}/connection-sections`
    (`brokers.svelte.ts:74-118`).
- **Fix:** Replace the four rows with a one-line pointer to the shared
  `/connection-sections` API.

### maintainability-2-10: Svelte stores keep orphaned pane state and 13 members that nothing calls (minor)

- **Where:** `ui/src/lib/stores/`
  - **`ui.svelte.ts`, four orphaned layouts:** `gitSideWidth`,
    `apiSideWidth`, `dbDockOpen` and `dbDockWidth`, each with its state,
    `LS` key, clamp function and setter:
    - state: `:238`, `:249`, `:263`, `:272`
    - LS keys: `:77`, `:81`, `:99`, `:102`
    - clamp functions: `clampGitSide` `:203`, `clampApiSide` `:222`
    - setters: `setGitSideWidth` `:680`, `setApiSideWidth` `:700`,
      `toggleDbDock` `:713`, `setDbDockWidth` `:729`
  - **`ui.svelte.ts`, other dead members:** `setWfSideWidth` (`:651`).
    `wfSideWidth` is now read only as a legacy seed
    (`WorkflowsPage.svelte:83-85`). Also `moveSidebar` (`:471`) and
    `isSidebarFavorite` (`:500`).
  - **`database.svelte.ts:2141`:** `selectConnection`, documented
    *"Backwards-compatible alias"*. It has no callers.
  - **`workspace.svelte.ts`:** `connectionSessions` (`:277`),
    `addSessionInSplit` (`:1188`), `singleSessionView` (`:1642`) and
    `setSessionDirs` (`:2133`)
  - **Elsewhere:**
    - `apiClient.svelte.ts:606` `setTabResult`
    - `assistant.svelte.ts:314` `forgetSignIn`
    - `clipHistory.svelte.ts:126` `togglePin`
    - `k8s.svelte.ts:263` `installRunning`
    - `product.svelte.ts:892` `contentBaseOf`
- **Evidence:**
  - A token count over `ui/src`, `ui/unit` and `ui/e2e` finds each name
    exactly once, at its declaration.
  - The four `ui` layouts are read nowhere outside the store, yet they
    still read and write localStorage on every boot.
  - Iteration 1 checked only module-level exports, so these class members
    were never covered.
- **Fix:**
  - Delete the members, plus the four layouts' `LS` keys and clamp
    functions.
  - Keep `wfSideWidth` only as the legacy seed, or inline the read in
    `WorkflowsPage`.

### maintainability-2-11: HTML escaping is hand-written in 12 files with four different character sets (minor)

- **Where:**
  - `lib/hl.ts:113` `escapeHtml`, the only exported one: `& < >`
  - `settings/Logs.svelte:117`: `& < >`
  - `& < > "`:
    - `lib/md.ts:31`
    - `vault/mdRender.ts:41`
    - `agents/conversation/codeBlocks.ts:46`
    - `agents/conversation/chatMarkdown.ts:16`
    - `product/OverviewTab.svelte:418` `escapeHtmlAttr`
  - `& < > " '`:
    - `design-hall/site/engine/render.ts:47`
    - `usage/reportHtml.ts:9`
  - `vault/noteRenderPlan.ts:14`: `& < > " '` as numeric entities
  - The same `.replace(/&/g, '&amp;')` chain also appears in
    `vault/SearchPanel.svelte`, `insights/InsightsPage.svelte` and
    `canvas/svgSanitize.ts`.
- **Evidence:**
  - Twelve files hand-write the escape chain.
  - The one shared export, `escapeHtml`, does not escape quotes, so it is
    unsafe in an attribute. That is presumably why the
    `OverviewTab.escapeHtmlAttr` copy exists.
  - Whether a given renderer is safe in attribute context therefore depends
    on which copy it happened to write.
- **Fix:**
  - Put one `escapeHtml(s)` that escapes `& < > " '` in `lib/` (for
    example `lib/html.ts`) and use it in all 12 files.
  - Add a `ui-guards` ratchet on `replace(/&/g, '&amp;')` outside that
    file.

### maintainability-2-12: Iteration 1 added a typed `patchWorkflow` that nothing calls, while WorkflowsPage keeps four untyped PATCHes (minor)

- **Where:**
  - `ui/src/lib/api/workflows.ts:9-12`, added in `66b55a25c`
  - `ui/src/modules/workflows/WorkflowsPage.svelte:778`, `:798`, `:813`
    and `:946`
  - `ui/src/lib/api/workflows.ts:47-49` (`getWorkflowVersion`)
- **Evidence:**
  - `patchWorkflow(id, body: UpdateWorkflowReq)` has no caller.
  - WorkflowsPage calls `api.patch<Workflow>(\`/workflows/${id}\`, {...})`
    four times with untyped bodies and without `encodeURIComponent`.
  - FIXES.md lists both leftovers under "Not done":
    - *"`WorkflowsPage`: should call the typed `patchWorkflow`"*
    - *"`getWorkflowVersion`: delete it together with its two assertions"*
  - `getWorkflowVersion` is still referenced only by
    `ui/unit/workflowVersionPaging.test.ts:25`.
- **Fix:**
  - Switch the four call sites to `patchWorkflow`, so that
    `UpdateWorkflowReq` type-checks the bodies.
  - Delete `getWorkflowVersion` and its test assertion.

### maintainability-2-13: `policy.rs` still grants access to the nonexistent `/connections` route (nit; 1-21 still open)

- **Where:**
  - `crates/otto-server/src/policy.rs:511-514`
  - Test-only routes: `tests/rbac_matrix.rs:134` and
    `tests/share_scope_guard.rs:125`
- **Evidence:** The branch is unchanged. FIXES.md says it has to be removed
  together with the `rbac_matrix` assertions that mount a fake
  `/connections` route.
- **Fix:** Delete the branch. Point those test assertions at
  `/workspaces/{id}/connections`.

### maintainability-2-14: Six `// Tests` banners sit above production code (nit)

- **Where:**
  - `crates/otto-product/src/chat.rs:770`, followed by `stop_turn`,
    `StopTurnReq` and `StopTurnResp`
  - `crates/otto-product/src/refine.rs:597`, followed by `stop_turn`
  - `crates/otto-product/src/watcher.rs:557`, followed by the pre-trust
    helper
  - `crates/otto-product/src/service.rs:1880`, followed by the tree-role
    validator
  - `crates/otto-git/src/local.rs:5580`, followed by `pub const
    HEAD_REMOTES_CAP`
  - `crates/otto-state/src/activity.rs:708`, followed by the sidebar SQL
    predicate
- **Evidence:**
  - Production items were appended below the file's `// Tests` section
    header and above `#[cfg(test)] mod tests`.
  - Someone skimming the file, or folding it in an editor, will skip them
    as test code.
  - `chat.rs` even refers to its own types by full path
    (`crate::chat::StopTurnReq`) from inside `chat.rs`.
- **Fix:** Move each banner down so it sits directly above `#[cfg(test)]`.

### maintainability-2-15: Two docs cite source files that do not exist (nit)

- **Where and evidence:**
  - `docs/features/vault.md:51` cites
    `crates/otto-state/migrations/0103_vault_docs.sql`. The file is
    `0105_vault_docs.sql`.
  - `docs/design/product-design-arena.md:16` cites
    `crates/otto-product/src/skills.rs`. It now lives at
    `crates/otto-skills/src/product.rs`, which iteration 1 fixed in
    `skills-library.md` but not here.
- **Fix:** Update both paths.

### maintainability-2-16: `cancel.rs` claims the registry lives "ONCE", but `cancel_signal.rs` has a second one (nit)

- **Where:**
  - `crates/otto-core/src/cancel.rs:5-8`
  - `crates/otto-core/src/cancel_signal.rs:138-176` (`RunCancels`)
  - `crates/otto-server/src/state.rs:83`, `:87`, `:102` and `:105`
- **Evidence:**
  - `cancel.rs` says *"The operations live here ONCE … they used to be
    re-implemented in six engines with diverging semantics"*.
  - `cancel_signal::RunCancels` is a second registry mapping run id to a
    cancel handle, with its own register, cancel and guard. It is used by
    scheduled tasks and personal agents.
  - In `state.rs`, the review and skill-review registries are typed through
    an unrelated module's re-export, `crate::skill_eval::CancelRegistry`.
    Line 129 uses `otto_core::cancel::CancelRegistry` and line 150 uses
    `otto_swarm::runtime::run::CancelRegistry`. That is three spellings of
    one type.
- **Fix:**
  - Make each module's doc point at the other and say when to use which
    one: polled `AtomicBool` versus a wakeable `CancelSignal`.
  - Spell the type `otto_core::cancel::CancelRegistry` everywhere in
    `state.rs`.
