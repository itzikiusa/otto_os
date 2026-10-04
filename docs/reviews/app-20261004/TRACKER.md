# Review findings tracker

Reports contain source traces, severity and proposed reproductions. A report is evidence to investigate, not proof that a fix passes. Status changes require inspection and verification.

## Current disposition

Closing Rust checks pass: formatting, clippy, all 4,592 nextest cases (85 skipped) and the doc-test command. Final design PR77 (`01f0c290`) is integrated as `65e13b4f`; overlap hunks were reviewed and Rust/build inputs are unchanged. Combined UI check passed with zero errors/warnings, all 1,081 units and production build passed, and the bundle guard passed after documenting only Snip's +451 gzip-byte feature cost. All 98 distinct cases in the affected browser group are verified across 97 initial passes and the repaired History fixture rerun, not one all-green 98-case invocation. The bounded scale and 15-minute sustained-memory runs completed; the final merged-UI scale confirmation also completed safely with 66 samples in 375.4 seconds. Renderer RSS growth remains a limit on leak conclusions.

Independent source rechecks are recorded in [partition 1](iteration-2-partition-1.md), [partition 2](iteration-3-partition-2.md), [partition 3](iteration-3-partition-3.md), [partition 4](iteration-3-partition-4.md), [partition 5](iteration-3-partition-5.md), and [design verification](iteration-3-design-verification.md). Role 5's final resumed-task repair is described in [its follow-up implementation report](iteration-2-implementation-5.md); its focused final source recheck approves the repair with zero remaining findings, and the final resume/foreign-thread regressions passed the closing Rust gate. The broader partition-5 dispositions are in [its iteration-2 report](iteration-2-partition-5.md).

“Source addressed/approved” below means the reviewer traced the repair; it does not mean browser, native, accessibility, or performance acceptance passed. Focused and combined test results apply only to the source present when run. [VERIFICATION.md](VERIFICATION.md) owns execution evidence and [PERFORMANCE.md](PERFORMANCE.md) owns measurements and their limits. The closing Rust/UI gates pass for the tested source. Post-PR77 checks also pass at the scope recorded below. Unexecuted native/manual checks remain evidence limits; the external full desktop suite was not clean.

## Original findings

