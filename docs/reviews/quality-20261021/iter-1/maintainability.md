# Maintainability review: iteration 1 (2026-10-21)

Lens: dead code, unused exports, routes and settings, duplicated helpers,
misleading names and comments, stale TODOs that point at finished work,
idioms that differ from the surrounding code, and code that is more clever
than it needs to be.
Worktree: `fix/quality-loop-20261021` @ `d447516c4`. This review was
read-only and used only grep, sed and git.

## Score

Rubric: start at 10.0 and subtract 1.0 per blocker, 0.3 per major, 0.1 per
minor and 0.03 per nit.

| Severity | Count | Deduction |
|----------|------:|----------:|
| blocker  | 0  | 0.00 |
| major    | 4  | 1.20 |
| minor    | 14 | 1.40 |
| nit      | 5  | 0.15 |
| **total**|    | **2.75** |

**Score: 10.0 − 2.75 = 7.25 / 10**

## Previous findings

None. This is the first maintainability pass. The earlier review rounds
(quality-20261007/08, quality-next-20261008) had no maintainability lens.

## Areas covered

- Every TODO, FIXME and HACK marker in `crates/`, `ui/src/` and
  `apps/desktop/src-tauri/src`.
- Every `#[allow(dead_code)]` site in the Rust workspace.
- Exports with zero references in `ui/src/lib/**/*.ts` (1,046 symbols) and in
  `ui/src/modules/**/*.ts` and `ui/src/shell` (1,394 symbols). Each was
  cross-checked against `src/`, `e2e/` and `scripts/`.
- Svelte components that nothing imports.
- `pub fn` items with no callers in otto-server, otto-sessions, otto-git,
  otto-state, otto-dbviewer and otto-core.
- Duplicated helpers:
  - UI: byte formatting, relative time, clipboard, clamp, canvas pan and zoom.
  - Rust: expand_tilde, sha256_hex, date formatting, require_root, AWS env
    building.
- Every `policy.rs` entry checked against the route table: 320 exact paths
  and 96 prefixes compared with 1,055 `.route(` templates.
- References in code and docs to source paths that no longer exist.
- Commits from the last 3 weeks:
  - agy MCP config (`8a9897823`)
  - git landed marks (`eb295dcc2`)
  - agy/grok usage (`f4f4eac7d`)
  - UI state preservation (`452ade172`)
  - workflows, EKS and k8s, impersonation

## Areas not covered yet (for iteration 2)

- Unused daemon settings keys. Settings are KV-shaped, so each key read has
  to be traced by hand.
- Unused HTTP routes, meaning routes with no UI, MCP or `ottod` caller (the
  reverse of the policy check above).
- `pub` items in the remaining crates: otto-product, otto-swarm,
  otto-automation, otto-assistant, otto-vault, otto-design*, otto-brokers,
  otto-k8s, otto-aws, otto-channels.
- `ottod/src/mcp_tools.rs` against `otto-server/src/mcp_outward.rs`, looking
  for catalog duplication and drift.
- Svelte stores (`ui/src/lib/stores/*.svelte.ts`): class fields and methods
  that no component reads. This pass only covered module-level exports.
- `docs/contracts/api.md` rows for routes that were removed. One stale row was
  found incidentally (finding 01).
- `apps/desktop/src-tauri` beyond the TODO scan.
- `ui/src/modules/rooms/games/**` was excluded on purpose because another team
  is rewriting it.

---

## Findings

### maintainability-1-01: Workflow engine docs describe a node-result cache that was removed (major)

- **Where:**
  - `crates/otto-server/src/workflow_engine.rs:15-20`
  - `crates/otto-server/src/workflow_engine.rs:1767-1770`
  - `crates/otto-state/src/workflows.rs:846-900`
  - `docs/contracts/api.md:2331`
- **Evidence:**
  - The module doc says *"When a node is re-run with the same params and the
    same assembled input … the engine reuses the stored output from
    `workflow_node_cache` … The cache is upserted on every successful node
    execution"*.
  - The `run_workflow` doc (line 1767) says *"Cache-eligible nodes are skipped
    if a matching `workflow_node_cache` entry exists"*.
  - Line 600 of the same file contradicts both: *"(Cross-run node-result
    caching removed … `workflow_node_cache` stays only as a dormant table"*.
  - `WorkflowsRepo::get_cached_output` and `set_cached_output`
    (workflows.rs:849 and :872) have no callers anywhere in the workspace.
  - The api.md retry row still says *"Retry re-entries bypass node-cache
    READS"*.
  - `docs/features/workflows.md:308` documents the removal correctly.
