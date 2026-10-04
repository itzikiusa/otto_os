# Review findings tracker

Reports contain source traces, severity and proposed reproductions. A report is evidence to investigate, not proof that a fix passes. Status changes require inspection and verification.

| ID | Finding | Owner | Status |
|---|---|---|---|
| C1-01 | Transcript filesystem errors can prune valid sessions | Implementation 1 | Accepted; regression pending |
| C1-02 | Long reconnect gaps create unreachable middle history | Implementation 1 | Accepted; regression pending |
| C1-03 | Delayed image paste targets newly selected session | Implementation 1 | Accepted; regression pending |
| C1-04 | Archive/unarchive membership fails to synchronize | Implementation 1 | Accepted; regression pending |
| P1-1 | Newline-free output repeatedly copies entire ring | Implementation 1 | Accepted; benchmark pending |
| P1-2 | Live folds lack aggregate byte budget | Implementation 1 | Accepted; bounded design pending |
| P1-3 | Live page requests clone full fold | Implementation 1 | Accepted; page-level snapshot pending |
| P1-4 | Load earlier disables trimming indefinitely | Implementation 1 | Accepted; preserve historical reading anchor |
| P1-5 | Transcript fallback resolution blocks async worker | Implementation 1 | Accepted; offload with focused test |

| C2-01 | Aliased SQL projections can mutate wrong row | Implementation 2 | Accepted; provenance regression pending |
| C2-02 | ClickHouse sorting keys are not unique row identity | Implementation 2 | Accepted; reject unsafe row mutations |
| C2-03 | Auto-stash loses index staging | Implementation 2 | Accepted; disposable Git regression pending |
| C2-04 | Concurrent API scripts overwrite unrelated variables | Implementation 2 | Accepted; operation-level merge pending |
| P2-1 | Sparse Mongo expansion exceeds response budget | Implementation 2 | Review with report; expanded budget needed |
| P2-2 | SQL batches drain capped selects | Implementation 2 | Review connection lifecycle before fix |
| P2-3 | Schema history fetches all versions serially | Implementation 2 | Review lazy/bounded fetching approach |

| C3-01 | Product in-page navigation drops unsaved drafts | Implementation 3 | Accepted; coordinate handlers with design effort |
| C3-02 | Design keep-mine conflict fallback discards draft | Implementation 3 | Accepted; regression pending |
| C3-03 | Stale Product responses replace another story | Implementation 3 | Accepted; deferred-response tests pending |
| C3-04 / C3-05 | Design commit publication/no-op concurrency | Implementation 3 | Accepted; serialize complete commit boundary |
| C3-06 | Vault case-only rename overwrites distinct target | Implementation 3 | Accepted; filesystem identity + no-replace |
| C3-07 | Browser annotations cross active-tab boundary | Implementation 3 | Accepted; ownership regression pending |
| C3-08 | Snip explicit Copy skips unchanged image | Implementation 3 | Accepted; clipboard response regression pending |
| P3 backlink | Vault first backlink page reads all sources | Implementation 3 | Review bounded context hydration |
| P3 history | Product slim lists parse all historical blobs | Implementation 3 | Review persisted summary migration |
| P3 status | Cached Vault status still recounts twice | Implementation 3 | Review count ownership/invalidation |

| C4-01 | MCP approval lookup shadows another caller's valid approval | Implementation 4 | Accepted; requester filter + atomic consume |
| C4-02 / C4-03 | Swarm pause/abort and competing dispatch races | Implementation 4 | Accepted; unified lifecycle/reservation boundary |
| C4-04 / C4-05 | Workflow Run mutable target and late duplicate guard | Implementation 4 | Accepted; capture identity before async work |
| C4-06 | Stale workflow history permits incorrect restore | Implementation 4 | Accepted; response ownership regression |
| P4 pool | Busy MCP pool launches unbounded fallback processes | Implementation 4 | Accepted; bounded admission for every transport |
| P4 buffering | MCP response cap applies after allocation | Implementation 4 | Accepted; bounded streaming reads |
| P4 shell | Scheduled shell output grows without bound | Implementation 4 | Accepted; bounded concurrent drain |
| P4 capabilities | MCP per-tool requests and retained refresh consumers | Implementation 4 | Accepted; batch + bounded consumers |
| P4 usage | Mission Control serial per-session usage queries | Implementation 4 | Accepted; scoped bulk lookup |
| C5-01 | Assistant contradicts canonical approval decision | Implementation 5 | Accepted; preserve canonical outcome |
| C5-02 / C5-03 | Assistant cancel/takeover targets wrong execution | Implementation 5 | Accepted; execution ownership regression |
| C5-04 | Enabled plugin reinstall invalidates running credentials | Implementation 5 | Accepted; lifecycle reconciliation |
| C5-05 | Athena history loses query region | Implementation 5 | Accepted; preserve historical region |
| P5-01 | Assistant index clones entire transcript | Implementation 1/5 | Shared page snapshot API; coordinate consumers |
| P5-02 | Assistant histories/cache reconnect unbounded | Implementation 5 | Accepted; visible refresh + bounded cache |
| P5-03 | Insights polling rereads complete archive | Implementation 5 | Accepted; summary/new-report lookup |
| D1-01 / D1-02 / D1-03 | Composer height, popup clamp, preview focus | Claude design iteration 2 | Accepted externally; verify merged fixes |
| D2-01 / D2-02 | Grid accessible cursor and row-detail contrast | Claude design iteration 2 | Accepted externally; verify merged fixes |
| UX1-01 | Session load failure lacks recovery and ownership | Implementation 1 | Accepted; explicit load/error state |
| UX1-02 | Composer sends during pending upload | Claude design iteration 2 | Accepted externally; verify merged fix |
| UX1-03 | Palette search failure appears as no results | Implementation 1 | Accepted; explicit search error |
| UX2-01 | API scratch close discards unsent work | Implementation 2 | Accepted; nonempty dirty draft guard |
| UX2-02 | Automation navigation discards unsaved steps | Implementation 2 | Accepted; guarded transitions |
| UX2-03 | Kafka tail failure retains active claim | Implementation 2 | Accepted; recoverable stale state |
| P-live-k8s | Rollup backfill hits memory limit and restarts indefinitely | Root integration | Confirmed in live logs; bounded migration repair pending |

