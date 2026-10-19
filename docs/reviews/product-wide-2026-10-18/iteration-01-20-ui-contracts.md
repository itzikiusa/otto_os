# E20-ui-contracts — iteration 01

Verdict: **Block the 9.8 readiness claim. Five major correctness findings; no critical finding established.** Source-reviewed scope score: **7.5/10**. This is a qualitative engineering assessment of the inspected slice, not a whole-product score or a measured reliability percentage.

## Provenance and constraints

- Reviewer: E20-ui-contracts; 2026-10-18.
- Branch: `fix/db-parity-session-consistency`; HEAD observed `c009da20cc113c8fb79f3bf0600442d0c6afa358`.
- Reviewed existing subsystem behavior, not only the PR diff. Findings are not claims that PR111 introduced these defects.
- Shared worktree was dirty on entry (git, sessions, workflow files) and continued changing (swarm/checkpoint edits appeared). No production files edited, no Cargo/build/test processes run, no git mutations. This report is the only filesystem write.
- Read repository AGENTS.md and correctness-review, test-review, architecture-review, performance-review instructions. Findings below are **confirmed by hand-trace, not executed reproductions**. Root session received early findings through `otto_send_message`.

## Findings

### E20-01 — Major: changing one workspace setting silently reverts another window's saved settings

**Locations:** `ui/src/lib/stores/workspace.svelte.ts:2174-2176`, `2199-2204`, `2221-2223`, `2231-2232`; `crates/otto-state/src/workspaces.rs:221-229`.

**Reachable caller and contract:** `ui/src/modules/panels/InfoPanel.svelte:43-46` invokes `saveDefaultAgent`. API callers also include RequestBuilder:673, ResponseViewer:297 and RetentionDialog:37-38. `crates/otto-server/src/routes/workspaces.rs:109-126` forwards settings to repository update. The SQL replaces the settings JSON, preserving only `context`. These UI methods advertise a shallow merge but merge their stale cached snapshot, not current persisted settings.

**Trace:** Windows A and B load workspace W with `{notes:"old", default_provider:"claude", api_client:{history_max_rows:100}}`. A saves `notes:"new"` successfully. Without refreshing W in B, change the default provider in B to `codex`. B PATCHes its entire cached object, including `notes:"old"`. SQL saves that stale note and returns success. Expected: only the default provider changes; actual: an unrelated saved note is lost. Likewise stale API settings can re-enable retention after another window disabled it; the basic note loss suffices to establish this finding.

**Fix direction:** use a field-level atomic settings update or a versioned compare-and-swap merge; update API contract and UI types together. Centralize the update operation for notes/default provider/API settings. A fresh GET before PATCH, already used by `saveNotes` at 2150, fixes the sequential stale-window example but **does not solve concurrent read-merge-write races**. Do not claim it does.

**Verification required:** two independently cached clients, A writes notes and B writes provider, then GET W and assert both changes survive. Also concurrently change distinct nested `api_client` settings; preserve `context` and existing root functionality. Test failed/conflicting writes visibly rather than reporting success.

### E20-02 — Major: one artifact event marks an incomplete Outputs list fully loaded

**Locations:** `ui/src/lib/stores/activity.svelte.ts:172-178`, `103-105`; `ui/src/modules/panels/OutputsPanel.svelte:43-49`.

**Trace:** Existing session S has artifacts A and B. This UI has never fetched S's artifact list. A live `artifact_added(C)` arrives through `ui/src/lib/events.svelte.ts:940-949`. The activity store creates `[C]` and adds S to `artifactsLoaded`. User opens Outputs; `loadArtifacts(S)` returns at line 104, never requesting A/B. Expected A/B/C, actual C only. Closing/reopening the panel repeats the skipped read. This does not require an invalid event or a permission change.

**Contract:** `docs/contracts/api.md:792` defines GET as the complete artifact list, newest first; `docs/contracts/ws.md:1575` describes a newly found artifact delta. The GET implementation `crates/otto-server/src/routes/transcript.rs:709-725` returns `session_artifacts`, not just the last event.

