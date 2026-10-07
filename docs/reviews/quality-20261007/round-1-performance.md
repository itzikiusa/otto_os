# Round 1 — performance and application telemetry

**Verdict: Approve with fixes.** Baseline `922ae483`; reviewed October 7, 2026. Counts: 0 blocker, 1 major diagnostic finding, 1 minor cost finding. One separately owned correctness defect invalidates UI telemetry evidence. No new demonstrated daemon capacity failure was found in this pass.

**Score: 8.0/10, provisional for the scope below.** This is not an app-wide 9.8 certification. No build, test, browser or synthetic load was run by this reviewer; the coordinator owns the heavy execution slot.

## Scope inventory and sized inputs

| Surface | Inspected evidence and bounds |
| --- | --- |
| Telemetry ingestion and export | `crates/otto-telemetry/src/{lib,types,otlp,collector}.rs`; daemon queue 4,096 spans, 512-span POSTs, 4,096 resource points, 256 buffered log bodies; UI queue 500, batch 100. Collector three exporter queues each 16,384 items, one consumer each, 192 MiB limiter, 160 MiB Go soft limit, GOMAXPROCS 2. |
| Retention and query cost | `schema.rs`; traces/logs 1–30 days, metrics 1–90 days; time-partitioned aggregate rollups; explicit result/time/memory caps; trace-ID time lookup. Self-time analysis still scans and joins raw spans within its window. |
| Cardinality and UI collection | `types.rs`, server telemetry middleware/routes, `ui/src/lib/telemetry*.ts`; server names derive from route templates, bounded attributes, UI component allowlist, disabled fast path, visible-page flushing. |
| Resource measurement | `resource.rs`, `lib.rs`; limited owned process discovery, five-second default sampling, 30-second desktop discovery, PID-start-time checks for cached desktop processes. |
| Session transport/concurrency | `otto-sessions/src/ws.rs` input budget/coalescing and credit gate; `otto-server/src/ws_fanout.rs` serialize-once Arc fan-out and cooperative yielding; transcript cache two workers, 8 pending folds, 128 MiB retained budget. Inspected bounds rather than assuming `unbounded_channel` is a leak: input semaphore bounds it. |
| Session/history maintenance | Session-manager stale archive path, state sessions queries, run-history retention, periodic scheduler. Retention batches and opt-in history preservation are deliberate. |
| Existing validation | `docs/testing/telemetry*.md`, load JSON, `ui/e2e/desktop-telemetry-perf.spec.ts`, terminal flood spec, daemon-budget script. Prior results are leads only. Changed-file hotspot scan against baseline correctly returned no changed files; scope was broadened manually. |

## Findings

### [major] Export blocks sampling and the collector never enters resource measurements — `crates/otto-telemetry/src/lib.rs:812`

**What:** The sole worker awaits `sample`, then awaits the entire `flush` at line 824. `flush` starts the collector at line 893 and stops it at line 915 before the next sample. `sample` at lines 850–856 supplies only daemon and ClickHouse; desktop discovery adds the owned UI processes, never the collector. Searching all telemetry sources found no second sampling caller. Thus the app cannot observe its own export process, and daemon/ClickHouse export-time resource peaks also fall inside an intentional sampling gap.

**Cost:** This is measurement coverage, not a claim of measured excess CPU. N = flushes, triggered every five minutes while active, by queue pressure at 2,048/3,072 records, or eligible page reads. Each flush suppresses all resource samples for startup + send + a minimum 11-second drain (up to 25 seconds, plus collector shutdown). At a configured five-second cadence, every flush misses at least two scheduled samples, including the period when the collector's configured 160 MiB heap/192 MiB memory limit matters. Higher event rate increases flush frequency and the blind fraction. The process is gone by the next measurement, so its omission is deterministic.

**Evidence:** Confirmed control-flow trace: `tick:812,824`, `flush:893,914–915`, `sample:850–856`, `drain:1014–1023`; sampler discovery in `resource.rs` only identifies Otto desktop/WebKit. Runtime magnitude has not been measured.

