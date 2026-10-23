# Quality iteration 6, wave 2: fixes

This file records the outcome of each package in [PLAN.md](./PLAN.md): what was fixed, what was
rejected and why, and what was not done (each with a hand-off). Eight fixers worked in parallel on
disjoint paths of the branch `fix/quality-iter6-20261023`. A single verifier then reviewed the
combined diff, applied the cross-package hand-offs, ran `scripts/check.sh --base origin/main`, and
fixed the integration problems listed at the end.

Each fixer had a budget of four locked cargo commands and was not allowed to run `npm run check`,
Playwright, or clippy on crates other packages were editing. The verifier's gate is therefore the
first run of clippy on every changed crate, of `npm run check`, of the full unit suite and of the
otto-server `it` suites the new tests live in.

## P1: Fan-out runner adoption, review-gate tests

### Fixed

- **test-quality-6w1-04:** `blended_score(review, goals)` extracted in
  `workflow_engine/nodes/review_run.rs`; `score_target` calls it. Tests
  `the_goals_grade_is_averaged_into_the_review_score` and
  `scoring_params_override_the_weights_and_bad_values_keep_the_defaults` (module `review_gate`).
- **architecture-1-04, vault docs (fully ported):** new generic `otto_agent_run::fanout::retry`
  (`RetryQueue`, `SlotControl`, `attempts`, `run_slots`, `summarize_with_retries`; `SlotState`
  gained `Retrying` and `Cancelled`). The vault docs writer pass runs on `run_slots`, the
  summarizer on `summarize_with_retries`, the reviewer and revision loops on `attempts`. The four
  hand-written turn loops, `SUM_RETRY_IDX`, `close_retry` and `wait_for_retry_flag` are deleted;
  `retry_target` reuses `terminate_session`. `vault_docs_agent.rs` is split into
  `vault_docs_agent/{authors,run,review_loop,retry,refine,prompts}.rs` (4,093 → 1,379 lines;
  `run_docs` 579 → about 338).
- **architecture-1-04, code review:** `summarize_and_persist` (467 → 148 lines) is an adapter over
  `otto_agent_run::fanout::summarize`. The managed turn (`SummarizerTurn`) fails on a provider error
  and on an empty reply when reviewers had findings, so both take the deterministic fallback.
  Review retry/stop paths map agent rows onto `fanout::Slot` via `retry::row_slot`.
- **Tests:** 5 in `otto-agent-run/src/fanout/retry_tests.rs`; 2 in
  `otto-server/tests/unit/vault_docs_agent.rs` (one writer retry reruns only that writer; a failed
  writer leaves the summarizer running and the failure recorded); `row_slot` mapping test.

### Not done

- **architecture-1-04, product part:** `otto-product/src/run.rs`'s lens fan-out and summarizer are
  not on `fanout::run` / `fanout::summarize` yet. Next step: the lens futures as reviewers,
  `build_summarizer_prompt` + the summarizer session as `summarizer`, the existing merge as
  `fallback`.
- **architecture-1-07r (not started):** the REST engine (`execute_service`,
  `resolve_environment`, `run_step`, `prepare_stream`, …) is still in `routes/api_client.rs`; no
  `ApiClientCtx` yet. A half-done crate move would not compile.

## P2: One assist turn engine over otto-agent-run; resume authority in session_turn

### Fixed

- **architecture-5-02r:** `otto_agent_run::session_turn::resume` (`ResumeAuthority` for
  `SessionManager` and `SessionsRepo`, `require_owned_resume`) and `session_turn::run_agent_turn`,
  the one assist turn (ownership check first, then the chat or done-file protocol).
  `CanvasAssistCtx` / `DesignAssistCtx` expose `sessions()`; the four run-turn forwarders and the
  `kill_session` forwarders are deleted. The cases of `tests/unit/assist_resume_ownership.rs` moved
  to `session_turn/resume.rs` and run against both authorities; route test
  `canvas_refs_api::canvas_assist_refuses_to_resume_another_users_session`.