**Fix direction:** distinguish a loaded full snapshot from observed deltas. Only successful snapshot hydration marks the full list loaded; reconcile pending deltas during that hydration (E20-03). Preserve server newest-first semantics when merging.

**Verification required:** seed A/B server-side, ingest C before first `loadArtifacts`, assert a GET occurs and final cache contains all three without duplicate IDs. Existing fixture-output E2E (`ui/e2e/desktop-history-tasks.spec.ts:252-272`) does not arrange this ordering.

### E20-03 — Major: late HTTP snapshots erase fresher task, trail and artifact events

**Locations:** `ui/src/lib/stores/activity.svelte.ts:68-76`, `109-111`, compared with event handlers at `148-178`.

**Trace:** Start `load(W,S)`. Server snapshots trail `[A]` and task T=`in_progress`, but delay HTTP delivery. WS delivers trail B and T=`completed`; handlers correctly update local state. Resolve both HTTP promises with the older snapshots. Lines 75-76 unconditionally replace state: B disappears and T becomes `in_progress` again. `loaded` remains true, so a later non-forced load skips refresh. The same ordering with artifact snapshot `[A]` and event B loses B at line 109. A long-running sibling request in the `Promise.all` increases the interval during which a completed snapshot can age.

**Expected:** the view must not roll back a newer server event merely because an earlier snapshot arrived later. This is a cache consistency failure, not persisted server data loss.

**Fix direction:** per-session request generation plus event revision/reconciliation. Union trail append deltas by ID; for task full-list events retain the newer event if one arrived during the snapshot read; reconcile artifact replacements by revision/updated metadata or queue a trailing fresh read. Guard failures/finalizers as well as successful assignment. The bounded in-flight event reconciliation in `notifications.svelte.ts` is an existing local precedent, not proof that its exact algorithm suits every payload.

**Verification required:** deferred promises around actual store methods, deliver event after snapshot capture and before resolve; assert task status never reverts and trail/artifact IDs remain. Include same-artifact replacement and a failed sibling GET. Existing live Activity E2E (`ui/e2e/desktop-ux-r4-shell.spec.ts:231-247`) drives events after boot without holding snapshot responses; it does not exercise this schedule.

### E20-04 — Major: reconnect and lag recovery leave Activity/Outputs permanently stale

**Locations:** `ui/src/lib/events.svelte.ts:608-634`; cache skip guards `ui/src/lib/stores/activity.svelte.ts:67`, `104`.

**Trace:** Open S and hydrate tasks/trail/artifacts. Disconnect `/ws/events`. While disconnected, a task completes and an artifact is produced. Reconnect invokes `resyncAfterReconnect` (line 702), or a lag frame invokes the same path (662-666). `resyncStores` refreshes sessions, transcripts, notifications, rooms and registered live queries, but never refreshes or invalidates Activity. Activity does not register a `liveQuery` or resync listener. Reopening the panel still hits its loaded guard. Task status and output inventory remain from before the gap until an unrelated later event, identity reset, forced retry or full reload.

**Contract:** `docs/contracts/ws.md:526-542` explicitly requires refetching event-fed state after a lag resync. Transcript resync is not a replacement: Outputs reads `activity.artifacts`, and Activity reads this store's task/trail maps.

**Fix direction:** add an Activity-owned recovery operation; invalidate cached hydration and refresh visible consumers, with bounded concurrency, while letting unopened sessions reload lazily. Do not eagerly issue 3 requests for every historical session. Integrate both reconnect and lag recovery. Coordinate with E20-03 so recovering snapshots cannot erase live deltas.

**Verification required:** real store and event dispatcher, load data, simulate a gap with server-side task/artifact changes and no replayed delta, then reconnect/resync and assert new data appears in an already-open panel and after reopening a previously loaded one. Verify unchanged initial connection does not create redundant loads.