**Fix and payoff:** Keep resource sampling on its own bounded lifecycle task, publish the owned collector PID plus process incarnation during flush, and clear it on success, failure and cancellation. Sample at the configured cadence while export awaits I/O. Preserve the disabled fast path, single sampler serialization, private ownership rules, and bounded buffers. Do not broaden collection to arbitrary user processes. This changes zero collector observations per flush into approximately lifetime / sample_interval observations and restores daemon/ClickHouse observations during export.

**Ownership/tests:** `otto-telemetry/src/lib.rs`, `collector.rs`, `resource.rs`, and existing telemetry unit tests. Test with a controllably blocked exporter: advance time through multiple sample intervals; verify sampling proceeds, collector appears while owned, disappears after exit/cancel, and disabling stops the sampler. Then corroborate with external `ps` samples during a real isolated flush. A test asserting only the presence of a PID field is insufficient.

### [minor] Hourly auto-archive materializes already archived history — `crates/otto-sessions/src/manager.rs:5370`

**What:** With auto-archive enabled, `auto_archive_stale` calls `SessionsRepo::list_all`, whose query at `crates/otto-state/src/sessions.rs:477` is `SELECT * FROM sessions ORDER BY created_at DESC`. Only afterwards does Rust discard archived, wrong-kind and recent sessions. Every archived session remains durable and is reread on every subsequent hourly sweep (`otto-server/src/boot/tasks.rs:245–249`).

**Cost:** N = all historical session rows, including JSON metadata; C = eligible stale rows. Every hourly pass reads, allocates and decodes O(N × row payload), even when C = 0. At 50,000 historical rows averaging 4 KiB payload, the scan transfers roughly 195 MiB of row content before Rust overhead. This is a sizing example, not a measurement of the user's DB. The path is cold and opt-in, so this is minor, not a blocker.

**Evidence:** Confirmed SQL/control-flow trace. Existing default is off; this is not an explanation of current idle CPU. No production DB was inspected.

**Fix and payoff:** Add a repository candidate query projecting IDs and selecting unarchived agents older than cutoff, with a small keyset page. Preserve the existing fresh `archive_if_stale` decision and live/attached checks to avoid races. Add an appropriate index only after checking the candidate query plan against existing indexes. Peak materialization becomes O(page size), and already archived payloads are never deserialized.

**Ownership/tests:** `otto-state/src/sessions.rs`, `otto-sessions/src/manager.rs`; migration only if the measured plan requires one. Seed many archived rows plus a few candidates and verify candidate result size and EXPLAIN plan; retain existing race tests. No live-history mutation is required.

## Separately owned defect and validation gaps

- **Correctness handoff:** `ui/src/lib/telemetry.ts:49–52` emits per-endpoint `http.client.<method>.<route>` names, while `crates/otto-server/src/routes/telemetry.rs:157–172` accepts only exact `http.client`. One such span rejects the complete UI batch. The coordinator sent this to the correctness reviewer. Until repaired, do not treat apparent absence of UI spans or low telemetry overhead as valid evidence. Validate a normal UI request through the actual ingest route and then query the exported operation; do not test client and server allowlists independently.
- Existing 1/3/5-agent load creates PTYs and direct WebSockets but the synthetic sockets do not render output through Terminal/xterm. It tests daemon transport and concurrent reads, not five rendered terminals or real provider reasoning workloads. It samples browser navigation separately and supplies no loaded-terminal render budget. Report those distinctions.
- The default 20-second phases are too short for memory trend claims and may straddle first-export setup. The documented 150-second phases give useful resource curves but should be repeated/reordered to control warmed caches and background merges.
- Raw self-time analysis uses a bounded 256 MiB/10-second join over all traces in the configured window (`schema.rs:self_times`). At 100 spans/second a day contains 8.64 million spans. Falling back on inclusive ranking is deliberate; this is an open capacity measurement, not an asserted failure. Seed representative volumes in an isolated database and report whether the fallback activates.
- Native WebKit GPU/WebContent memory is not attributable from a plain same-name `ps` filter; use the existing coalition ownership method or isolate the test browser. Do not sum unrelated browser/provider processes into Otto totals.

