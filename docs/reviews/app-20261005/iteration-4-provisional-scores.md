# Iteration 4 — current provisional scores after repairs

Evidence checkpoint: **2026-10-05, through the final broker replay 6/6 GREEN** in [VERIFICATION.md](VERIFICATION.md). Branch `fix/app-review-20261005`, current repair work over baseline `03f2bc3e`. Subsequent grouped state/Git/Product execution is in progress and receives no prospective credit here. This is an evidence-led aggregation of the original twenty reviews, implementation reports, independent rechecks and centrally observed execution, not a new discovery pass or final independent acceptance review. No source changes, tests or builds were performed for this report.

**Current provisional means: correctness 8.72/10, performance 8.40/10, UX 8.30/10.** The respective minimum partitions are **8.3, 8.0 and 7.9**. These describe the repaired checkpoint; the lower historical pre-fix scores remain preserved in [SCORES.md](SCORES.md). No category meets the requirement that every partition reach 9.8. Confidence is moderate overall, stronger for named reproduced/repaired behaviors and weaker for mounted/native behavior, broad integration and current performance measurements.

## Current partition totals

| Stable partition | Correctness | Performance | UX |
|---|---:|---:|---:|
| 1. Shell/navigation, agents/sessions, terminals/transcripts, conversation/history | 8.8 | 9.0 | 8.3 |
| 2. Git/workbench, databases/connections, brokers, API client | 8.7 | 8.0 | 8.3 |
| 3. Vault/canvas/Design Hall/product/browser/snip | 8.9 | 8.1 | 8.5 |
| 4. Workflows/loops/swarm/scheduling/Mission Control/MCP | 8.9 | 8.2 | 8.5 |
| 5. Settings/plugins/usage/insights/home/share/auth/cloud, rooms/assistant/personal agents/proof/shared components | 8.3 | 8.6 | 7.9 |
| **Mean** | **8.72** | **8.40** | **8.30** |
| **Minimum** | **8.3** | **8.0** | **7.9** |

All dimensions below use the unchanged [PLAN rubric](PLAN.md): five equally weighted dimensions, each /2 in 0.1 increments. None receives 2.0. Partition 3's independent [current provisional assessment](iteration-4-provisional-3.md) is preserved exactly. Other partitions are checkpoint synthesis judgments, not scores attributed to an original reviewer who has not rescored. Cross-partition tests can support multiple relevant judgments but their counts must not be added into a unique total.

## Correctness dimensions

| Partition | Contract/data integrity | State/concurrency ownership | Boundary/error behavior | Persistence/recovery | Executed regression coverage | Total |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 1.8 | 1.8 | 1.8 | 1.8 | 1.6 | **8.8** |
| 2 | 1.8 | 1.8 | 1.8 | 1.8 | 1.5 | **8.7** |
| 3 | 1.8 | 1.8 | 1.8 | 1.8 | 1.7 | **8.9** |
| 4 | 1.8 | 1.8 | 1.8 | 1.8 | 1.7 | **8.9** |
| 5 | 1.8 | 1.4 | 1.8 | 1.7 | 1.6 | **8.3** |

- **P1:** Independent recheck accepts resume, transcript and creation ownership repairs. Actual session resume 3/3 covers live PTY preservation, authorization and concurrent inactive starts. The current 39/39 UI batch includes bounded prefetch, collapse/reopen ownership, byte limits, adaptive pagination reachability and workspace-bound partial-creation retry. Coverage 1.6 exceeds substantial handler coverage because actual process/admission behavior also executes; it remains below 1.8 because mounted History and native/provider permutations are pending. Contract, errors and persistence retain bounded deductions for those integration seams and the breadth of the partition.
- **P2:** The final 80/80 UI ownership/dialog batch and independent follow-up support originating-connection guards, workspace visits including A→B→A, cancellation, preservation of later edits/secrets and partial collection saves. Workbench UI paging has 11/11 named controls. Broker replay now passes 6/6 through the isolated Kafka MockCluster, including binary bytes, nullable versus empty headers, tombstones, transforms and public produce semantics. Git raw-work/capture limits and Workbench HTTP/MCP paging have independent source approval, but their final post-repair executions are still pending at this checkpoint. Those pending protocols and VM identity-rune/component-function harnesses justify coverage 1.5 and prevent treating the four 1.8 source/behavior judgments as complete acceptance.
- **P3:** Exact independent dimensions retained. Current coverage includes UI recovery/ownership, Product HTTP 24/24, actual server approval/publication paths in 30/30 and Design library 128 passed/1 ignored. Latest Product byte-projection rerun, final consumer integration and mounted/native content journeys remain pending. See the partition report for the individual dimension deductions.
- **P4:** Atomic version publication/rearm state tests pass 3/3; the current actual server 30/30 covers pause/settlement, stale trigger admission, queue-failure rollback, same/different-trigger overlap, captured cursor settlement, pinned versions, bounded history and displayed approval identity. Goal/form/cache ownership has 34/34 and workflow history UI 22/22 named controls, with independent source follow-ups accepting the repairs. Coverage 1.7 credits substantial actual route/engine/persistence seams beyond handlers, while retaining mounted forms, broader automation/MCP and final consumer gaps. The four 1.8 dimensions retain those bounded integration deductions.
- **P5:** Platform ownership/compiler follow-ups pass 26/26; recap client behavior passes 10/10, and current backend revision tests pass 5/5 including ctime-only replacement detection and permission controls. The known **unkeyed RoomRecap modal archive-identity defect remains open**: changing archive A→B can retain A's page/request ownership. That concrete significant defect sets state/concurrency to 1.4 and constrains persistence/recovery to 1.7; this is not merely a missing test deduction. Coverage 1.6 credits current client, compiler and archive behavior but excludes the unrun mounted identity regression, current authenticated revision-route rerun and broad platform matrix. Contract/error scores retain those bounded gaps.