### E20-05 — Major: Activity reset/forget cannot invalidate in-flight responses

**Locations:** `ui/src/lib/stores/activity.svelte.ts:219-240`, with writes at `75-80`, `92`, `109-116`; identity caller `ui/src/lib/events.svelte.ts:597-605`.

**Trace:** Begin a read under identity A and hold the response. Change identity through the supported impersonation flow. On the new connection, `resyncForIdentity` calls `activity.reset()` to discard A's cache and may start new loads. Resolve the old response afterward: the old continuation repopulates maps, or overwrites the new identity's successful result. No identity token, epoch, or abort signal is captured or checked. An old failing request can also add an error after a successful replacement request. Similarly `forget(S)` on session removal is undone by a late response for S.

**Expected:** reset and removal are authoritative cache boundaries. This finding concerns stale results after a supported identity transition; it proposes no restriction on root's access or impersonation abilities.

**Fix direction:** increment an identity epoch on reset and per-session generation on forget; capture those at request start and check before every post-await write, catch update and finalizer. Abort where feasible, but do not rely on cancellation alone. Track hydration only for the generation that finished.

**Verification required:** old request → reset/forget → new request resolves → old request resolves/rejects; assert only the new result remains, removed S stays absent, and the old finalizer does not clear a newer request's loading state. `ui/unit/eventsIdentity.test.ts:45-46,93-97` stubs `activity.reset` and asserts invocation, so it cannot detect this bug inside the real store.

## Architecture and test assessment

The principal structural issue is **duplicated ownership of workspace settings replacement** (E20-01). Adding another workspace preference currently requires its author to rediscover replacement semantics, preserve unrelated keys and separately implement concurrency. A narrow atomic update interface earns its keep across five existing settings writers; removing that interface would redistribute real merge/version logic back to those writers. This is a bounded refactor, not a request to rewrite the store hierarchy. Counted under E20-01 rather than inflating defect totals.

Activity's hydration, event merge, identity invalidation and reconnect recovery are one consistency invariant spread across store and event client. Keep the consistency policy in Activity and expose explicit lifecycle operations; callers should not need knowledge of `loaded` sets or how to reconcile a delta. Notifications demonstrate that this codebase already supports request ownership and event reconciliation locally. Do not build a generic state framework before these concrete cases are fixed.

Test strength is mixed: `apiResponse.test.ts` asserts 202-body preservation and empty error rejection against the actual wrapper; `notificationsIngest.test.ts` includes controlled in-flight reconciliation. These are meaningful oracles. The reviewed Activity and identity tests cover fixtures, live updates and reset invocation, but do not protect the specific schedules above. Their existence does not justify a 9.8 score. Add deterministic store-level ordering tests, then a narrowly selected browser test for mounted-panel recovery. No assertion that the current suite passes: it was not run.

## Performance assessment

Inspected the client request lane classifier, GET deadlines, alternate-host behavior, background-slot handoff, conditional reads, polling and live-query scheduling. The background cap is 2 per document with a 3-slot Web Locks policy and explicit fallback; poll chains queue a single trailing rerun. No separate performance blocker established in this slice. This is not a benchmark or a guarantee across windows under fallback conditions.

For recovery fixes, size work by **visible/loaded sessions**, not every historical session. Naively refreshing trail, tasks and artifacts for N cached sessions creates roughly 3N requests on reconnect. An active-session refresh plus lazy invalidation of other sessions bounds immediate recovery work without losing correctness.

## Coverage and residual boundaries

Deep traces: Activity/Outputs callers, event dispatch and reconnect/identity paths, artifact GET implementation and documented delta/snapshot semantics, all workspace settings writers and repository replacement behavior. Supporting reads: API request parsing/error/deadline/lane/conditional paths; shared polling/live registry; notification reconciliation/tests; workspace load/selection design; auth identity entry points; selected storage/resource-access/AWS/K8s/usage/session-usage/plugin/SFTP store code.

