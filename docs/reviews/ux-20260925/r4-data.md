# Round 4 — Data tools, reviewer 6 (final round per updated user instruction)

Worktree only `/Users/itziklavon/otto-ux-audit-20260925`. Scope Database/API/Brokers/Connections/NetworkProfiles. Read prior R1–R3 reports, protocol, parent notes, AGENTS and design guidelines. No commits/subagents. Parent granted database-specific store ownership. No real DB/broker/SFTP mutations or provider-home writes. Daemon state/workspaces are disposable; external service operations intercepted with synthetic fixtures.

## Independent prior verification

`desktop-ux-data.spec.ts`, `desktop-ux-r2-data.spec.ts`, `desktop-ux-r3-data.spec.ts`, `desktop-network-profiles.spec.ts`, and `desktop-ux-r4-parent-api.spec.ts`: **40 passed (3.0m)**, `/tmp/otto-ux-r4-data-prior.log`, result `/tmp/otto-ux-r4-data-prior/.last-run.json` passed. This rechecks prior SFTP activation/phone, API response/RTL and delayed initial discovery, replay validation, consumer-group detail/reset races, saved/history retry/scope races, query save/tab and RTL keyboard, schema tablet focus, and network validation/conflict/workspace isolation. Parent's empty saved-list/typed URL negative finding reproduced; no API lifecycle patch was made.

## Confirmed repairs

1. **P2 — repeated query saves while pending.** `ui/src/modules/database/QueryEditor.svelte:527,542,1102`. Hold POST, press Enter twice in the name field: the Save button remained enabled. Added handler-level pending guard, disabled Save/Save-as-new, and visible Saving label. Red `/tmp/otto-ux-r4-data-red`; green in 17-test final run.
2. **P2 — connection switch dropped the saved-query association.** `ui/src/lib/stores/database.svelte.ts:3418`, persistence `:1454`. Save Primary query, switch Alternate, release save, return Primary: title stayed SELECT and toolbar offered Save instead of Update. Locate initiating tab across live and parked connection snapshots, update that exact tab, persist its owner's tabs. The alternate query remains untouched. Red `/tmp/otto-ux-r4-data-green1`, green `/tmp/otto-ux-r4-data-deep` and final.
3. **P2/P3 — embedded Network Profiles nested scrolling/composition and endpoint focus.** `ui/src/modules/connections/NetworkProfiles.svelte:19,25,110`. In the actual New Session phone sheet, manager's 60vh cap left **320px** hidden inside a second scrollbar. Removed the internal viewport; host sheet owns scrolling. Endpoint fields use an available-width grid, shared buttons/tokens, 36px phone controls and selected profile semantics. Add focuses the new endpoint; remove focuses the next/previous surviving endpoint. No second primary competing with Start session. Eight endpoints can be filled/saved/removed by keyboard on phone and tablet RTL. All five desktop themes plus actual embedded phone/tablet inspected, replacing prior standalone-only evidence.
4. **P2 — Kafka Consume invalid selectors.** `ui/src/modules/brokers/TopicDetail.svelte:165,281,539`. Blank From time enabled Peek and substituted current time. Added inline validation and handler guard: explicit finite timestamp; safe nonnegative integer offset; integer message limit1–5000. Removed timestamp fallback. Valid consume and failed-produce draft/retry complete against mocked transport.
5. **P2/P3 — Schema Registry tablet content collapsed to 92px and JSON inherited RTL.** `ui/src/modules/brokers/SchemaTab.svelte:20,119` and `SchemaVersionsPanel.svelte:151`. Actual834px WarmRTL screenshot shows 300px subject list consuming nearly all available pane; JSON fragments wrapped backwards. Container query now stacks list/detail below760px available width; detail remains >300px, list capped and scrollable, JSON/candidate source LTR. Subject view tabs have roving arrows/Home/End, respecting RTL. Versions error→Retry, diff, and compatibility check verified with faithful fixtures. Candidate textarea now labelled.
6. **P3 — long SFTP transfer identity unavailable on touch.** `ui/src/modules/connections/SftpBrowser.svelte:252,542`. Phone screenshot showed only `Download /srv/cust…` despite available width. Transfer path now wraps across full phone row and remains LTR. Cancel status and navigation persist; all transport writes intercepted.

7. **P2/P3 — Kafka phone retry obscured by error toast; message inspection pointer-only.** `ui/src/modules/brokers/TopicDetail.svelte:82,371,568,758` produce error/offset controls. WebKit failed clicking the Produce button to retry because the persistent error toast covered it; moved actionable failure beside the form, preserving all fields. Message rows had only `tr.onclick`, no keyboard control; new native offset button announces partition/offset and supports Enter/Space. Dedicated red test confirms missing control; final dual-engine recheck records outcome below.

## Deeper main flows verified

New `ui/e2e/desktop-ux-r4-data.spec.ts` adds17 tests: the repairs above; all seven embedded profile theme/device variants; Kafka consume/produce failure+retry; SFTP long-transfer cancellation; API SSE connect/event/disconnect; WebSocket receive/send/disconnect; gRPC reflection→failed invoke→retry→JSON response; schema registry versions/compatibility; Redis value edit→native SET review and Mongo array edit→native updateOne review. DB mutation reviews are cancelled; query interceptor asserts only original reads reach it.