- **Why it matters:** The engine's own header and its entry-point doc describe
  behaviour that was removed because it reviewed the wrong scope. A maintainer
  who trusts the header could re-wire the dead repo methods and bring the bug
  back.
- **Fix:**
  - Replace the header section and the `run_workflow` sentence with one line
    pointing to the removal note at line 600.
  - Delete `get_cached_output` and `set_cached_output`. The table stays,
    because migrations are append-only.
  - Drop the "bypass node-cache READS" clause from api.md:2331.

### maintainability-1-02: `otto-k8s` keeps a copy of the AWS env builder that has drifted from `otto-aws` (major)

- **Where:**
  - `crates/otto-k8s/src/clusters.rs:235-328`, with the test at `:1023-1033`
  - `crates/otto-aws/src/accounts.rs:267-314`
- **Evidence:**
  - The copy's doc says *"TODO(otto-aws): share `otto_aws::accounts::build_env`
    once the AWS crate exports it — this is a deliberately minimal mirror …
    while both are built in parallel."* `build_env` is now `pub`, so the reason
    given for the copy no longer applies.
  - The two builders now behave differently. In access-key mode the k8s copy
    pushes `("AWS_PROFILE", "")` (clusters.rs:324-325), and its test asserts
    that blank value. The `otto-aws` builder documents that the CLI fails on
    exactly that value: *"it must be REMOVED, not blanked: CLI v2 treats
    `AWS_PROFILE=""` as a profile literally named "" and fails every call"*
    (accounts.rs:306-311).
  - Every `otto-aws` spawn calls `.env_remove("AWS_PROFILE")` (cli.rs:374,
    s3.rs:637). The k8s spawns only call `.envs(...)` (cli.rs:71/131,
    logs.rs:211, gateway.rs:124), so kubectl's `aws eks get-token` child
    inherits the blank profile.
  - The k8s copy also ignores the account's `endpoint_url`.
- **Fix:**
  - Have one builder. Move `build_env` (plus the remove-`AWS_PROFILE` rule)
    into `otto-core`, or a small shared module both crates already depend on,
    and call it from `aws_env_for`.
  - At minimum, stop emitting `AWS_PROFILE=""` and `env_remove` it in the k8s
    spawns.
  - Delete the stale TODO and update the test.

### maintainability-1-03: The swarm `RunGraph` view and its store pipeline have been orphaned since June but are still maintained (major)

- **Where:**
  - `ui/src/modules/swarm/RunGraph.svelte`, 424 LOC
  - `ui/src/lib/stores/swarm.svelte.ts`: lines 92-95, 155-163, 181-191,
    729-739, 831-848 and 1175-1185
- **Evidence:**
  - No file imports `RunGraph.svelte`. Commit `51ebd806a` (2026-06-24)
    replaced it with `AgentGraph` and said *"RunGraph.svelte kept in the
    tree"*.
  - `swarm.watchGraph()`, `swarm.graph`, `graphLoading` and `graphError` are
    read only by `RunGraph.svelte`.
  - Nothing ever registers a graph viewer, so the following never fetch:
    - `graphViewers`, `graphStale`, `graphDebounce` and `graphFirstPending`
    - `loadGraph`, `refreshGraph` and `maybeLoadGraph`, which have 8 call
      sites
    - the `GET /swarm/swarms/{sid}/graph` route
  - The comment at swarm.svelte.ts:729 says *"the Graph tab reads it, so task
    mutations must refresh it"*. The Graph tab is `AgentGraph`, which does not
    read it.
  - The dead file was still edited in `452ade172` (2026-10-19).
- **Fix:** Delete `RunGraph.svelte` and the graph state, watch and load code
  in the store, along with its call sites. Either retire
  `GET /swarm/swarms/{sid}/graph` and its contract row #83, or note that it is
  API-only.

### maintainability-1-04: 11 clipboard writes bypass `lib/clipboard.copyText`, and a second `copyText` has a different contract (major)