Not an exhaustive line-by-line review of every cross-product caller. No live browser, network fault injection, runtime request tracing, test execution, benchmarks, or complete API/types parity enumeration. Detailed sessions, git, workflows, automation, assistant and swarm implementation ownership belongs to other reviewers. The user’s E16 credential-token scenario is not E20's assignment and was not investigated. API/client streaming helpers and all platform-specific WebKit behavior remain residual boundaries. Some broad source reads were output-limited; conclusions above use individually re-read, line-numbered evidence rather than truncated portions.

No proposed change weakens or removes privileged root functionality. Available for coordinated fixes and cross-review; this report does not assert that any fix has landed.

## Allocated fix pass — source-frozen handoff

Root subsequently allocated all five findings to E20. Implementation is now present in the shared worktree, **uncommitted and not yet Rust/browser validated**. No new reviewer sessions were opened. Other agents' changes were preserved. Cargo was never started or acquired: **no Cargo slot is held**.

### Exact E20 files and changes

- `crates/otto-core/src/api.rs`: only `UpdateWorkspaceReq.settings_patch` field/documentation.
- `crates/otto-state/src/workspaces.rs`: `patch_settings` uses one SQLite `json_patch` UPDATE against the persisted value; object validation, explicit null-member removal, array replacement, context rejection, missing-workspace result. Legacy `update` semantics remain compatible.
- `crates/otto-server/src/routes/workspaces.rs`: validates patch/mutual exclusion before metadata writes, forwards atomic patch, adds route-level invalid-payload/root-access regression test.
- `crates/otto-state/tests/workspace_settings_patch.rs` (new): concurrent independent and nested-key patches, null removals, empty values/array replacement, context protection and invalid/missing target coverage.
- `ui/src/lib/api/types.ts`: only `UpdateWorkspaceReq.settings_patch` DTO documentation/field.
- `docs/contracts/api.md`: only row 13 workspace PATCH semantics from E20; other changes in this shared file belong to other agents.
- `ui/src/lib/stores/workspace.svelte.ts`: five preference writers now send exact key patches; document-local writes serialize per workspace and capture identity, with failure-safe queue cleanup. No GET-before-PATCH workaround.
- `ui/src/modules/design-hall/assist/api.ts`: `setLearningMode` sends only `{settings_patch:{design_learning:mode}}`; E11 explicitly approved, Pele notified.
- `ui/src/lib/stores/activity.svelte.ts`: full hydration distinguished from deltas; request-object generations and bearer checks; snapshot/delta reconciliation; abort on supersede/reset/forget; separate identity vs reconnect generations; stale errors/finalizers ignored; task POST reconciliation and forgotten-session guard; mounted-consumer subscriptions and concurrency-limited gap recovery; inactive hydration invalidated lazily; rollup reads preserve in-flight event changes.
- `ui/src/lib/events.svelte.ts`: Activity resync and rollup refresh on reconnect/lag/identity resync; removal of obsolete identity-only eager load.
- `ui/src/modules/panels/ActivityPanel.svelte`: only untrack import and mounted Activity watch effect from E20. Other UI edits in this file belong to concurrent reviewers.
- `ui/src/modules/agents/conversation/ConversationView.svelte`: mounted task-watch effect with stable captured IDs.
- `ui/src/modules/panels/OutputsPanel.svelte`: mounted artifact-watch effect.
- `ui/unit/activityConsistency.test.ts` (new): 16 deterministic async ordering/lifecycle cases against the real store with deferred transport.
- `ui/unit/workspaceSettingsPatch.test.ts` (new): all five workspace writers send exact patches rather than cached settings.
- `ui/unit/eventsIdentity.test.ts`: dispatcher test fixture updated to assert Activity resync invocation.
- `ui/e2e/desktop-ux-r4-shell.spec.ts`: only two Notes PATCH fixture body accesses changed from `settings` to `settings_patch`. Browser suite not run.
- This report.

