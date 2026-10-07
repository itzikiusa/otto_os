# Independent final performance assessment

**Score: 9.8/10 for the declared reviewed scope, up from 8.0/10. Verdict: approve the reviewed performance repairs.** No remaining material performance defect was demonstrated within that scope. The score is a judgment under the agreed five-part rubric, not a statistical confidence level or an application-wide capacity guarantee. Performance outside the scope below is not certified by this number.

Baseline is `196048df` plus the current uncommitted repair set in `fix/quality-20261007`. The coordinator built the repaired daemon and owned all heavy execution; this reviewer inspected source and independently checked the retained runtime artifacts, mathematical results, attribution and coverage. The final plugin All time scoping correction was closed by its separately owned correctness review; these resource runs do not independently certify that change's business semantics.

## Scope and closure

Reviewed: telemetry ingestion/cardinality, bounded queues/batches/export lifecycle, consent-aware resource sampling and collector ownership, retention/query caps, session input/backpressure and fan-out, transcript-cache bounds, hourly auto-archive access, School actor/poll lifecycle, and plugin deployment-range cache/cancellation. Runtime measurements cover 33 built-in module navigation paths, N=1/3/5 raw local-agent transport, N=1/3/5 real WebGL terminals, fresh-browser matched telemetry modes, 100k/1M span self-time queries, and read-only native installed-app sampling. Navigation coverage is not populated-data scale coverage for every module.

The original major finding—export blocking resource sampling and excluding the collector—is closed by a separate bounded sampling worker, process-incarnation registration and consent-safe publication. Actual exported resource data now includes collector RSS in eight minute buckets, covering every minute in which the external sampler observed it. The UI span-name contract repair also passed the real browser→server→collector→ClickHouse trace-chain acceptance. Neither apparent low overhead caused by rejected spans nor absence of collector measurements is counted as success.

The hourly archive scan now pages eligible IDs through a covering index rather than materializing already archived session payloads. The coordinator's archive/query-plan regressions passed. The production self-time query returned correct per-operation exclusive totals in both repeats at 100k and 1M spans, under the unchanged 256 MiB/10-second limits. The initial oversized fixture INSERT failure is retained and distinguished from SELECT performance.

School actor disposal, hidden polling, paging and plugin persistent tag-range caching/cancellation received source review and targeted lifecycle/cache/behavior checks. The broader coordinator ledger supplies their exact functional results. Their complete populated-world/repository capacity is not inferred from the terminal benchmarks.

## Measured outcomes

| Evidence | Result and implication |
| --- | --- |
| [Raw transport, 150 s per mode/concurrency](telemetry-raw-load-results.md) | N=5 mean total CPU 42.30% off/43.69% on; 94.31/93.12 requests/s; zero drops/export failures while enabled. External sampling has no gap above 1.096 s. Quiet total CPU 0.69%, RSS 1535.86→1531.92 MiB. |
| [Fresh-browser matched N=5](telemetry-matched-load-results.md) | On-before-off counterbalance: mean browser RSS 448.60/448.16 MiB; total CPU 44.24/42.20%; total RSS 1206.76/1159.89 MiB; 92.74/92.48 requests/s. Every one of five sessions returned all 392,403 submitted bytes in each mode. Enabled drops/send failures 0; later 80 dropped equals the 80 queued spans intentionally cleared on opt-out. |
| [Real WebKit rendering](rendered-terminal-load-results.md) | All nine tiles rendered 96/96 markers; p95 latency 20 ms (N=1), 23–24 ms (N=3), 26–28 ms (N=5), worst 31 ms. Real 18 ACKs and drained queues. All 360 browser samples include root/network/GPU/content PIDs. N=5 mean total CPU 20.92%, mean RSS 1264.11 MiB. Quiet mounted-widget RSS is stable. |
| [Exact self-time query](self-times-capacity.md) | 100k spans 33–41 ms; 1M 458–467 ms, 147–148 MiB reported query memory. Four successful exact-result queries, no fallback. |
| [Installed native application](live-installed-observation.md) | Correctly aggregated 60 read-only observations: total CPU 41.05%, RSS 1139.55→1141.16 MiB. This is the installed version, not the repair build; it supplies real-world context, not proof of a repaired-app idle improvement. |

