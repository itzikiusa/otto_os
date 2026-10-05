## Latest focused checkpoints

- Workbench paging wrapper: 0/1 passed, expected missing `limit` query assertion (`null !== 100` string); production paging remains pending.
- Orchestration forms and shared node bodies: 21/22 passed. Original 12 form/cache failures now pass. New unchanged scheduled-store ownership regression fails (`task-A` replaced workspace B `task-B`); repair pending. No new heavy gates run while Claude holds the slot.

# Verification ledger

Baseline source `03f2bc3e`. Prior green CI is recorded in the preceding effort and PR78; it does not certify the new findings. Only actual command output is recorded here.

## Iteration 4 — role 1 red evidence

On unchanged production source (new review docs at `c5d4e999`), root ran from `ui/`:

```sh
node --test unit/historyActions.test.ts unit/newSessionRecovery.test.ts unit/transcriptLifecycle.test.ts
```

Result: **30 tests; 23 passed, 7 failed**, 592 ms reported. Log `/tmp/otto-review05-role1-red.log`. Failures were intended behavioral assertions, not compilation/harness errors:

- Working History action labels as resume; stale inactive row invokes unconditional restart.
- Open in Chat leaves warmed Terminal preference unchanged.
- Mixed-success session creation closes recovery surface.
- Collapsed child bodies remain retained; 1,200 child turns accumulate; late child body repopulates after collapse.

Existing running/idle History opens, all-success/all-failed batches and prior transcript lifecycle cases passed. Tests invoke current production functions through the existing AST/VM harness pattern and real transcript store; mounted-browser and server authority acceptance remain pending. Parent requested stronger mandatory newer-page reachability assertion before green acceptance.

## Iteration 4 — safe resume route red evidence

With production route still absent, root ran (heavy slot explicitly coordinated with Claude):

```sh
CARGO_BUILD_JOBS=2 cargo test -p otto-sessions --test isolation resume_ -- --test-threads=1
```

Result: **0 passed, 2 failed**, expected `404` instead of `200`/`409`. Compilation 15.07 seconds; tests 0.33 seconds. Log `/tmp/otto-review05-role1-rust-red.log`. Fixtures use migrated isolated DBs and a `/bin/cat` PTY; the failing live case removes its own session before asserting. The green tests will verify the same live handle survives all live/stale statuses, owner/admin/root/resource authorization, and archived/unsupported errors. These checks have not passed yet.

Root updated the shared History status type/contract and added the safe resume API contract plus route-policy regression; the existing generic session-control policy already requires Agents Edit. Implementation is in progress. No browser run or new load measurement is claimed yet.

## Iteration 4 — role 1 first green checkpoint

- Same focused UI command: **33/33 passed**, 648 ms. Log `/tmp/otto-review05-role1-green1.log`. Added cross-parent capacity, duplicate-consumer and oversized-turn cases; newer-page navigation assertion is mandatory.
- `npm run check`: **0 errors / 0 warnings**, guards passed. Log `/tmp/otto-review05-role1-check.log`.
- Same focused Rust command: **2/2 passed**, 14 filtered; compilation 6.92 seconds, tests 0.30 seconds. Log `/tmp/otto-review05-role1-rust-green1.log`.

These results cover the working-tree implementation at this checkpoint, not later modifications. Root source review identified pre-request admission as a remaining transient-fanout gap and requested a regression/repair. Also checking reverse stale History status (row live, actual inactive). Browser, full affected gate and independent final acceptance remain pending. Heavy slot returned to Claude after both completed gates.

Follow-up red run: `node --test unit/historyActions.test.ts unit/transcriptLifecycle.test.ts`, **32 tests, 30 passed, 2 expected failures**, 661 ms. Log `/tmp/otto-review05-role1-followup-red.log`. The new tests prove 100 deferred child expansions launch 100 requests before admission and a stale live History row skips safe resume. Production follow-up authorized only after this run.
# Role1 follow-up focused green

After the pre-fetch child admission and stale-live History follow-ups, root ran `node --test unit/historyActions.test.ts unit/newSessionRecovery.test.ts unit/transcriptLifecycle.test.ts` from `ui/`: **35/35 passed**, 838 ms, exit 0. This includes both previously failing follow-up cases. Latest UI check, mounted browser journeys, and additional independent-review coverage gaps remain pending. This subsecond focused run did not start a build or browser while Claude holds the heavy slot.
# Role2 dialog red

Root inspected and executed `node --test unit/dataToolsDialogs.test.ts`: **0 passed / 8 failed**, 297 ms, exit 1, before production repairs. All failures matched the intended assertions: importing submitted DELETE/INSERT/multi-statement drafts (3 cases), failed request save retained `__new__` instead of the created collection, Recovery Refresh did not retry/update history (2), environment Save overwrote newer typing or another selection (2). This runs extracted production handlers with controlled transport/state; mounted coverage is still pending. Four-component implementation authorized after observing these failures.
# Role2 database ownership red

Root inspected and ran `node --test unit/databaseWriteOwnership.test.ts`: **6 passed / 4 failed**, 1.327 s, exit 1, before production repair. Both entrypoints named selected B while retrying origin A; managed statements also retried after access revocation; an agent read-only refusal could use the untyped development gate after switching away from production. Accepted-origin, declined-confirmation and query cancellation/revocation controls passed. The fixture evaluates production store methods and derived expressions; no database write occurs. Node emitted a localStorage experimental warning; an explicit fixture shim is requested. All four failures matched intended behavior, not harness errors.
# Role2 API workspace ownership red

Root inspected the test diff and ran `node --test unit/apiClientOwnership.test.ts`: **26 passed / 21 failed**, 14.094 s, exit 1. Existing 26 controls passed. All new cases failed on intended stale state/error/loading assertions: collection/environment/automation reads and writes across A→B and A→B→A; stale errors; activation affecting B's next Send; bulk-load success and finally ownership. No build/browser or external HTTP request was launched. Production repair authorized after this evidence; complete producer return-value and deferred cleanup assertions requested.
# Role2 first UI green

Root ran `node --test unit/dataToolsDialogs.test.ts unit/databaseWriteOwnership.test.ts unit/apiClientOwnership.test.ts` after the six-file UI repairs: **68/68 passed**, 13.806 s, exit 0. Log `/tmp/otto-review05-role2-ui-green1.log`. This includes failed environment Save, former-workspace completion and import metadata selection controls added after red. Node still emitted a localStorage experimental warning, despite the VM storage shim; investigate the directly imported helper before calling output pristine. Mounted verification and combined UI check remain pending. Source inspection identified additional secret-marker/import-confirmation/partial-save ownership cases; focused regressions are being authored before acceptance.
# Role2 dialog follow-up red

Root inspected six added cases and ran `node --test unit/dataToolsDialogs.test.ts`: **11 passed / 6 failed**, 315 ms. Log `/tmp/otto-review05-role2-dialog-followup-red.log`. Failures: two saved-secret rename marker cases, request-save second mutation after workspace switch, import approval naming wrong connection, retry after Cancel during confirmation, and old import completion closing a replacement table's dialog. DatabasePage's keyed dialog establishes the replacement path. These are production-handler failures; fixes authorized. The shell command printed the log afterward, so its overall exit status was the log-reading command's; counts above come from the test runner output.
# Role2 follow-up UI green

