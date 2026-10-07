# Round 1 — test coverage and quality

**Verdict: Block. Counts: blocker 1 · major 8 · minor 0 · nit 0.**

**Score: 7.2/10, provisional for the inspected sample.** Baseline `196048df`, October 7, 2026. This is a source-based falsification review, not measured line coverage or an app-wide certification. This reviewer executed no tests, builds, browser sessions, mutation runs or production requests: the coordinator retained the heavy execution slot. The coordinator subsequently ran the existing telemetry browser test and supplied fresh baseline failure evidence, recorded under T4. “Confirmed” otherwise means the production/test trace was inspected and the named mutation cannot be detected by those assertions; it does not mean an executable mutant was run.

Read AGENTS.md, the test-review skill and references, the correctness/design/performance/UX round-1 reports, the implementation plan and FIX-PLAN. Reopened production paths rather than accepting prior findings as proof. Searched sibling tests before declaring gaps. The historical correctness/performance reports name `922ae483`; locations here refer to current `196048df` source.

| Dimension | /2 | Evidence and deduction |
|---|---:|---|
| Assertions against observable effects | 1.65 | Strong persisted checkpoint, token revocation and write-ownership assertions; report naming pins the current collision-prone representation, and some auth tests repeat handler wiring. |
| Error, edge and recovery coverage | 1.35 | Meaningful cancellation/save races exist; restart after task edit, collision deletion, School overflow and plugin refresh races are missing. |
| Real component and contract boundaries | 1.30 | Real telemetry end-to-end coverage exists, but blocking tests separate producer/consumer; School native controls and plugin modal stack are not exercised. |
| Determinism and fixture isolation | 1.65 | Deferred gates, temporary databases, controlled callbacks and isolated daemon ownership are established. School browser flows use shared serial state and long animation polling; no fresh flake evidence was collected, so no flakiness finding is asserted. |
| Execution and regression enforcement | 1.25 | Broad test inventory and a blocking smoke gate exist. Telemetry's meaningful full-chain test is advisory and now fails on the fresh baseline; other sampled paths have no fresh execution evidence in this report. |
| **Total** | **7.20** | Known material holes and unexecuted critical suites prevent a defensible 9.8 claim. |

## Ranked findings

### T1 [blocker] No test protects selected-only Redis list deletion

**Code:** `ui/src/modules/database/edit-redis.ts:350`, `:353`. **Tests inspected:** `ui/unit/dbRedisLineSafety.test.ts:35` onward; `ui/e2e/db-sweep-redis.spec.ts:224`, `:256`; sibling edit tests searched.

**Mutation that survives:** leave/reintroduce the fixed tombstone implementation, or replace list deletion with an operation that removes every matching marker. The unit suite tests SREM/HDEL/ZREM line safety and hash updates, never list deletion. Redis browser coverage round-trips strings and hash editing, not selected list indices. This destructive branch therefore has no result oracle for preservation of unselected elements.

**Why it matters:** deleting index 1 from `['__otto_deleted__', 'remove', 'keep']` must preserve the first and third values. The baseline deletes the first value too. Checking for LSET/LREM command names would certify the unsafe algorithm.

**Evidence:** confirmed source/test trace; corresponds to C1. **Fix:** if list deletion is refused, assert the real adapter returns no executable mutation and the UI cannot apply one, including a partial LRANGE view. If supported atomically, run generated commands against an isolated Redis fixture and assert the entire resulting list for markers before/after selection, duplicate values and multiple indices. Retain a safe list-edit regression so refusal does not disable unrelated editing.

### T2 [major] Generation tests bypass recovered completion

**Code:** `crates/otto-automation/src/scheduled_tasks_engine.rs:1177`, `:1209`. **Test:** same file `:1707` (`review4_old_settlement_does_not_consume_retimed_once`).

**Mutation that survives:** recovered completion substitutes the current task for the admitted task. The test retains `dispatched` in memory and calls `settle_schedule` directly; it never invokes `resume_workflow_handoff`. Searching the workspace found no test invocation of that recovery entry point. The existing test correctly protects ordinary in-memory settlement, but cannot protect restart semantics or delivery destination.

**Why it matters:** retime a once task or change destination A→B while a workflow runs; after restart, the old completion can consume the new occurrence or deliver to B.

