# Iteration 4 — implementation role 2

Worktree: `/Users/itziklavon/claude_ade-review`, branch `fix/app-review-20261005`. Tests execute production handlers/stores with controlled transport boundaries. Root alone runs tests/builds; this implementer has run neither. Production edits preserve Claude's markup/styles and existing placeholder/copy pending later main integration.

## UI evidence and repairs

- **C4-2-02, database approvals:** confirmed that the request captured connection A while typed approval reread selected connection B; the agent read-only retry also reread B's guard status. Both entrypoints now capture originating connection metadata. Typed approval and guard classification use it. Managed retry checks access and connection epochs; ordinary queries retain cancellation/access checks. Production access-revocation and Stop handlers are exercised by regressions.
- **C4-2-04, API workspace publication:** deferred list/mutation completions, errors and bulk-load finalizers could publish into a changed workspace, including A→B→A. A workspace visit generation now fences the inspected collection/request/environment/automation list/mutation methods, SSH list load, errors, freshness and bulk-load state. A completed save still returns its server result to its caller without publishing into another visit. Ownership bookkeeping does not restore tabs or replace drafts on first Save. This is scoped coverage, not an audit of every asynchronous API-store method.
- **C4-2-05, environment editor:** Save captures row identity and editor generation. Actual newer edits remain dirty; a clean A→B→A reselection reconciles with the saved result. Secret rename acknowledgments update persisted `storedKey` without overwriting a newer key/value, so a second rename addresses the name actually stored in Keychain.
- **R4-U2-01, import:** completion no longer executes the live editor draft. Metadata refresh belongs to the captured connection/object. Guard retries retain the submitted connection metadata, format, path, table and batch size; canceled or superseded confirmation cannot retry. Old completion cannot close a replacement table dialog.
- **R4-U2-02, request save:** the created collection becomes the dialog target before request persistence, making retry reuse it. Workspace/tab ownership prevents the second mutation in a changed context. The independent recheck found Cancel/unmount could still continue; synchronous close and actual `onDestroy` invalidation now guard that phase.
- **R4-U2-03, Git recovery:** Refresh reloads active history from offset zero, retains loaded rows on failure, and reports the error. Initial history preloading remains available when the dialog opens on another tool.

Independent spec/quality recheck: [iteration-4-role2-ui-recheck.md](iteration-4-role2-ui-recheck.md). It approved the six original source repairs but requested the cancel-lifecycle and environment-return corrections above. The follow-up source recheck approves both corrections with no new confirmed regression, and root's final focused run passed 80/80. Mounted/browser and full typecheck evidence remain separate.

## Executed by root

1. Initial dialog tests: **8 failed at intended assertions** (three destructive/unsubmitted import drafts, collection retry, two history refresh cases, two environment races).
2. Database ownership tests: **6 passed / 4 intended failures** (two wrong-origin prompts, managed access revocation, agent guard downgrade).
3. API ownership tests: **26 existing passed / 21 new intended failures** (workspace publication/errors/ABA/loading).
4. First combined UI implementation: **68/68 passed**, 13.806s, `/tmp/otto-review05-role2-ui-green1.log`.
5. Adjacent dialog cases: **11 passed / 6 intended failures** (secret marker variants, request second mutation after workspace change, import origin/cancel/replacement ownership).
6. Expanded combined UI implementation: **75/75 passed**, 14.151s, `/tmp/otto-review05-role2-ui-green2.log`.
7. Independent-recheck regressions: **18 passed / 3 intended failures**, 382ms (actual lifecycle destruction and plain/secret A→B→A clean views). Source fixes and two extra same-tick/new-typing cases were followed by **80/80 passing**, 13.998s, `/tmp/otto-review05-role2-ui-green3.log`. The independent reviewer inspected that root-run result and approved both corrections.

Exact focused command from `ui/`:

```sh
node --test unit/dataToolsDialogs.test.ts unit/databaseWriteOwnership.test.ts unit/apiClientOwnership.test.ts
```

The dialog harness executes production function declarations, lifecycle registration and primitive initializers. Destroy tests invoke the registered production callbacks, not a mocked `alive` flag. Database derived expressions are evaluated as getters under the VM to preserve changing selection; API tests use the existing identity-rune harness. These establish deferred-handler ownership, not mounted Svelte/browser/native acceptance. The independent recheck requested stronger lifecycle coverage and that boundary is now explicit. Root observed a non-failing Node localStorage experimental warning; the database VM has an explicit isolated storage shim.

## Remaining assigned work

