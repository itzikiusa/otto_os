# Telemetry repair evidence

Baseline: `196048df`, branch `fix/quality-20261007`. This worker owns telemetry source, its regressions and Otto Usage trace detail. The coordinator executes all builds, suites and browser checks; this worker has run formatting only. No live data, external publication, commits or staging were performed.

## Changes

- **C4 / T4:** browser API spans use the ingest contract's fixed `http.client` operation. The server's strict allowlist and whole-batch validation remain unchanged. Endpoint attribution remains available in parent-linked server spans. The tradeoff is intentional: browser operation rollups aggregate client latency by component instead of endpoint. Browser route headers, including plausible-looking private identifiers, cannot enter the operation name.
- A shared fixture under `crates/otto-telemetry/fixtures/ui-ingest.json` is consumed by the real HTTP wrapper/runtime unit test and the Rust ingress validator test. The browser test drives successful HTTP responses with route headers, mixed render/navigation/client measurements and parent relationships; the backend checks accepted names and rejects arbitrary names, raw URL/routes, excessive lengths and invalid components. The existing full-chain browser/collector test remains a separate integration check.
- **P1 / T9:** resource sampling has an independent, single worker. Export startup, HTTP waits, drain and analysis cannot stop its cadence. Resource maxima and spike logs retain the existing bounds. Disabled collection parks both workers; shutdown awaits both. A config revision and consent lock prevent a completed old sample from refilling buffers after disable/re-enable.
- The owned collector registers its PID plus process start time immediately after spawn, including readiness and drain. Registration clears on normal stop, observed exit and drop/cancellation. The sampler checks process incarnation; publication rechecks active registration so teardown cannot publish a stale collector point. No process-name-wide collector discovery is used.
- **D5:** trace detail renders the expired/not-yet-stored explanation through `LoadState.emptyView`. It retains the requested trace ID for Retry, clears it on close, and preserves request-sequence/disposal guards so late responses cannot reopen a closed dialog. Browser regressions cover empty, failed then successful retry, and closing while retry is pending.

## Executed baseline evidence

| Finding | Check | Result | Evidence |
| --- | --- | --- | --- |
| C4 / T4 | Existing browser → server → collector integration | Failed baseline at parent-chain assertion; ingestion returned 400 | `/tmp/otto-quality-20261007-telemetry-baseline.log` (coordinator) |
| C4 / T4 | `cd ui && node --experimental-strip-types --test unit/telemetry.test.ts` | 13 tests: 12 passed, 1 failed, as expected; endpoint-derived producer names differ from the shared fixed-name contract | `/tmp/otto-quality-20261007-telemetry-contract-red.log` (coordinator) |
| P1 / T9 | `cargo test -p otto-telemetry --lib resource_sampling_continues_while_export_worker_is_blocked` | 1 failed as expected: sampling stopped behind blocked exporter | `/tmp/otto-quality-20261007-sampler-red.log` (coordinator) |
| D5 | Browser `trace detail empty state` baseline | 1 failed, 2 not run due serial suite: dialog contained only its title | `/tmp/otto-quality-20261007-trace-red.log` (coordinator) |
| D5 | Browser `trace detail retry state` baseline | Failed: Retry button absent | `/tmp/otto-quality-20261007-trace-retry-red.log` (coordinator) |

First post-fix UI run: 21 passed, 1 failed across telemetry and Redis suites. The telemetry failure compared identical name arrays from different JavaScript VM realms; `Array.from` now normalizes the actual array into the test realm while preserving exact element/count equality. Log: `/tmp/otto-quality-20261007-contract-redis-green.log`. Re-run pending; this was a harness assertion failure, not an endpoint-name regression.

## Requested verification, pending results

Use the coordinator's fixed environment: Cargo jobs 2, dev debug 0, incremental 0; one heavy command at a time.