**Evidence:** confirmed trace; C2. **Fix:** persist admission, edit generation/destination, discard in-memory state, recover using a new context over the same isolated DB, complete the workflow and assert (1) new occurrence remains due, (2) only A receives delivery, (3) exactly one terminal run/report exists. Use an injected delivery sink; no real external delivery. Include missing legacy snapshot and malformed snapshot cases with explicit no-unproven-delivery/no-new-generation-settlement assertions.

### T3 [major] Report test locks in a filename without asserting per-run ownership

**Code:** `crates/otto-automation/src/scheduled_tasks_engine.rs:145`, `:332`, `:415`, `:1512`. **Test:** same file `:1816`.

**Mutation that survives:** omit run identity and truncate the timestamp to seconds. The test calls `report_rel` once and asserts exactly the current task/timestamp filename; it actively accepts the collision-prone representation. No second run, file contents or retained report is checked there.

**Why it matters:** two completed executions in one second overwrite history, and pruning one may remove the other's report (C3).

**Evidence:** confirmed trace. **Fix:** at one fixed instant create two different run IDs for one task, assert different paths, write different reports via the real helper, read each exact body back, then prune only one and verify the retained run's report still exists. Cover both success and failed-run report production; keep historical stored paths readable.

### T4 [major] The blocking telemetry checks allow producer/ingest disagreement

**Code:** `ui/src/lib/api/client.ts:336`; `ui/src/lib/telemetry.ts:49`; `crates/otto-server/src/routes/telemetry.rs:137`. **Tests:** `ui/unit/telemetry.test.ts:238`; Rust validator test at `routes/telemetry.rs:219`; gate inventory `ui/e2e/gate-specs.ts:22`; advisory job `.github/workflows/ci.yml:395`.

**Mutation that survives blocking tests:** keep the producer's `http.client.get.repos` naming while restricting ingress to exact `http.client`. UI tests assert the renamed value against a sender stub; Rust tests validate hand-built navigation and invalid names, not actual producer output. Each side independently passes its own expectation while ordinary mixed batches fail.

**Important correction:** `ui/e2e/desktop-telemetry.spec.ts:79` already drives browser → daemon → collector → ClickHouse. Lines 102–110 require a navigation/render/client/server parent chain. It should detect this mismatch; it is **not missing** and should not be described as weak or green without execution. It is absent from the blocking gate, and the full functional job has `continue-on-error: true`. This review does not claim the entire existing suite would pass the mutation.

**Fresh coordinator execution:** the coordinator ran the existing `actual browser` case against the freshly built `196048df` daemon with one worker. The log records **1 executed, 1 failed** at line 110 after the 30-second chain poll returned false. This reviewer read `/tmp/otto-quality-20261007-telemetry-baseline.log`; the coordinator additionally inspected the trace and reported a `/telemetry/ingest` HTTP 400. Trace/screenshot artifacts are under `/tmp/otto-quality-20261007-e2e-baseline`. This confirms the existing integration assertion catches the baseline failure; it does not replace the missing fast enforced contract guard.

**Evidence:** confirmed split-oracle and gate trace; C4. **Fix:** execute the existing end-to-end spec for fresh red/green evidence. Add a cheap enforced cross-language contract check that takes a batch produced by the real HTTP wrapper/runtime, including `x-otto-route`, and validates/posts it through actual ingress. Assert full mixed-batch accepted count, exact names, parent IDs and no raw identifiers. Keep malformed name/component/URL rejection and atomic no-partial-ingestion assertions. A copied valid fixture without a test comparing it to current producer output can drift again.

### T5 [major] School tests do not cover native control keys, complete fallback membership or restored state

**Code:** `ui/src/modules/home/boxes/ClassroomsBox.svelte:175`, `:213`, `:403`, `:697`, `:716`; `ui/src/modules/home/school/model.ts:398`. **Tests:** `ui/e2e/desktop-home-classrooms-3d.spec.ts:180`, `:235`; `desktop-home-classrooms.spec.ts:90`, `:111`; `ui/unit/school-life.test.ts:111`.

