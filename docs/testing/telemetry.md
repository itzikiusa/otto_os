# Telemetry verification and load tests

All load runs use temporary Otto/ClickHouse directories and harmless provider
CLI stand-ins. Never point them at the installed user daemon or user database.
Run one heavy command at a time. A worktree must have its own Cargo target.

## Reproduce

```sh
CARGO_BUILD_JOBS=2 cargo test -p otto-telemetry --lib
CARGO_BUILD_JOBS=2 cargo test -p otto-telemetry real_collector_delivers_all_signals_and_recovers -- --ignored --nocapture --test-threads=1
CARGO_BUILD_JOBS=2 cargo build -p ottod
cd ui
npm run check
npm run test:unit
OTTO_E2E_SLOT=telemetry OTTO_E2E_PORT=7841 OTTO_E2E_PW_PORT=5341 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN="$PWD/../target/debug/ottod" npx playwright test --project=desktop-browser --workers=1 e2e/desktop-telemetry.spec.ts
OTTO_TELEMETRY_LOAD_SECONDS=150 OTTO_E2E_SLOT=telemetry-load OTTO_E2E_PORT=7842 OTTO_E2E_PW_PORT=5342 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN="$PWD/../target/debug/ottod" npx playwright test --project=desktop-browser --workers=1 e2e/desktop-telemetry-perf.spec.ts
cd ..
OTTO_E2E_BIN="$PWD/target/debug/ottod" node scripts/perf/telemetry-components.mjs /tmp/otto-telemetry-components
node scripts/perf/telemetry-report.mjs /path/to/component-load.json /tmp/telemetry-summary.json
```

The second browser suite walks every canonical sidebar module and the secondary
Database, Brokers, Canvas, Settings, Help and Snip routes (warm-up then
measured navigation) with collection off and on. It then runs 1, 3 and 5
concurrent fixture agent PTYs plus API reads, sampling daemon, collector, ClickHouse, fixture-agent and owned browser CPU/RSS every
second. Requests are restricted to the current fixture IDs, and each phase
deletes its fixture records before the next begins. The report records completed
requests, response bytes and achieved throughput; its closed-loop request rate
is not assumed constant. The measured loop uses native Node HTTP, with browser
trace capture off: Playwright API calls retain test steps whose lookup cost grows
throughout a long test. The load driver is sampled separately and excluded from
the app total. Earlier Playwright-request runs are delivery/soak evidence only,
not an accepted measurement of telemetry overhead. A 150-second phase yields 15 minutes of load across off/on and the three
concurrency levels. `component-load.json` contains raw measurements. `ps` CPU is
a platform estimate, not exclusive function CPU; correlate with native profiles
before attributing cost. Browser module timings measure imports/navigation/paint,
not completion of every remote data dependency.

`ui/e2e/telemetry-load-manifest.ts` lists populated component workloads for
transcripts, terminal output, DB results/editor, Git graph/diff, API history,
Vault, Canvas, Design Hall, agents/workflows, infrastructure and transport.
Those complement module navigation; neither is presented as exhaustive coverage
of every possible external provider/database workload.
The component runner selects Chromium or WebKit per workload, enables telemetry
on its isolated daemon, and generates a 2,000-turn synthetic conversation.
Full application pages bootstrap collection. Standalone component fixtures
that bypass the app retain their direct paint/heap/DOM performance probes; they
are not represented as end-to-end telemetry delivery tests. No private transcript
data is required.
Third-party plugin content requires its own installed-plugin fixture.

## Evidence ledger

- Final core suite: **21 passed**, including all normally ignored collector and
  process-lifecycle tests, in 67.45 seconds. Relative data directories, parent
  crashes, canceled startup, ownership locks and decoy processes are covered.
- Real Collector 0.162.0 / ClickHouse 26.7 pipeline: delivered OTLP traces/logs/gauges, merged rollups, trace lookup,
  recommendation dismissal, collector and ClickHouse replacement/recovery,
  TTL changes, lease release and an actual native profile.
- Full workspace gate: lint passed, **4,725 Rust tests passed** (90 intentionally
  ignored), documentation tests passed, UI check **0 errors / 0 warnings**,
  and **1,349 UI unit tests passed**. The focused telemetry suite has 12 cases,
  including deferred runtime loading and consent revocation races.
