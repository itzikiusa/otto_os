# Iteration 6 — performance partition 1

**Verdict: Approve the inspected repairs; sustained performance acceptance remains incomplete.** No new proven performance findings: blocker 0, major 0, minor 0, nit 0. **Score: 9.5/10**, compared with the preserved iteration-5 provisional 9.4. This is an evidence rescore, not another broad scan.

Reviewed `PERFORMANCE.md`, the latest `VERIFICATION.md`, `measurements/child-recovery.json`, `measurements/scale-analysis.txt`, partial sustained sample boundaries and the 18/18 passing browser log `/tmp/otto-review06-acceptance.log`. Scale/sustained source is `64a850e6`, frozen Vite with fresh debug ottod, synthetic providers, headed Chromium/WebGL. Later History/navigation repairs and final version/tour integration postdate that measured revision. The repeated-child browser measurement uses default headless Chromium and must not be compared in absolute RSS with the headed run. Applied the previously read performance-review skill. No tests, builds, browsers, benchmarks, source edits or git mutations were run; only this report was written.

## New mapped evidence

The full N=0/1/3/5 scale retry completed in 375.4 seconds: 66 samples, exit zero and zero teardown leftovers. At N=5, steady daemon CPU was 2.3–2.6% and renderer CPU 12.7–14.8%, depending on view. GPU added 15.1–21.0% and ClickHouse 1.2–1.3%; the first two columns are not total-app CPU. Post-GC daemon/renderer RSS reached 272.4/712.4 MiB with 32.0 MiB JS heap; recovery was 272.8/699.3 MiB and 26.0 MiB heap. Recovery CPU approached idle. This maps directly to ordinary P1 terminal/session view lifecycle, with no demonstrated CPU cliff at five synthetic sessions; it does not establish memory convergence.

The actual mounted child journey now passes 100 expand/collapse inspections, 1,200 turns paged in both directions with 60 mounted turns per page, then 100 further inspections. Durable repeated-run evidence reports **p95 118.70 ms**, p50 100.17 ms, and zero observed long tasks. The earlier initial run's p95 82.78 ms is a separate observation and must not replace the repeated-run value. Forced-GC heap was 20.81→21.20→24.44→23.57 MiB; child bodies were zero at checkpoints, with DOM nodes settling from 641 to 632. The last 100 inspections reduced heap while browser-total RSS still increased 14.25 MiB, from 680.02 to 694.27 MiB. This substantiates bounded payload release and bidirectional reachability, but does not identify the RSS owner or close a leak investigation.

The requested 15-minute N=3 sustained run safety-aborted at 413.9 seconds: host load 12.03 exceeded unchanged cap 12; available memory 32.16 GiB and swap change −8 MiB. Teardown ended at 418.6 seconds with zero leftovers. Its 77 partial samples contain only the baseline forced-GC checkpoint, with no final recovery/GC. Neither a plateau nor the 15-minute requirement is proven. The earlier aborted scale attempt is also retained and is not a pass.

Read-only installed-app sampling adds 25 observations over 120 seconds: desktop RSS approximately 100.80→100.81 MiB; aggregate matching ottod processes 258.34→256.77 MiB. Those 15 processes include bridges, not 15 agents. This is a different installed revision and excludes unattributed native WebKit/GPU children, so it provides limited real-app context rather than candidate acceptance.

## Bounded source confirmation

The repaired cost model remains intact at `ui/src/lib/stores/transcript.svelte.ts:142–144`: eight admitted child bodies, 32 MiB aggregate charge and 8 MiB accepted-page charge. Admission precedes fetching (`:480`), release and current-reader fencing prevent closed bodies returning (`:434–439,470`), and one-page replacement (`:512`) avoids accumulating every traversed page. At S visited children and P traversed pages, payload is bounded by admitted windows rather than S×P×page size; small cursor metadata still grows with navigation. Actual RSS is not the accounting charge.

History now includes pending route ownership (`ui/src/modules/agents/history/HistoryPage.svelte:72,175–178`) and a synchronous origin-hash comparison. These constant-size checks do not add per-row I/O or fan-out. The mounted pending-page-readiness regression is RED→GREEN and included in the passing acceptance matrix. No new hot-path defect was demonstrated.

## Fixed PLAN rubric

| Dimension | /2 | Evidence and named remaining deduction |
|---|---:|---|
| query/network work | 2.0 | Preserve R5's bounded admission/shared-consumer evidence; the current scale run adds real transport observations without a demonstrated request multiplier. No known material gap in the stated repaired-path matrix. |
| algorithmic/serialization cost | 2.0 | Actual 1,200-turn older/newer traversal confirms fixed page rendering; the mounted repeat workload supports the already traced replacement/admission costs. No new cumulative-copy or superlinear defect. |
| retained memory/lifecycle | 1.9 | Mounted one-parent body release and repeated heap recovery now directly support the repair. Remaining 0.1: P1-M1's three-parent mounted budget rejection/retry/reopen allocation journey remains unmeasured; store tests establish shared admission but do not substitute for that runtime case. Process RSS attribution is separately retained in the measurement gap. |
| scheduling/responsiveness | 1.9 | Completed emitting-session scale run plus child p95/zero observed long tasks materially strengthen evidence. Remaining 0.1: P1-M2's child expansion/page/navigation latency specifically during N=1/3/5 concurrent output is not established by combining separate workloads. |
| representative CPU/RAM/latency measurements | 1.7 | Up from 1.6: current-source N=0/1/3/5 completes, child workload has mounted latency/heap/RSS observations, and read-only app sampling exists. Remaining 0.3: P1-M3 full 15-minute sustained/recovery plus equal-duration idle control is incomplete, and native WebKit/GPU ownership remains unattributed. This is still a meaningful measurement gap; partial sustained data is not a passing full matrix. |
| **Total** | **9.5/10** | **Below 9.8 with no invented source defect or arbitrary per-dimension ceiling.** |

Acceptance still requires the named three-parent mounted allocation case, interaction latency under concurrent emitting sessions, and a completed sustained/control/recovery investigation under unchanged safety limits with native process attribution. Do not claim that stable DOM or declining JS heap proves no leak. No historical report was changed and no out-of-scope defect is asserted.
