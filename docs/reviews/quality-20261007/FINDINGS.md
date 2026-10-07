# Finding-to-evidence ledger

Baseline: `196048df`. Source-traced findings are not represented as executed reproductions. Tests can be shared by multiple review lenses; counts are not added across overlapping reports.

| Finding | Owner/task | Baseline evidence | Repair verification |
| --- | --- | --- | --- |
| C1 / T1 Redis selected-only deletion | Data editor | 3 unsafe list operations; refusal feedback absent in both UI entry points | 22 combined telemetry/Redis unit checks pass; real Redis deletion deliberately refused |
| C2 / T2 recovered task snapshot | Scheduled | Fresh-context recovery tests failed old/new generation assertions | Focused Rust selection101pass includes edited destination/generation, legacy/malformed snapshot |
| C3 / T3 report identity/retention | Scheduled | Two reports collide; prune returns retained path; ownership EXPLAIN scans history | Focused101pass; indexed ownership regression1pass; full affected-consumer gate including0178–0180 migration compatibility passes |
| C4 / T4 client ingest naming | Telemetry | Existing full-chain browser fails HTTP400; producer regressionfails | Producer/validator checks and full repaired browser→collector→ClickHouse parent chain pass; export-throttle-aware browser test verifies accepted batch counts |
| Performance export blocks sampling / T9 | Telemetry | Blocked-export regression fails cadence | Focused tests and all four ignored lifecycle cases pass; full stored trace passes; 16.9-minute load records collector resource buckets during every externally observed export minute, with zero drops/send failures |
| D5 trace empty/retry | Telemetry | Empty/retry browser cases fail | Three browser cases pass incl close-pending; light/dark screenshots inspected |
| D1 School native button keys | School | Mounted native Back enters session on baseline | Final animated Enter/Space journeys pass; held-key blur cleanup and focus regressions pass |
| D2 School invisible tab stops | School | Source-traced hidden focus | Keyboard companion browser passes; inspected visible focus screenshot |
| D4 School initial loading/error | School | Initial loading missing on baseline | Initial loading/error/Retry browser passes; loaded refresh failure retains scene/list and Retry recovers |
| UX-03 School overflow membership | School | Model45vs51; DOM42vs46; new106-row pagination regression fails | Model checks pass; desktop/phone46rows/menu and106-row pagination browser regressions pass |
| UX-04 / NS4 School room restoration | School | Source trace (duplicate identified) | Saved-room browser restoration passes; restore precedes persistence |
| NS3 School cloned skeleton disposal | School | Owned helper missing; runtime magnitude unmeasured | Actor ownership units pass; real Three.js ten-cycle plus individual-removal disposal regression passes |
| NS5 School hidden-screen polling | School | Hidden in-flight request not aborted; hidden mount12requests | Visibility/abort/resume focused units pass |
| UX-01 / T6 API environment draft leave | Data editor | Both new leave-dialog browser cases fail | Four browser cases pass incl prior A-B-A coverage |
| D3 / T7 plugin nested overlays | Plugin | Nested Escape fails; later popover Tab edge fails | Focused15pass includes both; corrected test stub; subsequent full plugin suite470pass,2known skips |
| UX-02 / T7 plugin settings drafts | Plugin | Refresh loses drafts; later project-map scope edge fails | Focused15pass plus delayed post-save people/overview draft preservation regressions pass; independent closure |
| UX-05 / T7 plugin stale scan status | Plugin | Failed poll lacks stale state/Retry | Focused15pass includes stale status recovery |
| UX-06 plugin Stop | Plugin | Stop404; Jira/backoff/estimate cancellation and queued-account cases fail | Focused15pass includes stop/retained results/restart; accepted host agent limitation documented |
| NS1 plugin deploy-tag repeated quadratic work | Plugin indexing | Separate-worker cache regression fails | Focused15pass includes separate workers, invalidation, cachebounds/corruption; cold scan limitation documented |
| NS2 plugin stale account/scope responses | Plugin | Late account/scope browser cases fail | Focused15pass; stale prClient reference repaired; subsequent full plugin suite470pass,2known skips |
| Performance hourly archive materialization | Archive | Executable stale-history test returns0 vs10; source fullpayload scan | Bounded ID-only pages and partial covering index; manager/repository archive regressions pass |
| T8 login-handler regression gap | Auth tests | Existing tests duplicate wiring; no production bug asserted | Actual handler/router production-wiring regressions pass (3cases) |
| Plugin estimate producer/UI contract | Plugin | Accuracy producer omitted bins and correction candidates; browser case silently skipped | Three pure regressions pass; mandatory real All time correction and persistence browser passes; final full plugin suite 482 pass, 0 fail, 1 intentional skip |
| Plugin All time metric window | Plugin | UI All time retains all completed records but derived metrics substitute last 90 days; mandatory correction browser fails on old history | Closed: shared record-derived window, exact explicit cutoffs, bounded empty history, team/person equality and global-cache isolation verified; independent correctness and test reviewers confirm closure |

## Baseline commands already completed