Root reran the three role2 files after adjacent repairs: **75/75 passed**, 14.151 s, exit 0. Log `/tmp/otto-review05-role2-ui-green2.log`. Independent spec-then-quality source recheck dispatched; combined UI typecheck and mounted acceptance still pending.

# Role3 browser initial red

Root inspected and ran `node --test unit/browserNavigationOwnership.test.ts`: **0 passed / 6 failed**, 292 ms, exit 1. Five intended assertion failures establish reader/live creation scope contamination (A→B and A→B→A) plus reversed same-tab navigation completion. The cross-tab title case instead failed its setup assumption (`page` was null), so it is not accepted as a valid reproduction yet; fixture correction requested before claiming evidence for that case.
# Role3 corrected browser and Canvas red

Root inspected the component-handler harness and reran browser ownership with `unit/canvasRecovery.test.ts`: **1 passed / 11 failed**, 300 ms, exit 1. The corrected cross-tab fixture now fails the intended assertion (A receives B's title); all six browser cases are valid reproductions. Canvas fails prompt retention for all three editor hosts, pending prompt preservation, and restored background/grid persistence. The scene-switch success control passes. Repairs authorized; no mounted or native result claimed.
# Role3 browser and Canvas first green

Root ran the same two files after source repair: **12/12 passed**, 362 ms, exit 0. Inspected browser generation/local page/sequence changes, Canvas boolean acceptance handshake, and persisted Excalidraw background/grid subset. Additional success/empty controls and a possible overlapping persistence-order case are requested; that persistence concern is not yet a confirmed finding. UI typecheck, mounted tests and independent source recheck remain pending.
# Role2 independent-review regressions red

Root inspected the lifecycle-aware component harness and ran `unit/dataToolsDialogs.test.ts`: **18 passed / 3 failed**, 382 ms, exit 1. Actual production `onDestroy` registrations are invoked by the fixture. Cancel/unmount still started one unsent request mutation; returning A→B→A left stale plain value or secret name displayed clean. These failures substantiate R4-R2-UI-01/02; minimal repair authorized. Mounted behavior remains a separate acceptance requirement.
# Role3 lookup and browser persistence red

Root inspected and ran browser ownership, vaultStore, vaultLookupRecovery and publishDestinationOwnership: **22 passed / 8 failed**, 1.456 s, exit 1. Log `/tmp/otto-review05-role3-lookup-red.log`. Intended failures establish overlapping live navigation persisting the old URL last, stale Jira/Confluence destination replies, pending/failed switcher Enter creating a note, previous-vault results publishing, lost stale tags, and swallowed lookup errors. The earlier browser persistence concern is now a confirmed reproduction. RFC retry retains title/parent as a passing control. Repairs authorized; no real provider publication occurred.
# Role2 reviewer-follow-up green

Root reran all three role2 files after synchronous dismissal/destruction and clean-reselection repairs: **80/80 passed**, 13.998 s, exit 0. Log `/tmp/otto-review05-role2-ui-green3.log`. Includes same-tick Close invalidation and genuine edits after returning to A. Independent reviewer follow-up requested for its two findings; mounted/typecheck acceptance remains pending.
# Role3 lookup/persistence first green

Root reran browserNavigationOwnership, canvasRecovery, vaultStore, vaultLookupRecovery and publishDestinationOwnership after repairs: **39/39 passed**, 1.246 s, exit 0. Log `/tmp/otto-review05-role3-ui-green2.log`. Includes three editor acceptance/empty/busy controls. Inspection identified two presentation recovery remnants (Tags error also saying no matches, stale destination error after successful retry); corrections requested before independent source recheck. Backend publish-preview binding is still unimplemented and is not included in this green result.
# Role3 error-state follow-up

After local destination-error handling and the Tags no-match branch correction, root reran vaultLookupRecovery and publishDestinationOwnership: **7/7 passed**, 270 ms. Tags markup was inspected; it is not exercised by those unmounted handlers. Independent role3 UI spec/quality recheck dispatched. Backend/performance work remains explicitly open.
# Role3 independent-review regressions red

Root ran browserNavigationOwnership plus vaultLookupRecovery: **14 passed / 3 failed**, 336 ms, exit 1. Late reader success and failure both republished before close HTTP completion; pending Down left Enter without a selected result. Background-tab close and multiple/empty result controls passed. Repairs authorized; close-failure recovery must remain available.
# Role3 reviewer-follow-up green

Root ran browserNavigationOwnership plus vaultLookupRecovery after close/read invalidation and pending-selection fixes: **18/18 passed**, 436 ms, exit 0. Includes failed-close content recovery and background-close non-cancellation. Root inspected the repair and requested independent follow-up for the two findings. Mounted/typecheck/native evidence remains pending.
# Role4 initial UI/cache red

Root inspected and ran `node --test unit/orchestrationForms.test.ts unit/runProgress.test.ts`: **9 passed / 12 failed**, 332 ms, exit 1. Log `/tmp/otto-review05-role4-ui-red.log`. Intended failures cover schedule post-submit edits/created-ID retention/navigation/replacement ownership; Goal Loop navigation/budget-only drafts/late-launch callback; shared node cache byte admission, oversized non-retention, and access-based eviction. Existing cache/dedup controls and failed-save draft retention pass. Tests use actual handlers/registration expressions with controlled boundaries; runtime/mounted acceptance remains separate. Source repairs authorized after these failures.

# Role4 scheduled-loader follow-up

Root inspected and ran `node --test unit/orchestrationForms.test.ts unit/runProgress.test.ts`: additional deferred-load cases first produced **22/25 passed, 3 intended failures**, 372 ms. After request/workspace generation fencing, the same command passed **25/25**, 378 ms, exit 0. Form/cache acceptance remains focused and unmounted; independent source recheck, combined UI check and browser acceptance remain pending. Rust transaction/scheduling tests remain unexecuted behind the coordinated heavy slot.

# Role5 initial platform ownership red

Root inspected and ran `node --test unit/platformOwnership4.test.ts`: **1/23 passed, 22 intended assertion failures**, 382 ms, exit 1. Reproduces cross-agent lost schedules/runs, same-agent stale data/errors, Proof filter/cursor/loading ABA and closed-detail publication, undrained settings model saves/provider debounce, repeated mint replacing a one-time token, and absent account-list pending/retry state. Existing account provider/unmount stale-result control passes. Production repair authorized; these are production-function/lifecycle adapter checks, not mounted acceptance. No external token/provider mutation occurred.

# Role5 first UI green

Root inspected production diffs and ran `node --test unit/platformOwnership4.test.ts`: **23/23 passed**, 368 ms, exit 0. Failed model persistence now preserves the intended draft with inline error and explicit Retry; its regression verifies successful retry while committed config remains prior until accepted. Independent source review, Svelte check and mounted journeys remain pending. Recap performance/backend work is still test-planning only.

# Role4 independent-review regressions red

Root inspected and ran orchestrationForms plus runProgress after production-parent callback and actual LoopsStore deferred coverage: **26/34 passed, 8 intended failures**, 401 ms, exit 1. Confirms accepted Goal leave does not close the persistent parent, departed create refreshes old context, and stale list generations publish success/error/finally. Local Cancel once remains a passing control. A mounted real workspace/router/confirmation browser regression was authored but has not run. Minimal form settlement and loop list/create ownership repairs authorized.

# Role4 reviewer-follow-up green

Root inspected Goal form settlement and loop-store generation fencing, then ran orchestrationForms plus runProgress: **34/34 passed**, 407 ms, exit 0. This covers the eight independent-review regressions; one earlier clean-form harness case now uses a fresh form after accepted leave, matching actual unmount. Independent follow-up and mounted real workspace flow remain pending.

# Role5 independent-review regressions red

Root ran the three focused platformOwnership4 cases for rejected provider before/after debounce and Svelte Retry reactivity: **0/3 passed**, 378 ms, exit 1. Both provider cases issue a wrong provider/model pair. Direct single-component compilation confirms `non_reactive_update` for modelSaveTimer; this is now executed evidence, not only a predicted warning. Follow-up repair authorized; mounted test remains pending.

# Role5 reviewer-follow-up green

Root inspected provider-bound model drafts, failed-pair Retry and reactive timer changes, then ran the full platformOwnership4 file: **26/26 passed**, 475 ms, exit 0. Includes both failed-provider timing variants and direct Svelte compilation warning regression. Independent follow-up and mounted controlled-time Retry test remain pending; no full UI check is implied.

# Role3 large-data UI red

Root inspected and ran sharedDiffBounds, productTranscriptLoading and asyncFindProvider: **0/10 passed**, 398 ms, exit 1. Production DiffView treats identical50k-line input as100k changed rows and sparse two-line edits as50k deletions; real template row inputs remain10k–20k. Controlled transport fixture measures26,227,002 serialized bytes for100 collapsed256KiB transcripts. Thin summary expansion performs no body fetch, and shared find ignores the optional async provider path/errors. Repairs authorized after these intended assertions. This is algorithm/handler plus controlled-transport evidence, not real network, DOM frame timing or RSS measurement. Full-source/omitted-window navigation and backend paging/search remain to be tested.

# Role5 recap polling initial red

Root inspected and ran recapPolling4: **1/8 passed, 7 failures**, 306 ms. Unchanged15ticks produce16 full-body calls; summary/draft updates and error recovery also reread unchanged content. Explicit refresh/close control passes. Root found the tail-append fixture returns101 events despite limit100, so that specific acceptance case requires correction against actual pagination before being credited. Remaining assertions are useful production-script controlled-transport reproductions; no HTTP/backend cost test has run yet.

# Role5 corrected recap polling red

After fixture pagination correction, root reran recapPolling4: **1/9 passed, 8 intended failures**, 280 ms, exit 1. Separate partial99→100 and exact100→101 cases respect limit100 and expose the next cursor without moving pages. Both now fail only unnecessary subsequent body reads. UI revision-aware polling repair authorized; real revision HTTP/cost tests remain queued and backend production is unchanged.

# Role3 DiffView and asynchronous find first green

Root inspected algorithms, actual template page slices and optional async find path, then ran sharedDiffBounds plus asyncFindProvider: **9/9 passed**, 410 ms, exit 0. Includes real pagination handlers reaching every changed row across20 pages and exact before/after Blob bytes. Work/DOM inputs are bounded; full O(N+M) line/row metadata remains. This does not establish browser/native download behavior, frame timing, or RSS. Product transcript integration/backend search remains pending.

# Role5 recap client first green

Root inspected revision/body pairing and ran corrected recapPolling4: **9/9 passed**, 300 ms, exit 0. Covers15 unchanged ticks, metadata-only changes, partial/full page append boundaries, older page stability, draft replacement/removal, delayed body race, manual refresh/close and transient revision error recovery. Backend revision endpoint is still absent pending Rust RED; this green result is only the controlled-transport client protocol. Explicit Refresh presentation was synced with Claude.

# Role3 transcript first green and two follow-up reds

Root ran productTranscriptLoading, asyncFindProvider and sharedDiffBounds: **16/18 passed**, 435 ms. All six Product summary/lazy/cache/search handler cases pass using proposed mocked endpoints; backend routes remain absent. Two intentional new failures: departed provider rejection publishes obsolete error, and two distant edits within repeated50k lines still produce49,971 deletions. Repairs authorized.

# Role1 supplemental coverage first run

Root ran History/NewSession/transcript focused batch: **38/39 passed**, 674 ms. New shared-byte replacement/refused-page recovery, late collapse/reopen reservation, and workspace-bound retry controls pass. Adaptive test fails14-versus15 page-size assertion; inspection shows it hardcodes uniform page sizes although the reducer also uses actual returned turn count near the beginning. This is not yet a product defect; revise the assertion to bounded pages and complete old/new reachability, then rerun. Added Rust/browser coverage remains unexecuted.

# Role1 supplemental coverage green

With the adaptive fixture corrected to actual variable page sizes and full forward/backward reachability, root reran the same three files: **39/39 passed**, 1.035 s, exit 0. All180 turns are reached in both directions within the page payload/count limits; no production change was needed for that fixture failure. New inactive-resume Rust and mounted History cases remain queued.

# Role3 repeated-line diff and provider ownership green

Root inspected the bounded Myers fallback and provider ownership rejection guard, then reran Product transcript/DiffView/async find: **18/18 passed**, 493 ms, exit 0. Two distant substitutions in50k repeated lines now remain two changes, and departed provider errors do not leak. Sparse operation streams reconstruct originals; complete pages/downloads remain reachable. Product backend routes are still pending, so its six green cases remain mocked-transport UI evidence.

# Role2 Workbench History UI red

Root inspected and ran workbenchHistoryPaging: **1/8 passed, 7 intended failures**, 269 ms, exit 1. Query wrapper is green. Existing History handlers omit page bounds, publish superseded/ABA success/error/loading, restore a later selection instead of the approved revision, start a restore after document change, and apply old restoration into the new document. Production History repairs authorized; backend paging and MCP regressions remain unrun.

# Role3 additional reconstruction/download controls

Root reran the three large-data UI files after adding repeated-line insertion/deletion/reordering reconstruction controls and oversized-transcript download coverage: **22/22 passed**, 581 ms, exit 0. Full bytes remain downloadable for an oversized body that is deliberately excluded from the4MiB cache. Rust transcript/publish/metadata tests are authored but unrun.

# Role2 first HistoryPanel implementation checkpoint

Root inspected paging/visit/lifetime/restore changes and ran Workbench tests: **10/11 passed**, 292 ms. The remaining navigation test invokes a fire-and-forget handler and waits one host microtask across a VM/transport boundary; it still sees the prior page. This is not yet established as a product defect. Await the actual load completion and rerun before acceptance. All seven initial reproduced failures plus destroy/confirmation ABA controls now pass.

# Role2 HistoryPanel focused green

Older/Newer handlers now return their existing load promise, and the fixture awaits it rather than a single host microtask. Root reran Workbench history: **11/11 passed**, 319 ms, exit 0. Pagination behavior is unchanged by that synchronization correction. Independent source recheck and mounted/backend/MCP verification remain pending.

### Product transcript lifecycle independent follow-up — RED

Root ran `node --test unit/productTranscriptLifecycle.test.ts`: 1 passed / 11 failed, 516 ms. Actual store imports at 50/51/100 lose cursor-covered rows; delayed pre-import page replaces new import; off-page search displaces ordinary boundary; actual registry unregister continues one extra HTTP page and retains rows on rejection; actual async Overview reveal expands after close/no-results replacement. Normal real reveal control passes. Production repair now authorized; these are controlled transport/DOM-adapter tests, not mounted-browser or backend evidence.

### Workflow version history UI — RED

Root ran `node --test unit/workflowVersionPaging.test.ts`: 0 passed / 2 failed, 285 ms. Actual wrapper omits `summary=true`; actual drawer loadVersions does not request a bounded 50-row window. Handler/transport regression only, not mounted UI (the assertion message saying mounted does not change the harness scope). Rust HTTP/state tests authored but still unrun while Claude holds the heavy slot.

### Workflow version history UI — GREEN

Root ran `node --test unit/workflowVersionPaging.test.ts unit/orchestrationOwnership.test.ts`: 22/22 passed, 370 ms. The new three cases prove bounded summary-wrapper queries, complete oldest-version reachability across an intervening save, and stale request/error/finally rejection with exact-cursor retry; prior orchestration ownership cases stay green. UI component handlers/controlled transport only; production server paging and mounted validation remain pending.

### Product transcript lifecycle — GREEN

Root ran `node --test unit/productTranscriptLifecycle.test.ts unit/productTranscriptLoading.test.ts unit/asyncFindProvider.test.ts unit/sharedDiffBounds.test.ts`: 34/34 passed, 563 ms. The previously reproduced import cursor, unregister work, canceled-reveal and displaced ordinary-page cases now pass alongside cache and diff bounds. Controlled transport/DOM adapters; mounted and backend acceptance still pending.

### Recap retry scheduler — RED

Root ran the `real visible poller` case in recapPolling4.test.ts: 0/1 passed, 254 ms. First failed revision poll scheduled at 4000ms instead of expected 8000ms. Uses actual pollWhileVisible and component callback with controlled timers.

### Workflow state — RED

Root ran `CARGO_BUILD_JOBS=2 cargo test -p otto-state --lib review4_ -- --nocapture`: compilation11.48s, tests0/2passed in0.51s. Actual state regressions reproduce: retimed one-shot trigger retains fired last_run; interleaved workflow publication live/run definition reports saved X against saved Y. Backend repair now authorized for these reproduced paths.

### Design metadata — RED

Root ran `CARGO_BUILD_JOBS=2 cargo test -p otto-design --lib metadata_ -- --nocapture`: 0/2passed (8.27s compile,0.50s tests). Aliased concurrent metadata patches lose acknowledged title; metadata/approval bypass canonical publication lock. Actual service/state tests, not mocked persistence.

### Product HTTP — RED

Root ran `CARGO_BUILD_JOBS=2 cargo test -p otto-product --lib http::tests:: -- --nocapture`: 10passed/14failed (30.97s compile,15.30s tests). Ten Jira/Confluence stale or missing reviewed-content cases publish once to isolated mock transport and return200 instead of409/zero; both unchanged-payload controls pass. Transcript legacy full-list control passes; summary transfers26,234,081bytes, cursor envelope missing, search/detail route/auth cases return404. Four transcript failures establish pending API repairs. No real external publication occurs.

### Recap retry scheduler — GREEN

Root ran `node --test unit/recapPolling4.test.ts`: 10/10passed,404ms. Actual poll scheduler now backs off failed requests and recovers normal cadence. RoomRecap replacement-identity mounted case authored but unrun, wrapper unchanged pending RED. Backend revision route remains absent.

### Server workflow and scheduler — RED

Root ran `CARGO_BUILD_JOBS=2 cargo test -p otto-server --lib review4_ -- --nocapture`:1passed/4failed (1m25s compile,2.44s tests). Old scheduled settlement consumes retimed once next_run_at; version-list default and requested bounds/cursor ignored, summary retains definition bodies. Full version detail and oldest restoration control passes. Role4 authorized to implement actual route/state repairs and scheduled-generation fencing.

### Broker replay — RED

Initial focused compile failed in authored fixture E0283 (redundant pool.into() for generic Into<DbPool>); root removed only that conversion. Rerun `CARGO_BUILD_JOBS=2 cargo test -p otto-brokers --lib replay_integrity_tests -- --test-threads=1`:0/5passed (12.24s rebuilt dependencies,4.51s tests). Actual MockCluster replay losesbinarykey, mutatesbinaryheader, replacesnulltombstone withpresentempty; public documented emptytexttombstone mismatch; 63ASCII+é preview panics atbyte64. No production Kafka or userdata involved.

### Vault backlinks failure state — RED/GREEN

New actual-store tests reproduce3/3failures (309ms): failedrefresh wipesknownlinks, no pending/error state, acceptednewnote retainsoldrows. Root added scoped backlinksLoading/backlinksError and generation/workspace guards; note/vault transitions clear old state; same-note failure preserveslastgood links. Combined `node --test unit/vaultStore.test.ts unit/vaultLookupRecovery.test.ts`:26/26passed,1.303s. Claude owns StructuredNote loading/error/Retry markup using exposed fields and reloadBacklinks(); no rendered acceptance until integration.

### Git aggregate diff — RED

Root ran `CARGO_BUILD_JOBS=2 cargo test -p otto-git --lib working_diff_stops_untracked_patch_spawns_after_aggregate_budget -- --nocapture`:0/1passed,19.76scompile/0.32stest. Counting realgit shim observes32/32patchchildren despite4KiBaggregate cap. Actualtemporaryrepository, no userdata.

### Combined UI gate and Canvas boundary repair

First npm run check: guardsOK, svelte-check1error0warnings; installedExcalidraw requiresnumericgridSize but restore returnednull. Root inspectedinstalledtypes/defaults(number20, separategridModeEnabledfalse). Strengthenedactualrestore/snapshot test failsmissingmode; normalizedlegacynull andpersistedmode; canvasRecovery9/9passed267ms. Secondcheck:svelte0/0, unitTS2345 onworkflowfixtureassertionflow-narrowednull. Rootconvertedactualerror toString forregex assertion(noassertionweakening); workflowpaging3/3passed284ms. Third full `npm run check` completedexit0, svelte0errors/0warnings, guards and app/node/e2e/unit TypeScript gatespassed. Source snapshot prior to furtherbackendcontracts andClaudeintegration; not finalacceptance.

### Workflow state publication/rearm — GREEN

Root formatted changedRustfiles then ran `CARGO_BUILD_JOBS=2 cargo test -p otto-state --lib review4_ -- --nocapture`:3/3passed,18.80scompile/1.47stests. Atomic live/run definition, firedonce rearm and completepublicationrollback on snapshotconflict pass. Serveractualroute and scheduledgeneration integration stillrunning; not fullbackendacceptance.

### Canvas grid-only autosave follow-up — RED/GREEN

Independent reviewer found dirty fingerprint omittedgridSize/gridModeEnabled. Root added actualonSceneChange→timer→commitPending→snapshotDoc tests:0/2RED245ms, scroll-onlynegativecontrolpasses. Includedpersistedgridfields incheapfingerprint; `node --test unit/canvasRecovery.test.ts`:11/11GREEN281ms. This covers staging serializedsnapshot; realExcalidraw/browser/backendpersistence stillpending.

### Workflow/scheduler actual routes — GREEN

Initial repaired run7/8passed (1m29compile/5.55stests); the failed restorecontrol was caused by newfixture always markingemptybody application/json. Root madeContent-Type conditionalonactualSome(body), preservingbodylessrestoreacceptance. Rerun `CARGO_BUILD_JOBS=2 cargo test -p otto-server --lib review4_ -- --nocapture`:8/8passed,20.54scompile/6.13stests. Covers routeoverlappingpatch/restore plus runadmission, publicationrollback, scheduledactualPATCHaway/back+recurring→once andstaleok/error/canceledsettlement, boundedthinversionpages/fullhistoryrestore. Independentbackendreview/fullaffectedgatespending.

### Recap old-path cost baseline — measured

Ran just-compiled `target/debug/deps/otto_server-848bda0d6fdc59fd --exact rooms::recap::archive::tests::full_detail_polling_cost_reads_unchanged_bodies_on_every_tick --nocapture`:1/1passed,0.80s. Per15unchangedpolls, eachfixturehad15eventopens/15draftreads/0indexbuilds. Serializedbytes:1event×2048payload=42,210;1×20,480=318,705;100×2048=3,267,585;100×20,480=30,915,600. Existingfull-detailbaseline only; no speedup orendpointacceptanceclaimed.

### State package integration coverage

`CARGO_BUILD_JOBS=2 cargo test -p otto-state --lib` completed in 215.25 s: **376 passed, 1 failed, 2 ignored**. The sole failure was an existing Proof timing budget: scoped summary took 115.16 ms under the full suite; first page took 21.82 ms. The exact same compiled test rerun alone with `--test-threads=1` passed in 0.16 s (first page 0.34 ms; scoped summary 1.73 ms). This supports a contention-sensitive timing failure, not a clean full-suite pass. No timing threshold or product code was changed; final nextest gate will use the repository's resource limits. All workflow/schedule/state functional cases in the broad run passed.

### Workbench history and recap HTTP — RED

Integration compilation first exposed an async `FnOnce`/`Send` lifetime error in the new Git stream. A reference annotation did not resolve it; changing stream items to owned file-path strings did. No raw patch bodies are cloned by that fix. Server integration then compiled in 2m48s with a macOS linker warning about the large `__eh_frame` section.

`cargo test -p otto-server --test it workbench_api::revision_history_exclusive_cursor -- --nocapture --test-threads=1`: 0/1 passed in 0.19s; requested7 rows but got205. Using the freshly built `it-eaf8d2698e3a3139` executable, the ignored50k history case failed0/1 in0.64s (50,000 returned versus default100), and authenticated recap revision route failed0/1 in0.20s (404 versus200; existing detail control passed first). These establish the still-missing backend protocols; no repair acceptance claimed.

### Broker replay repair and nullable-header follow-up

Root corrected only the new fixture generic (`Header::<&[u8]>`); initial fixture compile failed because `[u8]` was unsized. `CARGO_BUILD_JOBS=2 cargo test -p otto-brokers --lib replay_integrity_tests -- --nocapture --test-threads=1`: **5 passed, 1 failed**, 7.35s compile / 11.70s tests. Binary key/header, tombstone versus present-empty, documented public produce semantics, selected text transforms and Unicode preview now pass. Independent consumer confirms the remaining nullable-header defect: null becomes `Some([])`. Nullable-header production repair is authorized; no full replay acceptance yet.

### Git bounded collection follow-ups — mixed GREEN/RED

`CARGO_BUILD_JOBS=2 cargo test -p otto-git --lib working_diff_ -- --nocapture --test-threads=1`: **3 passed, 2 failed**, 16.21s compile / 1.04s tests. Original aggregate cutoff, tracked/untracked shared budget, and individually large file followed by visible small file now pass. New cases confirm discarded oversized patches still launch32/32 children, and a4KiB total cap with unlimited per-file cap still drains the full2MiB child. Separate raw-work budgeting and child capture cap repairs are authorized; no complete performance acceptance.

### Session resume — GREEN

`CARGO_BUILD_JOBS=2 cargo test -p otto-sessions --test isolation resume_ -- --nocapture --test-threads=1`: **3/3 passed**, 14.21s compile /0.49s tests. Covers live PTY preservation and owner/resource validation, unsupported/inactive/archived cases, and concurrent inactive opens starting exactly one fixture process. Mounted History acceptance remains pending.

### Reviewed publication UI — RED

After correcting unsupported TypeScript parameter-property syntax in the fixture, `node --test unit/publishReviewedContent.test.ts`: **0/4 passed**,249ms. Both Jira/RFC requests omit reviewed content identity; empty-version preview lacks its null identity/empty digest; publication409 does not invalidate the old preview. Repair authorized. Controlled transport invokes actual component functions, not mounted UI.

### Product publication and transcript HTTP — GREEN

`CARGO_BUILD_JOBS=2 cargo test -p otto-product --lib http::tests:: -- --nocapture --test-threads=1`: **24/24 passed**,10.90s compile /3.50s tests. Stale/missing review rejects before isolated external send; unchanged Jira/Confluence controls preserve payload. Transcript summary omits100 large bodies, stable cursor handles ties/new imports, Unicode search reaches older collapsed content, and summary/search/detail authorize owning story. Legacy full-list compatibility passes. Independent review, UI integration and runtime performance evidence remain pending.

First ottod MCP compile attempt crossed an in-progress Product service signature adaptation and stopped at two workflow callers; those callers are now updated and rerun is underway. This was a compile failure, not MCP behavioral RED.

### Reviewed publication UI — GREEN

`node --test unit/publishReviewedContent.test.ts unit/knowledgeRecovery.test.ts unit/publishDestinationOwnership.test.ts`: **17/17 passed**,410ms. Exact preview bytes and metadata are frozen into Jira/RFC requests; missing version binds null and empty digest;409 invalidates the old preview until explicit reload; previous recovery/destination protections pass. Workflow node usability with the new required identity remains under review before integration acceptance.

### Workbench MCP — RED

After Product workflow callers were adapted, `CARGO_BUILD_JOBS=2 cargo test -p ottod workbench_history_ -- --nocapture --test-threads=1`: **0/2 passed**,2m45s compile/.01s tests. Catalog has no integer history_limit field; actual handler omits requested limit/cursor from its HTTP request. MCP paging repair authorized with the already-confirmed bounded HTTP backend work.

### Design metadata integration and adjacent writers — mixed GREEN/RED

`CARGO_BUILD_JOBS=2 cargo test -p otto-design --lib -- --nocapture --test-threads=2`: **125 passed, 3 failed, 1 ignored**,18.98s compile/10.64s tests. Both original metadata lost-update/publication-lock regressions pass; existing Design service/HTTP/storage coverage passes. Newly authored actual importer and thumbnail cases fail: stale importer overwrites acknowledged title, alias thumbnail bypasses publication lock, stale thumbnail snapshot skips requested A→B→A restoration. Adjacent writer repair authorized after these reproductions. No complete metadata acceptance.

### Workflow displayed approval identity — UI RED

During Claude's heavy browser window, root ran only the306ms controlled-handler check `node --test ui/unit/workflowPublicationPreview.test.ts`: **4 passed,4 failed**. Full-preview loading, mismatched detail rejection/retry, departed-run ownership and ordinary approval-shape control pass. Product approve/deny omit displayed expected_detail_version, missing-display still submits, and409 leaves the old identity usable. UI repair authorized. Real same-run retry/approval API regression remains to execute after the heavy slot releases; no backend approval-binding acceptance.

### Additional source rechecks

Independent role2 approves repaired broker/Git/Workbench/MCP source with final green pending; root corrected an inaccurate process-group-on-drop comment, documenting direct-child cancellation separately from explicit overflow/timeout group kills. Exact omitted-file counts remain linear disk I/O. A repository-wide search found Workbench MCP tools only in the session ottod catalog, not a duplicate control-plane catalog.

Independent role5 source review found no new revision-protocol defect and retained the known unkeyed modal issue. Root added a ctime-only in-place rewrite test preserving inode, length and exact mtime, still unrun. Independent role3 approved inspected Product/Design repairs, but requires exact displayed-preview approval binding and template/guide migration.

### Product summary payload materialization — query-plan evidence

Independent reviewer traced the locked bundled SQLite source: length(CAST(body AS BLOB)) defeats its direct-column length optimization. Root extracted the actual summary SQL projection and ran tiny EXPLAIN on local Python SQLite3.53.4 (separate engine, explicitly not the compiled Rust runtime). Current body OP_Column P5=0 followed by Cast; candidate octet_length(body) uses P5=192 and no Cast. Root changed summary/search byte-length projections to octet_length; selected-body byte-size semantics are unchanged. Bundled-engine HTTP/regression and file-backed measurements remain pending. No physical disk-read or elapsed speedup claim from EXPLAIN alone.

### Workflow displayed approval identity — UI GREEN

Root reran the299ms handler suite `node --test ui/unit/workflowPublicationPreview.test.ts`: **8/8 passed**. Approve/deny carry the displayed detail version, unread preview cannot submit,409 clears stale approval readiness, and ordinary approval keeps its request shape. Backend atomic approval check remains intentionally unchanged pending the authored real HTTP retry regression; this is not end-to-end closure.

### Consolidated server follow-ups — actual RED/controls

Initial compile failed because the new workflow test module used wiremock without an otto-server dev dependency. Root added the existing locked0.6.5 test dependency (Cargo.lock gains only that package edge) and aliased its path matcher. Rerun `CARGO_BUILD_JOBS=2 cargo test -p otto-server --lib review4_ -- --nocapture --test-threads=2`: **14 passed,14 failed**,1m50s compile/11.41s tests.

All6 once pause/settlement cases fail by rearming the same occurrence. Six trigger-admission cases fail: stale retime/disable/away-back, competing same-snapshot claims, and failed run insertion consuming the cursor. Normal current admission and earlier atomic version/history/retime controls pass. Actual Product API same-run retry accepts stale P1 approval for P2 (200vs409), establishing backend RED. Product preview/template/forged/stale-content/ordinary approval controls pass; frozen-destination control hits a fixture request-count mismatch3vs1, requiring method/path triage before any pass claim. New repairs now authorized against these concrete reproductions.

### Recap revision backend — GREEN and measured counters

Using the exact just-built server executable `otto_server-86fc452d8a236401`, `recap_revision --nocapture --test-threads=2`: **5/5 passed**,0.17s. Permission parity, same-size external changes/removal, symlink rejection and ctime-only restored-mtime in-place change pass. Across15 cold unchanged polls, all four1/100-event ×2/20KiB fixtures recorded **0 event opens,0 draft reads,0 index builds**, with positive real-detail instrumentation control. Serialized bytes total10,545/10,560/10,635/10,665 respectively. These are protocol/counter measurements, not whole-app CPU/RAM or mounted acceptance.

### Consolidated automation/publication repairs — GREEN

`CARGO_BUILD_JOBS=2 cargo test -p otto-server --lib review4_ -- --nocapture --test-threads=2`: **30/30 passed**,1m19s compile/15.56s tests. All6 pause/settlement cases, retime/disable/away-back stale admission, failed queue insertion rollback, same/different-trigger overlap, stale captured cursor after a run finishes, atomic version/pinning/history and Product exact-preview approval/retry controls pass. Actual same-run P1 stale approval now rejects P2; current P2 approval publishes the exact isolated Jira/Confluence payload. Ordinary approval compatibility and migrated template pass. The corrected transport assertions account for one publication plus Confluence's two known full-width property writes, rejecting others. Combined UI/browser/full consumer gates remain pending.

### Design metadata and adjacent writers — GREEN

`CARGO_BUILD_JOBS=2 cargo test -p otto-design --lib -- --test-threads=2`: **128 passed,0 failed,1 ignored**,20.49s compile/10.99s tests. Original concurrent metadata/publication boundary and all three importer/thumbnail regressions now pass with existing HTTP/storage/Design coverage. This closes the named backend defect at tested scope; it does not imply rendered Design acceptance or the ignored benchmark.

### Broker replay final focused check — GREEN

`CARGO_BUILD_JOBS=2 cargo test -p otto-brokers --lib replay_integrity_tests -- --nocapture --test-threads=1`: **6/6 passed**,16.78s compile/12.17s tests. Nullable header now remains null and present-empty remains distinct, with all binary/tombstone/selected-transform/Unicode/public-produce controls passing through the isolated Kafka MockCluster. Independent source recheck recorded no additional defect. Broader package gates remain pending.

## Current grouped backend verification — 2026-10-05

`CARGO_BUILD_JOBS=2 cargo nextest run --lib -p otto-state -p otto-git -p otto-product -E 'test(/working_diff_|review4_|workflow|scheduled|workbench|http::tests::/)' --profile ci --no-fail-fast` completed successfully: **106 passed, 636 filtered/skipped**, 30.22s compilation, 5.523s test execution. Includes all five Git working-diff budget regressions, Product HTTP tests after the octet_length projection change, and broader scheduled/workflow/workbench state consumers. This does not replace the pending server HTTP/MCP, mounted UI, or representative load checks.

## Authenticated history and recap HTTP — current GREEN

`CARGO_BUILD_JOBS=2 cargo nextest run -p otto-server --test it -E 'test(/workbench_api::revision_history|rooms_api::recap_revision_http/)' --run-ignored all --profile ci --no-fail-fast`: **3/3 passed**, 125 skipped, 0.257s execution after 2m53s compile. Covers exclusive history cursor after new save and oldest restore, explicit 50,000-revision default/max page bounds with retained history, and owner-scoped recap revision HTTP with external draft changes. Existing linker warning about the large test binary __eh_frame appeared; execution exited 0.

## MCP history final GREEN

`CARGO_BUILD_JOBS=2 cargo test -p ottod workbench_history_ -- --test-threads=1`: **2/2 passed**, 88 filtered, 0.01s tests after 3m01s compile. Verifies exposed bounded catalog arguments and actual handler cursor forwarding/direct revision preservation. Mounted UI and current combined gates remain separate.

## Current combined UI gate

Initial `npm run check` passed guards and Svelte0errors/0warnings but failed the unit tsconfig: `publishReviewedContent.test.ts:66` retained a `null` narrowing across the fixture load method. The invalidated value is now captured in a local before asserting; the same stale-null and reloaded-digest assertions remain. Rerun `npm run check` **passed all guards, Svelte0/0 and node/e2e/unit TypeScript configs**. `npm run test:unit` then **passed1314/1314**, no skips/failures,14.876s. Raw unit log: `/tmp/otto-review05-unit-current.log`. This covers our current source before Claude integration and before the pending RoomRecap identity fix.

## Mounted acceptance setup

Fresh `CARGO_BUILD_JOBS=2 cargo build -p ottod` passed in9s (known large unwind-section linker warning). Recap first selection used an anchored grep against Playwright's full title and selected no tests; the command now uses the unique unanchored title. The selected case then could not launch because WebKit2359 was absent. Neither attempt reproduced the product defect. Installing the project's WebKit/Chromium browsers before retry; all runs use explicit isolated slots, ports and current-tree binary, with orphan sweep disabled.

## RoomRecap mounted RED established

After installing WebKit, the first launched attempt failed its old-archive control with unauthorized (route fixtures bypassed). The recap spec now blocks service workers, and the new case preserves the isolated runner owner token. The corrected ipad-landscape case then reached and passed old-first-event and held-old-page controls, and **failed at the intended new-archive first-event assertion** after the WebSocket identity change. Clock remains paused to require immediate ownership reset. Root added only `{#key room.recap.id}` around RecapPanel inside the existing modal. Exact same regression GREEN rerun is in progress; no pass claimed yet. RED output `/tmp/otto-review05-recap-red.log`, trace/screenshot `/tmp/otto-review05-recap-identity-red`.

## RoomRecap mounted GREEN

Exact same corrected ipad-landscape identity case **passed1/1**,1.4s test/2.8s run, after the single keyed-child production change. It verifies B starts at its first event while A page2 is held and a late A response cannot replace B. Independent narrow review approves lifecycle/publication fencing; no request-abort claim. GREEN log `/tmp/otto-review05-recap-green.log`, output `/tmp/otto-review05-recap-identity-green`. This closes the known recap modal identity defect at mounted WebKit scope; broad native/manual acceptance remains separate.

## Focused desktop journeys

The six-spec serial desktop-browser invocation from iteration-4-browser-queue.md completed **10 passed /2 failed**,28.5s. Both failures were assertion defects: the Insights alert locator matched the inline retry error and a toast, and the History anchored text regex rejected an icon's leading whitespace before reaching navigation. Narrowed the first to the alert containing the Retry button and used exact normalized text for each expected History action. Reran **only those two cases:2/2 passed**,5.8s. Thus all **12 distinct authored desktop cases have current passing executions**, plus the separate1/1 WebKit recap identity case. Logs: `/tmp/otto-review05-browser-acceptance.log`, `/tmp/otto-review05-browser-rerun.log`. The five History statuses run inside one case, not five extra cases. This is affected-journey coverage, not the full desktop suite/native acceptance.

## Full-gate contract inventory finding

Workspace rustfmt and strict all-target clippy passed. The4,696-case nextest run is still collecting results; route_inventory::every_registered_route_is_documented failed because new workflow test router used `/workflow-runs/{id}/nodes/{node}` whereas production/docs use `{node_id}`. This is an internal test fixture literal captured by the source scanner, not a missing production endpoint contract. Aligned the fixture with the actual production route template (no behavior/assertion removal, no invented API documentation). Focused rerun pending after the promised film handoff; current full run remains a failed run even after this source repair.

## Full affected gate result and handoff

`CARGO_BUILD_JOBS=2 NEXTEST_PROFILE=ci scripts/check.sh --check --base 03f2bc3e` passed workspace rustfmt and strict all-target clippy (1m41s). Nextest compilation1m47s; execution221.835s: **4694 passed,2 failed,86 skipped** out of4696 executed. Besides the route fixture mismatch above, MCP tools/list measured82,012bytes against the unchanged82,000budget. Shortened the newly expanded Workbench description without removing pagination fields/limits/continuation caveat; budget unchanged. Both repaired checks require rerun. Script exited100 before doc-tests/UI phase; those phases are not claimed green here. Heavy slot released to Claude immediately on completion; no further builds/tests before film release. Full log `/tmp/otto-review05-integration-gates.log`.

## Main design integration (source checkpoint)

Fetched main515a8f80 containing designPR79/80/81. It adds no Rust/Cargo changes relative to03f2bc3e. Merge produced ten UI conflicts. Resolutions preserve originating-operation/approval/draft/pagination guards alongside main copy, Skeleton/LoadState, focus/ARIA and URL-selection presentation. Five candidates were prepared by the session implementer and independently inspected; root resolved the other five. Actual conflict files were byte-checked against the prepared merge before applying candidates. Added supplied Vault backlinks presentation patch and corrected one imported Overview toast typo. No conflict markers or whitespace errors remain. Integrated type/unit/browser acceptance is **pending** during Claude's exclusive film slot; source merge is not runtime acceptance.

## Iteration5 source follow-up queued

Independent C1 and root source trace confirm History resume has no lifecycle/context fence before follow-up navigation: a response held until after leaving History calls openInChat and returns to the old session. Added a mounted same-document navigation regression to desktop-review4-session-recovery.spec.ts. Test is **unrun**, production unchanged pending actual RED after film release. This is a new confirmed issue in an existing repaired flow, not a claimed passing fix.

## Iteration 5 merged source — 2026-10-05 07:58 local

- Repaired full-gate failures now GREEN: route_inventory::every_registered_route_is_documented (1/1, 0.374s) and MCP tools_list_catalog_stays_within_its_byte_budget (1/1, 0.02s). No catalog budget relaxation. Initial mistakenly library-scoped route selection ran zero tests and is not evidence. Logs: /tmp/otto-review05-{route,catalog}-gate.log.
- Mounted History departed resume: expected RED (late completion navigated back to Agents), then GREEN after lifetime/context guards.
- Mounted Product departed publication: first fixture selector matched both titles; corrected exact title selector, then expected RED (B lost active selection after A response), then GREEN after caller lifetime + store selection ownership fence. Existing store publication behavior unchanged. Both cases GREEN together, 2/2 in 5.1s; /tmp/otto-review05-departure-green.log.
- Full merged UI check/unit and complete focused browser controls still pending. Historical pre-merge results are not substituted.

- Merged unit run: initial 1292/1330 passed,38 failed due missing fixture imports (labels/plural/toastAgentEdit/toastError), not removed assertions. Updated adapters to actual helpers. New PersonalAgents list ABA/stale-loading tests established 3/3 RED; request-generation fix applied. Full rerun GREEN **1333/1333**,15.07s, /tmp/otto-review05-merged-unit2.log.
- Merged desktop run **14/15 passed**: only mixed session batch stopped before exercising behavior because New session tooltip changed; test updated to exact accessible button name. Includes ordinary History import/retry and Product exact content conflicts, destination recovery, departed publication and reopened-dialog A→B→A. Rerun pending; /tmp/otto-review05-merged-browser.log.
- Scheduled admission RED: **3 control passes /5 intended failures** (disable, retime, both ABA and same-snapshot concurrent admission),1.99s. Actual open_run boundary with database rows/events, /tmp/otto-review05-schedule-red.log. Repair delegated; no GREEN yet.

- Merged UI clean check GREEN: ui-guards1014files + svelte-check0errors/0warnings + node/e2e/unit type-checks, /tmp/otto-review05-merged-ui-check3.log.
- Production UI build GREEN6.63s; bundle budget GREEN without increasing budgets, /tmp/otto-review05-merged-ui-build.log and /tmp/otto-review05-merged-bundle-budget.log.
- Mixed session batch exact accessible-selector rerun GREEN1/1,4.5s. All15 distinct merged desktop cases have now passed, preserving initial failed attempt rather than claiming a single15/15run. /tmp/otto-review05-batch-rerun.log.

- Scheduled atomic admission and prior controls GREEN57/57,9.67s (/tmp/otto-review05-schedule-green.log). Follow-on0174 partial running-task index is verified with actual0084 schema+EXPLAIN in isolated in-memory SQLite; it changes task-history index traversal into a covering lookup on running rows. Rust migration rerun queued.
- Additional mounted API Automation Cancel/failed Save/successful Save/Discard case GREEN; publication reopened-dialog capture GREEN,2/2total7.2s (/tmp/otto-review05-visual.log). Inspected screenshots in screenshots/publication-{desktop,phone}-{light,dark}.png: labels, content preview, destination text and footer controls readable/reachable, dialog clamped at1440×1000 and390×844 in both schemes. Scope is this dialog; no whole-app visual acceptance inferred.

## Runtime checkpoint64a850e6 — complete pre-measurement checks

- `cargo fmt --all --check` GREEN; workspace/all-target `cargo clippy ... -D warnings` GREEN43.21s.
- After0174partial-index migration, scheduled admission+prior workflow/task controls GREEN57/57,9.79s. /tmp/otto-review05-schedule-index-green.log.
- `cargo test --workspace --doc` GREEN;33library targets each report0doc-tests (not33testcases). /tmp/otto-review05-doctests.log.
- Fresh `cargo build -p ottod` GREEN1m50s. Known large __eh_frame linker warning retained. /tmp/otto-review05-perf-build.log.
- UI check again GREEN0errors/0warnings including newly authored child-allocation measurement spec. /tmp/otto-review05-preperf-check.log.
- Performance run source/hash provenance root: /var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-review05-perf.aukj8tbd. Runtime diff to64a850e6 empty; remaining dirty files are evidence and test-only workload. Measurements pending.

## Performance measurement progress (runtime64a850e6)

- Actual mounted child history measurement passed1/1 (13.1s case/18.2s run):100 inspections and1200-turn bidirectional paging, zero retained child bodies at GC checkpoints. Inspection p50 74.27ms /p95 82.78ms; no recorded long tasks. Heap20.71MiB warm →21.06MiB after100 →23.36MiB after paging; all Chromium processes RSS1204.89→1265.28→1297.11MiB. The RSS increase is unassigned; this does **not** establish absence of a leak. Raw child-allocation.json in the provenance root. A second100-cycle recovery extension is authored but unrun.
- Read-only actual-app sample completed25 samples/120s: otto-desktop CPU mean3.27% /max4.1%,RSS100.80→100.81MiB; the15 matching ottod processes (main+bridges, not15 agents) total CPU mean1.64% /max2.8%,RSS258.34→256.77MiB. Excludes unattributed native WebKit/GPU processes and is the installed app, not the candidate revision. Raw real-app-process-samples.jsonl.
- First isolated scale attempt stopped at167.2s,N3focus: host load12.7 exceeded unchanged12 cap; available31.68GiB,swap growth0. Teardown left0 test processes. N5 was not reached; partial results are not acceptance. Teams and Spotlight were high CPU in a follow-up observation; causality is not established. Aborted raw output retained in scale/.
- Load settled7.67 and a same-limit retry started08:26 in scale-retry/. Sustained run still pending. No production source edits during measurements.

- Same-limit scale retry **completed exit0 in375.4s,66samples,0teardown leftovers**. Baseline0→N1→N3→N5→recovery completed, headedChromium/defaultWebGL. Mean steady CPU atN5: daemon2.3–2.6%,renderer12.7–14.8%,ClickHouse1.2–1.3%,GPU15.1–21.0% dependingview. Separate groups must not be mistaken for total-app CPU. Post-GC heap17.2MiB baseline→32.0MiB N5; daemonRSS162.7→272.4MiB;renderer530.1→712.4MiB. Recovery returns CPU nearidle, but RSS remains elevated; sustained memory acceptance remains pending. Raw and analyzer in scale-retry/,scale-retry-analysis.txt. Synthetic agents, not real provider workload.
-15minute sustained3-agent workload started08:32local under identical source/UI/safety limits, output sustained/. No passing result yet.

- Sustained run **incomplete**,exit1:safety abort at413.9s on hostload12.03>12;available32.16GiB,swap-8MiB. Teardown0leftovers418.6s. Full15minute memory acceptance not met; partial raw retained. Runtimefreeze released08:40 for final bounded fixes/acceptance.

## Final bounded browser acceptance and two follow-up repairs

- New History deferred page-readiness case established mounted RED: accepted Settings hash was replaced by Agents before its page could mount. Production guard now watches pendingTarget and synchronously compares originating hash; same mounted case GREEN plus13/13unit controls (2new boundary cases were intendedRED). Initial unit command accidentally requested absent tsx; corrected to repository node --test command before recordingRED.
- MCP Audit title-only identity/error information reproduced as absent keyboard/touch disclosure. Added native details/summary with wrapping full identifiers, decision reason and error. Same mounted case GREEN with keyboard activation and phone theme captures. Both repairs GREEN2/2,4.5s, /tmp/otto-review06-boundary-green.log.
- Expanded acceptance **18/18GREEN,1.4min**:8session/history cases (including200mountedchildinspections+1200turnpaging),WorkbenchfailedolderRetry/concurrentinsert/back/restore,3Canvasassistformats failed/empty/acceptedprompt,3PersonalAgentsmountedABA,scheduleddeep-link/failedSave/newerinput/Keep/finalpersist,sharedDiff50k/10kpages/fullsource downloads,VaultbacklinksfailureRetry. /tmp/otto-review06-acceptance.log. Numeric observations in PERFORMANCE.md/measurements; no memory plateau claim.
- Recovery follow-up **7/7GREEN,18.9s**:2environment Save controls (newer values plus double secret rename; clean A→B→A),Canvas3MBsave+versionrestore,VaultfailedopenRetry/stalefailure,Vaultfailedsave retainsnewtyping,Canvasfailedpendingdraft acrossscenes andExcalidrawfailedhandedit recovery. /tmp/otto-review06-recovery.log. No real network request or production secret used.

## Closing integration checkpoint — 2026-10-05 09:00 local

- Product explicit deep link initially opened the newest story instead of its target (actual mounted assertion failure). The route ran before workspace initialization, whose reset then cleared selection without rerunning the route. Commit `552b6f84` orders workspace initialization before selection, subscribes routed selection to the workspace, and suppresses premature URL write-back. Same uninstrumented initial-link / Keep editing / Discard regression passed **3/3 repeats**, 6.6 seconds; `/tmp/otto-review06-product-green.log`. Temporary diagnostic reads changed reactive dependencies and were removed before the passing run.
- Combined RoomRecap revision-failure / Retry / held old-page / new archive identity case passed **1/1 in WebKit**, `/tmp/otto-review06-recap.log`. MCP Audit disclosure passed with full server/tool/error text, keyboard operation, phone bounds and inspected light/dark captures; `/tmp/otto-review06-audit-capture.log`.
- Integrated main `9ab214dd` (PR82) in merge `0c4862ab`, with no conflicts. Its version/tour changes postdate the `64a850e6` runtime measurements; those measurements are not represented as an exact final-head benchmark.
- Final integrated UI check passed **0 errors / 0 warnings** (1,014 guarded files plus app/node/e2e/unit type checks); **1,335/1,335 unit tests** passed in 14.35 seconds; production build passed in 6.50 seconds. Existing browser-externalization warnings remain. Logs: `/tmp/otto-review06-integrated-{ui-check,units,build}.log`.
- Bundle budget passed without increasing thresholds: `/tmp/otto-review06-integrated-bundle-budget.log`. The initial command mistakenly named a nonexistent npm `budget` script; the repository's actual `node scripts/bundle-budget.mjs` command was then executed successfully. No missing-script attempt is counted as verification.
- Full desktop suite was not rerun by this continuation. Focused current-source journey results above and Claude's baseline-diffed design suite remain distinct. Native physical-device and complete sustained-memory acceptance remain open.