Broker replay binary key/value/header and tombstone preservation, Unicode evidence preview, Git untracked diff accumulation bounds, and Workbench history paging remain assigned for test-first work after UI review. No Rust/performance repair or test result is claimed here. No real database/broker writes, servers, builds, commits, staging or deploy were performed by this implementer. Full UI typecheck and mounted/browser behavior remain integration-gate evidence to record separately.


## Rust/performance test-authoring checkpoint

Production Rust, Workbench UI and API contract code are unchanged. These new regressions are authored but **not executed**; root will establish behavioral RED before repairs:

| Files | Existing production behavior exercised | Root command |
|---|---|---|
| `crates/otto-brokers/src/service.rs` (test-only module) | Service replay, actual producer and raw consumer against an in-process librdkafka MockCluster: binary key/value/header, tombstone versus empty, field-specific transform, UTF-8 preview boundary; public producer's documented empty-text tombstone versus base64 present-empty | `cargo test -p otto-brokers --lib replay_integrity_tests -- --test-threads=1` |
| `crates/otto-git/src/diff_tests.rs` | Real isolated repo and counting Git shim: small aggregate cap must stop untracked patch child launches early while retaining all 32 file summaries and explicit omitted-body metadata; ordinary per-file diff remains complete | `cargo test -p otto-git --lib working_diff_stops_untracked_patch_spawns_after_aggregate_budget` |
| `crates/otto-server/tests/workbench_api.rs` (existing registered suite) | Existing HTTP history route with 205 revisions: exclusive cursor, intervening checkpoint, oldest compare/detail/restore, no history deletion | `cargo test -p otto-server --test it workbench_api::revision_history_exclusive_cursor` |
| Same server suite, ignored scale case | Isolated SQLite 50k metadata rows sharing one small blob: bounded default100/max200, response bytes and unchanged stored count | `cargo test -p otto-server --test it workbench_api::revision_history_default_and_maximum_page_are_bounded_at_50k_revisions -- --ignored` |
| `ui/unit/workbenchHistoryPaging.test.ts` | Existing production API wrapper must send bounded limit/exclusive cursor while direct revision URL stays unchanged | `node --test unit/workbenchHistoryPaging.test.ts` from `ui/` |
| `ui/e2e/desktop-review4-workbench-history.spec.ts` | Mounted HistoryPanel requests/renders bounded pages, reaches revision1 through three older pages, compares directly to305, restores1, refreshes head and remains bounded | Root's isolated Playwright desktop slot, this spec only |

Runtime estimates are scheduling guesses, not measurements: broker fixture roughly 10–30 seconds plus compilation; Git under a few seconds plus compilation; focused/scale SQLite fixtures seconds plus the existing server fixture setup; wrapper test subsecond; browser journey under a minute after startup. No external Kafka, Docker or real model/DB connection is needed. The 50k case is ignored for ordinary gates per repository scale-test convention.

Workbench proposed contract (root approved segment): existing array shape, `limit` default100/max200, exclusive `before_seq`, direct revision GET/restore unchanged. Timeline retention/rendering must remain one bounded page, with reachable older pages and explicit revision comparison. Only HistoryPanel and the Workbench API wrapper call the timeline in UI; the state method's other direct callers are tests. MCP `workbench_get(history:true)` also calls the route and currently implies a full timeline: its narrow schema/handler paging args and cursor metadata are granted by root. Control-plane outward catalog has no Workbench entry in the inspected current source, so no existing outward Workbench schema needs synchronization; this observation does not assert broader catalog parity. Contracts/types are not edited in this checkpoint; the HTTP array response does not require a new TypeScript response shape.

Still to author after this checkpoint: focused MCP paging schema/handler regression, then supplemental bounds/ownership tests as implementation choices become concrete. Aggregate live-buffer/peak-RSS and child cancellation evidence need runtime instrumentation/measurement beyond the authored Git launch-count regression. No claim of measured memory improvement or complete native/rendered acceptance is made.


## Workbench follow-up authoring checkpoint

Root observed the API-wrapper regression **0/1 passed**, with the intended missing-query assertion `null !== '100'` and no harness error. The authorized wrapper repair now forwards optional `limit` and `before_seq` through `URLSearchParams`, preserving the existing array return type and direct-revision URL. It has not yet been rerun. No backend production or HistoryPanel production changes have been made.

New tests awaiting root execution:

- `ui/unit/workbenchHistoryPaging.test.ts` now contains the wrapper case plus seven production HistoryPanel handler cases: bounded exclusive page requests/replacement, superseded success/error/loading, A→B→A response ownership, revision captured across approval, document switch before approval, and old restore completion after document switch. Command from `ui/`: `node --test unit/workbenchHistoryPaging.test.ts` (expected subsecond, unmeasured). The function/primitive-initializer harness controls transport and component state; mounted reactivity remains covered separately by the authored browser spec, not claimed here.
- `crates/ottod/src/mcp_tools.rs` has two test-only cases exercising the real catalog and `run_tool` handler against an isolated loopback Axum response fixture. They require `history_limit` default100/max200, exclusive `before_seq` query forwarding, preserved direct revision1 lookup, and `next_before_seq` continuation metadata (full page yields last seq; short page yields null). Command: `cargo test -p ottod workbench_history_` (roughly subsecond execution plus compilation, unmeasured). The fixture uses a fallback rather than fictitious literal routes, preserving route-inventory scanners. No external daemon or user state is contacted.

MCP continuation semantics agreed with root: a full page supplies a cursor hint, not a claim that another row exists; requesting a final empty page is valid. No `has_more` assertion is invented from fullness. Backend HTTP/MCP repairs still await central observed RED. Shell slot released after this checkpoint; no local tests/builds/servers/commits performed.


## HistoryPanel source checkpoint

Root ran the eight-case Workbench UI batch: **1 passed / 7 intended failures**, 269ms. The wrapper is green; bounded paging, stale success/error/loading, A→B→A and restore approval/completion ownership were confirmed RED. Root authorized the HistoryPanel repair.

`HistoryPanel.svelte` now requests/render-retains 100 revision metadata rows, replaces the current page, and provides Older/Newer navigation using a trail of scalar cursors rather than accumulated revision objects. Revision-number comparison reaches a revision outside the current page; direct selection/restore remains available on every older page. Document visits, per-load generations and actual destruction fence asynchronous publication. Restore captures workspace/document/revision before confirmation, guards both mutation and completion, and invalidates synchronously on Close.

The unit fixture now evaluates real rune/primitive initializers and lifecycle registration, then establishes the initial production document ownership before applying user state. Added actual destruction and A→B→A confirmation controls plus newer-navigation coverage; **11 tests total await root rerun** via `node --test unit/workbenchHistoryPaging.test.ts` from `ui/`. The extra lifecycle/navigation controls were authored with the authorized repair; no independent pre-fix execution is claimed for those three. Mounted browser/typecheck remain pending. Rust and MCP production still unchanged pending their RED runs.


Root's HistoryPanel rerun passed **10/11**, 292ms: all seven original failures plus destruction and approval ABA are green. The additional navigation case inspected state after only one host microtask while the async response crossed the VM boundary. Navigation helpers now return their existing load promise, and the fixture awaits that completion instead of assuming a microtask count. Paging semantics are unchanged. This is a synchronization correction, not evidence of a new pagination defect; the corrected eleven-case rerun remains pending.


## Broker/Git observed RED and production checkpoint

Root established broker **0/5**, 12.24s compile / 4.51s test: binary key/header loss, tombstone versus present-empty loss, public empty-text contract mismatch, and a byte64 UTF-8 preview panic. Root corrected only the lazy SQLite fixture's ambiguous `pool.into()` to `pool`. Git's aggregate test was **0/1**, 19.76s compile / 0.32s test: all32 patch subprocesses launched despite a4KiB response cap. Root separately confirmed the corrected HistoryPanel batch **11/11 green**.

Authorized repairs now present, awaiting central verification:

- `crates/otto-brokers/src/kafka.rs`: an internal byte-slice producer adapter preserves optional key/value and arbitrary existing header bytes; the public text/base64 adapter now honors documented empty-text tombstone versus present-empty base64 semantics.
- `crates/otto-brokers/src/service.rs`: replay bypasses lossy text encoding for raw key/value/header data, retains field-specific text transforms, and constructs previews by Unicode scalar iteration rather than a byte offset.
- `crates/otto-git/src/local.rs`: Working diff parses/releases each untracked patch under the remaining exact rendered budget, with bounded eight-child read-ahead instead of collecting every raw output. Exhaustion stops further patch consumption/spawning and supplies summary metadata for remaining paths with explicit omission/truncation. Per-child safety cuts preserve counts through summary rows and report too-large. Uncapped internal calls remain complete or fail explicitly at128MiB aggregate raw bytes instead of silently losing content. This bound is an implementation safety ceiling, not a measured RSS claim. Child cancellation remains backed by the existing LocalRead process cleanup; independent cancellation/load measurement is pending.

New controls in `diff_tests.rs`: `working_diff_streaming_budget_is_shared_by_tracked_and_untracked_files` and `working_diff_streaming_large_file_keeps_later_small_file_and_exact_counts`. Run `cargo test -p otto-git --lib working_diff_` to include them and the original failure. Existing default/non-Working diff tests should remain in the broader affected-crate gate.

