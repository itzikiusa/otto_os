# ClickHouse CPU bursts after enabling application telemetry

## Diagnosis

The installed daemon repeatedly attempts the Kubernetes monitor's historical
rollup migration. A backfill query reaches its 384 MiB query-memory limit and
fails after about a minute. The collector retries with increasing backoff; each
attempt truncates the derived rollup targets and starts the migration again.
It does not truncate the raw samples. The migration never reaches the step that
creates the completion-marker materialized views.

This explains the repeat bursts observed while navigating the application. It
is not evidence that ordinary telemetry insertion consumes two CPU cores.

## Evidence (installed build f1f503f1, 2026-10-05)

The diagnosis below was read-only. Subsequent user-authorized cleanup and the
implemented repair are recorded in [the follow-up report](telemetry-followup-20261005.md). Times below are UTC (local time is UTC+3).

| Migration start | Failure | Error / next retry | Telemetry CPU peaks |
|---|---|---|---|
| 12:47:27 | 12:48:27 | 384.67 MiB requested, 384 MiB limit; retry 120s | 239% / 222% in the corresponding minute buckets |
| 12:50:27 | 12:51:30 | 384.49 MiB requested; retry 240s | 210% / 245% |
| 12:55:30 | 12:56:32 | Same memory limit; retry 480s | Later observation, outside the initial snapshot |
| 13:04:32 | 13:05:33 | Same memory limit; retry 900s | Later observation |
| 13:20:33 | 13:21:36 | Same memory limit; retry 900s | 324.2% in the live process watch |

The error is ClickHouse `Code: 241`, `MEMORY_LIMIT_EXCEEDED`, while executing
`SourceFromNativeStream`. Evidence comes from the daemon's dated local log and
the stored `otto.process.cpu` gauges; chart buckets contain maxima, not averages.

Read-only `system.parts` metadata showed **60,603,897 raw k8s_samples rows**,
9,093,992 rows in the partially built five-minute tier, and 792,625 hourly rows.
The telemetry trace table at the same snapshot contained only 2,481 rows.
These are physical active-part counts, not unique logical observation counts.

An eight-minute gauge sample showed ClickHouse mean CPU 41.64%, maximum 245.24%,
with 24 of 99 samples above 80%; 100% represents one core. CPU later fell to
2.29% during backoff. The telemetry Collector averaged about 0.11% in the earlier
seven-minute sample. ClickHouse RSS peaked at 847.7 MiB; the daemon at 298.3 MiB.

A subsequent two-minute read-only watch observed no active foreground query or
merge in its one-second snapshots. A brief native sample during a smaller
103.8% burst showed MergeTree old-part cleanup. This is additional background
activity; that sample does not itself measure the earlier backfill queries.

A follow-up live watch captured the next retry end to end. At 13:21:02 UTC,
ClickHouse consumed **324.2% CPU** while an `INSERT` aggregated raw
`k8s_samples` into `k8s_samples_5m` and two background merges processed that
same rollup table. The insert had read 14,963,279 rows at that snapshot.
The last observed backfill before the failure targeted `k8s_samples_1m`;
one-second sampling cannot prove the exact failing query, but the daemon log
confirms the same migration memory-limit failure at 13:21:36. CPU fell to
2.2% in the next second and generally below 1% afterward. This directly ties
the burst to historical backfill plus its merges, explaining why total process
CPU exceeds one core despite `max_threads=1` on the backfill query.

## Code path and scale

- `crates/otto-k8s/src/monitor/schema.rs::ensure` checks for all materialized
  views, truncates the rollup targets when they are absent, then visits raw
  `(cluster, day)` partitions before creating those views.
- `backfill_sql` aggregates an entire raw day per tier. It already limits the
  query to one thread / 384 MiB and enables external aggregation/sort spilling
  at 64 MiB. The observed failure establishes that these limits and spilling
  alone are insufficient for this installation's data shape.
- Thus a failed partition repeats all completed work on the next attempt.
  Cost is repeated scans/aggregation of the historical partitions, plus writes
  and cleanup of discarded partial rollups; the success marker is never reached.

## Repair implemented in the follow-up branch

The follow-up streams singleton aggregate values through bounded insert blocks
into staging tables, then atomically exchanges each completed tier and installs
its materialized view as a checkpoint. Retries preserve completed tiers. A
protocol marker distinguishes these checkpoints from unsafe partial legacy
migrations. The query limit remains 384 MiB and the embedded server limit 1 GiB.

Isolated tests cover high-cardinality data, interrupted inserts, lost acknowledgements,
legacy partial views, restart counts and raw-versus-rollup equivalence. See the
[follow-up measurements and limits](telemetry-followup-20261005.md).