**Mutations that survive:** intercept Enter from every descendant button; retain tabbable controls in the clipped companion; drop all list members beyond geometry capacity; erase the saved room before asynchronous scene mount. Existing 3D tests click card actions and focus the bare stage for Enter/Escape. List tests explicitly switch into visible List with two students. Each test's new browser context seeds no saved room; no remount with delayed assets is asserted. Model tests exercise geometry/life rather than complete accessible membership.

**Why it matters:** pointer success does not establish keyboard behavior; a summary can announce an overflow session that has no actionable row; saved navigation can disappear (D1/D2, UX-03/04).

**Evidence:** confirmed trace. **Fix:** focus real card buttons in room and screen modes and press Enter/Space, asserting navigation/archive/confirmation effects; separately retain bare-stage shortcuts. Tab through the widget in 3D, assert focused controls have visible, unclipped context, and reach every action. Seed 37 foreground plus seven background sessions, including overflow needs-input members, and assert all IDs are actionable through List/fallback while rendered geometry remains bounded. Remount with a saved non-default room and delayed scene loading, then assert restoration and intentional corridor persistence. Model-only or source-regex tests cannot prove the DOM/native-key integration.

### T6 [major] Environment save-race coverage does not cover leaving the editor

**Code:** `ui/src/modules/api/ApiPage.svelte:72`; `EnvironmentsView.svelte:148`. **Tests:** `ui/e2e/desktop-review6-api-environment.spec.ts:6`; `ui/unit/dataToolsRecovery.test.ts:73`; `ui/unit/apiClientOwnership.test.ts:443`.

**Mutation that survives:** unmount a dirty environment on request-tab/module navigation, or reseed it during New environment without a leave decision. The environment browser test is strong for delayed Save and A→B→A selection, including persisted values and secret renames, but never leaves this editor. Navigation tests cover AutomationEditor; store ownership tests cannot observe destroyed component-local rows.

**Why it matters:** correctly handling a pending save does not preserve unsaved variables on ordinary navigation (UX-01).

**Evidence:** confirmed trace. **Fix:** parameterize request-tab, environment-tab close, module/workspace exit and New environment. Assert Keep retains exact rows and selection, failed Save retains rows, successful Save must finish before navigation, and Discard navigates. Cover newer edits while Save waits and secret rename metadata; assert secret plaintext is absent from browser persistence.

### T7 [major] Plugin UI tests omit refresh ownership, nested overlays and stale polling

**Code:** `examples/plugins/team-performance/ui/views/app.js:418`, `:433`, `:541`; `ui/views/settings.js:247`; `ui/components.js:284`. **Tests:** `test/browser.e2e.test.js:259`, `:314`; `test/ui-views.test.js:84` (all paths within the plugin).

**Mutations that survive:** rebuild the settings DOM when a scan finishes or Save returns, close every document modal on one Escape, or silently ignore status poll failures indefinitely. The browser test saves time off and checks backend persistence with no later concurrent edit. Its Escape cases are toolbar popovers, not nested report/download dialogs. The string-view harness does not load app.js/settings.js and stubs document listeners, so it cannot exercise these transitions.

**Why it matters:** drafts/comments disappear, and a disconnected scan continues to look current (D3; UX-02/05). Existing section Retry/string tests remain useful for their narrower scope.

**Evidence:** confirmed trace. **Fix:** delay scan completion and Save independently, type a newer edit, then assert draft values and dirty state survive while backend reflects only the submitted revision. Open report → enter comment → Download → Escape; assert exactly the parent remains, comment preserved and focus restored. Fail status polls after running; use an injected clock to reach a stale threshold, assert stale/Retry UI, then return terminal status and assert recovery. If adding Stop (UX-06), assert server dispatch counts stop at a cooperative boundary, completed results survive and a subsequent scan succeeds; stopping polling alone is insufficient.

### T8 [major] Auth throttle tests mirror wiring instead of exercising login

**Code:** `crates/otto-server/src/routes/auth_routes.rs:60`, `:87`, `:142`. **Tests:** `crates/otto-server/tests/auth_security.rs:135`, `:143`, `:175`, `:221`.

**Mutation that survives:** remove global username failure recording from `handle_login`; test-local `record_failed_attempt` still records it, so the rotating-IP tests remain green. Likewise, `forwarding_headers_are_not_a_throttle_input` calls `ip_key(peer, user)` twice without constructing a request or a header; changing request extraction to trust a forwarding header cannot change its result. Workspace search found helper/AttemptStore tests but no direct `handle_login` test.