- **Where:**
  - `ui/src/lib/clipboard.ts:20-45` is the shared helper.
  - These call `navigator.clipboard.writeText` directly:
    - `database/ResultsGrid.svelte:1155`
    - `database/results-format.ts:163`
    - `database/QueryBuilder.svelte:784`
    - `agents/conversation/ToolStep.svelte:214`
    - `agents/conversation/TurnItem.svelte:77`
    - `agents/conversation/TurnItem.svelte:85`
    - `agents/conversation/PreviewPanel.svelte:76`
    - `agents/conversation/codeBlocks.ts:111`
    - `agents/history/HistoryPage.svelte:222`
    - `panels/OutputsPanel.svelte:225`
    - `design-hall/site/PublishModal.svelte:126`
- **Evidence:**
  - `lib/clipboard.ts` documents why direct calls are wrong. The async API is
    "secure contexts only". On an unfocused document an awaited rejection
    makes the `execCommand` fallback die as well. The helper falls back for
    "http / cert-degraded origins and older webviews".
  - Every site above skips that fallback. On a non-secure origin such as the
    share/remote UI over LAN http, `navigator.clipboard` is undefined, so those
    copies fail where the helper would succeed.
  - `database/results-format.ts:161` exports a different `copyText(s,
    successToast?)` that returns `void`, while `lib/clipboard.copyText` returns
    `boolean`. Same name, different contract, both in use.
  - HistoryPage, StructureView, Channels and RequestBuilder each define yet
    another local `copyText`.
- **Fix:**
  - Route every site through `lib/clipboard.copyText`, or its `OrThrow`
    variant plus a toast.
  - Rename the database helper (for example `copyWithToast`) and build it on
    the shared one.
  - Add a `ui-guards` ratchet that bans `navigator.clipboard.writeText`
    outside `lib/clipboard.ts`.

### maintainability-1-05: EKS import has a stale TODO, a doc comment on the wrong function, and a hand-rolled INSERT (minor)

- **Where:** `crates/otto-aws/src/eks.rs:249-262`, `:264`, `:273` with `:368`,
  and `:344-366`
- **Evidence:**
  - Line 252 says *"TODO(k8s): switch the insert to
    `otto_state::K8sClustersRepo` once it lands"*. `K8sClustersRepo::create`
    exists (`otto-state/src/k8s_clusters.rs:199`) and runs the same 12-column
    INSERT.
  - The `/// aws eks update-kubeconfig into an Otto-owned file + a k8s_clusters
    row` doc block (249-251) is separated from `import_kubeconfig` by the TODO
    and a second `///` block. Rustdoc therefore attaches it to
    `check_generated_kubeconfig`, and `import_kubeconfig` has no doc at all.
  - `import_kubeconfig` loads `UsersRepo::get(creator)` twice (273 and 368)
    and builds the returned `K8sCluster` by hand instead of reading back the
    row it inserted.
- **Fix:**
  - Use `K8sClustersRepo::create` (keeping the remove-file-on-error cleanup)
    and return its result.
  - Move the doc block onto `import_kubeconfig`.
  - Reuse the first `user` and delete the TODO.

### maintainability-1-06: 15 byte-size formatters with different output (minor)

- **Where:**
  - `fmtBytes`:
    - `lib/metric-format.ts:31` (shared)
    - `database/results-format.ts:153`
    - `aws/util.ts:6`
    - `agents/conversation/format.ts:22`
    - `agents/ContextPreview.svelte:64`
    - `product/AttachmentsPanel.svelte:98`
    - `product/OverviewTab.svelte:1244`
    - `product/design/DesignArena.svelte:790`
    - `mcp/StatsTab.svelte:34`
    - `mcp/AuditTab.svelte:110`
    - `brokers/OverviewTab.svelte:29`
    - `database/StructureView.svelte:620`
  - `formatBytes`:
    - `api/storageGauge.ts:37`
    - `kubernetes/k8s-util.ts:82`
    - `product/design/scene3d/presets.ts:240`
- **Evidence:** The copies disagree:
  - Six stop at MB, so 5 GB renders as "5120.0 MB".
  - k8s uses `Ki`/`Mi`; the others use `KB`.
  - Precision varies: 0, 1 or 2 decimals.
  - Null handling varies: `''`, `'—'`, or a crash on `null`.
  - The brokers module uses the shared `metric-format` in `TopicDetail.svelte`
    but its own copy in `OverviewTab.svelte`.
