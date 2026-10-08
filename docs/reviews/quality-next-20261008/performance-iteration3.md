# Performance iteration 3 — real rendered terminal runtime

**Verdict: approve the measured terminal envelope; 0 confirmed performance findings (0 blockers, 0 major, 0 minor).** This pass uses the existing named rendered-terminal workload against an isolated baseline daemon. It is runtime evidence for the terminal path, not certification of the new browser repairs or a whole-product score.

## Protocol

The unchanged `ui/e2e/desktop-terminal-rendered-load-perf.spec.ts` drives the production Terminal component through clipboard input, real WebSocket transport, a daemon-owned PTY, and xterm rendering. Only the provider CLI is substituted with the fixture-owned cat shim. Session concurrency is 1, 3 and 5, each with 90 seconds of bounded input and 30 seconds quiet recovery. Each terminal receives one approximately 1 KiB paste per second; one marker must render before the next is sent. Credit acknowledgements, every marker, fatal UI errors and final drained queues are asserted.

`process-resources.ts` samples once per second and identifies daemon children and the test-owned WebKit resource coalition. RSS is sampled process RSS, not retained heap; summing it may count shared pages multiple times. CPU is the platform ps estimate (100% = one core), not exclusive operation CPU. Browser frame intervals and per-marker input-to-render latencies are also retained. The fixture is a grid of actual Terminal components rather than the entire app shell.

The active binary is the existing debug `target/debug/ottod`, built 2026-10-08 03:21 local time, predating this review campaign's backend repairs. Its SHA-256 and fixture hashes are retained in [source-checkpoint.json](evidence/performance-iteration3/source-checkpoint.json). Host: macOS 27.0.1, Mac15,9, 48 GiB RAM, 16 logical CPUs. The actual Playwright worker executable was `/Users/itziklavon/.hermes/node/bin/node` **22.22.3**, confirmed from its running PID. Campaign/CI uses `/opt/homebrew/bin/node` **26.10.0**; these results therefore carry that explicit environment mismatch. Browser engine: Playwright WebKit, with the production WebGL terminal renderer selected in every phase. Cargo and other campaign browser runs are paused during measurement; ordinary user applications remain running. This is one local sequential run, not a controlled dedicated-host benchmark.

## Results

**One named test passed, zero failures/skips/retries.** Executed 2026-10-08 06:51:18–06:58:05 UTC; Playwright reports 6.4 minutes test time / 6.8 minutes total including setup. The three measured phases total 384.64 seconds. Each phase makes 90 load and 30 recovery samples; synchronous process sampling adds overhead, so actual phases are approximately 128 seconds rather than exactly 120. The one-second browser paste timer continues during that overhead and produces 95–96 inputs per terminal.

| Live terminals | Markers rendered / sent | Input-to-render p50 / p95 / max | Frame interval p95 / max | Actual load + recovery elapsed |
|---|---:|---:|---:|---:|
| 1 | 96 / 96 | 17 / 27 / 43 ms | 19 / 26 ms | 128.66 s |
| 3 | 285 / 285 | 21 / 35 / 41 ms | 19 / 40 ms | 128.06 s |
| 5 | 475 / 475 | 24 / 35 / 42 ms | 19 / 23 ms | 127.92 s |

Latencies pool all markers in each phase; the worst individual terminal p95 was **37 ms**. The assertion budget remains the existing 1,000 ms per-terminal p95; no threshold was relaxed. Total: **856/856 markers**, **737,016 input bytes**, **9 credit grants**, **18 credit ACKs**, **0 missed timer ticks**, **0 observed fatal-UI errors**. All nine terminal instances ended with pending=0 and queued=0. Sampled maximum pending bytes were 476 / 548 / 548; sampled queued bytes were zero. Frame-based queue sampling is not a proof that a short-lived queue peak never occurred. Marker assertions prove actual round-trip progress, not merely accepted input.

Resource observations below use 90 load samples per concurrency, with `ps` CPU percentages where 100% is one core. Totals exclude the test driver and include daemon, owned browser coalition, PTY/holder children, ClickHouse and the collector whenever it is active.

