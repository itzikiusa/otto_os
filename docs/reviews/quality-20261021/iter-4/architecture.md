# Architecture review: quality loop 2026-10-21, iteration 4

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

The review was read-only, using `git log`, `rg`, `sed`, `awk`, `wc` and local Python imports of
`scripts/loc-ratchet.py` (its own `production_lines` and `function_lengths`) to count production lines
and function spans. Nothing was built or tested. Every finding was confirmed against
`fix/quality-loop-20261021` at `6cff08270`.

These paths were excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ClassroomsBox.svelte`, `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Earlier findings still open, fully or as a residual | 0 | 3 (1-04, 1-09, 1-10) | 9 (1-01r, 1-05, 1-07r, 1-08r, 1-13, 1-16, 2-06, 2-08, 3-03) | 2 (2-01r, 3-02r) |
| New in iteration 4 | 0 | 2 (4-01, 4-02) | 2 (4-03, 4-04) | 1 (4-05) |
| **Total** | 0 | 5 | 11 | 3 |

**Score: 10.0 − 5 × 0.3 − 11 × 0.1 − 3 × 0.03 = 10.0 − 1.5 − 1.1 − 0.09 = 7.31 / 10**

The score went up from 6.77 to 7.31. Iteration 3 fully fixed eight findings: 1-12, 2-03, 2-05, 3-01,
3-04, 3-05 and 3-06, plus the TLS and client-cache core of 3-02. It also took `WorkflowsPage.svelte`
from 5,156 to 3,703 lines, so 1-08 drops to a minor residual.

To reach 9.8, the remaining work is:

- the two UI god files (1-09 GraphView, 1-10 DatabaseStore) and the fan-out runner (1-04);
- the two new majors. Both are shared-primitive extractions that have already caused real drift:
  - 4-01, the prompt-submit path;
  - 4-02, the design-document assist engines.

## Status of earlier findings