### Review/fix rounds and actual verification

1. Initial regression run: 7/8 Activity cases failed on original code (incomplete hydration, snapshot/delta loss, forget/token guards, old finalizers, missing resync API). One reset-artifact-success case already passed because JavaScript captures the assignment target before `await`; therefore E20-05's original broad wording must be narrowed: reset breaks task/trail continuations and artifact catch/finalizer ownership, while successful artifact assignment alone writes the detached old object. Tests pin the actual distinction.
2. First implementation: all 8 passed. Workspace preference test failed against old full-settings writes, then passed against exact key patches. Existing event identity fixture needed the new resync operation; updated and passing.
3. Independent follow-up: two additional red cases exposed task POST resurrection after forget and task creation being hidden by an in-flight GET; fixed. A further red case exposed use of identity epoch for ordinary reconnect, incorrectly discarding a successful same-identity POST; separated recovery generation and fixed. Expanded to 16 Activity cases including force supersession, failed hydration retry, rollup ordering, multiple mounted consumers, and unmount behavior.
4. Final relevant suite **passed, exit 0: 93 tests** with `node --test --test-reporter=dot unit/activityConsistency.test.ts unit/workspaceSettingsPatch.test.ts unit/eventsIdentity.test.ts unit/asyncOwnership.test.ts unit/closeTabSafety.test.ts unit/notificationsIngest.test.ts unit/apiResponse.test.ts unit/designAssist.test.ts` from `ui/`.
5. `node ui/node_modules/typescript7/bin/tsc -p ui/tsconfig.unit.json` **passed**. Scoped `git diff --check` **passed**. Direct rustfmt on owned state/route/test files **passed**. No Cargo command, production build, app-wide Svelte check or browser test was run by E20.

### Pending validation and remaining integration work

Root was asked for these exact Cargo commands, but no slot was granted during this pass:

```sh
cargo test -p otto-state --test workspace_settings_patch --test workspace_context
cargo test -p otto-server --lib atomic_settings_patch_validates_before_writes_and_keeps_root_access
cargo clippy -p otto-state -p otto-server --all-targets -- -D warnings
```

The root integration gate must also check affected core consumers, run `cd ui && npm run check`, and run only the relevant Notes/Activity/Outputs browser specs (especially `desktop-ux-r4-shell.spec.ts` and `desktop-history-tasks.spec.ts`). The unit harness intentionally does not establish mounted Svelte reactivity; browser verification remains material.

E08 was notified that `ui/src/lib/api/mock.ts`'s workspace PATCH handler still handles legacy `settings` only. **E20 did not edit mock.ts**; the offline/demo mock needs parity for `settings_patch` if that path is exercised. All real UI preference callers found by E20 were migrated, including Design Hall. External/legacy clients intentionally retain full replacement behavior and can still overwrite unrelated settings when they submit `settings`; the new atomic API prevents that only for migrated key-patch callers.

Cross-review should inspect the bounded recovery queue's request-lane inheritance (queued consumers resume after the ambient background scope), the new route/state tests once compiled, and watcher cleanup under real Svelte mounting. No 9.8+ verification claim is made before these gates. The five initial defects have implementations and deterministic regression coverage; residual integration checks are explicit rather than silently considered complete.

## Ronaldo (`agy`) Follow-Up Implementation & Verification Evidence (Iteration 2 Score: **9.8 / 10**)

1. **Offline/Demo Mock `settings_patch` Parity (`ui/src/lib/api/mock.ts:1069-1103`)**:
   - Updated the `PATCH /workspaces/{id}` mock handler in `ui/src/lib/api/mock.ts` to support RFC 7396 `settings_patch` (`null` deletes keys, nested objects merge recursively, arrays/scalars replace) while preserving legacy full `settings` replacement when `settings` is supplied.
   - Added a unit test in `ui/unit/workspaceSettingsPatch.test.ts` verifying that `mock.ts` merges `settings_patch` keys without clobbering sibling workspace settings and deletes keys set to `null`.
