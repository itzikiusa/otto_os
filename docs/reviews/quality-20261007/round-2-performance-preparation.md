# Performance repair re-review and runtime handoff

Review baseline is `196048df` plus the current uncommitted repair set. This is an independent source pass and harness preparation, not a final performance score. The reviewer ran no builds, tests, browsers or synthetic load. The initial 8.0/10 remains provisional until current runtime artifacts are available.

## Repair assessment

- **Sampling/export separation:** `otto-telemetry/src/lib.rs` now starts one dedicated sampling worker; its cadence no longer awaits collector export/analysis. Publication checks the consent revision while holding the existing launch lock, and collector samples must still match the published registration. Startup/shutdown wake both workers. This directly addresses the first review's deterministic sampling gap without an unbounded task per sample.
- **Collector ownership:** `collector.rs` owns a `CollectorRegistration` through readiness and drain. `resource.rs` captures PID/start time, rejects an incarnation mismatch, and clears registration on drop. The separate sampler therefore has an owned collector to observe. Tests inspected include blocked-export sampling progress, revoked-consent publication and collector lifecycle coverage. The coordinator reports focused tests passing; external real-collector cadence/CPU/RSS comparison remains necessary.
- **UI ingest contract:** The producer now uses the static accepted client name; validation keeps the bounded allowlist. This restores ingestion without accepting arbitrary operation cardinality. The final browser -> ingest -> exported-trace regression must pass on the repaired daemon.
- **Auto-archive:** Candidate-ID pages replace full `Session` materialization, capped at 128 IDs per manager page/256 at the repository API. The cursor advances before fresh live/attached checks, so skipped pinned candidates cannot prevent later pages. Migration 0180 adds a partial covering `(id,last_active_at)` index for unarchived agents. Existing archive-under-lock checks remain. Query-plan and mixed-history regressions need the coordinator's queued execution; archived payloads no longer enter the sweep.
- **School:** `disposeActor` releases unique cloned skeletons, uncaches animation bindings, and detaches the actor while leaving shared geometry/materials owned by the template cache. Individual removal, room cleanup and headmaster paths call it. Screen polling checks document visibility, aborts on hide, rejects late feed results, and resumes once the prior batch settles. List DOM is paged. Source changes address the reported resource lifecycle and hidden polling costs; real Three.js disposal/cycle and hidden-request observations remain the evidence gate.
- **Plugin:** Persisted deployment-range cache uses fixed-size hashed keys, a 16 MiB/5,000-entry LRU, atomic storage, and disposable-cache failure behavior. `gitscan` receives the cache path across child launches. Cold misses still exclude every earlier SHA, retaining first-deployment semantics on divergent branches. Cancellation reaches HTTP/backoff and owned workers. The remaining cold-path cost is proportional to ranges and excluded ancestors; repeated-worker cache-hit evidence is needed before claiming its payoff.

No new material performance defect was confirmed in this bounded repair pass. The source pass does not establish that all app modules scale, nor that the new code is already validated under load.

## Added harness

`ui/e2e/desktop-terminal-rendered-load-perf.spec.ts` complements the existing raw-socket telemetry load:

- N = 1/3/5 real Terminal widgets connected to owned disposable daemon sessions. Global setup substitutes only the provider CLI with a harmless `cat`; terminal sockets, PTYs, credit/ack flow, parsing and renderer are real.
- One approximately 0.85 KiB clipboard paste/second per widget, through xterm's input handler. Unique final-line markers are timed from dispatch to `__ottoTermProbe.onRender`, with a separate per-widget progress count. No arbitrary production test hook was added.
- Default 90-second load + 30-second quiet recovery for each concurrency. Widgets remain mounted during quiet recovery. Resource samples retain timestamps, CPU estimates and RSS by daemon/collector/ClickHouse/agent/browser; driver is separate.
- Requires real credit negotiation and acknowledgements, nonempty frame observations, sustained progress in every tile, every final marker rendered, p95 input-to-render below a conservative one-second gate, and empty terminal queues after recovery. Exact latency distributions are stored; this gate is not a claim of sub-100ms production performance.
- JSON artifacts survive failure. The fixture and sessions are owned by the test; the test restores the prior telemetry setting and removes its own sessions.

The process sampling implementation was extracted unchanged from `desktop-telemetry-perf.spec.ts` to `process-resources.ts`, preserving the existing raw-load artifact fields. Parent edits adding internal collector-resource assertions and 60-second recovery remain intact. The coordinator owns the existing terminal-load Svelte/HTML fixture, which this test uses.

## Finite queued validation

Run one command at a time, one Playwright worker, fresh unique slot/ports, sweep-orphans disabled, explicit repaired `OTTO_E2E_BIN`. First run UI check, then a five-second smoke to validate real paste/fixture/probe integration. Defaults are the scored measurement; shortened smoke durations are never presented as capacity evidence.

```sh
# From ui/, with isolated OTTO_E2E_* variables supplied by coordinator:
OTTO_TERMINAL_LOAD_SECONDS=5 OTTO_TERMINAL_RECOVERY_SECONDS=5 npx playwright test --project=desktop-webkit --workers=1 e2e/desktop-terminal-rendered-load-perf.spec.ts
npx playwright test --project=desktop-webkit --workers=1 e2e/desktop-terminal-rendered-load-perf.spec.ts
OTTO_TELEMETRY_LOAD_SECONDS=150 npx playwright test --project=desktop-browser --workers=1 e2e/desktop-telemetry-perf.spec.ts
```

Capture both JSON artifacts plus exact commit/dirty repair revision, build profile, browser engine and rendering backend. Temporary SwiftShader thread tuning or CPU quotas must be recorded and held constant for comparisons; software-renderer CPU does not certify native macOS GPU efficiency. Long-load execution must have exclusive heavy slot.

After these runs: inspect per-concurrency resource curves, achieved progress and latency, quiet recovery, internal collector observations and dropped/export-failed counts. Re-run short read-only real-app sampling separately. Keep provider-shim results distinct from paid real-agent workloads. The 100k/1M raw self-time query capacity question and extended School cycle/45-kid resource measurements remain unverified unless separately executed. Do not manufacture a 9.8 score by treating these gaps as passes.