| ID | Finding | Owner | Status |
|---|---|---|---|
| C1-01 | Transcript filesystem errors can prune valid sessions | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| C1-02 | Long reconnect gaps create unreachable middle history | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| C1-03 | Delayed image paste targets newly selected session | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| C1-04 | Archive/unarchive membership fails to synchronize | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| P1-1 | Newline-free output repeatedly copies entire ring | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| P1-2 | Live folds lack aggregate byte budget | Implementation 1 | Source addressed; admission/accounting checks recorded; closing gates passed and bounded scale run completed; retained-byte estimate is not a peak-RSS guarantee; sustained-memory run complete; long-term leak absence unestablished |
| P1-3 | Live page requests clone full fold | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| P1-4 | Load earlier disables trimming indefinitely | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| P1-5 | Transcript fallback resolution blocks async worker | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| C2-01 | Aliased SQL projections can mutate wrong row | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| C2-02 | ClickHouse sorting keys are not unique row identity | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| C2-03 | Auto-stash loses index staging | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| C2-04 | Concurrent API scripts overwrite unrelated variables | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| P2-1 | Sparse Mongo expansion exceeds response budget | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| P2-2 | SQL batches drain capped selects | Implementation 2 | Source approved; eligible SELECT caps verified with server counters; nonrewritable statements may still drain as documented |
| P2-3 | Schema history fetches all versions serially | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| C3-01 | Product in-page navigation drops unsaved drafts | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C3-02 | Design keep-mine conflict fallback discards draft | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C3-03 | Stale Product responses replace another story | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C3-04 / C3-05 | Design commit publication/no-op concurrency | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C3-06 | Vault case-only rename overwrites distinct target | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C3-07 | Browser annotations cross active-tab boundary | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C3-08 | Snip explicit Copy skips unchanged image | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P3 backlink | Vault first backlink page reads all sources | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P3 history | Product slim lists parse all historical blobs | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P3 status | Cached Vault status still recounts twice | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C4-01 | MCP approval lookup shadows another caller's valid approval | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C4-02 / C4-03 | Swarm pause/abort and competing dispatch races | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C4-04 / C4-05 | Workflow Run mutable target and late duplicate guard | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C4-06 | Stale workflow history permits incorrect restore | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P4 pool | Busy MCP pool launches unbounded fallback processes | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P4 buffering | MCP response cap applies after allocation | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P4 shell | Scheduled shell output grows without bound | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P4 capabilities | MCP per-tool requests and retained refresh consumers | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| P4 usage | Mission Control serial per-session usage queries | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| C5-01 | Assistant contradicts canonical approval decision | Implementation 5 | Source addressed; canonical approval/state regressions and closing Rust/UI gates passed |
| C5-02 / C5-03 | Assistant cancel/takeover targets wrong execution | Implementation 5 | Original and explicit-resume ownership repairs source-approved; final resume/foreign-thread regressions passed in the closing Rust gate |
| C5-04 | Enabled plugin reinstall invalidates running credentials | Implementation 5 | Source addressed; plugin replacement process/token fixture passes; complete install-route acceptance not established |
| C5-05 | Athena history loses query region | Implementation 5 | Source addressed; historical-region/stale-response unit coverage passes; mounted AWS flow unexecuted |
| P5-01 | Assistant index clones entire transcript | Implementation 1/5 | Bounded Folder API integrated into index consumer; source addressed; full initial parse/retention remains history-proportional |
| P5-02 | Assistant histories/cache reconnect unbounded | Implementation 5 | Source addressed; acquired-view refresh and count/byte cache bounds covered by passing UI units; bounded concurrent-load measurement complete; sustained-memory run complete; long-term leak absence unestablished |
| P5-03 | Insights polling rereads complete archive | Implementation 5 | Source addressed; bounded filesystem counter tests and route-policy follow-up passed in the closing Rust gate |
| D1-01 | Short-tile Composer height | Claude final design | Exact external pane-budget repair imported; nine short tiles with long drafts pass in light/dark and screenshots inspected; PR77 integrated; combined checks passed |
| D1-02 | Slash popup clamp | Claude final design | Exact external height-floor repair imported; closing source gates passed; PR77 integrated and combined checks passed; complete constrained-popup visual matrix unverified |
| D1-03 | Covering preview focus | Claude design | Integrated source addressed; native focus/VoiceOver acceptance unverified |
| D2-01 | Grid accessible cursor | Claude design | Integrated source addressed; active-descendant mechanism traced; virtualized announcements/AT acceptance unverified |
| D2-02 | Row-detail contrast | Claude design | Integrated source addressed; extra opacity removed; rendered contrast measurement unverified |
| UX1-01 | Session load failure lacks recovery and ownership | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| UX1-02 | Composer sends during pending upload | Claude design iteration 2 | Remount regression R2-P1-01 repaired by imported external source; deferred-upload/remount browser case passed; PR77 integrated; combined checks passed |
| UX1-03 | Palette search failure appears as no results | Implementation 1 | Source addressed; focused checks and closing Rust/UI gates passed |
| UX2-01 | API scratch close discards unsent work | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| UX2-02 | Automation navigation discards unsaved steps | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| UX2-03 | Kafka tail failure retains active claim | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| P-live-k8s | Rollup backfill hits memory limit and restarts indefinitely | Root integration | Fixed; 3M-series interrupted/retried migration passes under 1 GiB server limit |
| UX3-01 | Publish allowed before successful content preview | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| UX3-02 | Brand keep-mine nested save exits while busy | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| UX3-03 | Snip close abandons failed save | Implementation 3 | Source recheck addressed; combined and closing Rust/UI gates passed |
| D3-01 | Mockup annotation keyboard creation | Claude design | Integrated source addressed; full keyboard save/cancel acceptance unverified |
| D3-02 | Mockup annotation editor clamp | Claude design | Integrated source addresses original edge placement; phone/RTL/enlarged-text rendered bounds unverified |
| D3-03 | Diagram previews lack keyboard pan | Claude design iteration 2 | Integrated source addressed; rendered keyboard navigation acceptance unverified |
| D3-04 | Import request failures appear as empty results | Implementation 3 | Source addressed; failure/retry handling covered by focused units; complete rendered failure matrix unverified |
| UX4-01 | MCP replacement import conceals cross-workspace deletion | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| UX4-02 | Mission Control selection discards edited fields | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| UX4-03 | Goal Loop budget saved but failed Resume hidden | Implementation 4 | Source recheck addressed; combined and closing Rust/UI gates passed |
| D4-01 | Swarm library failure appears as empty suggestions | Implementation 4 | Source addressed; loading/error/retained catalogue handling implemented; rendered request-state acceptance unverified |
| UX5-01 | Personal-agent docs/autonomy drafts lost on navigation | Implementation 5 | Source addressed; dirty/pending-save guards covered by passing units and focused personal-agent browser cases; full keyboard/native acceptance matrix unverified |
| UX5-02 | Partial agent/template creation retries duplicate agent | Implementation 5 | Source addressed; retained identity and idempotent schedule retry covered by state/UI regressions; closing Rust/UI gates passed |
| D5-01 | Room Point/Highlight annotations pointer-only | Claude design iteration 2 | Integrated source addressed; room fixture/keyboard acceptance unverified |
| D5-02 | Kubeconfig checkbox group uses listbox semantics | Claude design iteration 2 | Integrated source addressed; accessibility-tree and Tab/Space acceptance unverified |
| D5-03 | Phone token rows conceal textual expiry | Claude design iteration 2 | Integrated source addressed; compact rendered acceptance unverified |
| C2-R2-01 | PostgreSQL identifier folding retargets mixed-case table edits | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| C2-R2-02 | LIMIT ALL/expression/newline rewritten with duplicate LIMIT | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| C2-R2-03 | Workspace change bypasses unsaved editor leave guards | Implementation 2 | Source addressed; focused checks and closing Rust/UI gates passed |
| History focus | Keyboard test expects focus to activate Load earlier | Claude test / Implementation 1 scroll test | Focus-only expectation rejected; Focus+Enter correction integrated via PR76; separate real-scroll pagination passes with HMR disabled |