Adjacent nullable-header loss is independently confirmed by source (`consume_raw` previously flattens null header values), and root requested inclusion. Added actual protocol regression `replay_preserves_nullable_headers_without_changing_text_transform`: direct producer seeds null and present-empty headers, service replay executes, and a separate librdkafka consumer observes nullable target headers without using the flattening production decoder as its oracle. Run `cargo test -p otto-brokers --lib replay_preserves_nullable_headers_without_changing_text_transform`. **Nullable-header production changes have not been made; this additional RED is pending.**

Current production sources are coherent for central compilation. No builds/tests/servers/commits were run by this implementer. Rust formatting is centrally owned for this checkpoint. Workbench backend and MCP remain test-only until observed RED.


## Aggregate raw-work follow-up tests (production unchanged)

Root's source recheck identified that individually cut/too-large files charge zero retained render bytes, allowing every oversized file to launch, and that an unlimited per-file cap still permits128MiB child captures despite a small total allowance. Two new real-Git controls are ready for central RED:

- `working_diff_many_cut_files_stop_aggregate_raw_work`:32 ×256KiB files,1KiB per-file/4KiB aggregate display cap; limits patch launches (bounded read-ahead allowance16), preserves all32 summary/count/status rows, and requires explicit truncation.
- `working_diff_small_total_bounds_child_capture_with_unlimited_file_cap`:2MiB one-line file, unlimited per-file/4KiB aggregate cap; a wrapper's post-Git completion marker proves whether the whole patch was drained. The small-budget read must terminate its LocalRead process group before completion, retain explicit omitted metadata, while a separate uncapped per-file read remains complete.

Run `cargo test -p otto-git --lib working_diff_many_cut_files_stop_aggregate_raw_work` and `cargo test -p otto-git --lib working_diff_small_total_bounds_child_capture_with_unlimited_file_cap`; rerun the existing `working_diff_streaming_large_file_keeps_later_small_file_and_exact_counts` control alongside repairs. These are work/capture assertions, not direct peak-RSS measurements. Expected seconds plus compilation, unmeasured. No corresponding production change has been made pending root-observed RED; source stays at the prior coherent checkpoint.


## Final authorized backend source checkpoint

Central results received: original broker regressions **5/5 green**; nullable-header case **RED** (`None` became `Some([])`, with fixture generic corrected to `Header::<&[u8]>`); original Git budget/controls **3/3 green**, follow-up raw-work controls **2/2 RED** (32 discarded-body patch children and full child drain); Workbench HTTP **RED** (205 versus7 and ignored50k versus100); MCP **0/2 RED**,2m45s compile /0.01s test (catalog missing history_limit and real handler missing limit query).

All corresponding authorized source repairs are now stable for central verification:

- Raw Kafka headers retain `Option<Vec<u8>>` through consume, replay transforms and production; untouched null and present-empty headers stay distinct. The existing public display header text remains its contract shape; no public response type changed. The real protocol oracle still observes nullable target headers independently.
- Git retains root's owned-String stream correction for Send. A separate aggregate raw collection ceiling (`2 × total_bytes +64KiB`, at most128MiB) counts discarded/cut bodies too; child capture is the lesser of the per-file and aggregate ceilings. Read-ahead is bounded at eight children. Raw-budget omission is explicit; one per-file cut still leaves allowance for a following small file. Internal complete reads fail explicitly above the128MiB aggregate safety limit. Metadata counting remains a streamed disk pass, and no RSS/CPU improvement is claimed without measurements.
- Workbench state listing now has bounded default100 and keyset `list_revisions_page(limit, before_seq)` with1–200 clamping. HTTP query extraction forwards that cursor. The existing `(doc_id,seq)` primary key supports the range; no migration was added. Direct detail/diff/restore and all stored history remain intact.
- MCP catalog/handler expose `history_limit` and exclusive `before_seq`, forward bounded HTTP parameters, validate integer types, and return `next_before_seq` as a continuation hint only. The Workbench-only API contract segment documents default/max/exclusive semantics and short/empty completion. Existing `WorkbenchRevision[]` and UI wrapper option typing remain aligned; no global types edit was needed.

Changed production files in this checkpoint: `crates/otto-brokers/src/{kafka,service}.rs`, `crates/otto-git/src/local.rs`, `crates/otto-state/src/workbench.rs`, `crates/otto-server/src/routes/workbench.rs`, `crates/ottod/src/mcp_tools.rs`, and Workbench-only `docs/contracts/api.md`. Broker assertions were updated for the internal nullable header type without changing their expected bytes. No tests/builds were run here; `git diff --check` of owned edits was clean before this report append. Root owns formatting and central test execution. Independent source review, full gates, mounted history browser acceptance and runtime performance evidence remain pending, not implied by the earlier focused greens.