| ID | Sev (then → now) | Status | Evidence now |
|----|------------------|--------|--------------|
| 1-01 (residual) `run_workflow_with_budget` | minor | Open | Still 1,250 production lines (`workflow_engine.rs:1850`), capped by the `functions.run_workflow_with_budget: 1250` ratchet. FIXES P01 deferred the `RunState` split. |
| 1-04 fan-out + summarizer per feature | major | Open | `skill_review.rs:400` (`agent.name == "summarizer"`), `vault_docs_agent.rs:356` (`SUM_RETRY_IDX = usize::MAX`) and `modules/review/summarize.rs` are unchanged. `otto-agent-run/src` still has no fan-out runner (`agent_run.rs`, `offload.rs`, `screen.rs`, `turn_oracle.rs`). |
| 1-05 two MCP catalogs | minor | Open | `ottod/src/mcp_tools/tests/catalog_tests.rs:16`: `DEFERRED_SCHEMA_DRIFT` still lists 7 tools. The two executors are now the two largest functions in the workspace: `otto-mcp/src/outward/exec.rs:282` `route_for` (1,620 lines) and `ottod/src/mcp_tools.rs` `run_tool` (1,235 lines). |
| 1-07 (residual) REST engine in a route file | minor | Partly fixed again, still open | `api_helpers.rs` moved to `otto-apiclient/src/rest.rs` (1,754 lines). `otto-server/src/api_helpers.rs` is a 24-line shim. **Still open:** `execute` (`routes/api_client.rs:1808`), `execute_service` (`:1826`) and `resolve_environment` (`:2017`) remain in the route file. Non-route code still calls into it: `workflow_api_request.rs:50`, `routes/grpc.rs:49,81` and `modules/browser_proxy.rs:117,229`, which reach `routes::api_client::net_guard` (a re-export of `otto-netguard` at `routes/api_client.rs:86`). |
| 1-08 `WorkflowsPage.svelte` | major → **minor (residual)** | Mostly fixed | 5,156 → 3,703 lines. The per-kind inspector forms, pickers and `NodeParams` live in `workflows/inspector/` (22 files). **Still open:** the page still owns both the editor and the run monitor in one 1,600-line script: live run sync (`refetchRun` `:285`), the final-output panel (`:357-433`), approvals (`approveRun` `:1445`), `waitRunTerminal` (`:1003`) and versions (`:1563`). |
| 1-09 `GraphView.svelte` | major | Open | 4,877 lines, unchanged. FIXES says the RefSidebar move needs `svelte-check`. |
| 1-10 `DatabaseStore` | major | Open | 4,423 lines (4,415 in iteration 3, so it grew by 8), capped at 4,423 in `ui/scripts/ui-guards-baseline.json:135`. Only the test-harness groundwork landed. |
| 1-12 product ↔ design-hall cycle | minor | **Fixed** | `design-hall` imports nothing from `product`. Only `product/MockupsTab.svelte:7` → `design-hall/DesignGraphStrip.svelte` remains, which is one direction and not a cycle. |
| 1-13 `lib/` imports `modules/` | minor | Open | 33 imports (was 36). The JsonTree helpers moved to `lib/jsonTree/`. The baseline (`ui-guards-baseline.json:434-451`) still allows 8 in `lib/stores/database.svelte.ts`, 5 in `k8s.svelte.ts` and 3 in `uiCommands/k8s.ts`. |
| 1-16 usage tailer in the binary; Codex model triplicated | minor | Open | `ottod/src/usage_tailer.rs` grew from 2,424 to 2,461 lines (11 commits in 3 weeks). The `"slug"` fallbacks are still at `usage_tailer.rs:1148`, `otto-transcript/src/usage.rs:512` and `codex.rs:1074`. |
| 2-01 (residual) terminal grid negotiation | minor → **nit (residual)** | Mostly fixed | `lib/components/termGridSync.ts` (113 lines, `TermGridSync`) owns the claim and grid feed. The resize half (`deferLocalResize` `Terminal.svelte:1108`, `sendResize` `:1324`) stays, interleaved with the resize-compact queue. |
| 2-03 (residual) API-client getters on `ws` | nit | **Fixed** | The getters moved to the apiClient store. |
| 2-05 shell root owns palette sources | minor | **Fixed** | `shell/paletteSources.svelte.ts` and `installShell.svelte.ts` hold them. `App.svelte` is 1,486 lines with 5 `registry.register(` calls (was 14). |
| 2-06 rooms inside otto-server | minor | Open (deferred to the games team) | `otto-server/src/rooms/` and `game_rooms/` are unchanged. |
| 2-08 game map ids hand-synced | minor | Open (deferred) | `game_rooms/types.rs:11-23` is unchanged. |
| 3-01 swarm activation ×4 | major | **Fixed** | `otto-swarm/src/runtime/lifecycle.rs:37` `activate(ctx, ws, sid, ActivatePolicy)` takes `operation_guard` (`:38`). `channels.rs:138` and product (`otto-product/src/swarm.rs:861` → `ProductStudioHost::activate_swarm`, `host.rs:145`) call it. `pause` and `abort` sit beside it (`:73`, `:89`). |
| 3-02 Atlassian transport ×2 | major → **nit (residual)** | Fixed in substance | `otto-issues/src/atlassian.rs:103` `AtlassianTransport` owns the TLS refusal (`:122`, enforced by `http()` `:151`, with an https-only backstop client), the bounded cache (`cached_client` `:57`, which Linear reuses at `linear.rs:64`) and `send()` (`:158`). Tests cover cleartext refusal and the bounded cache (`:181`, `:198`). **Residual:** `jira.rs` still has 21 and `confluence.rs` 5 hand-written `is_success()` branches, each building its own error text instead of calling `t.send(req, what)`. |
| 3-03 route files as the budget and transcript service layer | minor | Open | `routes/usage.rs:465` `load_budgets_pub` and `:471` `budget_status_pub` still exist. `check_budget` is called from `goal_loops.rs:88,237,424,481`, `budget_gate.rs:23`, `modules/review/routes.rs:367`, `swarm_host.rs:105`, `product_host.rs:511` and `monitor.rs:982,1003`. Transcript helpers are called from `assistant_host.rs:90`, `history_index.rs:165`, `agent_tasks_nudge.rs:85` and `transcript_tail.rs:368,771,825`. FIXES says this was not started. |
| 3-04 per-key lock registry ×22 | minor | **Fixed** | `otto_core::keyed_lock::KeyedLocks` replaces the unpruned and panicking registries: otto-git `http.rs`, swarm `engine.rs`, `merge.rs`, `workspace.rs`, product `analysis.rs`, `agent_attempt.rs`, `run_workspace.rs` and `bridge.rs`. `scripts/check.sh:214-223` fails on any new `HashMap<K, Arc|Weak<Mutex<()>>>` outside a fixed allowlist. The allowlisted ones that remain are bounded single-flight maps, and none can grow further. |
| 3-05 no UI issue-tracker client | minor | **Fixed** | `lib/api/issues.ts` and `lib/stores/issueAccounts.svelte.ts` exist. `rg "/issue/"` outside `issues.ts` finds only a doc comment (`lib/api/types/integrations.ts:33`). |
| 3-06 `otto-core/src/api.rs` at the cap | minor | **Fixed** | `api.rs` is 88 lines of re-exports over 20 area files in `otto-core/src/api/`. |