**Why it matters:** a critical integration could regress while tests named for the endpoint's security properties remain green. This is a coverage finding, not an assertion that current production throttling is broken.

**Evidence:** confirmed falsification; new independent sample beyond prior lens findings. **Fix:** call actual login handling with an isolated attempt store and real fixture user. Rotate remote peer IPs through the failure threshold and assert 401→429 with Retry-After; assert local desktop exemption remains narrow. At the route/extraction boundary send different forwarding headers with the same peer and prove the throttle bucket does not change. Keep existing pure key/store tests, which cover their own behavior well.

### T9 [major] Resource-export tests require some samples, not samples during export

**Code:** `crates/otto-telemetry/src/lib.rs:812`, `:824`, `:850`, `:893`. **Tests:** `crates/otto-telemetry/src/tests.rs:185`, `:237`, `:253`, `:464`.

**Mutation that survives:** serialize sampling behind a blocked flush and omit the collector from sampled process identities. The integration test checks nonempty metric tables and `overview.resources`, which pre-export daemon samples satisfy. The bounded-buffer test directly injects ResourcePoints and correctly proves aggregation/eviction, not sampling cadence or process discovery.

**Why it matters:** metrics can exist while omitting the busiest export interval and the entire collector lifetime, weakening the performance evidence.

**Evidence:** confirmed assertion/control-flow trace, matching the performance report. **Fix:** controllably block export, advance through multiple sample periods, assert sampling continues and the owned collector identity is observed only during its lifetime. Assert exit/cancel/disable remove it and stop the appropriate tasks; test PID reuse. Corroborate with externally owned-process sampling during an isolated real flush. Do not infer cadence from rollup row count: multiple samples intentionally collapse into minute maxima.

## Additional sampled suites and useful existing protections

| Suite / production boundary | Falsification checked | Result of source review |
|---|---|---|
| `otto-workflows/src/checkpoint.rs:180`, production `:25` | Execute successful external action again on reentry. | Fails call-count=1 and output=42 oracle; real temp DB/current migrations are used. Unknown-outcome test at `:203` panics if action is repeated and asserts Conflict. |
| `otto-server/tests/auth_security.rs:80`, AuthRepo | Skip revocation or revoke another user's tokens too. | Mint/auth/revoke and cross-user preserved-token checks would fail. The separate route-wiring gap is T8. |
| `ui/unit/databaseWriteOwnership.test.ts:99`, database store | Retry canceled/declined/revoked write, or mutate its statement/scope. | Exact count, statement, node and confirm flag assertions catch it for both entry points. Deferred promises drive the race without sleeps. |
| `ui/unit/apiClientOwnership.test.ts:443`, API store | Publish late workspace A results into B or a new A visit. | Parameterized generation checks protect list/save ownership. They do not establish editor leave behavior (T6). |
| `otto-state/tests/migration_compat.rs:126`, migration application | Drop/rename a column, or add required column without default. | Actual sequential migrations and negative examples at `:158` exercise structural guard. This is not proof an older binary can recover newly added run metadata; add the focused reader/write compatibility case for T2. |
| `ui/unit/telemetryHarness.test.ts:65`, fixture teardown | Mutate or delete a reachable fixture with stale/replaced PID identity. | Assert zero API mutations/signals/deletes; valuable safety protection, **not telemetry ingestion coverage** despite its filename. |
| `ui/unit/school-actions.test.ts:45`, actions | Delete on Cancel or archive a non-manageable session. | Exact no-call and restoration assertions catch action-layer breaks. They cannot catch event bubbling or focus visibility in the mounted widget. |

No measured test flakiness is claimed. Serial School tests intentionally mutate common seeded sessions; long animation polling and plugin browser `waitForTimeout(300)` warrant execution observation, but their presence alone is not proof of a flaky test. Broad suites outside the sampled boundaries, native WKWebView/VoiceOver, all database drivers and provider resume formats remain unreviewed here.

## Minimal regression matrix

Each row requires a recorded baseline result and repaired result; “test already exists” is separate from “executed and passed.” Coordinator owns all execution.