## Performance dimensions

| Partition | Query/network work | Algorithmic/serialization cost | Retained memory/lifecycle | Scheduling/responsiveness | Representative CPU/RAM/latency measurements | Total |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 1.9 | 1.8 | 1.8 | 1.9 | 1.6 | **9.0** |
| 2 | 1.7 | 1.8 | 1.8 | 1.7 | 1.0 | **8.0** |
| 3 | 1.8 | 1.8 | 1.8 | 1.7 | 1.0 | **8.1** |
| 4 | 1.8 | 1.8 | 1.8 | 1.8 | 1.0 | **8.2** |
| 5 | 1.9 | 1.9 | 1.8 | 1.8 | 1.2 | **8.6** |

- **P1:** Existing bounded request/scheduling behavior and applicable previous session concurrency/sustained evidence retain their credit; repaired child-prefetch/cache ownership now has named bounds and lifecycle tests, supporting memory 1.8. Algorithmic work retains 1.8 because representative child-heavy scale remains unmeasured. Measurement 1.6 credits the previously executed N=0/1/3/5 session workloads, sustained run and bounded real-app samples where they map to this partition. It does not establish the new child-heavy workload or resolve the prior renderer RSS trend.
- **P2:** Git now has separate rendered/raw-work/capture budgets by source, and Workbench has bounded indexed history plus exposed HTTP/MCP pagination. Final post-repair protocol/budget execution is pending, yielding query 1.7. Streaming, bounded buffers and lifecycle cleanup support the algorithm/memory judgments; actual scheduling latency is still unmeasured, yielding 1.7. Exact omitted-file counts still require linear disk scanning: no bounded total disk-work claim. Measurement 1.0 credits the verified baseline without pretending session samples represent large diffs, DB/Kafka or Workbench. Broker semantic fixtures establish integrity, not CPU/RAM scalability.
- **P3:** Independent 8.1 retained: thin Product pages, bounded diff expensive stages/rendered rows, body-cache bounds and cancellation have actual scoped evidence. Total diff metadata and explicit downloads retain input-sized work. Latest bundled-engine byte-projection validation, mounted compare/find latency and representative CPU/RAM/sustained memory remain pending.
- **P4:** Thin bounded workflow history, bounded cache bytes, captured-generation admission and scoped response ownership improve all four source-cost dimensions to 1.8, with current paging/engine regression support. Broad scheduler bursts, mounted long-task behavior and sustained CPU/RAM remain unmeasured; measurement 1.0 is baseline credit only.
- **P5:** Actual recap instrumentation shows zero event opens, draft reads and index builds across fifteen unchanged polls for all four 1/100-event × 2/20KiB fixtures, with positive detail-read controls. Serialized revision traffic is roughly 10.5–10.7KiB across those polls, compared with the recorded full-detail baseline of roughly 42KiB–30.9MiB. This directly supports reduced network/serialization work, not whole-app speedup. Measurement 1.2 credits current quantitative workload-scaled I/O/wire evidence beyond unmapped baseline, but stays below substantial representative CPU/RAM/latency coverage. Lifecycle 1.8 and scheduling 1.8 retain the open identity boundary and absent mounted/sustained evidence; no measured memory recovery claim.

## UX dimensions

| Partition | Task completion/discovery | Feedback/state clarity | Recovery/retry | Draft/scope/trust preservation | Executed end-to-end journeys | Total |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 1.8 | 1.7 | 1.8 | 1.8 | 1.2 | **8.3** |
| 2 | 1.8 | 1.8 | 1.8 | 1.8 | 1.1 | **8.3** |
| 3 | 1.8 | 1.8 | 1.8 | 1.8 | 1.3 | **8.5** |
| 4 | 1.8 | 1.8 | 1.8 | 1.8 | 1.3 | **8.5** |
| 5 | 1.8 | 1.7 | 1.8 | 1.5 | 1.1 | **7.9** |