No earlier architecture finding was rejected.

## Areas covered in this iteration

- **Earlier findings.** Re-verified all 20 earlier findings against the code, as listed above.
- **Prompt submission.** Traced every paste-and-Enter submitter end to end. This means every
  `bracketed_paste(`, `wait_for_tui(`, `submit_text(`, `submit_prompt(` and `submit_lock` call across
  otto-sessions, otto-review, otto-channels, otto-swarm, otto-automation, otto-insights,
  otto-assistant and the otto-server hosts. This produced 4-01.
- **otto-design-assist, otto-canvas assist, otto-server `mockup_assist.rs` and `db_assist.rs`.**
  Compared turn runner, busy registry, live poll, format adapters and fence parsing, and also the
  `otto-product::design_format` and `otto-design::format` registries. This produced 4-02.
- **otto-design.** Covered the crate layout, the `format.rs` spec table and the `DesignCtx::validate_content`
  host hook. The `design_scene3d` re-export of `otto_product::design_scene3d` is covered under 4-02.
- **Workflows.** Compared `routes/workflows.rs` with `otto-workflows`: trigger validation, templates,
  the event-trigger listener in `otto-workflows/src/triggers.rs`, `workflow_trigger_scheduler.rs` and
  the UI `triggerForm.ts`. This produced 4-03.
- **`policy.rs`** (39 commits in 3 weeks; not re-examined since iteration 2). `policy_for` (1,404 lines)
  and `route_class` are order-dependent string tables. They are backed by `tests/policy_coverage.rs`
  (every routed template classified), `tests/rbac_matrix.rs` and the `policy_decisions.txt` snapshot.
  That makes them central by design and guarded, so there is no finding.
- **Function sizes across the whole workspace.** Used the ratchet's own counter. This produced 4-04.
  Also covered `serve_terminal` (`otto-sessions/src/ws.rs:1641`) and the `review_run` node.
- **SQL classifiers.** Swept the remaining read-only SQL classifiers after the 2-02 consolidation.
  This produced 4-05. The SQLite `otto-state/src/pool.rs:382` classifier is legitimately separate.
- **otto-sessions.** Covered the function spans in `manager.rs` (shrank from 7,648 to 6,732 lines in 3 weeks) and in `ws.rs`.
- **otto-dbviewer `service.rs`.** At 3,609 production lines it is still cohesive: the largest functions
  are `import_from_path` (186) and `build_schema_graph` (172). No finding.
- **Crate graph.** Re-derived the graph from every `Cargo.toml`. There is no new wrong-direction edge.
- **UI cross-module imports.** Built an exact matrix by resolving every relative import. The edges are:
  - five modules → `canvas/{mermaid,d2}`;
  - `aws` → `database/ResultsGrid`;
  - `loops`/`swarm` → `agents/SessionView`;
  - four modules → `home/LazyNeedsYouLink` (23 lines).

  `canvas/mermaid.ts` and `vault/mdRender.ts` are 3-line shims. `canvas/d2.ts` (159 lines) is a
  shared lazy renderer that still lives in canvas. It is low cost, so it is noted only.
- **UI files.** Covered `shell/Navigator.svelte` (cohesive session list), `lib/stores/workspace.svelte.ts`
  (session- and layout-centric after 2-03) and `lib/api/mock.ts` (gated dev-only).
- **Desktop shell** (`apps/desktop/src-tauri`, 6,045 lines). Covered size and daemon coupling: only
  health and meta probes plus room-origin allowlists. No finding.

## Areas NOT covered yet (next iteration)