## Fresh read-only live sample

Artifact: [round-1-performance-live.json](round-1-performance-live.json). Six samples at two-second intervals, approximately ten seconds, executable basename and CPU/RSS only; no process arguments, secrets, API changes or production database reads. This is a short observation with concurrent development activity, not an idle benchmark or leak test. The inferred installed daemon is PID 32397 (parent of sampled ClickHouse); other `ottod` names include PTY holders/MCP processes and were not folded into its figure.

| Observed process | Mean CPU | Maximum CPU | RSS MiB min–max |
| --- | ---: | ---: | ---: |
| Daemon 32397 | 0.10% | 0.20% | 132.63–132.64 |
| Desktop shell 32180 | 2.87% | 3.10% | 132.86–132.86 |
| ClickHouse 99188 | 0.00% | 0.00% | 341.11–342.13 |

CPU is `ps`'s platform estimate (100% = one core), not exclusive operation CPU. No collector was present during this sample. Provider PIDs are recorded only as observations, not asserted to be controlled workloads. No conclusion about memory leakage follows from this duration.

## Bounded measurement matrix for coordinator execution

1. After a fresh build and ingest repair, run the existing load spec with one Playwright worker, isolated daemon/data, unique ports, `OTTO_E2E_SWEEP_ORPHANS=0`, collection off/on, N = 1/3/5 harmless fixture agents, 150 seconds per phase. Preserve raw samples, throughput, accepted/exported/dropped counts and CPU/RSS distributions. Record exact commit/build profile. Do not share another worktree's target.
2. During export, independently sample daemon, collector, ClickHouse and owned browser at one-second cadence; verify internal resource points cover the same period and collector lifetime. Include a 60-second quiet recovery interval to distinguish temporary buffers from persistent growth.
3. Add a bounded rendered-terminal phase with N = 1/3/5 Terminal instances using the real renderer and credit/ack path. Feed at most 4 KiB/second per fixture for 90 seconds, then stop input and observe recovery. Record parsed-byte progress, p95 frame/input latency, total CPU, peak RSS and final-minus-start RSS. Existing raw sockets remain a separate transport phase.
4. Re-run only the affected transcript/terminal transport scale gates if evidence points there. Do not launch all performance suites together. Real provider CPU cannot be inferred from `cat` shims; complement with read-only real-app sampling without creating paid provider sessions.
5. Evaluate self-time query at bounded 100k/1M trace rows only after coordinator assigns the heavy slot; capture query duration/peak memory and whether ranking falls back. Do not seed the user's ClickHouse.

## Score rubric

| Subdimension | Score / 2 | Deduction |
| --- | ---: | --- |
| Queue bounds, batching, backpressure | 1.9 | Static bounds are strong; fresh overload/drop/recovery evidence pending. |
| Data access and retained state | 1.8 | Hourly history materialization; self-time join capacity unmeasured. |
| Concurrent session CPU/RAM | 1.6 | Transport/cache controls inspected and real sample captured; fresh N-agent rendered workload and recovery curves pending. |
| Telemetry overhead and diagnostic integrity | 1.1 | Deterministic export sampling gap/collector omission plus separately owned UI batch rejection. |
| Reproducibility and scale validation | 1.6 | Runnable isolated harness exists; no current-baseline load run yet and no long-duration evidence. |
| **Total** | **8.0** | **9.8 cannot be defended before material findings and runtime gaps are resolved.** |

The review did not audit every external provider, broker/database engine, design/canvas/vault algorithm or all UI pages. Those remain explicit scope limits; nothing here certifies app-wide capacity.