| Priority / surface | Smallest decisive check | Required oracle |
|---|---|---|
| P0 Redis deletion | Adapter refusal test, or isolated Redis selected-index result test | No unsafe executable mutation, or exact survivor list including unselected markers. |
| P1 task recovery | New-context recovered workflow with edited task | Old destination, new occurrence still due, one completion; legacy/malformed snapshot conservative behavior. |
| P1 report identity | Same timestamp/different run IDs plus real write/prune | Distinct paths/content; retained run survives pruning. |
| P1 telemetry ingress | Existing full-chain browser test plus fast producer/validator contract test | Accepted whole batch and stored parent-linked spans; invalid batch accepted count remains zero. |
| P1 telemetry sampling | Paused exporter + deterministic sampler clock | Multiple sampling callbacks during blocked export; collector ownership/lifetime/disable assertions. |
| P1 School | Real card Enter/Space; complete list overflow; delayed remount | Correct action, visible focus, overflow sessions actionable, saved room restored. |
| P1 environment drafts | Navigation matrix with deferred failed/successful Save | Exact rows and selection preserved on Keep/failure; no early leave or persisted plaintext secrets. |
| P1 plugin | Delayed refresh/save, nested Download, failed status polls | Newer draft/comment/focus survive; truthful stale state and recovery. |
| P1 login integration | Actual handler/extractor threshold and spoof-header requests | Correct 401/429/Retry-After behavior and unchanged same-peer bucket. |
| P2 load/trace UI (D4/D5) | Delay/fail initial School data/scene; trace []/503/Retry/close | Visible initial loading, no invented stale-data copy, expired explanation, functional Retry, closed dialog stays closed. |
| P2 archive query repair | Many archived rows + paged eligible rows + attachment race | Same eligible archived IDs as independent oracle, no live/attached session archived, complete pages without irrelevant payload materialization. |
| P2 plugin Stop addition | Controlled pacing/backoff cancellation and restart | No subsequent dispatch; committed results retained; next scan admitted. |

After focused checks, run the planned affected-consumer gate with `--base 196048df`; run plugin tests separately. Retain exact command/environment, exit status, expected/executed/pass/fail/skip counts and logs. Browser pass counts must not be padded by skipped projects. Screenshots require inspection, including actual focus visibility, rather than creation alone.

## Plan review — separate from test findings

Both plans are proportionate and preserve existing product/module boundaries. No broad rewrite is needed. FIX-PLAN's decision to refuse unsafe list deletion is the smallest safe repair; arbitrary tombstone replacement is not an acceptable shortcut. Cooperative plugin Stop adds a real new surface, but it follows the identified long-work control requirement and explicitly preserves completed work. Do not broaden it into a general job engine.

1. **Make enforcement explicit.** Original plan `:26` / FIX-PLAN `:23–25` say gates/browser checks, but do not identify which assertions are blocking versus advisory. T4 demonstrates why this matters. Require the fast producer/ingest contract in an enforced suite, and log the full-chain browser execution separately. No need to promote an unstable heavy suite wholesale.
2. **Complete migration/recovery acceptance.** FIX-PLAN `:10` states the right legacy policy but lacks explicit malformed-snapshot and older-reader/new-schema checks. Add those cases and verify the new nullable schema does not force changes to older insert/read paths. Assert recovery through a fresh context, not merely deserialize the snapshot.
3. **Tie evidence to findings.** Original plan `:28` / FIX-PLAN `:28` should require a ledger mapping C/D/UX/T IDs to test name, baseline result, repaired result, code revision and remaining limitation. Preserve “not run,” infrastructure failure and behavior failure separately; report test totals and skipped counts. Historical results at `922ae483` do not certify `196048df`.
4. **Add the independent auth integration sample.** T8 needs tests of current behavior, without production changes unless a reproduction reveals a bug. Keep it a small guard alongside the seven repair groups.
5. **Bound the performance evidence claim.** FIX-PLAN `:26–27` correctly separates controlled fixture load and read-only real-app sampling. Record duration, throughput and recovery interval; collector inclusion must be proven during export before its measurements support a final score. Neither a short live sample nor absence of a detected leak proves long-term stability.

No permissions, publication or further source edits are required for these review recommendations. This report is the only file authored by this reviewer.
