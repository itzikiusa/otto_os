# Performance measurements

## Terminal ring allocation regression

For the same 1 MiB newline-free input and 64 KiB retained ring, the counting allocator measured 201,326,592 allocated bytes before the repair and 131,072 afterward (1,536 times fewer allocated bytes). This measures cumulative allocation traffic in the isolated ring path, not application RSS or elapsed speed. The repaired test passed from the closing gate's compiled executable; logs `/tmp/otto-review-ring-allocation-red.log` and `/tmp/otto-review-ring-allocation-final.log`.

## Read-only real application sample

25 samples over 120 seconds, at five-second intervals. This samples the already-running application during two real agent sessions; no sessions or application data were changed. `ps` CPU percentages are observational snapshots, not interval CPU-time deltas. Build/review activity on the host may affect results. Native WebKit child processes are not included in this first sample.

| Process | Mean observed CPU % | RSS first / last / max (MiB) |
|---|---:|---:|
| otto-desktop | 3.09 | 101.12 / 101.14 / 101.17 |
| ottod | 1.18 | 105.05 / 105.34 / 105.41 |

This short observation does not establish absence of leaks. The isolated scale result below supplements this observation; sustained-load measurements use synthetic providers, a separate HOME/data directory, and a dedicated port.

## Initial scale attempt

The first `--ui-url` run failed at startup because the harness configured the browser API base to the Vite URL. Changed that argument to the isolated daemon origin. The rerun booted and collected 0/1-agent samples, then the existing safety limit aborted at host load 13.26 (>12), with 37.92 GiB available and no swap growth. Teardown reported zero leftovers. This is an incomplete scale run, not a pass. Logs/samples remain in `/tmp/otto-review-load-baseline-fixed`.

A read-only sample of the already-running application identified an active Kubernetes historical rollup backfill in ClickHouse (~10.8 million rows read in 9.8 seconds, ~212.7 MB query memory), plus host Spotlight indexing. This explains external load during measurement; no real query or application process was stopped and no user state was changed. Full scale and sustained measurements will be retried after fixes when headroom permits.

## Confirmed runtime finding: K8s rollup migration retry loop

Read-only follow-up on 2026-10-04 confirmed the live ClickHouse CPU is a failed migration loop, not merely a long successful backfill. The daemon log has repeated migration-start messages followed by `MEMORY_LIMIT_EXCEEDED` under the embedded server's 1 GiB total limit. For example, starts at 17:59:07Z and 18:00:31Z failed at 17:59:31Z and 18:01:19Z; the next start was 18:02:19Z. No live state was changed.

`crates/otto-k8s/src/monitor/schema.rs:653` truncates rollups before aggregating each entire raw day; `:673` creates completion views only after all partitions succeed. `collector.rs:1183` retries after the collection interval. A high-cardinality day repeatedly rebuilds earlier work and never reaches the completion marker. One sampled query had read 10.85 million raw rows. This blocks schema readiness and keeps migration CPU active indefinitely at the observed production size.

Repair direction: bound backfill aggregation/read/insert memory and partition work into smaller time slices while preserving aggregate semantics and restart safety; report migration failure in monitor status and back off repeated failures. Verify exact totals and idempotent retry on a throwaway ClickHouse instance under the embedded memory limit. Do not touch the live database or raise its memory cap to conceal the defect.

