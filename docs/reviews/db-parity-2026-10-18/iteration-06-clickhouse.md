# ClickHouse parity — iteration 06: Administrative rowsets and SQL suffix forms

Confirmed major: BACKUP and RESTORE return operation ids/status, but the write acknowledgement path discarded their rows. `clickhouse.rs:2386` now recognizes their rowsets without changing write classification. Official reference: https://github.com/ClickHouse/clickhouse-docs/blob/main/docs/operations_/backup_restore/00_overview.md . Live BACKUP TABLE system.one TO Null confirms id plus BACKUP_CREATED on both transports without persisting a backup.

Rechecked existing FORMAT before/after SETTINGS, comments, SELECT aliases, a table named format, EXISTS, CHECK, SHOW/DESCRIBE/EXPLAIN classification, and DDL/INSERT execution. CHECK system.one returns the server's Not Implemented error because that engine cannot CHECK; this is not filtered locally. RESTORE row shape is covered by the mock route and documentation; a real destructive restore was not run. INSERT file/inline-data protocols are not claimed equivalent to clickhouse-client.

| Lens | Score | Remaining evidence boundary |
|---|---:|---|
| L09 statements | 9.7 | Broad SQL/admin execution covered; not every server input/output protocol. |
| L10 transport/session parity | 9.6 | Native modern types and strict readonly=1 startup profiles remain limited. |
| L11 fidelity/errors | 9.4 | Direct native Dynamic/JSON unsupported; now explicit errors, HTTP/workaround available. |
| L12 resources/limits/cancel | 9.7 | Live bounded export/cancel coverage; exhaustive races and old server framing not proven. |

These are distinct review passes over the evolving shared working tree. Final consolidated test results are recorded in iteration 10; no 9.8 target claim is made.
