# Matched fresh-browser N=5 result

**The large browser-memory difference did not reproduce in the matched setup.** Collection-on browser RSS averaged 448.60 MiB versus 448.16 MiB off, a 0.44 MiB difference. Both modes used a fresh Chromium process, identical five-session transport/request workloads, and no prior module walk. This resolves attribution of the earlier +396.41 MiB browser difference for this bounded workload; it does not prove that repeated module navigation cannot retain memory.

Evidence: [complete raw artifact](evidence/telemetry-matched-load-full.json), [independent analysis](evidence/telemetry-matched-load-analysis.json), [method](matched-load-method.md). The coordinator's full run passed in 6.2 minutes. Offline checks verified disjoint browser PID sets between modes, complete equal echo counts for all five sessions, and status transitions.

| N=5 measurement | Collection on, first | Collection off, second |
| --- | ---: | ---: |
| Actual load duration | 90.992 s | 90.959 s |
| Mean total CPU | 44.24% | 42.20% |
| Mean total RSS | 1206.76 MiB | 1159.89 MiB |
| Mean browser CPU | 27.97% | 27.72% |
| Mean browser RSS | 448.60 MiB | 448.16 MiB |
| Requests/s | 92.744 | 92.481 |
| Mean request latency | 2.257 ms | 2.274 ms |
| Input/output bytes per session | 392,403 / 392,403 | 392,403 / 392,403 |
| Used JS heap after recovery | 19.830 MB | 19.563 MB |

Total mean CPU differs by 2.04 percentage points (100%=one core), and mean RSS by 46.87 MiB. The collector is present in 34/87on-mode load samples, with 178.08 MiB mean RSS while alive: its whole-phase contribution is about 69.59 MiB. ClickHouse's off-mode mean is 23.74 MiB higher after the earlier on-mode export warmed it. Therefore 46.87 MiB is an observed total difference in this pair, not a universal steady telemetry footprint. Browser/daemon/agent differences are small by comparison. Throughput and request latency are effectively similar in this single pair; no statistical significance is asserted.

All five sessions in both modes returned exactly all 392,403 submitted bytes. The on-mode status has 8,442exported spans, zero dropped spans, zero exporter send failures, zero exporter queue, no last error and 80newly queued local spans. The later off-mode status has 0 queued and 80 dropped. That counter increase equals the buffered spans deliberately discarded by consent revocation; it is not an export-loss observation.

## Recovery

There are 60quiet observations per mode. On-mode total RSS moves 1237.92→1063.30 MiB as the collector exits; off-mode moves 1069.30→1068.97 MiB. Mean quiet total CPU is 0.79% on and 0.71% off. Browser quiet RSS is 449.97→451.14 MiB on and 451.77→450.52 MiB off. The retained browser curves remain closely matched; neither mode reproduces the earlier roughly 1GiB browser state.

Natural-GC used JS heap grows from 17.944→19.830 MB on and 17.648→19.563 MB off (decimal MB, unlike RSS MiB). Similar growth in both modes supplies no evidence of a telemetry-specific heap-retention regression. This observation is short and no forced GC was used; it is not a leak-freedom proof. There are 87load observations per mode with a maximum total-process sample gap of 1.096 seconds.

The original off-before-on run and this on-before-off run agree on small observed CPU differences at N=5, while only the shared-browser/navigation run has the large memory discrepancy. The matched result removes that discrepancy from a claim about telemetry's N=5 browser-memory cost. Attribution to a specific module or allocation was not established and is not invented.

Same-backend cache warming, one machine, one pair, process RSS accounting, and synthetic local CLI behavior remain explicit limits. No further load is required to close the original bounded comparison; repeated module-cycle allocation profiling would be a separate investigation if that retained state recurs as a user-visible problem.