ClickHouse documents external aggregation and the memory headroom needed for merging spilled aggregates in its [GROUP BY reference](https://clickhouse.com/docs/reference/statements/select/group-by#group-by-in-external-memory). This informs the proposed bound; runtime validation is still required.

### Verified repair

Backfill queries now use one thread, a 384 MiB query budget, 64 MiB external aggregation/sort thresholds, 8,192-row read blocks and no large minimum insert buffer. Initialization failures surface in monitor status and use exponential retry backoff up to 15 minutes. This bounds work without changing the installed server's 1 GiB limit. A smaller 256 MiB query budget failed during external merge; the final budget was validated rather than assumed.

The isolated `high_cardinality_backfill_fits_embedded_memory_limit` regression passed in 51.59 seconds including seeding: three million distinct series with 384-character labels across 600 pods, an injected interruption after the first rollup, restart, exact three-million totals in every rollup tier, and a completed initialization that leaves counts unchanged. Log: `/tmp/otto-review-backfill-retry-green.log`. The test's synthetic data and server were temporary. The installed application still needs a future deployment to receive this runtime repair.


## Closing scale run: 1, 3 and 5 concurrent sessions

The first closing attempt used headless Chromium and stopped at 136.9 seconds when host load reached 12.63, above the unchanged limit of 12. Available memory was 35.04 GiB, swap growth was zero, and teardown left zero processes. The owned GPU process explicitly used `--use-angle=swiftshader-webgl` and was observed near 346% CPU. This incomplete run is not a performance pass; it also does not represent native Tauri GPU cost. Evidence: `/tmp/otto-review-load-final`.

The harness now accepts optional `--headed true` (the default remains headless). The completed closing run used headed Chromium, the current debug daemon, and a Vite server with HMR disabled. Its GPU arguments did not force SwiftShader; this observation alone does not identify the physical graphics backend. Both browser modes remain proxies for the native WebKit app, and results must not be compared across modes as an application improvement.

Command from `ui/` (environment selects the worktree's `target/debug/ottod` and isolated port 7814):

```sh
node scripts/loadtest/loadtest.mjs --mode scale --steps 1,3,5 \
  --hold 80 --baseline 20 --recover 40 --sample 5 --stacks false \
  --headed true --deadline 600 --ui-url http://127.0.0.1:5314 \
  --out /tmp/otto-review-load-headed
```

The 375.4-second run collected 66 samples, with 20 seconds per view at each session count. Peak sampled host load was 6.60, available memory stayed at or above 35.04 GiB, and swap did not grow. No page errors were captured; teardown reported zero leftovers. Providers were synthetic ANSI/transcript producers, not real model calls. CPU is interval process CPU and can exceed 100% across cores. RSS below is the last sample in each phase, in MiB; CPU means exclude the first transition sample.

| Sessions / view | Daemon CPU / RSS | Renderer CPU / RSS | GPU CPU / RSS | ClickHouse CPU / RSS |
|---|---:|---:|---:|---:|
| 0 / baseline | 0.0% / 162.1 | 2.3% / 542.6 | 1.3% / 144.4 | 1.1% / 401.5 |
| 1 / tiled | 0.7% / 176.8 | 9.2% / 575.5 | 20.4% / 165.3 | 1.3% / 404.7 |
| 3 / tiled | 1.4% / 239.8 | 9.5% / 665.6 | 15.9% / 176.8 | 1.1% / 410.6 |
| 5 / tiled | 3.4% / 299.0 | 20.2% / 718.0 | 32.2% / 184.8 | 1.4% / 412.1 |
| 0 / Home after closing | 0.1% / 305.6 | 3.8% / 703.3 | 1.9% / 182.0 | 1.4% / 414.7 |

Across the four five-session views, daemon CPU means ranged from 2.2% to 4.0%, renderer from 11.4% to 20.2%, and GPU from 14.1% to 32.2%. Synthetic agents added 0.8–1.7% CPU and up to 402 MiB RSS; other daemon children added 0.6–1.1% and up to 312 MiB. Browser main/utility groups remained below 1.2%/0.7% respectively, with final RSS about 246/209 MiB. [All process groups and views](measurements/scale-summary.csv) are retained, including peak RSS.

After closing sessions, CPU and terminal WebSocket traffic returned near idle. Memory did not return to the initial empty-app baseline: after GC the daemon retained 305.6 MiB and renderers 700.0 MiB, versus 162.3/531.6 MiB after baseline GC. These different UI/cache states and the short recovery window do not prove a leak or its absence. The sustained three-session view-switch run below provides comparable GC checkpoints.


## Sustained three-session view switching

The same daemon/browser mode completed `--mode leak --n 3 --leak-min 15 --state-s 20 --baseline 20 --recover 40 --sample 5 --stacks false --headed true --deadline 1200`, with the same isolated ports and Vite source. Eight complete cycles took the active workload slightly beyond 15 minutes; baseline, recovery and teardown brought total duration to 986.8 seconds. It collected 184 samples. Peak host load was 7.21, estimated available memory never fell below 35.04 GiB, swap growth was zero, page errors were empty, and cleanup left zero processes.

Each checkpoint returned to Home and forced browser GC. RSS is MiB, summed across each process group.

| Cycle | Seconds | Daemon RSS | Renderer RSS | JS heap | Live DOM | Listeners |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 149.2 | 246.3 | 708.3 | 35.0 | 887 | 375 |
| 2 | 262.0 | 276.4 | 719.7 | 35.8 | 887 | 375 |
| 3 | 374.8 | 284.5 | 724.0 | 36.4 | 887 | 375 |
| 4 | 487.6 | 286.2 | 727.1 | 36.0 | 887 | 375 |
| 5 | 600.4 | 287.4 | 733.5 | 36.8 | 887 | 375 |
| 6 | 713.1 | 290.8 | 735.8 | 37.5 | 887 | 375 |
| 7 | 826.0 | 291.0 | 739.8 | 37.0 | 887 | 375 |
| 8 | 938.7 | 291.5 | 741.1 | 37.2 | 887 | 375 |
| Closed + GC | 982.0 | 290.3 | 736.8 | 33.8 | 730 | 187 |

Across the five active view buckets, the analyzer reported mean daemon CPU of 1.8–2.2%, renderer CPU of 11.1–16.9%, and GPU CPU of 17.5–27.5%. Synthetic agents and other daemon children each averaged below 1%; ClickHouse averaged 1.2–1.4%. These are current-workload observations, not before/after speedup estimates.

Comparable live DOM and listeners did not accumulate across remounts. Heap stayed in a 35.0–37.5 MiB band; daemon RSS growth slowed to 0.7 MiB over the last two cycles. Renderer RSS still rose by 32.8 MiB between the first and eighth checkpoints and did not return to its empty baseline after closing. This run provides bounded workload evidence, not a proof of no long-term/native leak or a before/after application-speed comparison. [Checkpoint data for every process group](measurements/leak-checkpoints.csv) preserves ClickHouse, GPU, synthetic-agent and browser overhead too. Full local evidence: `/tmp/otto-review-leak-headed/{samples.jsonl,analysis.txt,driver.log,page-errors.json}`.


## Final merged-source scale confirmation

After integrating PR77 as `65e13b4f`, repeated the identical headed 0/1/3/5-session scale command against the fully merged UI. The run completed in 375.4 seconds with 66 samples, peak host load 6.66, at least 36.0 GiB estimated available memory, zero swap growth, no page errors and zero leftover processes. Source stayed unchanged throughout measurement. [Full grouped measurements](measurements/final-scale-summary.csv) retain all process groups; raw local evidence is `/tmp/otto-review-load-merged`.

| Sessions / tiled view | Daemon mean CPU / final RSS MiB | Renderer mean CPU / final RSS MiB | GPU mean CPU / final RSS MiB |
|---|---:|---:|---:|
| 0 / baseline | 0.0% / 161.8 | 1.5% / 542.6 | 0.9% / 141.2 |
| 1 | 0.6% / 155.1 | 10.0% / 577.5 | 22.2% / 166.2 |
| 3 | 2.9% / 244.2 | 16.5% / 695.1 | 27.0% / 175.0 |
| 5 | 4.4% / 296.3 | 21.2% / 739.8 | 31.6% / 183.7 |
| 0 / closed Home recovery | 0.1% / 301.0 | 1.9% / 715.3 | 1.0% / 179.5 |

After recovery GC, heap was 25.8 MiB and renderer RSS 710.5 MiB. The same short-recovery/native-browser limits described above apply. The 15-minute sustained run preceded the final mechanical design integration; it is not represented as a second post-merge sustained run. Session output is variable-rate and these short runs are not controlled before/after speed comparisons.