- **otto-design.** `store.rs` (2,392 production lines) and `service.rs` (1,965) internals:
  versioning, links, retention.
- **Large backend files.**
  - `otto-usage` `engine.rs` (2,829).
  - `otto-dbviewer` `drivers/mongodb.rs` (4,225, ratcheted) and `drivers/clickhouse.rs` (3,602).
    These are two drivers near the 4,000 cap.
  - `otto-state/src/swarm.rs` (2,679) and `product.rs` (1,966): whether domain rules leaked into
    repositories.
- **Large functions.**
  - `otto-product/src/run.rs` `run_analysis` (580 lines) and `run_generate_plan` (446).
  - `otto-automation/src/goal_loop.rs` `controller_inner` (472).
  - `otto-channels/src/bridge.rs` `handle` (451).
  - `otto-k8s` `run_cycle_with` (413).
- **otto-rbac `tokens.rs`.** Not covered yet.
- **Shared MCP surface.** `otto-mcp/src/outward/exec.rs` `route_for` (1,620 lines) against the
  native `run_tool` (1,235 lines). This is the 1-05 subject at function level.
- **UI.**
  - `git/DiffViewer.svelte` (2,344), `git/ReviewPanel.svelte` (2,529) and `git/WipPanel.svelte` (1,606).
  - `database/QueryEditor.svelte` (2,230), `QueryBuilder.svelte` (2,160), `DatabasePage.svelte`
    (2,814) and `StructureView.svelte` (1,792).
  - `usage/UsagePage.svelte` (2,062), `api/RequestBuilder.svelte` (1,961) and
    `vault/DocsAgentsView.svelte` (1,802).
  - `lib/components/FloatingBar.svelte` (1,580).
  - `aws` reuses `database/ResultsGrid.svelte`, which pulls the edit flow (`EditFlow`,
    `MongoFilterBar`, `edit-sql`) into AWS views. The coupling has not been measured.
- **Examples.** `examples/plugins/**`.

---

## New findings

### architecture-4-01: major: the "submit a prompt to an agent TUI" primitive lives in the code-review crate, so 8 engines carry their own copy, and only some take the submit lock

**Location:** `crates/otto-review/src/session.rs:883`, `:986`, `:1148-1153`; `crates/otto-automation/src/ctx.rs:111-120`; `crates/otto-swarm/src/runtime/host.rs:103-107`; `crates/otto-automation/src/scheduled_tasks_engine.rs:1057-1068`; `crates/otto-automation/src/goal_loop.rs:2079-2093`; `crates/otto-server/src/product_host.rs:156-190`; `crates/otto-server/src/modules/ctx_impls.rs:593-610`; `crates/otto-channels/src/bridge.rs:438-460`, `:521-590`; `crates/otto-swarm/src/runtime/engine.rs:107-128`; `crates/otto-sessions/src/manager.rs:2215`

**Evidence**

Pasting a prompt into a provider TUI takes a fixed sequence:

1. wait for the TUI to draw;
2. take the per-session `SessionManager::submit_lock`;
3. bracketed paste;
4. settle;
5. Enter;
6. if no dispatch, Enter again.

The most complete version is `otto_review::session::submit_prompt` (`session.rs:1153`). It adds the
paste-echo wait and the Ctrl-U re-paste, and its doc comment calls itself "the one submit path every
engine should use" (`:1148-1149`). It lives in **otto-review**, the multi-agent code-review crate.
otto-server re-exports it as `review_session` (`otto-server/src/lib.rs:70`). Domain crates such as
otto-automation and otto-swarm cannot depend on a review crate for a session primitive. So their host
traits expose the raw parts instead (`AutomationCtx::{wait_for_tui, dispatched, bracketed_paste,
paste_to_enter}`, `ctx.rs:111-120`; `SwarmHost::{bracketed_paste, wait_for_tui, dispatched}`,
`host.rs:104-107`), and every engine assembles the sequence again.