- **architecture-4-02 (major):** `otto_agent_run::assist` (`AssistTarget`, `TurnRunner`,
  `AssistSlots` / `AssistClaim`, `run_assist_job`): claim check, load, the turn with the stat-gated
  live poll inside the job future, the turn cap, stop (a stopped job never parses or applies),
  parse, apply once, state reports. Canvas (`SceneTurn`), Design Hall (`DesignTurn`, now in
  `otto-design-assist/src/job.rs`) and mockups (`MockupTurn`) are thin adapters. Behaviour notes: a
  canvas Stop now always restores the pre-turn document, and the live poll stops the moment Stop
  fires. 8 assist tests.
- **architecture-6w1-01 (session_turn part):** `session_turn::transcript_path` resolves through
  `RootsEnv::for_session(..).claude_session_file`; the `subagent_dir` fallback and
  `agent_run::progress_idle` derive their directory from the transcript. Test
  `transcript_path_follows_the_injected_transcript_roots`.

### Not done

- None beyond the hand-offs applied by the verifier (AGENTS.md crate rows).

## P3: Async run guard, swarm submit conversion, per-session transcript paths, rooms UI

### Fixed

- **Step 0:** `otto_transcript::roots::{encode_cwd, default_claude_root}` and
  `TranscriptRoots::claude_session_file`, with tests.
- **architecture-6w1-01:** `persist_transcript_path` takes the session's `TranscriptRoots`
  (`SessionManager::transcript_roots_for` / `transcript_roots_in`); `locate_activity_artifact`,
  `lifecycle`, `claude_pty::{project_dir, session_jsonl_path}` and `otto-improve`'s `pathsafe` use
  the shared roots and encoder. Test `persisted_transcript_path_stays_inside_an_injected_root`.
- **test-quality-6w1-05:** `TurnSession { reuse_user, creator }` built once per turn; tests
  `a_turn_session_is_owned_by_the_acting_user_on_both_paths` and
  `swarm_identity::a_turns_session_is_reused_and_created_as_its_acting_user`.
- **architecture-4-01r:** `RunGuard` is async; swarm turns submit through
  `engine::submit_turn_prompt` → `submit_prompt_while(.., VERIFIED, guard)`. `bracketed_paste` and
  `wait_for_tui` are gone from `SwarmHost`. Tests `ended_run_guard_blocks_the_paste`,
  `a_run_stopped_between_the_wait_and_the_paste_pastes_nothing`,
  `run_guard_ends_on_a_stop_status_or_the_cancel_flag`.
- **ux-design-6w1-01:** RecapPanel / StartRoomModal failure copy and a single `LoadState` with the
  stale bar; `ui/unit/w2p3-recap.test.ts` (6).
- **accessibility-responsive-6w1-01 (rooms):** 17 → 0 inserted live regions under
  `modules/rooms`; e2e specs `room-recap` and `desktop-room-window` updated.

### Not done

- **architecture-2-06 (otto-rooms crate):** not started (a ~15-module move behind a new
  `RoomsCtx`; the plan is unchanged).
- Hand-offs left open: `swarm_router.rs`'s interleave test still calls the `#[doc(hidden)]`
  `engine::submit_run_prompt`; once it is ported, delete `submit_run_prompt` / `send_run_input` /
  `guarded_dispatch`. `otto-sessions/src/nested.rs` still joins `.claude/projects` by hand,
  `lib.rs` still re-exports `lifecycle::transcript_path`, and `otto-context/src/materialize.rs`
  `enc_project` is a fourth cwd encoder (should call `encode_cwd`).

## P4: MCP governance stage runner, public entry gate and database-change authority tests, MCP UI

### Fixed

- **architecture-5-03:** `otto_mcp::governance::run` with a fixed stage order and two named stage
  sets (`REGISTERED_SERVERS`, `OTTO_TOOLS`). `McpService::invoke` (383 → 219 lines; stage bodies in
  `service_stages.rs`) and `governed_invoke` (557 → 395; `OttoStages`) both call it. Table test
  `governance::tests::the_stage_table_for_both_entry_points` (15 cases).
