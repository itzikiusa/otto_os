# Architecture review: quality loop 2026-10-21, iteration 6 (wave 3, final)

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

This is the re-score after fix wave 3. It was read-only: `git diff 4f6d6ab30..HEAD` (the wave-3
diff), `git diff origin/main...HEAD` (the whole branch), `git show`, `rg`/`grep`, `sed`, `awk` and
`wc`, plus a small Python filter over the diff's added import lines. Nothing was built or tested.
Every status was confirmed against `fix/quality-iter6-20261023` at `2065f00e9`.

Excluded: `target/`, `node_modules/`, `docs/reviews/`. The scope is the same as the wave-2 report:
the same areas at the same depth, plus the wave-3 diff read for this lens.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Earlier findings still open | 0 | 0 | 0 | 0 |
| New in wave 3 (inside the covered areas) | 0 | 0 | 0 | 4 (6w3-01, 6w3-02, 6w3-03, 6w3-04) |
| **Total** | 0 | 0 | 0 | 4 |

**Score: 10.0 − 0 × 0.3 − 0 × 0.1 − 4 × 0.03 = 10.0 − 0.12 = 9.88 / 10**

The score went from 9.11 to 9.88. Wave 3 fixed all nine findings that were open at wave 2: the
major 1-09 (GraphView), the five minors 1-04, 1-05, 1-07r, 2-06 and 6-03, and the three nits
4-01r, 6w2-01 and 6w2-02. None was only partly fixed.

The four new nits are small:

- 6w3-01 is the same pattern as 6w2-01 (a hand-built Claude layout or cwd encoder), at two sites
  that were not on 6w2-01's list. FIXES P7 already lists one of them as a known hand-off.
- 6w3-02 is the automation run loop, which stayed in an axum handler after the REST engine moved.
- 6w3-03 is a type/component name collision that the GraphView split created. It is the same
  pattern as 6w2-02, which wave 3 fixed.
- 6w3-04 is product's per-lens retry, a second copy of the lens turn that the fan-out move left in
  place.

None of the four is a wrong-way dependency, a cycle or a god file.

## Status of earlier findings