| Implementation | Location | Submit lock | Echo wait / re-paste |
|----------------|----------|-------------|----------------------|
| `otto_review::session::submit_prompt` | `session.rs:1153` | yes | yes |
| `SessionManager::submit_text` / human submit | `manager.rs:2215`, `:4369` | yes | no |
| channel bridge (own `wait_for_tui` + `Readiness` enum) | `bridge.rs:448`, `:521-590` | yes | no |
| swarm `submit_run_prompt` | `engine.rs:107-128` | yes | no |
| scheduled tasks | `scheduled_tasks_engine.rs:1057-1068` | **no** | no |
| goal loops | `goal_loop.rs:2079-2093` | **no** | no |
| product agent runs (plus a Codex-only settle loop) | `product_host.rs:156-190` | **no** | no |
| `InsightsCtx::submit_prompt` impl | `ctx_impls.rs:593-610` | **no** | no |

Concurrency-async 3-01 (iteration 3) added the lock to `submit_prompt`. Its iteration-4 follow-up
(concurrency-async 4-01) finds the four unlocked copies above. That is a bug class found and fixed
site by site twice, because there is no single owner for the operation. The Codex settle loop
(`product_host.rs:163-178`) is a fix that only product runs get.

**Cost**

Every submission fix, whether the lock, the echo check, a provider-specific settle, or a new TUI quirk,
has to be found and repeated in up to 8 places across 6 crates. The trait shape (primitives rather
than one operation) invites each new engine to write a 9th copy. The copies that lag interleave with
broadcasts and human input, so a prompt can be merged into another or lost.

**Fix**

1. Move `wait_for_tui[_strict]`, `bracketed_paste`, `dispatched`, the paste-echo probe and
   `submit_prompt` out of `otto-review/src/session.rs` into `otto-sessions`, as
   `SessionManager::submit_prompt(&self, sid, prompt, SubmitOpts { echo_wait, extra_settle })`.
   otto-sessions already owns `submit_lock` and `submit_text`. otto-review then calls it. Keep a
   `pub use` in otto-review for one release.
2. Replace the host-trait primitives (`AutomationCtx::{wait_for_tui, dispatched, bracketed_paste,
   paste_to_enter}`, `SwarmHost::{bracketed_paste, wait_for_tui, dispatched}`) with one
   `submit_prompt` method. Rewrite scheduled tasks, goal loops, product (`extra_settle` for Codex),
   insights and the channel bridge as calls to it. Swarm keeps `send_run_input`'s run-active check by
   passing a guard closure.
3. Add a `check.sh` grep, like the keyed-lock guard, that fails on `bracketed_paste(` or `b"\r"` writes
   outside `otto-sessions`.

### architecture-4-02: major: three engines let an agent edit a design document, and they treat the same formats differently

**Location:** `crates/otto-canvas/src/assist.rs:458-505`, `:595-612`, `:694-915`, `:999-1034`, `:1194`; `crates/otto-canvas/src/assist_ctx.rs:29-39`; `crates/otto-design-assist/src/lib.rs:419-566`, `:581`, `:1041-1063`, `:1690-1733`; `crates/otto-design-assist/src/ctx.rs:33-49`; `crates/otto-server/src/mockup_assist.rs:183-193`, `:435`, `:453-503`; `crates/otto-server/src/modules/ctx_impls.rs:545-566`, `:653-683`; `crates/otto-product/src/design_format.rs:1-60`; `crates/otto-design/src/format.rs:40-101`

**Evidence**

"Write the document to a scratch file, run one agent turn, live-poll the file, fall back to a fenced
block in the reply, then wrap the result back into the document" is implemented three times. All
three are live routes.

| Piece | otto-canvas (`/canvas/.../assist`) | otto-design-assist (Design Hall) | otto-server `mockup_assist.rs` (product mockups) |
|-------|------------------------------------|----------------------------------|-------------------------------------------------|
| Per-document busy registry | `busy_scenes`/`SceneBusy` `:458-505` | `registry`/`BusyGuard` `:1008-1063` | `claim_mockup` (`:111`) |
| Live file poll | `spawn_file_poll` `:999` (no validation) | `spawn_live_poll` `:1690` (validates before pushing) | `spawn_file_poll` `:453` (validates scene3d only) |
| `extract_fenced` | `:1194` | `:581` | `:435` (also in `db_assist.rs:809` and `otto-swarm/src/recruiter.rs`) |
| Host `AgentTurn` type | `otto-canvas/src/assist_ctx.rs:29` | `otto-design-assist/src/ctx.rs:33`, the same fields plus `stuck_after`, `done_file` and `quiet_done` | calls `agent_session::run_session_turn` directly (`:183`) |
| Completion rule (host impl) | `run_session_turn(.., STUCK_IDLE)`, idle only (`ctx_impls.rs:545-566`) | `run_session_turn_with(.., TurnOpts { done_file, kill_on_stall: true })` (`:653-683`) | `STUCK_IDLE`, idle only (`mockup_assist.rs:193`) |
| Excalidraw handling | `split_excalidraw`/`merge_excalidraw` (`:694-915`) keep the images, freehand and bound elements the agent can't express, plus `files`/`appState` | the agent gets and must rewrite the whole JSON (`Adapter::agent_source`/`wrap`, `:517-551`) | the whole JSON |

