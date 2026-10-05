# Current performance evidence

The synthetic concurrent run completed; the full sustained-memory requirement remains incomplete. Scores are reviewer judgments, not reliability percentages. These measurements do not certify every module or the native app.

## Provenance and workload

Scale and sustained runtime: `64a850e6`, a fresh debug ottod build and an empty runtime diff. Binary and package-lock SHA256 values are captured in the raw artifact root. Vite HMR was disabled; the browser was headed Chromium with default WebGL. The run used isolated HOME/data, a provider emulator and ports 7814/5314. Only one heavy workload ran locally. Safety limits stayed unchanged: host load 12, at least 2 GiB available memory, and at most 500 MiB swap growth.

The repository [load harness](../../../ui/scripts/loadtest/loadtest.mjs) streamed synthetic terminal/transcript activity while views cycled through tiled sessions, focus, Home and Git (plus DB for sustained). It did not generate real provider calls or representative queries for every visited module.

Raw artifacts are located through `/tmp/otto-review05-perf-current`. Durable numerical samples are under [measurements](measurements/). Subsequent History navigation, Audit disclosure, Product startup and main version/tour changes are outside this exact measured revision.

## Concurrent agents

The first attempt stopped at 167 seconds when host load reached 12.7; cleanup left zero test processes. It is not counted as a pass. The same-limit retry completed in **375.4 seconds**, with **66 samples, exit 0 and zero leftovers**.

Each agent count was measured in four views. CPU values are steady-sample means after dropping each view's first transition sample. RSS values are checkpoints. CPU is per process group; 100% represents one core.

| Agents | Daemon CPU by view | Renderer CPU by view | Post-GC daemon RSS | Post-GC renderer RSS | Post-GC JS heap |
|---|---|---|---|---|---|
| 0 baseline | 0% | 1.9% | 162.7 MiB | 530.1 MiB | 17.2 MiB |
| 1 | 0.5–1.0% | 7.8–13.4% | 203.9 MiB | 652.4 MiB | 27.2 MiB |
| 3 | 1.5–2.4% | 10.5–16.2% | 229.5 MiB | 687.8 MiB | 29.9 MiB |
| 5 | 2.3–2.6% | 12.7–14.8% | 272.4 MiB | 712.4 MiB | 32.0 MiB |
| 0 recovery | 0.0–0.1% | 2.6–4.2% | 272.8 MiB | 699.3 MiB | 26.0 MiB |

At five agents, GPU processes added 15.1–21.0% CPU and ClickHouse added 1.2–1.3%. Browser helpers and synthetic agents also consumed resources. The daemon and renderer columns are not total-app usage.

Recovery brought CPU near idle while memory stayed above baseline. Baseline and recovery views differ; the difference alone does not diagnose a leak.

## Sustained run limitation

The requested run was 15 minutes at three agents with 20-second view cycles. It stopped at **413.9 seconds** on host load 12.03, exceeding the unchanged limit of 12. Available memory was 32.16 GiB and swap decreased by 8 MiB. Cleanup finished at 418.6 seconds with zero leftovers.

The 77 partial samples contain only the baseline forced-GC checkpoint. Final recovery and GC were not reached. This run cannot establish a memory plateau or meet the 15-minute acceptance requirement.

## Mounted allocation and reachability

The actual SubagentCard/transcript store passed 100 expand/collapse cycles, bidirectional paging through 1,200 turns at 60 mounted turns per page, and another 100 inspections. This ran within the 18-case acceptance batch in default headless Chromium. Its absolute RSS is not comparable to the earlier headed observation.

Forced-GC heap: 20.81 MiB warm → 21.20 after 100 inspections → 24.44 after paging → 23.57 after the additional 100 inspections. Total browser RSS: 573.23 → 630.72 → 680.02 → 694.27 MiB. Retained child bodies remained zero; mounted DOM count fell from 641 to 632 and stayed there. Inspection latency was p50 100.17 ms and p95 118.70 ms, with zero observed long tasks.

The final 100 cycles reduced JS heap but increased RSS by 14.25 MiB. Native allocator/browser retention is unassigned; no absence-of-leak claim is made.

The actual shared DiffView fixture rendered 50,000 equal lines in 16.3 ms with zero diff rows; 50,000 sparse lines in 43.4 ms with 18 rows; and a 10,000-line rewrite in 30.7 ms with 500 mounted rows. Timings include two animation frames. All 20 rewrite pages reached the final line, and both downloads matched the complete source bytes. One 54 ms long task was recorded. Forced-GC heap was 2.94 MiB at baseline, 12.15 MiB with the rewrite loaded and 4.32 MiB after release. These are client observations, not daemon/native resource measurements.

## Read-only installed app

The 120-second observation collected 25 samples. The desktop process averaged 3.27% CPU (maximum 4.1%), with RSS 100.80 → 100.81 MiB. The 15 matching ottod processes were the main daemon and bridges, **not 15 agents**; aggregate CPU averaged 1.64% (maximum 2.8%), with RSS 258.34 → 256.77 MiB.

The installed revision differs from the candidate. Native WebKit/GPU helpers were not attributed. Real user sessions were not changed. This sample does not establish controlled workload throughput.