- **P1:** Safe reopen/resume, complete transcript navigation and partial-failure retry now have concrete behavior evidence. Feedback 1.7 retains the loaded-search/live-status presentation and integration gap; external Claude work is not automatically credited as integrated. Journey 1.2 adds actual resume/process interaction beyond baseline, but authored mounted History/session flows have not run and mixed-provider browser creation remains fixture-bound.
- **P2:** Origin-specific confirmations, canceled-dialog ownership, retained environment/request drafts and secrets, import scope and recovery retry have source plus scoped handler controls. Each of the first four dimensions retains mounted Svelte/keyboard/native deductions. Journey 1.1 credits narrow actual broker/backend interactions beyond baseline, while most multi-step data tools/API/Git human journeys are still handlers or pending protocols; 80 passing unit cases are not 80 end-to-end journeys.
- **P3:** Independent 8.5 retained. Actual preview→review rejection/retry→exact isolated publication executes through API/engine; recovery/find/paging controls support the other dimensions. Journey 1.3 reflects that real multi-step service path without a mounted human UI or native content matrix.
- **P4:** Persistent-page leave behavior, later-edit preservation, retry and stale-workspace launch ownership pass production-handler controls. Actual version/admission/approval routes support multi-step automation behavior, yielding journey 1.3 above baseline; human schedule/goal/workflow forms and broader Mission Control/swarm/MCP journeys remain unmounted. The four 1.8 dimensions explicitly retain those rendered/integration gaps.
- **P5:** Account Retry, coherent provider/model retry, proof selection ownership and one-time token preservation improve the source/handler task paths. The open recap archive identity issue directly affects visible scope/trust (1.5) and feedback (1.7). Journey 1.1 credits current service/client interaction beyond baseline, not mounted end-to-end proof; platform settings/auth/cloud/rooms coverage remains broad and incomplete. Recovery 1.8 acknowledges verified backoff and draft/model retry, with mounted identity recovery still pending.

## Evidence boundaries and acceptance work

The durable sources are the fifteen original `iteration-4-{correctness,performance,ux}-{1..5}.md` reports, five original design reports as contextual inputs, implementation reports, `iteration-4-role{1..5}-*recheck.md` and correctness follow-up appendices, [partition 3's score report](iteration-4-provisional-3.md), and [VERIFICATION.md](VERIFICATION.md). Earlier RED results remain historical evidence after a named repair passes; an independent source disposition alone does not turn a pending final execution GREEN.

At this checkpoint:

- **Known open behavioral defect:** RoomRecap archive identity. The authored mounted regression and repair/green result remain required.
- **Pending focused executions:** final Git raw-work/capture budgets, Workbench HTTP/50k history and MCP paging, authenticated recap revision route, and latest Product summary byte-projection validation. Later results should append a dated disposition rather than silently rewriting this snapshot.
- **Pending integrated gates:** current affected consumers and combined UI/Rust checks after all repairs and Claude integration. An earlier UI check was clean, but predates later changes. The broad state run was **376 passed, 1 failed, 2 ignored**; its timing-sensitive Proof test passed alone. That is not an all-green broad gate and has not been discarded from the assessment.
- **Pending mounted/native acceptance:** changed journeys, light/dark/accessibility and relevant native provider/WebKit/clipboard/desktop behavior. Handler extraction and identity-rune VMs do not validate mounted reactivity. Passing Rust service fixtures do not establish human task completion.
- **Pending representative performance:** current affected workloads at N=0/1/3/5, CPU/RAM/latency, sustained memory and read-only real-app sampling; investigation of the prior renderer RSS trend. Counters, query plans and test duration do not substitute for these measurements.

Claude's separate design source rescore is **9.96 mean / 9.80 minimum** under its unchanged design rubric. It is not folded into these C/P/UX scores and does not certify rendered or integrated acceptance. The current means show material verified progress over the pre-fix review, while the minimum partitions and explicit execution deductions prevent a target score from concealing the remaining work.

## Same-day execution addendum received at publication

Root reports the grouped nextest CI selection completed **106/106 GREEN, 636 filtered/skipped, 5.523s** across `otto-state`, `otto-git` and `otto-product`, selected by `working_diff_|review4_|workflow|scheduled|workbench|http::tests::`. This includes all five Git raw/capture/render-budget regressions, the latest Product `octet_length` HTTP cases and broader selected workflow/scheduled/state history consumers. Those two focused pending items above are now satisfied at that scope; Workbench actual HTTP/50k and MCP final results, recap authenticated route, mounted/native and full integrated/performance acceptance remain pending. This bounded pass does not erase the previous broad state timing failure or certify excluded tests. Scores remain the stated provisional checkpoint judgments; no automatic numerical uplift is inferred from another passing batch, and partition 3's supplied scores remain intact.