- **test-quality-6w1-01 / 6w1-02:** `public_entry_gates.rs`:
  `ingest_routes_write_only_with_the_sessions_own_token` (every `/ingest/*` route: no, empty and
  another session's token write nothing; the own token writes exactly one of each) and
  `plugin_host_routes_need_an_enabled_plugins_bearer_token`.
- **test-quality-6w1-03:** `database_changes_api.rs` covers peer cancel of an awaiting-review
  change, peer and non-executing reviewer cancel of a running change, and peer reconcile of an
  outcome-unknown attempt (all 403, state unchanged).
- **ux-design-6w1-04:** `mcp/sessionAttach.ts` (`attachView`); the attach checkbox renders only
  once the daemon reported the value. `ui/unit/w2p4-ottoServerHome.test.ts` (5).
- **accessibility-responsive-6w1-01 (mcp):** 7 → 0 inserted live regions; copy toasts use
  `COPY_FAILED_BODY` where the body must not carry a secret.

### Not done

- **architecture-1-05:** routed twins / one generated session catalog / parity test in
  `ottod/src/mcp_tools.rs` not started.
- A DB-backed characterization table over the real `McpService::invoke` (needs a fake MCP server).

## P5: GraphView split, list-window residue, home/assistant lib moves, design hall and 3D sweeps

### Fixed

- **architecture-6-05r:** `listRange` gained an optional extra band; `kubernetes/monitor/tableWindow`
  maps over it; the unused `git/list-range.ts` and `git/list-window.svelte.ts` are deleted.
  `ui/unit/w2p5-tableWindow.test.ts` pins the expanded-row bug.
- **architecture-1-13 (assistant + home):** `lib/assistant/model.ts`, `lib/home/kinds.ts`, and a
  `homePort` (`createPagePort`); `lib/uiCommands/home.ts` no longer imports `modules/`.
  `ui/unit/w2p5-homePort.test.ts`.
- **test-quality-6w1-06 (p06 + p10):** source regexes replaced by behaviour tests through
  `componentFunctions`.
- **ux-design-6w1-02 / 6w1-03 (design hall):** `copyWithToast` from `lib/clipboard`; failure copy
  reads "Couldn’t …".
- **accessibility-responsive-6w1-01:** 40 → 0 inserted live regions in git, panels,
  mission-control, personal-agents, assistant, home, design-hall, `lib/design`, `lib/three`.
- **architecture-1-09 (partial):** `git/graph/graphActions.svelte.ts` and `git/graph/graphMenus.ts`
  extracted; GraphView 4,879 → 4,082 lines. `ui/unit/w2p5-graphActions.test.ts` (8).

### Not done

- **architecture-1-09:** GraphView is not yet ≤ 3,000 lines. Next: `RefSidebar.svelte` (the aside,
  its CSS and sidebar-only script), then the ref popover and the commit-detail pane.

## P6: DatabaseStore split; data-tool module sweeps

### Fixed

- **architecture-1-10 (major):** `DatabaseStore` is a façade (3,210 → 1,003 lines) over
  `lib/stores/database/{connections,schemaTree,objectOps,queryRuns,workbenchPersist}.svelte.ts`,
  each with a narrow `Pick<DatabaseStore, …>` host contract and one shared `ConnEpochs` token.
  `ui/unit/w2p6-dbStoreSplit.test.ts` (3).
- **ux-design-6w1-03 (AWS, browser):** "Couldn’t …" copy; `RegionErrors` announces at the edge.
- **Clipboard:** `results-format.ts` uses the shared `COPY_FAILED_BODY` (menu copies stay silent on
  success by design). `ui/unit/w2p6-resultsCopy.test.ts` (2).
- **accessibility-responsive-6w1-01:** 58 → 0 inserted live regions in database, brokers,
  connections, aws, kubernetes, skills-eval, skills-lab, insights, proof, browser and workbench;
  `aws/S3TransferBars.svelte` extracted; `desktop-network-profiles.spec.ts` updated.

### Rejected

- `RetentionDialog:56` and `LearnedPage:458` belong to P7 / P5 (handled there).
  `AccountWizard` `.test` and `LiveEngineSetup` progress card: always-mounted regions whose text
  changes, which the rule allows.

## P7: apiClient store split, Workflows RunMonitor, Product JiraIssueSection + TestCasesTab

### Fixed

- **architecture-6w1-02:** `api/rowFocus.ts` → `lib/rowFocus.ts`, no shim.
- **architecture-1-13 (swarm):** `swarm/runStats.ts` → `lib/swarm/runStats.ts`;
  `lib/stores/swarm.svelte.ts` has no `modules/` import.
- **ux-design-6w1-03:** SnipEditor and SourceSearch failure copy.
- **accessibility-responsive-6w1-02:** test-case reorder moved to `product/caseOrder.svelte.ts`
  (Move buttons stay enabled; a save in flight queues the final order; the pressed button keeps
  focus).
- **test-quality-6w1-06 (p09):** behaviour tests for the Mermaid race, the debounce, the reorder
  queue and focus.
- **architecture-5-04:** `product/JiraIssueSection.svelte` + `product/jiraIssue.svelte.ts`;
  OverviewTab 2,494 → 1,772 lines. `ui/unit/w2p7-jiraIssue.test.ts` (3).
- **architecture-1-08r:** WorkflowsPage 3,710 → 2,688 lines: `RunMonitor.svelte`, the run state
  module (renamed `runSync.svelte.ts` by the verifier, see below), `RunApprovals`,
  `RunContextSidebar` (shared `<Tabs>`), `RunFinalOutput`, `RunInputPanel` + `runInputForm`.
  `ui/unit/w2p7-runMonitor.test.ts` (13, incl. the 8 ported publication-preview cases).
- **accessibility-responsive-6w1-01:** 29 → 0 inserted live regions in api, workflows, product,
  loops, scheduled-tasks, swarm, canvas and snip; three e2e specs updated.

### Not done

- **architecture-6-03:** the `apiClient.svelte.ts` split (2,113 lines) was not started; next step
  follows `lib/stores/apiClient/automations.svelte.ts` (collectionsSync, tabs, cookies, curl).
- The rest of the rule-4 sweep (focus trapping, logical CSS, 390 px) beyond what ui-guards reports.

## P8: LoadState announce, copyWithToast, UI guard rules, shell and small-module sweeps

### Fixed

- **Step 0:** `copyWithToast(text, what)` and `COPY_FAILED_BODY` in `lib/clipboard.ts`.
- **accessibility-responsive-6w1-01:** `LoadState` announces a stale refresh once per distinct
  error (`announceEdge`); the `announceShown` action; every inserted `role=status` / `aria-live`
  site in the shell, agents, help, run-with-otto, share, usage, vault and settings.
- **ux-design-6w1-02:** copy failures name a next step everywhere in the shell, settings and
  agents; `Markdown.svelte` and `Terminal.svelte` copy no longer fail silently.
- **architecture-1-13 (vault):** `lib/uiCommands/vaultPort.ts`; `rg "modules/" ui/src/lib` is
  empty.
- **Guard rules:** `lib-imports-modules` is now hard (max 0); new ratchets `cross-module-import`,
  `guessed-checked`, `clipboard-no-next-step`, `unit-source-regex`; `icon-button-name` logic
  extracted into a tested detector. `ui/unit/p04-uiGuards.test.ts` (11).
- Docs: review checklist, accessibility guideline, agent UI control.

### Not done

- 57 inserted `role="alert"` lines are kept on purpose (WebKit speaks an inserted alert; about 40
  e2e specs locate them by role). The policy is written in `accessibility.md`.

## Verifier: integration fixes

- **Rust compile/lint:** `public_entry_gates.rs` built its count query with `format!`, which sqlx
  0.9 refuses (E0277) and which kept the whole `it` binary from compiling; it now passes fixed
  statements. Clippy fixes: `manual_async_fn` in `assist/mod.rs` and `assist/tests.rs`,
  `type_complexity` in the governance table test (a `Case` alias), a needless borrow in
  `review/summarize.rs`.
- **LOC ratchet:** `otto-sessions/src/manager.rs` grew 21 lines past its cap; the transcript-root
  helpers (`persist_transcript_path`, `transcript_roots_for` / `_in`) moved to
  `manager/transcript_roots.rs` (now 6,499). Baseline lowered: `vault_docs_agent.rs` dropped (under
  `new_file_max`), the `run_docs`, `summarize_and_persist` and `governed_invoke` function entries
  dropped (all under 400 now).
- **LOC ratchet, raised:** the otto-server `src` total is raised 108,217 → 108,541 (+324). The
  wave moved the assist job and the fan-out/retry machinery into `otto-agent-run`, but the vault
  docs split (+37 net with module headers), `review/summarize.rs` (+204: the 467-line monolith
  became named functions) and `mcp_outward.rs` (+93: the `OttoStages` impl) add more than the
  deleted forwarders save. The payback is the not-done REST-engine move into `otto-apiclient`
  (architecture-1-07r), which removes well over 324 lines.
- **UI type check:** `workflows/runMonitor.svelte.ts` and `workflows/RunMonitor.svelte` differed
  only in case, so on macOS `./runMonitor.svelte` resolved to the component (svelte-check errors,
  and a Vite resolution hazard). The state module is renamed `runSync.svelte.ts` (imports, the
  unit test and the ui-guards key follow). Also fixed: `JiraIssueSection`'s dev-status Retry
  called the pre-move `devLoaded` / `loadDevStatus` (now `issue.devLoaded = false;
  issue.loadDev()`); `runInputForm`'s `placeholder` read `this.kinds` before initialization (now
  `$derived.by`); `RunDetail`'s `run` was narrowed to `never` (now `$state<SkillEval | null>`); a
  null-narrowed assertion in `w2p4-ottoServerHome.test.ts`.