| Terminals | Total load RSS mean ± population SD / max | Total load CPU mean / max | Browser load RSS mean | Daemon load RSS mean | Total quiet RSS range | Total quiet CPU mean |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 793.60 ± 6.85 / 804.89 MiB | 13.21% / 72.5% | 454.34 MiB | 150.58 MiB | 805.11–806.02 MiB | 9.68% |
| 3 | 895.31 ± 16.12 / 920.09 MiB | 19.12% / 109.7% | 500.20 MiB | 161.37 MiB | 920.20–920.52 MiB | 9.72% |
| 5 | 1012.11 ± 53.45 / 1161.38 MiB | 21.93% / 117.1% | 539.28 MiB | 174.23 MiB | 1027.03–1027.31 MiB | 12.26% |

The five-terminal peak includes a transient collector process (10 of 90 load samples, mean RSS 164.70 MiB during those samples); comparing the total maximum directly to the earlier phases would conflate terminal concurrency and scheduled collection. Across the sampled quiet windows, total RSS was stable within 0.92 / 0.32 / 0.29 MiB, respectively. Widgets and sessions deliberately remain mounted during recovery, so retained scrollback/process state is expected; this does not measure RSS returning to an unmounted baseline or establish leak freedom. The fixture also keeps a requestAnimationFrame probe alive through recovery, so its 9–12% quiet total CPU cannot be treated as uninstrumented production idle CPU.

**Cost interpretation:** this bounded, low-bandwidth workload (861-byte paste per second per terminal; up to roughly 4.2 KiB/s application input) scales from one to five terminals without a measured latency or queue cliff. Five times the terminal concurrency raised pooled p95 from 27 to 35 ms and browser mean RSS from 454 to 539 MiB in this run. It does not cover a high-throughput PTY flood, many-hour residency, or dozens of terminals. No new production fix is justified by these observations alone.

**Cleanup:** all sampled runtime PIDs were absent after teardown; the isolated daemon data directory was removed. The fixture's separately created empty workspace directory was explicitly removed after checking its recorded ownership and emptiness. Only owned fixture resources were affected. [cleanup.json](evidence/performance-iteration3/cleanup.json) records verification. The log's expected “no matching session” provider-ID warnings arise because the provider is a cat shim; they are not failed assertions or real-provider validation.

**Evidence:** [raw samples](evidence/performance-iteration3/rendered-terminal-load.json), [summary with variability and per-process distributions](evidence/performance-iteration3/summary.json), [reproducible summarizer](evidence/performance-iteration3/summarize.py), [execution log](evidence/performance-iteration3/run.log), [Playwright result](evidence/performance-iteration3/playwright-results.json). The result attachment was recovered from this slot's HTML-report data after another slot cleared shared `ui/test-results`; the slot-isolated HTML artifact and JSON result remained intact.

Command, from `ui/`:

```bash
OTTO_E2E_SLOT=qualityperf3 OTTO_E2E_PORT=7897 OTTO_E2E_PW_PORT=5297 \
OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod \
OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_TELEMETRY=1 \
OTTO_TERMINAL_LOAD_SECONDS=90 OTTO_TERMINAL_RECOVERY_SECONDS=30 \
npx playwright test --project=desktop-webkit --workers=1 \
  e2e/desktop-terminal-rendered-load-perf.spec.ts
```

The existing global setup uses a fresh data directory, fixture CLI shims, file-backed fixture secrets and loopback listeners, and the telemetry export stays local. It downloads the pinned/checksummed collector into fixture storage and waits for readiness before the measured test. No user browser profile, real agent credentials, port 7700, production mutation or security-default relaxation was used. Browser/profile and daemon cleanup belong to the existing fixture; the benchmark fixture and production sources were not modified.

## Coverage limits

Real Chromium is installed, but no real-Chromium workload against the newly repaired backend is executed here because the available daemon predates that code. This measurement cannot close the actual browser navigation-burst, guarded proxy and CDP allocator gaps. It also does not measure native Tauri WKWebView, production agent providers, transcript folding, large database grids, long-duration leak freedom, or simultaneous cross-module contention. Those are separate scopes; no whole-product 9.8 judgment is inferred from a passing timing gate.