The `otto-canvas` whiteboard model is spelled out twice:

- `design-assist`'s `canvas_base` (`:554`) carries the comment "mirrors `canvas_assist`", and copies
  `otto-canvas`'s `base_source` and `file_name` (`assist.rs:595-612`) and its
  `{type: "otto-canvas", format, source}` wrap (`build_doc` `:619`, `Adapter::wrap` `:527`).

Formats are also registered twice:

- `otto-product/src/design_format.rs` declares itself "The ONE design-artifact format enum" (four
  formats with mimes).
- `otto-design/src/format.rs:40-101` has a second spec table (12 formats) with the same mime strings.
- otto-design validates `scene3d`, one of its own formats, only through a host callback
  (`DesignCtx::validate_content`). The reason is that the model lives in
  `otto_product::design_scene3d`, which otto-server re-exports as `design_scene3d.rs`.

So the same whiteboard edited from the Canvas module keeps its images. Edited from Design Hall, it
depends on the agent echoing them back, and it completes on a done-marker instead of idle.

**Cost**

A fix to completion detection (Design Hall got the done-file and kill-on-stall), to element
preservation (Canvas got split/merge) or to live-preview validation lands in one engine only. Users
see different, sometimes lossy, behaviour for the same document type depending on which screen they
use. Each new format must be added to two registries and up to three adapters.

**Fix**

1. Make `otto-design-assist` the single engine.
   - Extract its turn core (busy guard, scratch-file write, live poll with validation, fence fallback,
     restore on failure) behind a small `AssistTarget` trait: `load_doc`, `agent_source`, `wrap`,
     `commit`, `emit_live`.
   - Implement the trait for Design Hall artifacts, canvas scenes and product mockups.
2. Move `split_excalidraw` and `merge_excalidraw` from otto-canvas into that engine's Excalidraw
   adapter, so every surface preserves images and bound elements.
3. Keep one `AgentTurn` type, the design-assist one with `done_file`, in the shared engine. Make
   `CanvasAssistCtx` and the mockup route use it.
4. Make `otto-design::format` the only format registry.
   - Move `design_scene3d`'s model and validator into otto-design, so `validate_content` loses its
     scene3d case.
   - Make `otto_product::DesignFormat` a thin view over `dformat::spec`.
5. Put `extract_fenced` in `otto-agent-run` and use it from all five copies.

### architecture-4-03: minor: workflow triggers are split across otto-workflows, a route file and an otto-server scheduler, and the fireable event kinds are listed three times

**Location:** `crates/otto-server/src/routes/workflows.rs:1616-1712` (`validate_trigger_spec`, `FIREABLE` `:1660`), `:1733` (`webhook_trigger`); `crates/otto-server/src/workflow_trigger_scheduler.rs:1-228`; `crates/otto-workflows/src/triggers.rs:140-158`; `crates/otto-workflows/src/chat.rs`; `ui/src/modules/workflows/triggerForm.ts:3-7`

**Evidence**

`otto-workflows` exists to hold the workflow engine pieces that have no server dependency. It owns the
event-trigger listener (`triggers.rs:190`, mapping `Event` → `event_kind` in `event_to_kind` `:140`)
and the chat trigger (`chat.rs`). The rest of the trigger subsystem lives in otto-server:

- **Validation:** `validate_trigger_spec` (`routes/workflows.rs:1616`) is in the HTTP route file. It
  covers kinds, timezone, result channel and webhook URL, and the per-kind rules.
- **Event-kind list:** the route keeps its own `FIREABLE` list (`:1660-1667`) of the six kinds that
  `event_to_kind` maps. The UI keeps a third list, `EVENT_KINDS` (`triggerForm.ts:3-7`). No test ties
  the three together. `rg insight_ready` finds the three lists and no cross-check.
