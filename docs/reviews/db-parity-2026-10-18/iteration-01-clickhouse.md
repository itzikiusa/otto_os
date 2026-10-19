# ClickHouse parity — iteration 01

Verdict: fixes implemented; parity target not yet met. Scores below are engineering assessments, not measured coverage percentages.

| Lens | Before | After (provisional) | Evidence / remaining gap |
|---|---:|---:|---|
| L09 Statement coverage | 7.2 | 8.6 | `drivers/clickhouse.rs:2059` now recognizes comment-prefixed reads, EXISTS, CHECK, KILL as rowsets. Before, EXISTS TABLE t succeeded but discarded its result and reported OK. SELECT/SHOW/DESCRIBE/EXPLAIN and DDL/DML paths traced. Raw INSERT FORMAT payload splitting and native INSERT protocol need live verification. |
| L10 Transport / batch / FORMAT / SETTINGS | 6.0 | 7.6 | `drivers/clickhouse.rs:1989` replaces an explicit output FORMAT instead of appending a second one, including export. Tokens preserve strings/comments and SETTINGS. `:528` creates an operation-owned native connection for a selected database differing from the profile, avoiding cross-tab USE mutation. Batch `SET`/`USE` state is not retained across independent HTTP requests (`:860`). Native configured timezone remains unapplied. |
| L11 Typed results / errors | 7.5 | 8.6 | `drivers/clickhouse.rs:1762` now stringifies native Int64/UInt64 matching HTTP's default precision-safe JSON behavior. Previously UInt64 MAX reached browser JSON as a number and rounded. Nested conversion inherits fix. Midstream exception tests already present. Native enum values are still backing integers, while HTTP returns labels; native timestamp shape differs. |
| L12 Limits / pagination / cancel / timeout | 7.0 | 7.0 | HTTP row/byte caps, early kill, bounded parsing and exception handling traced. Native has row cap and bounded draining but byte budget occurs after native materialization. `:1738` is only a client timeout: underlying native query cancellation is not implemented, and no configured timeout is unbounded. No 9.8 claim is justified. |

## Regression coverage

Added unit cases for comments/admin rowsets, FORMAT plus SETTINGS/comments/non-ASCII literals, and native integer extrema. Added a fake HTTP driver-level test proving explicit FORMAT executes once as JSONCompact and admin rows reach the grid. `rustfmt --edition 2021 crates/otto-dbviewer/src/drivers/clickhouse.rs` passed. Cargo execution is coordinated by the parent agent and was not run by this worker. Live HTTP probe `curl --max-time 3 http://127.0.0.1:18123/ping` failed to connect; no live-engine validation claimed.

## Source checks

Official documentation confirms [EXISTS](https://clickhouse.com/docs/reference/statements/exists) and [CHECK TABLE](https://clickhouse.com/docs/reference/statements/check-table) return result rows. [JSON format](https://clickhouse.com/docs/reference/formats/JSON/JSON) describes quoting 64-bit integers to preserve JavaScript precision. Existing implementation inspected end-to-end for transport selection, HTTP request/stream processing, native block conversion, run/batch/export, and cancellation. Changes deliberately add no command restrictions.
