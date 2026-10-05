# Iteration 6 — final bounded scores

All twenty same-role follow-up reports are complete. They recheck the existing repairs, merged interactions and recorded acceptance evidence; they are not a fresh whole-application audit. The unchanged PLAN rubric applies. Earlier iteration and provisional scores remain preserved in their reports. Reviewers ran no builds, tests, browsers or benchmarks; execution is inherited from the central verification ledger.

| Category | Mean | Minimum partition | Result under the every-partition 9.8 target |
|---|---:|---:|---|
| Correctness | **9.88** | **9.8** | Meets the bounded review target |
| Performance | **9.20** | **8.9** | Target not reached; measurement gaps remain |
| Internal five-partition static design | **10.00** | **10.00** | Static source target met; rendered/native limits remain |
| External ten-lens static design | **9.96** | **9.80** | Separate external source assessment; not runtime certification |
| UX | **9.82** | **9.8** | Meets the bounded review target |

**The overall 9.8 goal is not achieved because performance remains below it.** These judgments certify only the stated repair/acceptance matrices. A 10.0 bounded score does not claim the app or every module is defect-free.

## Partitions and dimensions

| Partition | Correctness | Performance | Internal static design | UX |
|---|---:|---:|---:|---:|
| 1 Shell, sessions, transcripts, History | 9.9 | 9.5 | 10.0 | 9.8 |
| 2 Git, Workbench, data tools, API | 9.8 | 9.1 | 10.0 | 9.8 |
| 3 Content, Canvas, Product, Browser | 9.8 | 9.1 | 10.0 | 9.8 |
| 4 Automation, scheduling, MCP | 9.9 | 8.9 | 10.0 | 9.9 |
| 5 Platform, rooms, personal agents | 10.0 | 9.4 | 10.0 | 9.8 |

Dimension order is exactly PLAN's order: correctness = contract/data, ownership, boundary/error, persistence/recovery, executed regressions; performance = query/network, algorithm/serialization, memory/lifecycle, scheduling, representative measurements; UX = completion/discovery, feedback, recovery, draft/scope/trust, executed journeys.

| Partition | Correctness dimensions | Performance dimensions | UX dimensions |
|---|---|---|---|
| 1 | 2.0 / 2.0 / 2.0 / 2.0 / 1.9 | 2.0 / 2.0 / 1.9 / 1.9 / 1.7 | 2.0 / 2.0 / 2.0 / 2.0 / 1.8 |
| 2 | 2.0 / 1.9 / 2.0 / 2.0 / 1.9 | 1.9 / 1.9 / 1.9 / 1.9 / 1.5 | 2.0 / 2.0 / 2.0 / 2.0 / 1.8 |
| 3 | 2.0 / 2.0 / 2.0 / 1.9 / 1.9 | 2.0 / 1.9 / 1.8 / 1.9 / 1.5 | 2.0 / 2.0 / 2.0 / 2.0 / 1.8 |
| 4 | 2.0 / 2.0 / 2.0 / 2.0 / 1.9 | 2.0 / 1.9 / 1.9 / 1.8 / 1.3 | 2.0 / 2.0 / 2.0 / 2.0 / 1.9 |
| 5 | 2.0 / 2.0 / 2.0 / 2.0 / 2.0 | 2.0 / 2.0 / 1.9 / 2.0 / 1.5 | 2.0 / 2.0 / 1.9 / 2.0 / 1.9 |

Design uses its fixed severity deductions, with all five evidence dimensions described in the reports rather than invented numeric /2 values. The internal static score closes the prior MCP Audit hover-only detail minor after its persistent native disclosure repair. The external ten-lens aggregate remains separate.

## Repair disposition and evidence checkpoint

The four iteration-5 correctness findings are repaired: departed History resume ownership, dismissed/reopened Product publication ownership, atomic scheduled admission after pause/retime, and Personal Agents workspace-return list ownership. Independent round-6 rechecks accepted them. The remaining History lazy-destination ownership branch was reproduced, repaired and independently rechecked; mounted and 13-case unit logs were inspected. Root also reproduced Product initial routed selection being replaced during workspace boot, fixed the effect/dependency order, and passed the uninstrumented initial-route/Keep/Discard regression three times. C3 independently approved the fix. No confirmed blocker or major remains open in this bounded matrix.

