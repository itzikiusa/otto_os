# Telemetry raw transport load — measured results

The repaired daemon completed the bounded Chromium workload: 33 built-in modules, then 1/3/5 owned fake-CLI sessions with collection off and on, 150 seconds per phase, followed by 60 quiet observations. The coordinator reported the test passed in 16.9 minutes. This is raw WebSocket/API concurrency evidence; actual rendered-terminal performance is measured separately.

**The observed +542.73 MiB at N=5 is not an established telemetry memory cost.** Browser mean RSS alone accounts for +396.41 MiB, and off/on phases shared a browser and ran sequentially after module navigation. Browser RSS was still about 1,001 MiB during quiet recovery. Stable recovery is evidence against continued growth during that minute, not proof that retained memory is harmless or that navigation cannot leak.

Evidence: [raw report](evidence/telemetry-raw-load-full.json), [original summary](evidence/telemetry-raw-load-summary.json), [independent breakdown and recovery analysis](evidence/telemetry-raw-load-analysis.json). The driver is excluded from totals. CPU is `ps`'s process estimate; 100% represents one core.

| Concurrent sessions | Collection | Mean total CPU | Mean total RSS MiB | Requests/s | Mean request ms |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | Off | 26.50% | 1197.19 | 18.76 | 1.495 |
| 1 | On | 31.19% | 1531.89 | 19.07 | 1.193 |
| 3 | Off | 34.63% | 979.55 | 56.81 | 1.563 |
| 3 | On | 35.59% | 1583.75 | 56.89 | 1.556 |
| 5 | Off | 42.30% | 1154.90 | 94.31 | 1.841 |
| 5 | On | 43.69% | 1697.63 | 93.12 | 2.228 |

At N=5, observed mean CPU rises 1.39 percentage points and achieved request throughput falls about 1.25%. These are descriptive paired observations, not statistically isolated overhead estimates. The workload is closed-loop with a 50 ms pause and real local PTY traffic, not paid provider inference.

N=5 mean RSS difference decomposes into browser +396.41 MiB, daemon +23.48 MiB, ClickHouse +39.22 MiB, agent children −0.14 MiB, and collector +83.77 MiB averaged across the complete phase. The collector is present in 69/144 samples: its 174.83 MiB live-only mean must not be added directly to other full-phase means. At N=1/3, it is present in 22/144 and 43/144 samples. It intentionally starts for export and stops after drain (`crates/otto-telemetry/src/lib.rs:986`). No collector was alive in the quiet samples.

## Recovery and sampling coverage

| Process | Mean quiet CPU | First quiet RSS MiB | Last quiet RSS MiB |
| --- | ---: | ---: | ---: |
| Daemon | 0.05% | 144.70 | 144.30 |
| ClickHouse | 0.10% | 387.80 | 386.81 |
| Browser | 0.55% | 1003.36 | 1000.81 |
| Total | 0.69% | 1535.86 | 1531.92 |

The 60 observations span 62.405 seconds; total RSS slope is −2.47 MiB/minute, with a 1537.64 MiB peak. Recovery happened once after all six phases, not independently after each mode/concurrency. Off N=5 browser RSS also grew during load (about 49.46 MiB/minute); on N=5 growth was about 10.27 MiB/minute. A short positive slope does not establish a leak, and the different phase history prevents treating the lower later slope as a fix.

All load phases retain 143–144 total observations over approximately 149–150 seconds. Maximum external sample gap is 1.096 seconds; no sustained external sampling stall is visible. The production telemetry overview contains nine consecutive minute buckets each for daemon/ClickHouse/host and eight collector buckets. All eight collector buckets contain RSS; seven contain CPU. Every minute with an external collector observation has an internal collector bucket. The one missing intermediate collector minute also has no external collector observations. The additional initial internal collector bucket predates external load sampling. These observations close the original never-observed-collector defect; minute maxima cannot validate the exact internal sample cadence.

Final status records 25,308 exported spans, zero drops, zero collector send failures, zero collector exporter queue, no last error, and 1,805 newly queued local spans plus seven buffered resource points. This is healthy completed-export evidence, not a claim that every post-flush sample has drained: telemetry continues recording its own observed traffic between exports.

## Attribution and remaining inference limit

Chromium used the already-imported ancestry sampler before the macOS WebKit repair. Chromium's browser children remain in that owned process tree; these raw rows lack the newer explicit PID arrays. The corrected WebKit rendered run must verify its root and three detached XPC process roles separately.

A finite follow-up is warranted before claiming a precise telemetry memory overhead or closing performance at 9.8+: run N=5 alone in fresh browser instances, with equal setup and no preceding 33-module walk, counterbalance on before off, 90 seconds each plus 60 seconds of recovery, and retain per-process RSS curves. This isolates retained browser state from the current phase ordering. It is not necessary to declare the current run a functional/capacity pass; it is necessary to resolve the material memory attribution gap. Do not classify the observed retained browser memory as a leak without a reproducible retained-growth sequence or allocation evidence. No follow-up load was launched by this reviewer.
