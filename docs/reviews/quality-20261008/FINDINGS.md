# Post-merge findings ledger

Snapshot: `a0bd718b9fbc008d72c164ce643a24ed78e6368c`. All 16 assignments are complete; confirmed findings are repaired. Whole-product scores remain unawarded. Historical per-finding checkpoints below are superseded by the final integrated results in [VALIDATION.md](VALIDATION.md). IDs below summarize domains; exact severity, location and evidence are in each domain report.

| Area | Finding | Evidence / status |
| --- | --- | --- |
| R01 authentication | Token refresh grows reverse index | Actual regression red; HashSet fix passes all 61 authentication tests; independently reviewed |
| R01 membership | Failed replacement persists prior removals | Actual handler regression red then green; failed/duplicate replacements preserve members, valid replacement succeeds |
| R01 portable backup | Canvas references point to excluded sessions | Actual export/preview/restore regression red then green; full refs preserved, portable refs omitted |
| R01 archive permissions | Imported automatic approval rules remain enabled | Actual restore regression red then green, including older enabled rule input |
| R01 impersonation | Failed identity load splits bearer and displayed account | Identity blocking/stale-response fix passes focused UI regressions |
| R01 settings | Save response marks newer unsaved draft clean | Submitted-baseline fix passes production-function and deferred browser regression |
| R01 network settings | Saved listener setting presented as running state | Live listener fix passes actual handler and desktop/phone browser checks |
| R01 startup | Different port permits duplicate data-directory owner | Source trace; isolated lock regression planned |
| R02 transcript | Changed provider/path retains stale live tail | Actual regression red; generation ownership fix independently reviewed; full tail module 19/19 green, including stale publication and A→B→A |
| R02 passive Chat | Viewing saved tile resumes suspended CLI | Actual browser regression red then green; passive keepalive/recovery and explicit/active resume controls (3/3 session cases) |
| R02 clipboard | Delayed read pastes into replacement session | Actual browser regression red then green after target ownership fix; stale destination rejected and intended destination accepted (1/1, 4.5 s) |
| R02 repo map | Byte truncation splits UTF-8 and panics | Actual parser panic red; UTF-8 boundary fix, entire context suite 62/62 green |
| R03 approvals | Historical approvals survive changed review opinion | Provider/readiness/UI trace; wire regression red then green |
| R03 remote identity | Origin edit leaves stale forge target | Origin regression red then green; broader consumer checks pending |
| R03 Jira | Same credentials alias caches across sites | Two-site regression red then green (4 walk tests); broader suite pending |
| R03 graph | Ref rewrite retains unreachable history tail | Actual production TypeScript reproduction; graph reachability repair and focused tests green |
| R03 force confirmation | Branch change can change confirmed push destination | Immutable source/destination and explicit lease implemented; focused UI green, Rust integration queued; no real force push performed |
| R04 Mongo edit identity | Computed identifiers can target another document | Strict query/projection validation; real Mongo identity regression green |
| R04 Mongo projection | Replacing projected JSON deletes hidden fields | Field patch preserves unselected fields; real fixture regression green |
| R04 import | Sparse objects expand into quadratic dense allocation | Expanded-cell budget and linear key discovery; red then green |
| R04 truncated edits | Display-limited cells can overwrite stored values | Explicit cells_truncated provenance and write guards; decoder gates green; fresh-daemon Mongo2/2green, hidden >1MiB value preserved |
| R04 Redis | Complete reply/pipeline allocation precedes row/cell bounds | Bounded stream plus cumulative request budget;30focused tests green including protocol/TLS/cancellation/reconnect; pipeline-budget mutation fails as intended; full package gate pending |
| R05 API | Cross-tab/workspace asynchronous results change another request | Request/stream/reflection/import/upload ownership repairs; named browser regressions green |
| R05 replay | Scripts/GraphQL automation differ from interactive semantics | Deletion replay and strict variable validation; handler/engine tests green |
| R05 reflection | Unbounded reflection and second-request readiness panic | Deadline/message/byte bounds and tonic readiness; real h2 regressions green |
| R06 broker replay | Offset bounds, partial failure, silent cap/deadline and memory growth | Range filtering, persisted acknowledged prefix, validated bounded reads;66broker tests and partial-result browser green |
| R06 schema cache | Distinct failed IDs retained indefinitely | Serialized512-entry cap and expired-entry pruning; actual1024-ID red then green |
| R06 cloud operations | Confirmation/request target changes with selected region | Captured mutation destinations and correct replica count; selected browser checks green |
| R07 annotation | Foreign workspace tab reference leaks title | Same-workspace creation and legacy-read filtering;54server browser route tests green |
| R07 reader/live | Stale summaries, cancelled gates, reconnect navigation, missing restored reader | Ownership/RAII/reattach/initial-load repairs;116browser crate,64UI,15browser cases green |
| R01 cache follow-up | Delayed lookup restores revoked entry; distinct expired contexts accumulate | Two actual reds; scoped invalidation history and4096-entry/index cap; full RBAC67/67green |
| R08 workflows | Stale action targets, delivery suppression, orphaned commands and startup recovery race | Reproduced repairs;121domain/134server/80UI tests and named browser journeys pass; see [R08 report](reports/R08-workflows.md) |
| R08 publication | Cached proof/wrong revision or failed push can bypass PR publication safeguards | Current proof/revision/clean-tree checks and push-error propagation; local no-origin negative control; stale-proof UI red then green |
| R09 agents | Notification baseline/destination, unsafe rollback, verifier/cancellation/planner admission and stale schedule state | Ten repaired families;188Rust/44UI/16latestbrowser cases; see [R09 report](reports/R09-agents.md) |
| R09 scale | Unbounded personal-agent fanout and Assistant thread history | Two concurrent fetches; paged SQL history with0181 index; real SQL and routed browser checks; fresh paging wire check pending |
| R10 MCP | Secret-load failures and omitted fields lose authentication; queued invocations bypass changed authorization | Four reproduced primary defects repaired;102unit/11integration tests; see [R10 report](reports/R10-mcp-channels.md) |
| R10 channels | Detach races metadata persistence; ingress/rejection queues unbounded; retry/acknowledgement gaps |123channel tests, shared per-daemon admission with recovery reserve, three mutation controls, webhook3/3; identity key change is defensive hardening, not a demonstrated production exploit |
| R11 privacy/refinement | Foreign refinement reuse, stale detach callbacks and private-memory graph/mutation/dedup/export exposure |37memory/116ordinaryVault/36server cases and focused UI/browser checks; see [R11 report](reports/R11-vault.md); pre-existing exported files are not deleted |
| R11 scale/liveness | Unbounded search counts and watcher queue; continuous events starve refresh and late overflow loses wakeup | Bounded search/watch queues, deadline flush and republished overflow wake; both10k-note/50k-revision scale gates pass |
| R12 Product/Design | Stale consent/drafts, assist admission/persistence, session reuse authority, graph traversal/races |15repair groups; initial310Rust and selectedmountedjourneys; finalgraph/assist tests queued in sharedintegration; see[R12 report](reports/R12-product-design.md) |
| R13 shell/native | Workbench/Home ownership, hiddenrendering, searchprivacy/materialization, LSPallocation |55native tests+actualvisibility/panelprobes,10ksearchprojection andfocusedUI/HTTP; see[R13 report](reports/R13-shell-native.md) |
| R14 evaluator/plugins | Caller/foreign-golden authority, concurrentfixer/scoring, staleproof, cancellation and boundedplugin/usagework |Repairs under finalverification;492TeamPerformancetests pass with one inapplicablelight-onlyassertiondarkskip |
| R15 publication follow-up | Awaited credentials let unproved HEAD replace captured proof revision |PinnednormalSHApush+draftbranchmatch; actualTempDirbareoriginregression and completeGit/issues481tests green; serverconsumer rerunpending |
| R16 draft/approval feedback | Clearedchatdrafts reappear, oldfailedsend replacesnewdraft, currentunapprovedhead labeledApproved |Fouractualmountedreds; draftownership/emptydecision andheadbadgefixes applied; independentgreenrerunpending |

Falsified: PR draft leakage across actual navigation. The durable named browser regression passes; no production change warranted.

Excluded from this pass: R02 command-execution candidate stopped by automatic security-content review. No injected-content test was run; this remains a coverage gap, not a confirmed finding.

Merge verification: all 17 non-advisory checks passed on PR head `0ea2dec98c70d2d987908df89a49ed9594fbf119` before admin merge. Subsequent advisory functional shards 1–4 and red-count ratchet all passed; advisory WebKit performance CANCELLED after 60m24s during Install Playwright WebKit; both performance steps were skipped (job113021988394), so no WebKit performance result exists. No thresholds relaxed.

CI setup repair: cancelled WebKit job logs show Ubuntu Azure mirror metadata requests stalled from21:18:58 through22:18:19; benchmarks never started. All four Playwright dependency-install steps now prepare the official mirror plus30s acquisition timeouts/one retry and bound the complete install to15min. There is no automatic reinstall retry after an interrupted dpkg phase. Eight fake-apt tests pass, preserving package signing/suites/security sources; workflow YAML and all four UI working directories validated. Initial PR #96 hosted WebKit setup and benchmarks passed; no host apt configuration was modified locally.
