# Performance measurements

## Read-only real application sample

25 samples over 120 seconds, at five-second intervals. This samples the already-running application during two real agent sessions; no sessions or application data were changed. `ps` CPU percentages are observational snapshots, not interval CPU-time deltas. Build/review activity on the host may affect results. Native WebKit child processes are not included in this first sample.

| Process | Mean observed CPU % | RSS first / last / max (MiB) |
|---|---:|---:|
| otto-desktop | 3.09 | 101.12 / 101.14 / 101.17 |
| ottod | 1.18 | 105.05 / 105.34 / 105.41 |

This short observation does not establish absence of leaks. Isolated scale and sustained-load measurements are pending and will use the repository load-test driver with synthetic providers, separate HOME/data directory, and a dedicated port.

## Initial scale attempt

The first `--ui-url` run failed at startup because the harness configured the browser API base to the Vite URL. Changed that argument to the isolated daemon origin. The rerun booted and collected 0/1-agent samples, then the existing safety limit aborted at host load 13.26 (>12), with 37.92 GiB available and no swap growth. Teardown reported zero leftovers. This is an incomplete scale run, not a pass. Logs/samples remain in `/tmp/otto-review-load-baseline-fixed`.

A read-only sample of the already-running application identified an active Kubernetes historical rollup backfill in ClickHouse (~10.8 million rows read in 9.8 seconds, ~212.7 MB query memory), plus host Spotlight indexing. This explains external load during measurement; no real query or application process was stopped and no user state was changed. Full scale and sustained measurements will be retried after fixes when headroom permits.

## Confirmed runtime finding: K8s rollup migration retry loop

Read-only follow-up on 2026-10-04 confirmed the live ClickHouse CPU is a failed migration loop, not merely a long successful backfill. The daemon log has repeated migration-start messages followed by `MEMORY_LIMIT_EXCEEDED` under the embedded server's 1 GiB total limit. For example, starts at 17:59:07Z and 18:00:31Z failed at 17:59:31Z and 18:01:19Z; the next start was 18:02:19Z. No live state was changed.

`crates/otto-k8s/src/monitor/schema.rs:653` truncates rollups before aggregating each entire raw day; `:673` creates completion views only after all partitions succeed. `collector.rs:1183` retries after the collection interval. A high-cardinality day repeatedly rebuilds earlier work and never reaches the completion marker. One sampled query had read 10.85 million raw rows. This blocks schema readiness and keeps migration CPU active indefinitely at the observed production size.

Repair direction: bound backfill aggregation/read/insert memory and partition work into smaller time slices while preserving aggregate semantics and restart safety; report migration failure in monitor status and back off repeated failures. Verify exact totals and idempotent retry on a throwaway ClickHouse instance under the embedded memory limit. Do not touch the live database or raise its memory cap to conceal the defect.

ClickHouse documents external aggregation and the memory headroom needed for merging spilled aggregates in its [GROUP BY reference](https://clickhouse.com/docs/reference/statements/select/group-by#group-by-in-external-memory). This informs the proposed bound; runtime validation is still required.