Initial fixture corrections: reopening Connections is necessary after switching a selected DB because it returns to Schema; waiting for profile Save response completion is necessary before focusing the temporarily disabled Remove button. These were test corrections, not relaxed product expectations. A non-reactive endpoint DOM-ref warning was fixed by using `$state`. The first combined deeper run restored a prior synthetic broker during later page navigation, attempting DNS resolution of `fixture.invalid` (no real host reached). Added earliest per-test fallback interception for all broker descendant transports; specific faithful routes override it.

## Verification commands

Common prefix from `WT/ui`:

```sh
OTTO_E2E_SLOT=ux4data OTTO_E2E_PORT=7866 OTTO_E2E_PW_PORT=5366 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod
```

- Prior40 suite above passed3.0m.
- New initial red five checks:4failed/1passed1.6m;3 genuine defects plus connection test navigation fixture. `/tmp/otto-ux-r4-data-red.log`.
- New full17 checks: **17passed1.8m**, `/tmp/otto-ux-r4-data-final.log`; `.last-run.json` passed. Includes final schema composition and Redis/Mongo review flows.
- Entire new17 suite on actual `iphone-portrait` WebKit (the broad grep matched the project name):15passed,2failed3.0m. `/tmp/otto-ux-r4-data-webkit.log`. One was desktop-only connection picker selector; corrected helper now uses phone Connections accordion. Other was actionable failed-produce toast obscuring Retry, repaired inline as finding7.
- `desktop-ux-r4-data.spec.ts --grep 'saved query response|Kafka consume' --project=desktop-browser --project=iphone-portrait`: **4passed1.1m**, `/tmp/otto-ux-r4-data-last.log`, `.last-run.json` passed. Both initial WebKit failures resolved; all17 deeper behaviors have passing WebKit evidence across the full run and this focused rerun. Keyboard inspect also passed in both engines.
- Dedicated missing-inspect-control red:1failed as expected (`Inspect partition 0 offset 0` absent), `/tmp/otto-ux-r4-data-keyboard-red.log`.
- `desktop-ux-data.spec.ts desktop-network-profiles.spec.ts desktop-api-run-recovery.spec.ts desktop-database-changes.spec.ts --grep-invert PKCE --project=desktop-browser`: **10passed32.0s**, `/tmp/otto-ux-r4-data-regression.log`, `.last-run.json` passed. Confirms old profile conflict/save/workspace and SFTP checks remain green, plus automation dataset/report persistence across reload, stop-on-failure, cancellation, authorized SSE relay and DB change draft→validate→submit to review. Automation uses its own throwaway local HTTP server and disposable data; PKCE excluded because irrelevant to scope.
- Final36px touch target/profile CSS verification: `desktop-ux-r4-data.spec.ts --grep 'Kafka consume|embedded network profile.*endpoints phone$' --project=iphone-portrait`: **2passed15.3s**, `/tmp/otto-ux-r4-data-touch-final.log`, `.last-run.json` passed. Final phone Kafka screenshot viewed.
- `npx tsc --noEmit -p tsconfig.e2e.json` passed, `/tmp/otto-ux-r4-data-tsc-final.log` empty.
- `node scripts/ui-guards.mjs` passed805files/no new debt, `/tmp/otto-ux-r4-data-guards-final.log`; no baseline increase.
- `git diff --check` passed. Parent owns full npm check/build/CI/native install.

## Rendered evidence viewed

Fresh prior-run images in `/tmp/otto-ux-r2-data-screenshots/`: native-light database; native-dark API/SFTP; warm-light SFTP; warm-dark database; pro-dark API/replay; phone database/API; tabletRTL brokers. R3 component phone image was freshly regenerated and viewed as before-composition evidence.

`/tmp/otto-ux-r4-data-screenshots/`: actual embedded profiles native-light/dark, warm-light/dark, pro-dark, phone, tabletRTL all viewed; tablet schema before/after; Kafka consume; SFTP cancelled long transfer before/after; SSE, WebSocket, gRPC, Redis review, Mongo review all viewed with view_image; actual WebKit phone protocol/editor/review screenshots were also viewed. Some filenames are refreshed by later successful runs; final screenshots represent those executions, not acceptance of prior reports. These use synthetic values and contain no host transcript/account content.

## Scores /10

Scores judge the rendered, executed browser flows, not untested feature completeness. Repairs earn their change from prior9.4 composition by real embedded/available-width evidence; no score is increased merely because this is the last round.

