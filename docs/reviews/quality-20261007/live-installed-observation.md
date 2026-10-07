# Installed-app resource observation — corrected aggregation

Evidence: [live-installed-60s-corrected.json](evidence/live-installed-60s-corrected.json). Source capture: `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-quality-live-z28uqf08/report.json`. The corrected artifact retains all **480 original raw process rows unchanged** and adds 360 label/timestamp totals. No new sampling was performed while correcting it.

Sixty requested one-second samples span **63.67 seconds** from first observation to last; process inspection adds overhead. CPU is the platform `ps` estimate, where 100% is one core. Ownership comes from the installed daemon's listening PID/start identity, owned child processes and the desktop's WebKit resource coalition. Provider agents are excluded.

The old summary treated three WebContent PIDs as 180 consecutive observations. This divided combined mean CPU by three and compared the first large PID's RSS with the last small PID's RSS. The durable [sampler](benchmarks/live-sample.py) now sums CPU/RSS **per label and timestamp**, then summarizes the 60 totals. Raw identities and minimum/maximum simultaneous-process counts are retained so process-population changes remain visible.

| Process group | Concurrent PIDs | Mean CPU | Peak CPU | RSS first → last MiB | Peak RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Daemon | 1 | 0.63% | 2.30% | 97.14 → 97.22 | 97.22 |
| ClickHouse | 1 | 0.03% | 0.30% | 340.53 → 339.34 | 340.53 |
| Desktop shell | 1 | 3.02% | 3.60% | 121.27 → 121.38 | 121.39 |
| Owned WebKit GPU | 1 | 13.86% | 16.10% | 61.94 → 63.00 | 63.27 |
| Owned WebKit networking | 1 | 0.89% | 1.50% | 14.39 → 14.41 | 14.41 |
| Owned WebKit content | 3 | 22.62% | 30.30% | 504.28 → 505.81 | 505.81 |
| **All observed groups** | **8** | **41.05%** | **48.40%** | **1139.55 → 1141.16** | **1141.16** |

The overall peak is computed after summing groups at each timestamp, not by adding individual group peaks. This is a whole-process RSS sum, not unique physical memory: shared mappings can be counted by more than one process.

**Limits:** This is the already installed version, not the uncommitted repair build. No collector was observed; its absence is not a collector-performance pass. The sample contains native UI resource use even during otherwise quiet interaction, and supplies no attribution to a particular page or function. Roughly one minute and a 1.61 MiB change in summed RSS do not establish a leak or leak freedom. No score increase follows from this observation alone.

Reproduce the correction without process access or resampling:

```sh
python3 docs/reviews/quality-20261007/benchmarks/live-sample.py \
  --summarize /path/to/original-report.json \
  --output /path/to/new-corrected-report.json
```

The output must be a new file; the script will not overwrite prior evidence. The pending controlled sequence remains: repaired build → trace-chain regression → rendered-terminal smoke/full → 150-second off/on telemetry phases → isolated self-time queries at 100k/1M spans. These operations still belong to the coordinator's exclusive heavy slot.