- **Fix:** Use `lib/metric-format.formatBytes`, extended with a nullable
  overload and an IEC option for k8s. Delete the local copies.

### maintainability-1-07: 12 local relative-time helpers bypass the reactive shared `rel()` (minor)

- **Where:**
  - `lib/stores/now.svelte.ts:64` (shared `rel`)
  - Local copies:
    - `mission-control/lib.ts:179`
    - `mission-control/WorkItemDetail.svelte:317`
    - `canvas/ConversationPanel.svelte:101`
    - `aws/util.ts:20`
    - `git/FocusView.svelte:269`
    - `product/OverviewTab.svelte:1250`
    - `product/ChatTab.svelte:119`
    - `product/DiscoveryTab.svelte:139`
    - `product/RefineTab.svelte:152`
    - `product/LinkedCanvases.svelte:131`
    - `kubernetes/monitor/monitor-util.ts:87`
    - `database/DatabasePage.svelte:939`
- **Evidence:**
  - `rel()` is documented as *"Reactive — re-evaluates as the clock ticks"*,
    with tuned per-minute subscription.
  - The copies read `Date.now()` (for example FocusView:273 and
    DatabasePage:940), so their labels freeze until something else
    re-renders.
  - They use four different output styles: "5m", "5 min ago", "just now", and
    a date.
- **Fix:** Replace them with `rel()`. Add an option to `rel()` for the
  "min ago" style if product copy needs it.

### maintainability-1-08: Unused UI API client wrappers (minor)

- **Where (zero references in `src/`, `e2e/` and `scripts/`):**
  - `lib/api/client.ts`:
    - `listRepoRules`, `toggleRepoRule`, `deleteRepoRule` (966-978)
    - `dbNlToSql` (825)
    - `postForText` (741)
    - `altLoopbackState` (260)
  - `lib/api/proof.ts`: `listProofPacks` (38), `patchProofPack` (113),
    `listSnapshots` (165), `attachMedia` (175)
  - `lib/api/browserLive.ts`: `getLive` (39), `liveNav` (47),
    `liveControl` (51)
  - `lib/api/browser.ts`: `queryPage` (55), `browserLogin` (145)
  - `lib/api/design.ts`: `getProject` (75)
  - `lib/api/workflows.ts`: `getWorkflowVersion` (42)
  - `lib/api/workspaces.ts`: `fetchWorkspace` (16)
  - `lib/api/share.ts`: `openShareTerminalWs` (62)
  - `lib/api/aws.ts`: `isNotInstalled` (80)
  - All of `lib/api/modelCatalog.ts`, whose only export `modelCatalogApi` is
    never imported
- **Evidence:** A grep for each name finds only its definition. The repo-rules
  server routes exist (`routes/repo_rules.rs`), but no UI reaches them.
- **Fix:** Delete the wrappers and `modelCatalog.ts`. If a surface is planned
  (for example repo-rule management), track it in an issue rather than keep
  untested client code.

### maintainability-1-09: Canvas flow-adapter layer and the `@xyflow/svelte` dependency are unused (minor)

- **Where:** `ui/src/modules/canvas/scene.ts:90-166` and `:186-216`;
  `ui/package.json:79`
- **Evidence:**
  - `FlowNode`, `FlowEdge`, `sceneToFlow`, `flowToScene` and `assistToNodes`
    have no references.
  - Only `emptyScene` and `parseScene` are imported
    (`lib/stores/canvas.svelte.ts:16`).
  - `@xyflow/svelte` appears only in `package.json`. No file under `ui/`
    imports it.
- **Fix:** Delete the flow adapter functions and types and remove
  `@xyflow/svelte` from `package.json` and the lockfile.

### maintainability-1-10: Other dead UI helper exports, one with a misleading comment (minor)