| ID | Sev at wave 2 | Status | Evidence now |
|----|---------------|--------|--------------|
| 1-09 `GraphView.svelte` | major | **Fixed** | 4,082 → 1,064 lines (script block `:1-216`). The refs sidebar is `git/graph/RefSidebar.svelte` (786) + `RefSideLists.svelte` (213). The ref popover is `RefPopover.svelte` (203) over `refPopoverState.svelte.ts` (131). The detail pane is `CommitDetailPane.svelte` (296). State sits behind narrow host factories: `graphData` (705), `graphRefIndex` (279), `graphViewport` (241), `graphSelection` (343), `graphSync` (173) and `refDrag` (74), and GraphView wires them (`:79-130`). `RefSidebar` takes a typed `Props` (`RefSidebar.svelte:21-41`) that holds the `GraphActions`, `GraphMenus`, `GraphRefIndex` and `GraphSelection` handles. The new `graph/*` files import only from `lib/` and the git module. See the new nit 6w3-03 about naming. |
| 1-04 fan-out + summarizer per feature (product) | minor | **Fixed** | `run.rs` is a 194-line module root over `run/{analysis,plan,rewrite,tests_gen,retry,reap,prompts,context,scratch,narrative}.rs`. `run_analysis` (`run/analysis.rs:73`) runs on `fanout::run` (`:153`). Lenses are `LensTurn` (`:187`), the summarizer is `SummarizerTurn` (`:323`) and the fallback is `merge_lens_findings` (`:459`). `run_generate_plan` (`run/plan.rs:58`) runs on the new `fanout::run_with_quorum` (`:195`; `otto-agent-run/src/fanout.rs`). The hand-rolled `JoinSet` loop, the inline summarizer and the "stuck in running" guard are gone. All four engines now share the runner. See the new nit 6w3-04 about the per-lens retry. |
| 1-05 two MCP surfaces | minor | **Fixed** | The session twins are generated from the control-plane specs: `NATIVE_TWINS` (`ottod/src/mcp_tools/catalog.rs:84`, 46 entries) is built over `otto_mcp::outward::otto_tool_specs` (`:34-37`). `ROUTED_TWINS` went from 4 to 30 entries (`mcp_tools/dispatch.rs:122`), with a pre-step (`TwinScope`, `twin_args` `:208`) and reply shaping (`TwinReply`). The twins that stay native are listed with their reasons (`dispatch.rs:108-121`: the `/refs/directory` lists, `list_repos`, and the `api_*` name resolution). Parity tests are in `mcp_tools/tests/routing_tests.rs` (`:48`, `:77-85`). `run_tool` is down to `dispatch.rs:672-905` (was 1,189 lines) and `base_tool_catalog` to `catalog.rs:150-451`, which now holds only the session-only tools (was 687). `route_for` (1,536 lines) is still in `loc-baseline.json`. It is now the one route table, so it no longer counts as a second surface. |
| 1-07r REST engine in a route file | minor | **Fixed** | The engine is now `otto-apiclient/src/engine/{env,binding,client,tunnel,send,auth,multipart,history,execute,steps,vars}.rs` behind `ApiClientCtx` (`engine/mod.rs:78-95`, 7 methods). The `ServerCtx` impl is `routes/api_client/ctx.rs` (63 lines). `routes/api_client.rs` went from 3,261 to 917 lines. `workflow_api_request.rs:49` calls `otto_apiclient::engine::execute_service`. `otto-apiclient` gains `otto-state` and `otto-ssh`, both leaf-ward. `api_secrets.rs` is a 6-line re-export. See the new nit 6w3-02 for the automation run loop, which was not on this finding's list. |
| 2-06 rooms inside otto-server | minor | **Fixed** | New crate `crates/otto-rooms` (registry, actions, presentation, annotations, capability, auth, authority, lifecycle, ICE, recap, recap engines, games). Its only otto dependencies are `otto-core` and `otto-pty`, and `rg 'ServerCtx\|axum\|otto_server' crates/otto-rooms` is empty. `RoomsCtx` (`otto-rooms/src/ctx.rs`, 14 methods) is implemented in `otto-server/src/rooms/ctx.rs` (87 lines). `otto-server/src/{rooms,game_rooms}` now hold only handlers, sockets and the terminal run loop (1,224 lines including 220 lines of tests). AGENTS.md lists the crate (`:81`). |
| 6-03 `apiClient.svelte.ts` god store | minor | **Fixed** | 2,113 → 764 lines. It is a façade over `lib/stores/apiClient/{model (342), tabs.svelte (267), drafts.svelte (312), execute.svelte (278), cookies.svelte (69), curl (117), collectionsSync (245), automations.svelte (269)}` (imports `:29-36`). Each part has a narrow host contract. |
| 4-01r swarm paste path kept public for one test | nit | **Fixed** | `submit_run_prompt`, `send_run_input`, `guarded_dispatch`, `SwarmHost::dispatched` and `PtyTimings::paste_to_enter` are gone (`rg` finds none). The interleave test drives `submit_turn_prompt` (`otto-server/tests/swarm_router.rs:29`, `:488`, `:516`). |
| 6w2-01 Claude layout copies in nested.rs / otto-context | nit | **Fixed** | `nested::claude_transcript_in_window` takes `&TranscriptRoots` and scans every Claude root (`otto-sessions/src/nested.rs:282-302`). In otto-context, `materialize.rs:147`, `:399-401` use `roots::{encode_cwd, default_claude_root, home_dir}`, and `enc_project` is gone. `otto-context/Cargo.toml:12` adds `otto-transcript`, which is leaf-ward. See 6w3-01 for two sites that were not on this finding's list. |
| 6w2-02 `RunMonitor` class/component name clash | nit | **Fixed** | The class is `RunSync` (`workflows/runSync.svelte.ts:77`), and its five importers use that name. `RunMonitorState` no longer appears anywhere. |

No earlier architecture finding was rejected.

## Areas covered in this iteration

These are the same areas as wave 2 (and waves 0 and 1), re-read at `2065f00e9`:

- **Earlier findings.** Re-verified all 9 findings that were open at wave 2, as above.
- **Recent churn.** The top-50 churn files from the wave-2 ranking. Sizes of the large ones were
  compared at `4f6d6ab30` and `HEAD`. These are unchanged in wave 3: `otto-sessions/manager.rs`
  6,502, `modules.rs` 1,089, `policy.rs` 3,449, `workflow_engine.rs` 4,615, `otto-git/local.rs`
  5,533, `ottod/main.rs` 1,129, `types.ts` 59, `workspace.svelte.ts` 2,129, `exec.rs` 2,248,
  `mcp_outward.rs` 2,290, `ctx_impls.rs` 730, `WorkflowsPage.svelte` 2,688, `database.svelte.ts`
  1,004, `OverviewTab.svelte` 1,772 and `vault_docs_agent.rs` 1,379. `ottod/mcp_tools.rs` went from
  7,370 to 4,347, of which tests start at `:1853`.
- **Transcript discovery and parsing across crates.** `otto-usage/src/tailer/driver.rs`,
  `otto-server/src/{transcript/resolve.rs, history_index.rs}`, `otto-improve/src/evidence.rs`,
  `otto-sessions/src/{manager.rs, lifecycle.rs, nested.rs, accounts.rs}`,
  `otto-agent-run/src/{turn_oracle.rs, session_turn.rs}`, `otto-context/src/materialize.rs` and
  `ottod/src/main.rs::start_usage_tailer`. A sweep for the encoder and layout patterns
  (`is_ascii_alphanumeric() { c } else { '-' }`, `.claude/projects`) found two production copies
  left: 6w3-01. `otto-sandbox` spelling `.claude/projects` is a deliberate leaf with no
  `otto-transcript` dependency, and `otto-workflows::slug` and the others are unrelated slugs.
- **otto-sessions `manager.rs`** (6,502 lines). Wave 3 changed only the `nested` call (+8/−8).
  No further finding.