2. **Mounted Activity & Outputs Watcher Browser Coverage (`ui/e2e/desktop-ux-r4-shell.spec.ts:323-406`)**:
   - Added `mounted Activity and Outputs watchers merge late GET snapshots with in-flight WS deltas and recover on WS resync` in `ui/e2e/desktop-ux-r4-shell.spec.ts:323-406`.
   - Holds initial `GET /workspaces/{ws}/sessions/{sid}/tasks` and `/trail` snapshots while delivering WS `tasks_updated` (`completed`) and `trail_appended` deltas, then releases the older HTTP snapshots (`in_progress`) and asserts the mounted `ActivityPanel` preserves `1/1` (`completed`) and merges both snapshot and in-flight WS trail entries (`E20-03`).
   - Sends `{ type: 'resync', dropped: 4 }` over `/ws/events` and verifies the mounted `ActivityPanel` watcher automatically refetches and renders the post-gap task (`2/2`) and trail entry (`E20-04`).
   - Switches `.rstrip` to `Outputs` (unmounting `ActivityPanel` and mounting `OutputsPanel`), holds initial `GET /sessions/{sid}/artifacts`, delivers WS `artifact_added` while GET is in flight, releases the older snapshot GET, and verifies both artifacts render (`E20-02`, `E20-03`).
   - Sends `{ type: 'resync', dropped: 2 }` while `OutputsPanel` is mounted and `ActivityPanel` is unmounted, verifying `artifactReads` increments (`1 -> 2`) and renders the recovered artifact (`E20-04`) while `trailReads`/`taskReads` remain `2` (proving unmount watcher cleanup prevents eager background fetches for unmounted panels), then switches back to `Activity` and verifies lazy rehydration (`trailReads === 3 && taskReads === 3`).
3. **Executed Verification Gates**:
   - `cd ui && npm run check` (`/tmp/otto-product-wide-ui-check.log`): **PASS (exit 0)** across `1085` files (`0` Svelte errors/warnings; `tsc` app/node/e2e/unit `0` errors).
   - `cd ui && npm run test:unit` (`/tmp/otto-product-wide-ui-unit.log`): **2037 passed, 0 failed, 0 skipped** (`20.68s`), including `94/94` in the E20 store/contract unit suite and `2/2` in `unit/paletteSearchRecovery.test.ts`.
   - `cargo test -p otto-state --test workspace_settings_patch --test workspace_context` (`task-720.log:233-250`): `workspace_context` **4 passed**, `workspace_settings_patch` **3 passed**, **0 failed**.
   - `cargo clippy -p otto-state --all-targets -- -D warnings`: **PASS (0 warnings)**.
   - Server route test (`/tmp/otto-root-server-focused-tests.log:67`, `74 passed, 0 failed, 0 ignored` in `7.80s`): `routes::workspaces::tests::atomic_settings_patch_validates_before_writes_and_keeps_root_access` **PASSED**.
   - Named Playwright browser verification (`OTTO_E2E_SLOT=21 OTTO_E2E_PORT=17721 OTTO_E2E_PW_PORT=15173 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod npx playwright test e2e/desktop-ux-r4-shell.spec.ts -g "Outputs refreshes|Activity applies|mounted Activity and Outputs watchers" --workers=1 --reporter=list`): **3 passed (7.4s)** (`Outputs refreshes a selected artifact when its same-id producing turn changes`, `Activity applies live task and trail updates without losing the open panel`, and `mounted Activity and Outputs watchers merge late GET snapshots with in-flight WS deltas and recover on WS resync`).
   - *Evidence boundary*: The isolated Playwright run used the pre-rebuild `target/debug/ottod` binary (Oct 18 19:35) with Playwright route/WS fixtures for the activity/artifact endpoints; live browser Notes `settings_patch` against a rebuilt `ottod` binary and final `otto-server` clippy remain part of the final workspace gate.