- **Where:**
  - `run-with-otto/runStatus.ts:144-148` (`sourceColor`)
  - `lib/sidePane.ts:133` (`SIDE_PANE_KEY`)
  - `lib/stores/splitLayout.ts:178` (`toLegacy`)
  - `lib/providers.ts:95` (`providerAliasMap`)
  - `lib/uiCommands.ts:311` (`uiConnId`)
  - `lib/uiCommands/catalog.ts:21` (`uiCommandSpec`)
  - `lib/uiCommands/frames.ts:19` (`ERROR_CODES`)
  - `lib/ambient.ts:176` (`ambientSamples`)
  - `lib/cssColor.ts:65` (`readTokenColors`)
  - `lib/hl.ts:283` (`hljsLoaded`)
  - `lib/rightTabs.ts:43` (`isPageTabId`)
  - `modules/agents/conversation/api.ts:30` (`touchTranscript`)
  - `agents/conversation/format.ts:65` (`toolSubtitle`)
  - `agents/conversation/toolDetail.ts:64` (`clearToolDetailCache`)
  - `git/diff-load.ts:148` (`prCacheSnapshot`)
  - `kubernetes/k8s-util.ts:102,112` (`parseCpu`, `parseMem`)
  - `insights/capabilities.ts:73` (`statusClass`)
  - `assistant/model.ts:125` (`messageTurns`)
  - `database/grid-view-cache.ts:107` (`scanProgress`)
  - `database/results-format.ts:70` (`vvTree`)
  - `design-hall/model.ts:266,301,358`, `design-hall/brand/tokens.ts:224,230,305`,
    `design-hall/brand/starters.ts:161`, `design-hall/site/engine/*`
  - `snip/annotations.ts:433`
  - `home/school/life.ts:47`
  - `product/design/scene3d/*` (`degToRad`, `radToDeg`, `nodeKind`,
    `setTurntable`, `sceneStats`)
- **Evidence:** Each name is referenced only at its definition. Two are worse
  than dead:
  - `sourceColor(_kind)` is documented *"Kept as a function so call sites stay
    put"*, but there are no call sites.
  - `SIDE_PANE_KEY = 'otto_side_pane'` is exported while
    `stores/sidePane.svelte.ts:93,145` repeats the literal
    `'otto_side_pane'`.
- **Fix:**
  - Delete the dead exports.
  - In `sidePane.svelte.ts`, use `SIDE_PANE_KEY` instead of the literal.
  - Consider wiring a `knip` or `ts-prune` check into `npm run check` to stop
    the regrowth.

### maintainability-1-11: Rust `pub` functions with no callers in core crates (minor)

- **Where:**
  - `otto-state`:
    - `goal_loops.rs:382` `set_run_started_at`, `:406` `add_cost`,
      `:440` `set_name`
    - `swarm.rs:2071` `runs_for_swarm`
    - `assistant.rs:337` `get_thread_any`, `:500` `thread_by_session`
    - `product.rs:846` `set_watch_cursor`
    - `product_attachments.rs:175` `set_assist_result`
    - `sessions.rs:511` `list_all_restorable`
    - `git.rs:307` `set_repo_remote`
  - `otto-server`:
    - `proof.rs:982` `pack_workspace`, `:1236` `ignore_err`
    - `history_index.rs:27` `is_running`
    - `api_secrets.rs:64` `parse_env_ref`
    - `spa.rs:19` `spa_fallback`; `lib.rs:211` uses only
      `spa_fallback_with_assets`
  - `otto-sessions`: `manager.rs:2018` `with_mcp_tools_bin`
  - `otto-core`:
    - `secret_paths.rs:172` `protected_dirs`, `:401` `in_otto_data_dir`
    - `expr.rs:86` `try_eval_bool`
  - `otto-dbviewer`: `types.rs:171` `param_bool`
  - `otto-context`: `materialize.rs:213` `append_context_block`, which its own
    doc calls *"Currently unused: its one caller … removed in Vault v3"*
- **Evidence:** A whole-workspace word search, including tests and `ui/src`,
  finds only the definition for each one. `set_watch_cursor` has been
  superseded: the watcher persists its cursor through the write at
  `otto-product/src/watcher.rs:397`.
- **Fix:** Delete them. They compile, so `dead_code` never warns on these
  `pub` items, and nothing else will remove them.

### maintainability-1-12: Confluence converter keeps write-only state and hides it with `let _ =` (minor)

- **Where:** `crates/otto-issues/src/confluence.rs:914-915`, `:1279` and
  `:1421`
- **Evidence:**
  - `in_code` and `pending_newline` are assigned in 9 places (1231-1334) and
    never read.
  - `is_self_close` is computed at 1279 and never read.
  - Line 1421 is `let _ = (is_self_close, in_code, pending_newline); //
    suppress unused warnings`.
  - A reader will assume the variables drive inline-code and newline handling.
    They do nothing.