- UI type/style gate: pass, zero errors/warnings at current baseline.
- UI unit suite: 1,686 passed, zero failed/skipped.
- Plugin `node --test --test-concurrency=1 test/*.test.js`: 453 passed, zero failed, two skipped, 62.62s. The fixture rendered no estimate-correction action, and one light-theme token case is intentionally skipped under dark; these are not passes.
- Updated daemon build: pass, reduced-debug dev profile, 5m08s after dependency warmup.
- Telemetry full-chain browser case: one executed, one failed. Existing test catches current defect but is advisory in CI.

Exact logs and raw browser traces live under `/tmp/otto-quality-20261007-*`; final durable summaries and selected evidence will be retained in this directory. Final affected-consumer gate and real telemetry/load evidence remain in progress; focused passes do not certify app-wide completion.


## Integration-discovered regression

The full affected-consumer gate found a WebSocket revocation race in `ws_events.rs`: logout between successful handshake authentication and socket initialization could be missed until the60s periodic check. A deterministic real-auth/router regression fails before the repair. Capture-before-auth generation handoff is implemented; green gate pending. No timeout was loosened. The separate workflow recovery success fixture now admits the run through production `admit_run`; that targeted case passed and legacy recovery behavior stays covered.

Final plugin visual finding (repeated full weak-input warnings) is closed: compact cards retain visible warning badges and keyboard-operable native disclosures. Four light/dark390/1280browser cases pass; independent review inspected eight screenshots, latest bounded design9.5/UX9.5.


Final functional checkpoint: UI type/style and1,695units pass; production build/bundle pass; plugin471pass/2knownskips; broad Rust4,616pass/91skips followed by27focused auth/events/recovery passes for the final private-handler changes. Both WebSocket revocation races have deterministic red/green coverage. The separate four ignored collector-ownership cases pass serially. The initial ignored selection used the source filename instead of the actual module name and selected zero tests; corrected selection executed all four, so zero-selection is not credited.

Fresh telemetry suite passes all six cases (with Design Hall preview,7total); fifteen representative automation/data/content/theme/accessibility browser cases pass. Design/UX/correctness/test quality latest bounded scores9.6each; performance score awaits valid sustained CPU/RSS curves. The initial rendered full run was interrupted because macOS WebKit XPC resource attribution was incomplete; corrected attribution must be tested and measured before scoring.


## Final functional closure — 11:02 UTC

All confirmed findings in the reviewed repair set are closed. The latest additions were the loading report dialog choosing a disabled autofocus target and the shared Modal focus effect tracking reactive reads from synchronous focus handlers. Both have observed failing-before/passing-after regressions. The strict native SPA acceptance passed without post-zoom refocusing: real panes retained DOM/drafts, the production workspace dialog retained its exact input/draft at100→200→190→100% zoom, and dialog bounds stayed inside the viewport. One earlier post-fix run lost whole-window foreground activation; its cause remains unexplained and is retained as a test-environment/reliability limitation, not silently credited as a pass.

Final latest UI gates: type/style0errors/0warnings;1698unitpasses;productionbuildpass. Plugin required-browser serial suite:487passed,0failed,1intentional skip. Shared-modal browser regression1pass; nested removed/inert-trigger focus-return2passes. Native example build and strict Clippy passed. The prior broad Rust4616pass and subsequent focused WS27pass remain their exact source checkpoints, not an invented final all-workspace rerun. Actual four-boot scheduled crash/recovery test passed. See the current master scorecard and individual final reviews for scope and limitations.


## Populated-report follow-up — confirmed navigation defect

The production-renderer acceptance exposed a further defect in `ui/views/reports.js`: iframe `srcdoc` inherits the embedding URL as its base, so the report's `#phases` Contents link navigates to the plugin index rather than scrolling inside the report. The one-case diagnostic failed with pre-navigation `about:srcdoc`, resolved link equal to the plugin index with `#phases`, and post-navigation heading `Team Performance` instead of the report. Log: `/tmp/otto-quality-20261007-report-fragment-red.log`. A viewer-only repair is in progress; the preceding all-findings-closed checkpoint is historical until this new regression passes.

Native AX public in-process traversal reaches remote proxies, not document content. A nonprompting own-process client capability read succeeded with existing trust and two children. This is capability evidence only, not a dialog accessibility pass. A bounded own-application public AX traversal is the next check; no system permission or VoiceOver setting changed.


## Final follow-up closure

The report Contents defect is closed: viewer-only base insertion passes all four populated-report keyboard/data/download cases and the complete required plugin suite (491passed,0failed,1intentional skip). Download bytes and sandbox are unchanged. Public own-application AX traversal then passed the actual dialog/name/value/enabled-action checks at100/200% zoom with no error, deadline overrun or truncation; strict native focus/pane assertions and example Clippy also passed. The earlier proxy-only capability failures remain evidence, not passes. Independent final design9.8/UX9.8/correctness9.8/testquality9.8 and performance9.8 are recorded in their reports. No confirmed finding remains open within the reviewed scope.