## Follow-up verification findings

| ID | Finding | Owner | Status |
|---|---|---|---|
| R2-P1-01 | Pending upload state is lost on Composer remount | Claude final design | Browser RED recorded; exact 32d9b161 repair imported; deferred-upload/remount browser rerun passed; PR77 integrated; combined checks passed |
| R2-P1-02 | Slash popup height floor exceeds remaining pane space | Claude final design | Exact 32d9b161 repair imported; closing source gates passed; PR77 integrated and combined checks passed; complete constrained-popup visual matrix unverified |
| D1-01 follow-up | Nine-pane long draft leaves only 3px of conversation after resize | Claude final design | Measured browser RED; exact 32d9b161 pane-budget repair imported; nine-tile long-draft browser cases pass in light/dark; PR77 integrated; combined checks passed |
| R2-P3-01 | Canceling Product tab transition leaves focus on inactive tab | Implementation 3 | Final source recheck approves; real-draft canceled keyboard transition browser test passes |
| R2-P3-02 | Backlink refresh blanks previously expanded contexts | Implementation 3 | Final source recheck approves; real status-generation refresh/expanded-context regression passes in 36-test recovery group |
| R2-P3-03 | Product workspace effect clears every selected story | Implementation 3 | Final source recheck approves; Product browser flows reach and use the editor, including real local-draft HTTP save/reopen on the rebuilt daemon |
| Product draft revision | Story detail looks up imported source rather than saved draft revision | Implementation 3 | Final source recheck approves; Rust saved-revision reopening and null-source UI tests pass; real local-draft HTTP save/reopen browser test passed on the rebuilt daemon |
| C3-R2-01 | Keyboard Copy still skips unchanged Snip; failed flatten must block leave | Implementation 3 | Source addressed; actual shortcut and persistence-failure regressions included in passing recovery tests; native close remains unexecuted |
| C4-R2-01 | Swarm readiness waits hold dispatch mutex and block Stop | Implementation 4 | Source recheck addressed; stop-readiness regression passes in combined Rust library gate |
| C4-R2-02 | Same-item Mission Control refresh replaces active draft | Implementation 4 | Final source recheck approves; nine ownership/draft regressions pass within the 1,081-test UI run; closing Rust/UI gates passed |
| R2-5-01 | Explicitly resumed Assistant task keeps obsolete execution binding | Implementation 5 | Final focused source recheck approves caller-thread validation/atomic running rebind; resume/foreign-thread regressions passed in the closing Rust gate |
| Insights route policy | New report-status route falls through to Deny | Root integration | Exact Insights:View policy and contract repaired; middleware-policy regression passed in the closing Rust gate |
| Mission compact focus | Compact Mission Control detail needs dialog focus ownership | Claude design | Integrated source addressed; native/AT and nested-confirmation acceptance unverified |

## Executed verification and remaining gates