- Final production UI build and bundle budget passed. Collection runtime loads
  only after opt-in; initial entry grows from 95,115 to 95,880 gzip bytes
  against the built main baseline (+765 bytes, 0.80%).
- Browser acceptance: **3 passed**, covering root authorization, invalid input,
  opt-out, unsaved settings, real parent-linked browser/server traces in
  ClickHouse, CPU/RAM charts and responsive layouts.
- Populated components: **43 cases verified** across 14 workload files
  (Chromium and WebKit). The first run passed 41; the Git Fetch selector and
  opt-in tray traffic budget were corrected, and both focused reruns passed.
  No component cases were skipped. The 2,000-session scenario uses mocked
  routes to exercise real UI filtering/paging; it is not a daemon DB scale test.
- Selected component measurements with collection enabled: 12k-commit/20-lane
  graph first paint **276 ms**, refresh reads one 2k-row page; WebKit 100k×30
  results grid **7 ms p95 scroll work / 18 ms p95 painted step**, sort **28 ms**,
  search-to-filtered **137 ms**. Saturated transport alternate-lane requests
  remained **2–6 ms** in Chromium and **2–3 ms** in WebKit.
- Read-only installed-app sample: 60 samples per process over about 2 minutes,
  during isolated load. Existing main-build daemon CPU averaged 0.47% (max
  1.5%), RSS 86.1–88.3 MB; native desktop CPU averaged 3.92% (max 4.3%), RSS
  118.3–119.6 MB. This excludes WebKit helpers and does not measure feature
  overhead; collection was not enabled in the installed app.
- Final native-HTTP load: **15 minutes**, 33 modules, 1/3/5 concurrent fixture
  agents, collection off/on. Collector acknowledged **27,459 records** and
  queries returned **251 operation groups**. The cumulative discarded counter
  was 23 after the suite's consent cycles; it includes deliberate opt-out queue
  discards and was not sampled separately per phase. Five records remained queued
  at the snapshot. Collector acknowledgement is not a durable-storage receipt.

## Measured collection cost

[Machine-readable measurements](telemetry-load-20261005.json) include every
module navigation, process CPU/RSS distribution and achieved request rate.
These are one sequential run on macOS using a debug daemon, Vite and Chromium;
they are not a packaged-app benchmark or a fixed-rate causal experiment.

| Concurrent agents | Requests/s off → on | Mean request ms off → on | Daemon CPU % off → on | Total CPU % off → on |
|---|---:|---:|---:|---:|
| 1 | 19.10 → 19.12 | 1.00 → 1.07 | 2.43 → 2.77 | 29.75 → 31.22 |
| 3 | 57.34 → 57.35 | 1.28 → 1.28 | 7.74 → 8.02 | 35.48 → 35.95 |
| 5 | 95.20 → 94.21 | 1.58 → 1.66 | 14.37 → 14.48 | 42.84 → 47.42 |

CPU percentages use 100% per core. Total includes daemon, owned browser,
ClickHouse, collector and fixture agents; the load driver is separate. The
largest observed mean request delta was **0.077 ms**, with about **1.04%** lower
achieved throughput at five agents. These are observed deltas, not statistical
bounds. Collector CPU averaged **0.02–0.07%** and ClickHouse **0.94–4.09%** while
collecting. The driver remained at 1.3/2.1/2.6% in both modes.

Collection has a measurable memory cost: collector mean RSS was **171–179 MB**;
ClickHouse increased from **377–379 MB** to **395–415 MB**; daemon mean RSS was
**145–147 MB** during enabled phases versus **130–138 MB** disabled. Total RSS
was **1.36–1.57 GB** disabled and **1.89–1.99 GB** enabled. The shared browser
also grew across the sequential phases (758–880 MB to 1,074–1,086 MB), so the
whole difference cannot be assigned exclusively to telemetry. Its final enabled
RSS slope was negative; the collector's final slope was +0.12 MB/min. Neither
short-phase slope establishes long-term leak freedom. A longer packaged WebKit
soak remains useful for deployment-specific memory characterization.

## Screenshots

Synthetic test data only. The installed application was not modified.

- [Light dashboard](telemetry-images/telemetry-active-light.png)
- [Dark dashboard](telemetry-images/telemetry-active-dark.png)
- [Phone settings](telemetry-images/telemetry-phone.png)