- **Fix:** Remove the three variables, their assignments and the suppression.
  If newline coalescing was intended, implement it and add a test.

### maintainability-1-13: `BarHost` keeps "not built yet" scaffolding for a component that exists (minor)

- **Where:** `ui/src/modules/desktop/BarHost.svelte:5-8`, `:20-38` and
  `:57-66`
- **Evidence:**
  - The comment says the bar *"is resolved lazily through import.meta.glob so
    this route builds with or without it; until it exists, a minimal
    pill…"*.
  - `lib/components/FloatingBar.svelte` has existed since `6c78510ee`
    (2026-09-24, the same day as BarHost).
  - The glob plus `.catch(() => {})` now only hides a real chunk-load failure.
    The user then sees *"The assistant bar arrives with the next Otto
    update."*, which is untrue.
- **Fix:**
  - Import `FloatingBar` directly, or with a plain dynamic `import()`.
  - On a load failure, show an inline error with Retry instead of the
    "next update" pill.
  - Remove the stale comment.

### maintainability-1-14: A stale `TODO(mobile)` implies impersonation lacks a share-scope check (minor)

- **Where:** `crates/otto-server/src/routes/impersonate.rs:143-144`
- **Evidence:**
  - The TODO says *"reject when ctx has a share scope … `scope` does not exist
    until the mobile feature lands"*.
  - `AuthContext.scope` exists now. It is read in `feature_guard.rs:210`,
    `modules.rs:165` and `api_stream.rs:83`.
  - `start()` already calls `require_human` (line 104). That rejects share
    credentials (*"an agent session's, MCP or share credential cannot"*,
    `auth.rs:154`), and the feature guard confines scoped tokens to their
    session's allow-list.
  - The TODO therefore points at a gap that is already closed and misleads
    anyone reading the access rules.
- **Fix:** Replace the TODO with a one-line note that `require_human` (line
  104) already refuses share-scoped tokens.

### maintainability-1-15: Five copies of `expand_tilde` (minor)

- **Where:**
  - `otto-core/src/paths.rs:71` (canonical, documented)
  - `otto-connections/src/keyperms.rs:29`
  - `otto-k8s/src/clusters.rs:139`
  - `otto-workflows/src/location.rs:11`
  - `otto-server/src/workflow_engine.rs:7542`
- **Evidence:**
  - All four other crates already depend on `otto-core`.
  - The `otto-workflows` and `workflow_engine.rs` copies are byte-identical.
    The `otto-workflows` one says it expands *"the same way the engine
    does"*, which is a sign it was copied to keep the two in sync by hand.
  - Home lookup varies: `$HOME`/`$USERPROFILE`, `var_os("HOME")`, or
    `dirs::home_dir()`.
- **Fix:** Call `otto_core::paths::expand_tilde` everywhere. k8s can wrap the
  result in `PathBuf::from`.

### maintainability-1-16: Code and docs point to source files that have moved (minor)

- **Where and evidence:**

  | Reference | Says | Now lives in |
  |-----------|------|--------------|
  | `ui/src/lib/api/types.ts:11322` | mirror of `crates/otto-server/src/assistant/types.rs` | `crates/otto-assistant/src/assistant/types.rs` |
  | `docs/features/product.md:51` | `crates/otto-server/src/product_agent.rs` | `product_host.rs` (`impl ProductRunHost`) |
  | `crates/otto-skills/assets/skills/development/otto-canvas/references/endpoints.md:11` | verified against `crates/otto-server/src/product_chat.rs` | `crates/otto-product/src/chat.rs` |
  | `docs/features/skills-library.md:295` | `crates/otto-product/src/skills.rs` | `crates/otto-skills/src/product.rs` |

- **Why it matters:** AGENTS.md makes `types.ts` mirror the Rust contract, so
  a wrong mirror path sends contract edits to the wrong place. The skill file
  ships to agents.
- **Fix:** Update all four paths.

### maintainability-1-17: Date-formatting helpers duplicated within `otto-transcript` (minor)

- **Where:**
  - `crates/otto-transcript/src/agy.rs:677-704`
  - `crates/otto-transcript/src/grok_usage.rs:526-553`
