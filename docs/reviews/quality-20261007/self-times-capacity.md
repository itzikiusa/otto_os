# Telemetry self-time query: bounded capacity result

**The production query passed at 100,000 and 1,000,000 spans, with correct self-time totals in both repeats.** None of the four measured queries returned a capacity error or required the inclusive-time fallback. This closes the initial review's capacity question for this fixture size and shape; it does not certify every retention window or trace distribution.

Evidence: [successful raw report](evidence/self-times-100k-1m.json), [execution log](evidence/self-times-100k-1m.log), and [independent arithmetic check](evidence/self-times-math-verification.json). The raw report includes the exact query, exporter-generated raw-table DDL, source hash, server version, query summaries, results and process observations. Its schema-source SHA-256 matches the current `crates/otto-telemetry/src/schema.rs` at this review.

## Controls and fixture

The coordinator ran the durable [probe](benchmarks/self-times.py) against ClickHouse **26.7.1.237** in a disposable directory with loopback-only ephemeral ports. The pinned installed collector generated the actual trace-table schema and was stopped before measurement. Query/insert concurrency was capped at two threads, merge pool at two, server tracked memory at 1 GiB. The self-time query retained its production **256 MiB / 10-second** limits and a 24-hour lookback. Two query threads is not a two-thread limit on the entire server.

Half the spans are distinct parents and half their direct children, producing 50,000/500,000 trace pairs. Each pair has one 100 ms parent and one 25 ms child. The expected exclusive contribution is `(100 − 25) + 25 = 100 ms` per pair, or **50 × span count ms**. There are 20 operation names and four component values; the fixture correlates these into exactly 20 component/operation groups, not 80 independent combinations.

The independent offline check verified every expected group and value, not just the grand total:

- 100k spans: ten parent-operation groups × 375,000 ms plus ten child-operation groups × 125,000 ms = **5,000,000 ms**.
- 1M spans: ten parent-operation groups × 3,750,000 ms plus ten child-operation groups × 1,250,000 ms = **50,000,000 ms**.
- Both repeats at each size returned all 20 correct groups. Query summaries report exactly 2N rows read, consistent with the parent scan plus child aggregation scan.

## Measurements

| Spans | Repeat | Client elapsed | Reported query memory | Sampled process peak RSS | Process samples |
| --- | ---: | ---: | ---: | ---: | ---: |
| 100,000 | 1 | 40.76 ms | 26.04 MiB | 405.30 MiB | 1 |
| 100,000 | 2 | 32.89 ms | 23.90 MiB | 408.75 MiB | 1 |
| 1,000,000 | 1 | 457.80 ms | 146.88 MiB | 531.06 MiB | 5 |
| 1,000,000 | 2 | 467.23 ms | 148.18 MiB | 534.94 MiB | 5 |

The 1M query's reported memory was 154,016,908–155,378,038 bytes, below the 268,435,456-byte production cap. Query memory and whole-process RSS are different measures. The latter includes retained server allocations, caches and background work; it is not evidence that the query violated its memory cap. Sampling at approximately 100 ms cannot establish the true transient RSS maximum for a 33–41 ms query, and these short `ps` CPU samples do not provide meaningful sustained CPU distributions.

## Initial fixture failure, preserved

The first attempt passed the 100k queries, then tried to insert the remaining 900k spans in one statement. That **seed INSERT**, not the self-time SELECT, exceeded its 256 MiB limit (attempted 259.86 MiB). Consequently the first attempt did not execute the 1M capacity query. Its [raw report](evidence/self-times-initial-seed-failure.json) and [failure log](evidence/self-times-initial-seed-failure.log) remain available.

The coordinator changed only fixture seeding to 100k-span batches and reran in a fresh isolated directory. Production query text and limits were unchanged. This is a fixture-construction correction, not a production-query optimization or evidence of exporter failure; production export uses much smaller batches.

## Remaining limits

The successful fixture has shallow, nonoverlapping parent/child pairs, limited attribute payload and only 20 operation groups. It does not exercise deeper/concurrent child trees, much larger retained volumes, varied attribute maps, or many simultaneous analysis requests. Two repeats in one server are observations, not a latency percentile or universal capacity bound. No fallback was triggered, so fallback behavior itself was not exercised. No production data was queried or modified.

The final performance score still awaits the repaired browser-to-export trace chain, rendered-terminal smoke/full run, and the 150-second off/on concurrent raw-transport phases. This successful capacity result alone does not justify 9.8/10.
