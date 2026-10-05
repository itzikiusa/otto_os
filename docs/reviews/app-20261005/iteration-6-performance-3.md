# Iteration 6 — performance partition 3

**Verdict: Approve the bounded repairs; performance acceptance remains incomplete.** No new proven findings. Fixed-rubric provisional score **9.1/10**, up from iteration 5's unchanged **8.5/10**. This is an evidence-only rescore for Vault/Canvas/Design Hall/Product/Browser/Snip, not a new source audit or a whole-application performance score.

Candidate checkpoint supplied by root: `c425aa82` plus the Audit draft. Scale/sustained runtime was measured at `64a850e6`; later History/Audit/version/tour changes are not that exact measured revision. Reviewed the iteration-5 report, current PERFORMANCE.md, measurements/shared-diff.json, measurements/scale-analysis.txt, and the current acceptance entries in VERIFICATION.md. No tests, builds, browser runs, benchmarks, source edits or git mutations were performed. Only this report was written.

## New evidence and what it establishes

The actual mounted shared DiffView fixture passed within the **18/18** acceptance run (`/tmp/otto-review06-acceptance.log`). Equal 50k lines painted in **16.3 ms with 0 rows**, sparse 50k lines in **43.4 ms with 18 rows**, and rewritten 10k lines in **30.7 ms with 500 rows**. These observations include two animation frames; they are not latency SLO assertions. All **20** rewrite pages reached the final line 9999, and before/after downloads matched full source bytes. One **54 ms** long task was observed, so this is not a zero-long-task result. Forced-GC JS heap was **2.94 MiB baseline → 12.15 MiB loaded rewrite → 4.32 MiB released**. This directly confirms bounded mounted output and substantial release of fixture allocations; one release checkpoint does not prove a sustained memory plateau or native allocator recovery.

The same 18-case run exercised failed/empty/accepted assistance in all three Canvas formats and Vault backlink retry. The subsequent **7/7** recovery run includes a **3 MB Canvas save and version restore**, cross-scene failed-draft recovery, Excalidraw failed hand-edit recovery, Vault open retry/stale failure and failed-save draft preservation (`/tmp/otto-review06-recovery.log`). These establish mounted journeys and state recovery; no Canvas/Vault throughput or retained-memory conclusion is inferred. Root reports final UI check **0 errors/0 warnings** and **1335 unit passes**.

Current synthetic session scale retry passed **375.4 seconds / 66 samples / N=0/1/3/5 / zero leftovers**. At N=5, steady daemon CPU was **2.3–2.6%** and renderer **12.7–14.8%**, with post-GC RSS **272.4/712.4 MiB** respectively; GPU and other process groups are additional costs. Recovery renderer RSS was **699.3 MiB**, versus **530.1 MiB** at a different-view baseline. These session/Home/Git workloads do not measure Product imports or content comparisons under concurrent load.

The requested 15-minute sustained run **aborted at 413.9 seconds** when host load reached **12.03 > 12**; cleanup finished with zero leftovers. Its 77 partial samples lack final recovery/GC. The earlier scale attempt also safety-aborted and is not counted as a pass. RSS attribution and plateau remain unresolved; neither stable DOM nor released JS heap establishes no leak. Read-only installed-app sampling (25 samples/120 seconds) supplies real-process context, but used a different installed revision and did not attribute native WebKit/GPU helpers.

## Fixed PLAN dimensions

| Dimension | /2 | Evidence and concrete remaining deduction |
|---|---:|---|
| Query/network work | 2.0 | Retains iteration-5 directly verified thin transcript projection, indexed keyset paging and lazy/detail/search HTTP evidence. No new material gap in this bounded query assessment. |
| Algorithmic/serialization cost | 1.9 | Budgeted diff plus actual 50k-line equal/sparse and 10k rewrite/page/download evidence. Deduct 0.1: near-25-MiB JSON and long-single-line comparison costs remain unmeasured. |
| Retained memory/lifecycle | 1.8 | Cache bounds and actual mounted diff release now have direct evidence. Deduct 0.2: repeated Product body/compare open-close recovery at representative sizes remains unmeasured, and the sustained run lacks a completed final-GC/plateau result. |
| Scheduling/responsiveness | 1.9 | Mounted diff paint/row observations and three-format Canvas recovery now close the prior basic mounted gap. Deduct 0.1: content interaction/long-task behavior under N=1/3/5 session load and native WebKit remain unmeasured; the isolated fixture observed one 54 ms task. |
| Representative CPU/RAM/latency measurements | 1.5 | Current synthetic scale, installed-app context and mapped mounted diff latency/heap measurements establish substantial additional evidence. Deduct 0.5: no completed 15-minute sustain/recovery, no matched Product/Design Hall CPU/RAM series under concurrency, and no native helper attribution. |
| **Total** | **9.1/10** | **Improvement reflects new executed evidence; no automatic 1.9 ceiling. The 9.8 target remains unmet.** |

## Finite remaining acceptance

1. Complete the planned 15-minute N=3 sustained run with the same safety limits and comparable beginning/end views and GC checkpoints; explain renderer RSS separately from JS heap. Preserve aborted attempts.
2. Measure the repaired Product transcript workload (1/20/100 × 256 KiB, page/expand/find/cancel/close) and Design Hall compare open/close under N=0/1/3/5 background sessions; record response bytes, daemon/renderer CPU, heap/RSS and interaction/long-task latency.
3. Add the existing maximum-artifact acceptance cases: near-25-MiB JSON and a long single line, with full-source reachability and post-close recovery. Confirm native WebKit behavior and attribute native helpers separately from the different-revision installed-app observation.

Prior Vault graph-degree, Canvas history-byte, Browser live-page and Snip pixel-workload limits remain explicit coverage exclusions. This bounded rescore adds no speculative defect or new optimization requirement in those surfaces. Both original P3 majors remain closed on the evidence preserved in iteration 5.