- **Evidence:** Commit `f4f4eac7d` added `format_rfc3339_millis` and
  `civil_from_days` (Hinnant's algorithm) to both files. A `diff` of the two
  30-line blocks shows them identical. The crate has a `util.rs` for shared
  helpers.
- **Fix:** Move both functions into `util.rs`, with one unit test covering a
  leap-day and a pre-1970 value, and call them from both modules.

### maintainability-1-18: Mermaid and D2 canvases duplicate about 85 lines of pan and zoom code (minor)

- **Where:**
  - `ui/src/modules/canvas/MermaidCanvas.svelte:61` and `:156-235`
  - `ui/src/modules/canvas/D2Canvas.svelte:68` and `:167-246`
  - `pointerZoom.ts:25`
- **Evidence:**
  - `diff` of the two ranges shows the first 86 lines identical:
    - `clamp`
    - `fit`
    - `fitView`
    - `zoomBy`
    - `onWheel`
    - `onSurfaceKey`
    - the pointer down, move and up handlers
  - Commit `452ade172` extracted only the pinch maths into `pointerZoom.ts`.
  - The scale bounds `0.1..6` now live in three places.
- **Fix:** Move fit, zoom, wheel and keyboard pan, together with the bounds
  constants, into `pointerZoom.ts`, for example as a `PanZoom` class or a
  Svelte action. Both canvases then keep only their rendering code.

### maintainability-1-19: Stale `allow(dead_code)` attributes and keep-alive shims (nit)

- **Where:**
  - `otto-dbviewer/src/drivers/clickhouse.rs:484`
  - `otto-product/src/http.rs:1495`
  - `otto-rbac/src/cache.rs:646-648`
  - `otto-rbac/tests/resource_access.rs:943-946`
  - `otto-workflows/src/context.rs:469-475`
- **Evidence:**
  - `connect_id` is called at clickhouse.rs:3180 and :3290.
  - `TestCtx.issues` is used at http.rs:1917-1921.
  - `_uses_workspace_role` exists only to keep an unused import alive. No
    "scope-bearing context test" uses `WorkspaceRole`.
  - `_ids_are_strings` has no purpose.
  - `brief_name` is dead behind `TODO(task 5)`.
- **Fix:** Remove the two stale attributes, the two shim functions and the
  unused `WorkspaceRole` import, and either delete `brief_name` or use it.

### maintainability-1-20: `api_client.rs` imports a name only to silence the unused warning (nit)

- **Where:** `crates/otto-server/src/routes/api_client.rs:3010` and
  `:3118-3119`
- **Evidence:**
  - `AUTHORIZATION` is imported into the function scope, then
    `let _ = AUTHORIZATION; // Silence unused import`.
  - `apply_auth` imports it separately at :3356.
- **Fix:** Drop `AUTHORIZATION` from the :3010 import list and delete the
  `let _`.

### maintainability-1-21: `policy.rs` grants access to a route that does not exist (nit)

- **Where:** `crates/otto-server/src/policy.rs:501-504`
- **Evidence:**
  - The code is `if p == "/connections" { // Workspace-less (global)
    connection create — management tier. return Require(Connections, Admin);
    }`.
  - None of the 1,055 `.route(` templates is `/connections`. Creation is
    `POST /workspaces/{id}/connections`. The branch has been dead since
    `627483785` (2026-06-19).
  - Of the 320 exact paths in `policy.rs`, this is the only one with no
    route.
  - It misleads anyone auditing who can access what.
- **Fix:** Delete the branch.

### maintainability-1-22: `sha256_hex` is defined twice inside `otto-state` (nit)

- **Where:** `crates/otto-state/src/mcp_audit.rs:43` (private, `&str`) and
  `crates/otto-state/src/workbench.rs:191` (`pub`, `&[u8]`)
- **Fix:** Make `mcp_audit` call `crate::workbench::sha256_hex(s.as_bytes())`,
  or move the helper into `convert.rs`.

### maintainability-1-23: `retire_user_server` is placed after the test module (nit)

- **Where:** `crates/otto-sessions/src/mcp.rs:992-1063`
- **Evidence:** This public production function, which commit `8a9897823`
  extended for agy, sits after the `#[cfg(test)] mod tests` block (560-991)
  and before a second test module. Every other item in the file comes before
  its tests.
- **Fix:** Move `retire_user_server` above `mod tests`, and merge
  `retirement_tests` into `tests`.