| Page family / flow | Layout/readability | Interaction | Accessibility | States/recovery | Responsiveness |
|---|---:|---:|---:|---:|---:|
| Database query/results/saved/history + schema |9.5|9.6|9.5|9.6|9.5|
| Redis/Mongo result edit → review/cancel |9.5|9.5|9.5|9.5|9.5|
| API HTTP/JSON |9.5|9.5|9.5|9.5|9.5|
| API SSE/WebSocket/gRPC |9.5|9.5|9.5|9.5|9.5|
| Kafka groups/topics/replay |9.5|9.6|9.5|9.6|9.5|
| Kafka registry schema/versions/compatibility |9.5|9.5|9.5|9.5|9.5|
| Connections SSH/SFTP + transfer cancel |9.5|9.5|9.5|9.5|9.5|
| NetworkProfiles actual New Session host |9.5|9.6|9.5|9.6|9.5|

Important variants: five-theme desktop HTTP/SQL/group/SFTP flows retain9.5 after independent recheck; new all-five-theme embedded profile desktop9.5, phone375×812NativeLight9.5, tablet834×1112WarmDarkRTL9.5. Registry's specific tablet834WarmRTL9.5 after92→394px detail repair. API protocol and Redis/Mongo review native-light desktop/phone9.5; their other themes were not separately exercised in these new flows. Rounded9.5 reflects coherent layouts and successful primary interactions, not a claim of perfection or exhaustive engine coverage.

## Limits

Browser UI fixtures establish UI behavior, not production service success. No physical VoiceOver/native Tauri check, real DB/broker changes or remote SSH transfer was performed. DB dashboard/ERD authoring, saved-query UPDATE/DELETE races beyond the verified create/switch path, all Redis collection command families, Mongo whole-document replacement, Schema Registry multi-subject async races and every API automation authoring variant were not exhaustively exercised. These are unverified coverage boundaries, not asserted defects. No known confirmed feasible defect remains in the repaired/rechecked scope. No claim of whole-app completion.

Final handoff: all scoped tool processes exited; last relevant browser runs and local type/guard/diff checks passed. Parent owns aggregate checks, review, integration and installation. No commits were made.

## Final R4 parent-review followup (same round)

Parent review correctly identified an introduced pointer regression in TopicDetail's native offset-button repair. Reproduced by clicking the key cell: the pane remained “Select a message”. Restored the whole row as a pointer target using tbody pointer-up delegation while preserving native table semantics and the offset button's keyboard action. Delegation handles only the primary button, excludes nested buttons, and preserves noncollapsed text selections. Executed key/partition/nested position-bar clicks, secondary-button no-op, desktop mouse text drag, Enter and Space selection with distinct payload/aria-pressed assertions. The table contains Key rather than a Value column; tests assert the associated Value in its detail pane.

SchemaTab's delegated tablist keyboard handler triggered a Svelte focus warning. Added tabindex=-1, keeping the tablist out of sequential Tab navigation. Both browser engines retained selected-tab focus and RTL arrow navigation. Direct Svelte compiler reports zero warnings for SchemaTab and TopicDetail after the final changes.

Parent also found the existing live-tail masking omission. Rust service.rs1178 masks only when req.mask==Some(true), and the incremental request omitted this flag. Timer-driven red test confirmed the second request lacked mask:true. Incremental requests now propagate the current Mask toggle; merged results retain the masked badge only while all appended nonempty batches were masked. The regression advances the browser clock 60 seconds twice: masked seed → masked incremental cursor → explicit Mask off with mixed results and no misleading masked badge. Only synthetic responses were used; no external Kafka request was sent.

Followup evidence:
- `/tmp/otto-ux-r4-data-row-red.log`: first attempt failed at page.goto due slow cold Vite load, not a behavior assertion.
- `/tmp/otto-ux-r4-data-row-red-retry.log`: confirmed key-cell interaction failure with screenshot/trace under matching output directory; screenshot viewed.
- `/tmp/otto-ux-r4-data-row-green.log`: 4 passed (3.2m), both engines, initial pointer/schema verification.
- `/tmp/otto-ux-r4-data-mask-red.log`: 1 expected failure (15.3s), incremental outbound mask missing.
- Final stable-source run: `OTTO_E2E_SLOT=ux4data OTTO_E2E_PORT=7866 OTTO_E2E_PW_PORT=5366 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod npx playwright test e2e/desktop-ux-r4-data.spec.ts --grep 'Kafka consume|Kafka schema tablet|Kafka masked' --project=desktop-browser --project=iphone-portrait --workers=1 --timeout=180000 --output=/tmp/otto-ux-r4-data-final-followup`: **6 passed (1.0m)**. Log `/tmp/otto-ux-r4-data-final-followup.log`. The temporary command timeout accommodates observed cold Vite load; the suite's default timeout was unchanged.
- Final E2E TypeScript check passed: `/tmp/otto-ux-r4-data-followup-tsc.log`; UI guards passed805files: `/tmp/otto-ux-r4-data-followup-guards.log`; git diff --check passed. No full npm check or baseline edit.
- Refreshed Kafka selection and Schema tablet screenshots were viewed after the final run; earlier durable screenshot recommendations remain valid.

The spec now contains18 tests (one added masking regression). Six relevant followup executions passed on the final source; this does not claim all18 were rerun after the narrow broker followup. Scores and broader coverage limits above remain unchanged. All followup test/check processes finished; parent owns aggregate npm check and integration. No commits.