- Closing Rust gate: **rustfmt, clippy, all 4,592 nextest cases (85 skipped), and the doc-test command passed**, including final Assistant resume/foreign-thread cancellation and Insights route-policy regressions (`/tmp/otto-review-final-gate2.log`). Earlier seven-package library coverage remains recorded in [VERIFICATION.md](VERIFICATION.md).
- Post-PR77 UI check passed with zero errors/warnings; **1,081 unit tests and the production build passed**. The bundle guard passed after documenting only Snip's measured **+451 gzip bytes** for its persistence/copy/leave guards; other budgets and the 3% tolerance remain unchanged. Rust/build inputs did not change during the merge, so the completed Rust gate remains applicable.
- The actual isolated daemon was rebuilt. Earlier coverage verified **63 cases as 61 initial passes plus two repaired reruns**, including actual Product draft HTTP save/reopen, personal-agent draft guards, Composer deferred-upload/remount, nine short tiles in light/dark, and History phone title/scope behavior. The combined post-merge group verified **98 distinct cases across 97 initial passes and the repaired History fixture rerun**. History/terminal repeated **33/33** (11 cases three times, 28.5s); Rooms passed **3/3** (4.2s) and again after merging. No single all-green 98-case invocation is claimed. Final E2E TypeScript check passed; merged tile screenshots were inspected. Focused recovery units also passed **36 cases**.
- Disposable SQL batches passed **3 MySQL / 2 PostgreSQL** tests, including actual server counters and connection continuity. API persistence passed **4 browser tests**; automation leave/save/discard passed its rendered regression. History real-scroll paging reached cursors **120 then 60** on HMR-disabled Vite.
- The `otto-pty`/`otto-transcript` tests, MCP endless HTTP/SSE cap regression, and isolated 3-million-series interrupted/retried K8s migration passed as recorded in [VERIFICATION.md](VERIFICATION.md).
- Bounded concurrent-session CPU/RAM measurement completed safely: **0/1/3/5 sessions, 375.4 seconds, 66 samples, no page errors and zero leftover processes** (`/tmp/otto-review-load-headed/`). The separate **15-minute three-session sustained run completed in 986.8 seconds with 184 samples**, no page errors, swap growth or leftovers. Across eight comparable checkpoints, heap stayed at **35.0–37.5 MiB**, live DOM/listeners stayed at **887/375**, and renderer RSS grew **32.8 MiB**. These measurements do not establish absence of long-term/native leaks; see [PERFORMANCE.md](PERFORMANCE.md).
- **Local review and verification complete:** final merged-UI scale confirmation recorded in [PERFORMANCE.md](PERFORMANCE.md); documentation and screenshots prepared. Delivery next: publish this effort's single PR, wait for green CI and merge. The PR checks/merge record establish delivery; this ledger records pre-publication acceptance.
- **Acceptance limits:** the external full desktop suite was not clean; its supplied failure/rerun ledger remains in [VERIFICATION.md](VERIFICATION.md). The affected browser passes do not certify the complete desktop/light/dark/responsive matrix or native keyboard/focus, assistive-technology and physical-device behavior. The original D1-01 tile regression has rendered acceptance; the complete constrained-popup visual matrix for D1-02 remains unverified. These limits are not additional manual work promised by this tracker.

## Closing browser integration

| Finding | Disposition |
|---|---|
| Assistant metadata update releases active history | Observed extra GET and lost live turn; primitive thread-ID acquisition dependency repaired. Rendered incremental messages, unchanged GET count, and task Stop pass. |
| History scope switch hidden during initial/empty load | Final design markup repair keeps the switch reachable; delayed first-page scope regression passes. |
| History fixture assumes one startup request | Merged run correctly paged but issued three startup reads. Fixture now separates startup reads and preserves exact wheel cursors 120 then 60, anchoring and oldest-turn assertions; repeated verification passed. No production change. |
| Rooms 200-message fixture uses oldest page for tail request | Contract/source trace confirms stale fixture. Updated to newest 200 plus explicit earlier-page traversal; 3/3 passed before integration and passed again in the merged group. No production defect asserted. |
| Terminal-link tests click `(0, 0)` during row replacement | Four supplied failure traces prove invalid test coordinates. Helper now waits for connected, nonzero text geometry; History/terminal verification passed 33/33 across three repeats of 11 cases. No production link change. |

## Baseline verification

- `npm run check`: passed on baseline a16f4c71.
- `npm run test:unit`: 989/989 passed.
- Fresh daemon build: passed (12m08s), two jobs, dedicated target. macOS debug linker warned about large unwind section.
- Baseline page-chrome E2E: 5/5 passed.
- Load harness with `--ui-url` failed before measurement: browser API origin incorrectly pointed at Vite. Fixed origin to isolated daemon. Rerun booted and sampled 0/1 sessions, then safety-aborted at host load 13.26 (cap 12), with no swap growth and zero leftover processes. This historical aborted attempt is superseded by the completed bounded scale run recorded above; the sustained run also completed with the leak limits recorded above.

## External effort

Claude's visual design effort owns shared primitives, RTL, shell overlays, module accessibility, documented error states, chart palettes, trust-action confirms, and copy. PR75 and PR76 are integrated. The exact final Composer file from 32d9b161 and matching dependencies are imported with Claude's authorization; deferred-upload/remount and nine-tile long-draft browser acceptance passed. Final PR77 is integrated as `65e13b4f`; combined UI/build/bundle gates and affected browser verification passed. See shared coordination note in `/tmp/otto-app-review-coordination-20261004.txt`; the integration preserved both efforts. The independent design report is a bounded source verification, not a rendered acceptance result.