Final central integrated UI evidence is **0 errors / 0 warnings**, **1,335 unit tests**, production build and unchanged bundle budgets GREEN after the final source integration. Earlier Rust formatting, strict clippy, doc-tests and build passed; the scheduler/index selection passed **57/57**. The historical broad Rust invocation remains **4,694 passed / 2 failed / 86 skipped**, followed by individual passing reruns of both failures. We do not rewrite that invocation as an all-green broad run.

Named mounted execution adds **18/18** acceptance cases; **7/7** follow-up recovery cases; **1/1 WebKit** combined recap revision failure→Retry→identity change→late obsolete response; **3/3** repeated Product routing cases; and the History/MCP disclosure controls. These are overlapping named batches, not a deduplicated grand test count. Exact commands and logs are in [VERIFICATION.md](VERIFICATION.md). Sources advanced from measured `64a850e6` through History `c425aa82`, Product `552b6f84`, integration `0c4862ab` and final UI checkpoint `0b685e73`; report-only later commits do not imply new browser runs.

## Why performance remains below target

Current N=0/1/3/5 scale execution completed in **375.4 seconds**, **66 samples**, with **zero leftover test processes**. Mounted child inspection/paging and large shared-diff workloads also passed. The repeated child workload's p95 is **118.70 ms**; **82.78 ms** belongs to the earlier initial workload and must not be substituted. Actual 50k/10k diff paging and complete download bytes add useful bounded measurement evidence.

The required full **15-minute sustained run did not complete**: the safety guard stopped it at **413.9 seconds** when host load reached **12.03**, retaining zero leftover test processes. Renderer/browser RSS remained elevated; even the second 100-cycle child workload retained approximately **14 MiB** more RSS while heap declined. This does not prove a leak, and it does not establish recovery or leak absence. Session CPU/RAM cannot certify unrelated Git/DB/broker, content or automation workloads. These limitations produce the real 9.20 mean rather than an automatic uplift to the requested number. See [PERFORMANCE.md](PERFORMANCE.md) and its durable measurements.

Finite remaining performance acceptance: complete a safely controlled sustained/recovery run with CPU/RAM attribution; map representative Git/Workbench/API lifecycle and data-tool costs; measure concurrent Product/Design Hall content memory/latency; measure scheduler/history/MCP workload costs; and measure recap lifecycle/resource behavior directly beyond protocol counters. No new hot-path defect was asserted merely because those measurements are incomplete.

## Remaining bounded acceptance limits

- P1: older-only transcript search and disconnected LiveStatus mounted journeys; some workspace-return navigation permutations. The proven departed/lazy-route resume defects are closed.
- P2: overlapping routed leave/superseded confirmation, mounted save-sheet cancellation, Git Refresh failure recovery, import preserving its draft without query submission, and one-collection partial-save retry. Existing handler controls remain credited. Workbench retry/insertion and environment newer-edit/secret-return cases now have mounted passes.
- P3: exact mounted Excalidraw settings version restore→hand edit→persistence sequence; native Snip clipboard/CDP boundaries. Canvas three-format assistance/recovery, Vault failures, diff access and Product initial/dirty-route cases are mapped to executed passes.
- P4: mounted workflow-version failed-page Retry→oldest-version restore. Scheduled Tasks save/leave/newer-edit and MCP disclosure gaps are closed.
- P5: native one-time token clipboard rejection/manual-copy/Done path remains controlled-test-only for UX. The bounded correctness matrix's Personal Agents and combined recap recovery gaps are closed, supporting its 10.0 without inventing a new ceiling.
- Design: complete light/dark/phone/native assistive-technology acceptance is broader than the supplied publication and MCP captures. Static 10.0 is not a rendered/native acceptance claim.

The final report inventory is `iteration-6-{correctness,performance,design,ux}-{1..5}.md`. C1, C2, C3, C4 and C5 retain their earlier provisional checkpoints and append final evidence calibrations. Scores are copied from the responsible reviewers; no earlier iteration report was replaced.