The initial raw comparison's +542.73 MiB total mean difference included +396.41 MiB browser memory after sequential navigation. It is **not** recorded as telemetry overhead. Fresh browsers reduce the browser difference to 0.44 MiB with closely matched post-recovery heaps, resolving that attribution gap for N=5 transport. The cause of retained browser state after module navigation was not identified; neither “harmless warming” nor “leak” is asserted.

The matched pair's 2.04 percentage-point CPU and 46.87 MiB total RSS differences are descriptive, not universal overhead constants. The collector contributes about 69.59 MiB averaged across the on phase, while previously warmed ClickHouse is 23.74 MiB larger in the later off phase. The driver is excluded, but summed process RSS can count shared pages more than once.

## Equal-weight rubric

| Subdimension | Score / 2 | Evidence and remaining deduction |
| --- | ---: | --- |
| Queue bounds, batching, backpressure | 1.95 | Source bounds and overflow/consent/cancellation regressions, real sustained input/echo, rendered ACK and queue-drain evidence. Deduct 0.05 because the runtime workload is below deliberately forced end-to-end exporter saturation; that failure case is covered by bounded unit/integration scenarios, not this load run. |
| Data access and retained state | 1.95 | Archive projection/paging/index repair; exact 1M self-time results under production limits; finite retention and caches. Deduct 0.05 for shallow trace fixtures and lack of multi-day/high-variety retained-data capacity measurements. |
| Concurrent-session CPU/RAM | 1.95 | N=1/3/5 raw and actual rendered workloads, owned-process attribution, recovery, counterbalanced fresh-browser N=5 and real installed-app observation. Deduct 0.05 for finite duration and local CLI substitutes; complex provider TUIs/paid inference are outside measured capacity. |
| Telemetry overhead and diagnostic integrity | 2.00 | Original deterministic defects repaired and corroborated by actual collector resource/export and complete trace-chain evidence; fresh-browser comparison resolves the material memory confound; disabled behavior and zero enabled drops/failures verified. |
| Reproducibility and scale validation | 1.95 | Durable isolated harnesses, exact raw artifacts, attribution regression tests, preserved failed-fixture evidence and explicit measurement methods. Deduct 0.05 for one host and limited repetitions rather than statistical/multi-host capacity characterization. |
| **Total** | **9.80** | **All original material findings and specified bounded runtime gaps are closed; stated limits remain.** |

## Limits and follow-up boundary

The finite evidence does not prove long-duration leak freedom, exact internal sampling cadence from minute maxima, or compositor presentation timing from render callbacks. Rendered quiet CPU includes mounted widgets and the fixture's continuing animation-frame observer; it is not app-idle CPU. The nominal 90 render samples span roughly 95–96 seconds due to sampling overhead, which is disclosed rather than silently presented as exact wall time.

This review does not certify populated-scale behavior of every database/broker engine, canvas/vault graph, remote integration, every School scene size or every repository-history shape. No paid agents were started, no production data was changed, and installed-app CPU was not attributed to specific functions without a profile. Query fallback at larger/deeper volumes remains legitimate behavior, not disproved by a 1M fixture success.

No additional load is required to support the bounded conclusions above. If the high retained browser state after repeated module navigation recurs as a user-visible issue, a separate repeated-cycle allocation profile is the finite next investigation. Do not silently turn that unconfirmed observation into either a new defect or a universal memory-health pass. The score applies to the reviewed repairs and measured scenarios, not an unqualified claim that every application module is 9.8/10 at every scale.

Artifact integrity: [SHA-256 manifest](evidence/performance-artifact-digests.json) verifies all four primary retained runtime reports against their source JSON and records source/artifact and derived-analysis digests. The retained matched report adds a final newline; its JSON is identical. All local report/evidence links were checked.