- **otto-dbviewer drivers.** Not touched in wave 3. Same conclusion as wave 2: no finding.
- **UI stores and large files.** `lib/stores/apiClient.svelte.ts` and its parts (6-03 fixed). The
  `/repos/{id}/…` call sites (6-04). The raw `api.get`/`api.post` calls in `graph/graphData` and
  `graph/graphActions` were already raw in GraphView on main, as wave 2 noted. The windowing
  primitives (`graph/graphViewport` uses `lib/listWindow`'s `listRange`).
- **`ui/src/lib/api/types.ts`.** Still a 59-line façade. No finding.
- **`otto-server/src/modules.rs`, `policy.rs`, `ottod/src/main.rs`.** Unchanged in wave 3. No
  finding.
- **New domain files in the window.** `otto-automation/scheduled_tasks_engine.rs`,
  `otto-insights/lib.rs`, `otto-apiclient/{rest,grpc}.rs` (now with `engine/` and `secrets.rs`),
  `otto-pty/holder.rs` and `otto-issues/linear.rs`. No finding.

**The wave-3 diff for this lens** (`git diff 4f6d6ab30..HEAD`, 190 files under `crates/` and
`ui/src`) was read inside those areas:

- **The new crate and the moved engines.** `otto-rooms` (`ctx.rs`, the `RoomsCtx` impl, the
  handlers left in otto-server), `otto-apiclient/src/engine/*` (the `ApiClientCtx` impl, and the
  callers in `workflow_api_request.rs`, `api_stream.rs`, `grpc.rs`, `api_oauth.rs` and
  `api_automation_runs.rs`), `otto-product/src/run/*`, `otto-agent-run/src/fanout.rs`
  (`run_with_quorum`, `fan_in`) and `ottod/src/mcp_tools/{catalog,dispatch,listing,…}.rs`.
- **The dependency edges** (every added `otto-*` line in a `Cargo.toml`):
  - `otto-product` → `otto-agent-run`
  - `otto-context` → `otto-transcript`
  - `otto-apiclient` → `otto-state`, `otto-ssh`
  - `otto-server` → `otto-rooms` (`otto-core`, `otto-pty`)

  All of these point leaf-ward. No cycle, and no edge into otto-server.
- **The UI splits:** `git/graph/*`, `lib/stores/apiClient/*` and `workflows/runSync.svelte.ts`. Every
  import line the diff adds under `ui/src` was checked by script for module → other-module and
  `lib/` → `modules/` imports. There are none.

The fixes added no dependency cycle, no wrong-way crate edge and no cross-module UI import. One
small naming problem is new: 6w3-03, from the GraphView split. The other three nits are
leftovers next to the moved code (6w3-01, 6w3-02, 6w3-04).

## Areas NOT covered yet (next iteration)

These are unchanged from wave 2:

- otto-usage `engine.rs` / `clickhouse.rs` internals.
- otto-dbviewer `service.rs` / `types.rs` as a whole.
- otto-git `local.rs` cohesion.
- The large ratcheted functions: `controller_inner`, `bridge::handle`, `static_review`,
  `run_skill_eval_core`, `route_result_inner`, and `route_for` as a function.
- The large UI files: `ReviewPanel`, `DatabasePage`, `QueryEditor`, `QueryBuilder`, `UsagePage`,
  `RequestBuilder`, `DocsAgentsView`, `FloatingBar`, `SessionView`.
- Typed-client gaps in settings and workflows.
- Hand-rolled drag-to-reorder.
- `webhook_key_guard`.
- `otto-design-assist/src/lib.rs`.
- `mcp_outward.rs::governed_invoke`.

---

## Open findings

### architecture-6w3-01: nit: two more cwd-encoder / Claude-layout copies outside `otto_transcript::roots`

**Location:**
- `crates/otto-server/src/routes/handover.rs:587-613` (`find_claude_transcript`), called at `:567`
  from `gather_source_context` (`:560`)
- `crates/otto-agent-run/src/session_turn.rs:1046-1050` (`claude_tasks_dir`'s `enc`), called at
  `:832`

**Evidence**

6w1-01 and 6w2-01 made `otto_transcript::roots::{encode_cwd, default_claude_root,
TranscriptRoots}` the one Claude layout and cwd encoder (`roots.rs:46-57`), and fixed every site
they listed. Two copies remain:

- **`handover.rs`.** `find_claude_transcript` reads `HOME`/`USERPROFILE`, builds
  `.join(".claude").join("projects")` by hand, and carries its own copy of the encoder
  (`c.is_ascii_alphanumeric() { c } else { '-' }`, `:593-596`). It looks only under `$HOME`. The
  session's `TranscriptRoots` (account roots, `OTTO_TRANSCRIPT_ROOTS`, the E2E fixture) are
  ignored. So a handover from a Claude session running under a provider account falls back to
  scrollback. The same crate already has the resolver this needs:
  `transcript::resolve_transcript_sync(&data_dir, persisted, &Session)`
  (`otto-server/src/transcript/resolve.rs:65`, used by `routes/history_page.rs:202`). FIXES P7
  lists it as "left for a follow-up".
- **`session_turn.rs`.** `claude_tasks_dir` has a sixth copy of the encoder. Its doc says it is
  "the same non-alphanumeric→`-` map `project_dir` uses". `otto-agent-run` already depends on
  `otto-transcript`.

**Cost**

This is the drift 6-01, 6w1-01 and 6w2-01 removed step by step. The next change to the encoding or
to account roots has to find these copies again. The handover lookup already misses account
sessions.

**Fix**

- In `gather_source_context`, call `crate::transcript::resolve_transcript_sync(&ctx.data_dir, None,
  source)` inside the existing `spawn_blocking`, and delete `find_claude_transcript`. The
  resolver's own scan fallback covers the "cwd doesn't encode 1:1" case.
- In `claude_tasks_dir`, use `otto_transcript::roots::encode_cwd(cwd_canon)`.

### architecture-6w3-02: nit: the API automation run loop stayed in an axum handler, and another handler calls it

**Location:**
- `crates/otto-server/src/routes/api_automation_runs.rs:86-287` (`start`: the dataset-row × step
  loop in the `tokio::spawn` at `:172-285`, plus `prune_history` `:312` and `runs_keep` `:297`)
- `crates/otto-server/src/routes/api_client/automations.rs:83-117` (`run_automation` builds
  `Path`, `State`, `CurrentUser`, `CurrentAuthContext` and `Json` to call `start` at `:91-99`, then
  polls the run at `:102-116`)

**Evidence**

P4 moved the whole REST engine, including the per-step `engine::run_step`, into `otto-apiclient`
behind `ApiClientCtx`. The loop that drives a run stayed in the route file. That loop handles
dataset rows, cancel through the `live()` registry, the per-step history row and its
`ApiHistoryAppended` event, retention every `PRUNE_EVERY` steps, step persistence and the final
status. The handler is 200 lines.

The synchronous `run_automation` route (which the MCP `api_run_automation` twin reaches,
`ottod/src/mcp_tools/dispatch.rs:1143`, `otto-mcp/src/outward/exec.rs:697`) runs the automation by
calling the `start` handler with hand-built extractors.

**Cost**

There is no engine entry point for "run this automation". A workflow node or a scheduled task that
wants one has to call an axum handler or copy the loop. The handler-calls-handler shape ties
`run_automation` to `start`'s extractor signature: adding an extractor to `start` breaks the other
route.

**Fix**

Move the loop into `otto_apiclient::engine::automation::start_run(ctx: &impl ApiClientCtx, …) ->
RunHandle`, with the cancel/progress registry, `prune_history` and `runs_keep` going through the
ctx (`workspace_settings`, `broadcast`). Then both `start` and `run_automation` call it.
`run_automation` should await the `RunHandle` instead of re-entering a handler.

### architecture-6w3-03: nit: the `RefChip` type and the new `RefChip.svelte` component share a name

**Location:**
- `ui/src/modules/git/ref-menus.ts:62` (`export interface RefChip`)
- `ui/src/modules/git/graph/RefChip.svelte:8` (imports `type RefChip` into a component of the
  same name)
- `ui/src/modules/git/GraphView.svelte:24` (`type RefChip as Chip`) next to `:34` (`import RefChip
  from './graph/RefChip.svelte'`)
- `ui/src/modules/git/graph/CommitDetailPane.svelte:12-13` (the same pair and alias)

**Evidence**

This is the 6w2-02 pattern, which wave 3 fixed for `RunMonitor`, re-created by the GraphView split.
At `4f6d6ab30`, GraphView imported `type RefChip` with no alias (`:51`). The new component took the
name, so the two files that render chips now have to alias the type to `Chip`. The component's
`chip` prop is typed by its own name. `graphRefIndex`, `graphSelection`, `graphMenus`,
`refPopoverState` and `unit/gitRefMenus.test.ts` use the type unaliased.

**Cost**

One name means two things, and every new file that renders chips has to choose an alias.
Go-to-symbol and grep for `RefChip` return both the type and the component.

**Fix**

Rename the component `graph/RefChipView.svelte` (or rename the type `RefChipData`), then drop the
`as Chip` aliases in `GraphView.svelte:24` and `CommitDetailPane.svelte:13`.

### architecture-6w3-04: nit: product's per-lens retry is a second copy of `LensTurn::run`

**Location:**
- `crates/otto-product/src/run/retry.rs:38-260` (`retry_analysis_agent_owned`; the session run,
  parse and persist at `:195-259`)
- `crates/otto-product/src/run/analysis.rs:201-320` (`LensTurn::run`)

**Evidence**

Wave 3 moved the analysis onto `fanout::run`, and `LensTurn::run` became the one place a lens
session runs. The user's per-lens Retry (and the boot reaper, `run/reap.rs:148`) still runs its own
copy of the same steps:

- build the analysis prompt plus its out-path
- `run_agent_with_recovery` with `LENS_TIMEOUT` and `SessionAppearance::analysis`
- `set_agent_session`
- `extract_json_as::<Findings>`
- `set_agent_status` done or error
- the `SUPERSEDED` skip
- the failure `Notice`

Its own comment says it "mirrors the run_analysis per-agent logic" (`:221`), and the copies have
already drifted:

- The retry's error text drops the provider's failure reason and attempt count. `LensTurn` says
  "agent failed after {MAX_AGENT_ATTEMPTS} attempts ({r})" and "stopped by user". The retry says
  only "retry: session produced no output".
- The retry's notice title drops the provider.

The other engines' retries re-enter the shared slot instead: vault docs through
`fanout::attempts`, code review through `retry::row_slot`.

**Cost**

A change to how a lens turn runs or records its result has to be made twice. The retry path is the
one with less test coverage (the 8 new engine tests drive `run_analysis`).

**Fix**

Give `LensTurn` a `run_on_row(agent_id, spec, cwd)` that does everything after the row exists:
prompt, session, parse, status and notice. `LensTurn::run` creates the row and calls it.
`retry_analysis_agent_owned` keeps only its route-param confinement and context-file rebuild, then
calls it on the existing row.