1. UI telemetry unit regression, now repaired.
2. `cargo test -p otto-telemetry --lib`: blocked exporter cadence; disabled sampling; revoked-consent in-flight publication; owned collector stop/exit/drop and PID-reuse protection; existing privacy/buffer/export tests.
3. `cargo test -p otto-telemetry relative_data_directory_launches_with_recoverable_absolute_config -- --ignored --test-threads=1`: actual collector-start fixture records ownership before readiness and clears it on canceled startup. No downloads or ClickHouse needed.
4. `cargo test -p otto-server --lib routes::telemetry::tests`: shared ingest contract and existing privacy limits.
5. Playwright `desktop-telemetry.spec.ts -g 'trace detail'` repaired execution (empty and Retry baseline failures confirmed). Run the complete spec against the rebuilt daemon for the actual browser→ingest→collector chain.
6. Affected-consumer gate, UI check/unit suite and clippy.
7. Coordinator-owned isolated external resource sampling during export, including collector lifetime, then a subsequent flush to persist points gathered after the previous export pass. Preserve throughput and 1/3/5-agent CPU/RSS results separately from source/test evidence.

## Limits and follow-up

Resource points gathered after a flush's export pass remain in the bounded buffer until the next flush. Internal resource rollups intentionally store per-minute maxima, so row count is not sampling cadence evidence. First observation of each process incarnation has unavailable CPU (RSS is available); later samples calculate CPU deltas. There is no claim of long-duration memory stability, native WebKit coverage, or app-wide performance certification from these focused changes.

The coordinator has updated the shared API contract and middleware comment: state fixed browser names, server-side endpoint attribution, whole-batch privacy validation, and independent sampling of the owned collector. Public DTOs did not change. Screenshots and full UI checklist execution remain pending; the trace repair uses existing Modal/LoadState and introduces no styling system.

## Follow-up: real pipeline harness timing

The coordinator reports the repaired UI contract run passed (telemetry + Redis: **22/22**, no skips; `/tmp/otto-quality-20261007-contract-redis-green-2.log`) and the broader Rust/UI gates passed. A subsequent rebuilt-daemon full browser suite still failed its stored-trace predicate: **2 passed, 1 failed, 3 not run** due serial ordering (`/tmp/otto-quality-20261007-telemetry-final.log`). This runtime failure was investigated separately rather than attributed to the already-repaired ingestion contract.

Offline trace evidence, sanitized to status/count/timing fields:

- Initial export completed at **09:39:44 UTC**, `collector_ready=true`, `exported=0`, `last_flush_at=1791365984`.
- Browser batches at **09:39:45 UTC** returned HTTP 200 and accepted **38, 4 and 4** spans. Producer batches include the navigation/render/client parent chain.
- All **96** subsequent trace responses were empty, within the test's **30-second** polling window.
- `refresh_on_read` intentionally requests export only when the previous successful flush is at least **120 seconds** old (`READ_REFRESH_AFTER`). Periodic export is **300 seconds**, and this workload did not reach queue pressure. The test waited for startup export to finish before generating its browser spans, then gave their next export an impossible 30-second deadline.

Only harness code changed in this follow-up. Production throttles remain intact. The full-chain test now has a **160-second** bounded storage wait with 1/3/5-second poll intervals and a **300-second** overall deadline. It independently asserts that actual producer batches receive HTTP 200 and `accepted == submitted`, then retains the exact stored navigation→render/client→server chain requirement. A sanitized ingestion-count attachment makes transport success distinguishable from storage delay.

The load harness had the same premature operation check before its on-load phase. That assertion now runs after the measured workload and recovery, alongside persisted collector coverage, so a 150-second × 3 on-load run does useful measurement instead of waiting idle. Its bounded storage wait is 160 seconds (also covers short default phases), and total timeout explicitly accounts for six load phases, configured recovery, collector startup, deferred export, and navigation/setup/cleanup. CPU/RSS budgets and throughput assertions were not relaxed.

Validation of these harness changes is pending the coordinator's next full-chain/load run. `git diff --check` passed for both edited browser specs. No daemon, collector, build or heavy test was started by this worker during the investigation.