- **Hand-offs applied:** `settings/AssistantSettings.svelte` import; `assistantModel`,
  `assistantThreadPaging`, `platformRecovery`, `recoveryOwnership`, `swarmRunStats` paths;
  `databaseWriteOwnership`, `databaseDashboardsScope`, `dbRestoreWarm` follow the store split;
  `apiHistoryBanner` asserts the `announceShown` banner; `workflowInspector`'s final-output case
  and `workflowPublicationPreview.test.ts` removed (ported to `w2p7-runMonitor`);
  `k8sMonitorTableWindow.test.ts` removed (covered by `w2p5-tableWindow`); the two moved
  branch-delete cases removed from `gitUiSafety` / `gitUiSafety3` (covered by `w2p5-graphActions`);
  `desktop-ux-r3-automation.spec.ts` locates the agent save error by `.err`;
  `PublishModal`'s copy-failure toast names a next step; AGENTS.md crate rows (otto-canvas,
  otto-design-assist, otto-agent-run); `docs/features/workflows.md` and `product.md` list the new
  files.
- **Baselines:** `ui/scripts/ui-guards-baseline.json` lowered with `--update-baseline` (155
  file/rule counts went down; nothing raised). `UI_LIB_SHIM_MAX` stays 3 (the deleted git files
  were not re-export shims).
- **Bundle budget (raised, deliberate):** `page:home` 25,797 → 27,220 B gzip (the `homePort` /
  `pagePort` binding that replaced the lib → module import) and `page:workflows` 254,638 →
  264,735 B (the RunMonitor split's component boilerplate plus the shared `<Tabs>` the run
  context sidebar now uses). Only these two targets were re-recorded; every other budget is
  unchanged.