| UX3-01 | Publish allowed before successful content preview | Implementation 3 | Accepted; preview state + submit gate |
| UX3-02 | Brand keep-mine nested save exits while busy | Implementation 3 | Accepted; single save/retry state machine |
| UX3-03 | Snip close abandons failed save | Implementation 3 | Accepted; await persistence + retry/discard |
| D3-01 / D3-02 | Mockup annotation keyboard creation/editor clamp | Claude design iteration 2 | Accepted externally; verify merged changes |
| D3-03 | Diagram previews lack keyboard pan | Claude design iteration 2 | Accepted externally; verify merged changes |
| D3-04 | Import request failures appear as empty results | Implementation 3 | Accepted; explicit errors + contextual retry |

| UX4-01 | MCP replacement import conceals cross-workspace deletion | Implementation 4 | Accepted; explicit verified scope before replacement |
| UX4-02 | Mission Control selection discards edited fields | Implementation 4 | Accepted; guarded selection/close, coordinate compact layout |
| UX4-03 | Goal Loop budget saved but failed Resume hidden | Implementation 4 | Accepted; partial-success recovery |
| D4-01 | Swarm library failure appears as empty suggestions | Implementation 4 | Accepted; explicit error/retry |

| UX5-01 | Personal-agent docs/autonomy drafts lost on navigation | Implementation 5 | Accepted; dirty leave decisions |
| UX5-02 | Partial agent/template creation retries duplicate agent | Implementation 5 | Accepted; retain created identity + schedule retry |
| D5-01 | Room Point/Highlight annotations pointer-only | Claude design iteration 2 | Accepted externally; verify merged fixes |
| D5-02 | Kubeconfig checkbox group uses listbox semantics | Claude design iteration 2 | Accepted externally; verify merged fixes |
| D5-03 | Phone token rows conceal textual expiry | Claude design iteration 2 | Accepted externally; verify merged fixes |

| C2-R2-01 | PostgreSQL identifier folding retargets mixed-case table edits | Implementation 2 | Confirmed regression; repair underway |
| C2-R2-02 | LIMIT ALL/expression/newline rewritten with duplicate LIMIT | Implementation 2 | Confirmed regression; tokenizer repair underway |
| C2-R2-03 | Workspace change bypasses unsaved editor leave guards | Implementation 2 | Confirmed regression; guarded identity change and dependent actions |
| History focus | Keyboard test expects focus to activate Load earlier | Claude test / Implementation 1 scroll test | Focus+Enter correction passes on Claude branch; actual scroll regression pending |

## Baseline verification

- `npm run check`: passed on baseline a16f4c71.
- `npm run test:unit`: 989/989 passed.
- Fresh daemon build: passed (12m08s), two jobs, dedicated target. macOS debug linker warned about large unwind section.
- Baseline page-chrome E2E: 5/5 passed.
- Load harness with `--ui-url` failed before measurement: browser API origin incorrectly pointed at Vite. Fixed origin to isolated daemon. Rerun booted and sampled 0/1 sessions, then safety-aborted at host load 13.26 (cap 12), with no swap growth and zero leftover processes. Full scale/leak measurements remain pending.

## External effort

Claude's visual design effort owns shared primitives, RTL, shell overlays, module accessibility, documented error states, chart palettes, trust-action confirms, and copy. See shared coordination note in `/tmp/otto-app-review-coordination-20261004.txt`; verify its merged changes rather than reimplement them. Our independent design and UX reviewers still report uncovered issues and regressions.