- **Schedule and webhook kinds:** the `schedule` kind is fired by `workflow_trigger_scheduler.rs`,
  which is otto-server, `ServerCtx`-bound and calls `crate::workflow_engine::spawn_run`. The `webhook`
  kind is fired from a route handler (`routes/workflows.rs:1733`).

**Cost**

Adding a triggerable event means editing `event_to_kind`, the route's `FIREABLE` and the UI list, and
nothing fails if one is missed. A kind the listener maps but the validator rejects cannot be
configured. A kind the validator accepts but the listener drops never fires, silently. Trigger rules
can't be unit-tested without the server.

**Fix**

1. Move `validate_trigger_spec` into `otto-workflows/src/triggers.rs` as
   `validate_spec(kind, &Value) -> Result<()>`.
2. Derive the accepted kinds from one `pub const FIREABLE_EVENT_KINDS` that `event_to_kind` is tested
   against, with a test asserting that every `Event` variant mapped by `event_to_kind` is in the list.
3. Expose the list on the existing `/workflows/node-types` catalog (or a sibling route), so
   `triggerForm.ts` reads it instead of hard-coding it.
4. Move the schedule tick behind `WorkflowCtx`, mirroring `spawn_event_trigger_listener`.

### architecture-4-04: minor: the per-function ratchet guards three names in otto-server only, and the `execute_node` split moved an 872-line arm into an unguarded function

**Location:** `scripts/loc-ratchet.py:41` (`FUNCTIONS_ROOT = "crates/otto-server/src"`), `scripts/loc-baseline.json` (`functions`); `crates/otto-server/src/workflow_engine/nodes/review_run.rs:7`; `crates/otto-sessions/src/ws.rs:1641`

**Evidence**

Iteration 2's function ratchet (the fix for 2-07) caps only `execute_node` (206), `run_review_core`
(406) and `run_workflow_with_budget` (1,250). It searches only `crates/otto-server/src`. Running the
ratchet's own `function_lengths` over every production file shows the largest functions it does not
see:

- `workflow_engine/nodes/review_run.rs:7` `execute`: **872 lines**. It was "moved verbatim out of
  `execute_node`'s match" (`:1-2`). So the split that brought `execute_node` from about 2,830 lines to
  206 left most of the body in one new, uncapped function. It mixes mode resolution, five-way target
  resolution, launch, polling and scoring.
- `otto-sessions/src/ws.rs:1641` `serve_terminal`: **584 lines**. It is one `tokio::select!` loop with
  12 arms and 11 `ClientFrame` handlers, and the file had 17 commits in 3 weeks. It is outside
  `FUNCTIONS_ROOT`, so no cap can apply.
- Others outside the guard: `run_docs` (579, `vault_docs_agent.rs`), `run_analysis` (580,
  `otto-product/src/run.rs`), `governed_invoke` (557) and `controller_inner` (472, `goal_loop.rs`).

**Cost**

These are the hottest god functions, and none of them can be stopped from growing. The ratchet's
promise ("the remaining hot spots can no longer grow unnoticed") holds only for the three names that
were already split or recorded.

**Fix**

1. Change `measure_functions` to scan every `crates/**/src` production file.
2. Add a `function_max` ceiling, for example 400. Every function over it must be listed in
   `functions` (keyed `path::name`, so the name collisions `function_lengths` already merges stay
   distinct), and can only shrink.
3. Seed the baseline with today's spans: `review_run::execute` 872, `serve_terminal` 584, `run_docs`
   579 and so on, plus the generated tables `route_for` and `policy_for`.
4. Then split `review_run::execute` along its own comment headings (`resolve_targets`,
   `launch_reviews`, `await_and_score`). For `serve_terminal`, move the `ClientFrame` match into a
   `TerminalConn` struct with one method per frame.

### architecture-4-05: nit: otto-mcp keeps a fourth read-only SQL classifier after the 2-02 consolidation

**Location:** `crates/otto-mcp/src/outward/exec.rs:27-38`, `:324`; `crates/otto-dbviewer/src/sql_class.rs`

**Evidence**

