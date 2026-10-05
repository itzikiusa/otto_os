# Telemetry-guided performance follow-up — 2026-10-05

## Findings and changes

1. **Repeated Kubernetes backfill failure:** the installed build retried a
   day-wide aggregation that exhausted its 384 MiB query limit, rebuilding
   completed tiers on every retry. A live sample reached 324.2% ClickHouse CPU
   while a backfill insert and two merges were active. The repair streams bounded
   blocks into staging tables and atomically replaces/checkpoints each tier.
   Raw history is not deleted by recovery. Legacy partial checkpoints, interrupted
   writes, lost acknowledgements and orphan staging tables are covered.
2. **Misleading monitoring switch:** changing it previously changed only a draft
   until Save. It now persists immediately, shows pending state, and retains the
   confirmed state with an inline error on failure. Other draft fields remain
   drafts. Save and probe-test operations serialize with the switch.
3. **Git fetch outside Git:** one hour contained 275 server fetch spans totaling
   607,823 ms (p50 2,112 ms, p95 2,643 ms, maximum 14,618 ms). Automatic fetching
   now runs only while Git is active, stops queued work on departure and resumes
   when due on return. Manual fetching remains available. Client and server spans
   overlap; their durations must not be added together as independent work.
4. **Foreign-key authorization N+1:** Database Explorer reloaded permission state
   for each foreign key, even when they referenced one schema. The regression
   reproduced 18 versus 1,008 counted state reads for 5 versus 500 keys. The fix
   reduces both cases to 10 counted reads, evaluates distinct normalized scopes
   in one request phase and keeps the final
   fresh authorization check. Permission results are not cached across requests.

## User-authorized local cleanup

The persisted cluster setting was still enabled despite the earlier UI state.
Audit history establishes enable on September 29 and our disable on October 5;
it does not establish what happened during the user's earlier attempt.

Monitoring was disabled through the API, preserving its other settings. The
supervisor stopped its loop at 13:26:42 UTC. Only the old Kubernetes monitoring
history was cleared, after checking that it belonged to the sole cluster:
60,603,897 raw samples, 9,093,992 five-minute rows, 792,625 hourly rows, 9,679
pod-presence rows and 474 events (about 300 MiB of active parts). Tables/views,
cluster configuration, provider usage history and application telemetry remain.
Monitoring stays disabled after deployment; the fix supports future re-enabling.

## Other telemetry signals and limits

In the captured one-hour window, database schema calls had p50 2,395 ms and
maximum 3,884 ms (three calls); object detail had one 2,227 ms call; close reached
5,003 ms (three calls, matching the bounded cleanup timeout). These spans do not
identify whether the database, tunnel or local processing dominated. The FK
regression independently proves redundant permission reads; it does not prove
that they caused these particular observed latencies.

After the Kubernetes cleanup, the sampled minute-max ClickHouse CPU values were
at most 39.7%, with the latest about 2.99% and RSS 364.75 MiB. The daemon's latest
sample was about 0.95% / 97.7 MiB, the collector 0.07% / 105.9 MiB. Web content
was 633 MiB after a 759 MiB peak; this alone does not demonstrate a leak.
Collector status reported no queued or dropped records and 7,724 exported.
These are observations from one visit window, not before/after benchmarks.

Capability requests were frequent (737) but fast (p95 1.84 ms); frequency alone
is not a reason to prioritize them over slow operations. Early UI render markers
were also small (Agents p95 151 ms, Git 67 ms, Usage 85 ms); they are not a
measurement of all page data being ready.

## Verification

- Git browser regressions: inactive boot, leaving during deferred bootstrap,
  queued fetch cancellation and returning when due (2 passed).
- Monitoring browser regressions: immediate persisted switch, pending acknowledgement,
  failed save, unrelated drafts, probe-test save followed by switch, existing
  settings/presets/run-now behavior (5 passed).
- ClickHouse transport rejects truncated HTTP 200 bodies and waits for execution
  before treating an insert/DDL response as success.
- Recovery fault matrix covers partial writes, failure before/after exchange,
  lost MV acknowledgement, failed cleanup and partial legacy MV sets.
- Existing five ClickHouse query/retention tests pass; rollups retain equivalence
  to raw and wide-query references.
- Foreign-key budget and mixed-scope authorization regressions exercise the real
  state/access layer with a stub database driver.

Scale resource measurements follow below. Final integration gates are recorded
on the pull request; focused results here are local measurements.

### Isolated recovery load

The ignored scale test seeds 3 million distinct label maps (384-character label
payload), interrupts recovery after the first tier, retries, verifies all counts
and re-runs initialization for idempotence. It passed in 45.18 seconds including
seeding, reads and a five-second idle observation. Limits: 1 GiB embedded server,
384 MiB migration query. Four concurrent readers during recovery each completed
321 queries; maximum observed request latency was 196 ms.

External `ps` sampling at roughly 100 ms intervals measured the isolated
ClickHouse process (100% CPU = one core):

| Phase | Samples | Mean CPU | Maximum CPU | Maximum RSS MiB |
|---|---:|---:|---:|---:|
| Seed fixture | 44 | 307.5% | 467.0% | 649.3 |
| Recovery + four readers | 187 | 294.9% | 432.8% | 802.5 |
| One reader, 3 seconds | 25 | 103.4% | 246.6% | 583.3 |
| Four readers, 3 seconds | 26 | 73.2% | 87.0% | 486.2 |
| Eight readers, 3 seconds | 25 | 115.7% | 140.7% | 493.0 |
| Idle, 5 seconds | 46 | 7.2% | 119.7% | 483.1 |

The final approximately three idle seconds averaged 0.4% CPU (maximum 10.8%).
The first reader and idle phases include residual merge work, so the short
serial phases are not comparable capacity benchmarks. These are synthetic DB
readers, not provider-agent sessions. Live-process sampling above supplies
complementary real-app evidence; neither is a long-duration memory-leak test.

Recovery still performs up to five linear raw scans and background merges.
The query-memory cap does not cap total process CPU or resident memory, and
one-time recovery can exceed 200% CPU. The improvement is bounded aggregation
memory and restart progress, removing the observed endless retry workload.