Finding 2-02 consolidated statement classification into `otto_dbviewer::sql_class::ReadClass::of`.
otto-mcp does not depend on otto-dbviewer. `otto.query_db_readonly` gates on its own prefix check:
`trim`, reject on any `;`, then `starts_with` on SELECT, SHOW, DESCRIBE, DESC, EXPLAIN or WITH. The
two classifiers disagree:

- `-- note\nSELECT 1` is refused by MCP but read-classified by `sql_class`, which skips trivia.
- `SELECT ';'` is refused by MCP because of the `;` inside a literal.

The route it forwards to (`/db/mcp-query`) re-classifies and runs read-only, so this is not a safety
gap. It is a second rule set that an agent sees first.

**Cost**

An agent gets "only permits a single read-only statement" for queries the database explorer accepts as
reads. Each classifier fix has to be made twice.

**Fix**

Drop the prefix check and let `/db/mcp-query`'s `ReadClass` verdict be the only gate. It already
returns a `Forbidden` the governed call surfaces. If a pre-check is wanted for the error text, move
`sql_class.rs` (pure, no driver dependencies) into `otto-core` and call it from both crates.

---

## Carried-over findings (still open; full text in `../iter-1/architecture.md`, `../iter-2/architecture.md` and `../iter-3/architecture.md`)

| ID | Severity | Location | Remaining work |
|----|----------|----------|----------------|
| architecture-1-04 | major | `crates/otto-server/src/skill_review.rs:400`, `vault_docs_agent.rs:356`, `modules/review/summarize.rs` | Add a shared fan-out and summarizer runner in `otto-agent-run`. |
| architecture-1-09 | major | `ui/src/modules/git/GraphView.svelte:1` | 4,877 lines. Extract RefSidebar and the git action handlers. |
| architecture-1-10 | major | `ui/src/lib/stores/database.svelte.ts:1` | 4,423 lines. Split per concern now that the harness resolves relative imports. |
| architecture-1-01 (residual) | minor | `crates/otto-server/src/workflow_engine.rs:1850` | `run_workflow_with_budget` is 1,250 lines. Introduce `RunState`, then split admission and scheduling. |
| architecture-1-05 | minor | `crates/ottod/src/mcp_tools/tests/catalog_tests.rs:16` | 7 drifted native/governed pairs. `route_for` (1,620) and `run_tool` (1,235) map the same tools twice. |
| architecture-1-07 (residual) | minor | `crates/otto-server/src/routes/api_client.rs:1808-2100`, `workflow_api_request.rs:50`, `modules/browser_proxy.rs:117` | Move `execute`, `execute_service` and `resolve_environment` behind an `ApiClientCtx` in `otto-apiclient::rest`. Call `otto_netguard` directly instead of `routes::api_client::net_guard`. |
| architecture-1-08 (residual) | minor | `ui/src/modules/workflows/WorkflowsPage.svelte:208-470`, `:1003`, `:1445` | Move live run sync, the final-output panel and approvals into a `RunMonitor.svelte` beside `RunSteps.svelte`. |
| architecture-1-13 | minor | `ui/scripts/ui-guards-baseline.json:434-451` | 33 `lib` → `modules` imports. Start with `database.svelte.ts` (8) and `k8s.svelte.ts` (5). |
| architecture-1-16 | minor | `crates/ottod/src/usage_tailer.rs:1148` | Move the tailer driver into otto-usage, and keep one Codex model extractor. |
| architecture-2-06 | minor | `crates/otto-server/src/rooms/`, `game_rooms/` | Move them to an `otto-rooms` crate after the games team merges. |
| architecture-2-08 | minor | `crates/otto-server/src/game_rooms/types.rs:11` | Keep one game catalog JSON (depends on 2-06). |
| architecture-3-03 | minor | `crates/otto-server/src/routes/usage.rs:465-502`, `routes/transcript.rs` | Add a `BudgetGate` service and a `transcript/resolve.rs` module, and delete the `*_pub` shims. |
| architecture-2-01 (residual) | nit | `ui/src/lib/components/Terminal.svelte:1108`, `:1324` | Move the resize half into `TermGridSync` once the resize-compact queue moves with it. |
| architecture-3-02 (residual) | nit | `crates/otto-issues/src/jira.rs` (21 `is_success()` sites), `confluence.rs` (5) | Route the remaining calls through `AtlassianTransport::send(req, what)`. |
