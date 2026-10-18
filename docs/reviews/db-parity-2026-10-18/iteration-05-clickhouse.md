# ClickHouse parity — iteration 05: Export caps and decompression bounds

Confirmed major: HTTP export previously injected LIMIT conservatively; an existing LIMIT or SETTINGS made the requested export cap ineffective. `clickhouse.rs:3137` now decodes JSONCompactEachRowWithNamesAndTypes incrementally for capped exports and stops/kills at the actual client count. Zero rows still preserves metadata and creates valid output.

Confirmed major: JSON-array HTTP exports previously buffered the entire result under the interactive 128 MiB cap. They now stream JSONEachRow with array output. The new 140 MiB regression exposed a second issue: one highly compressed network chunk expanded beyond the old 64 MiB decoder limit. `clickhouse.rs:3334` now sends 128 KiB decompression windows directly to export sinks; the interactive allocation bound remains in place.

Verification: HTTP/native capped export 0 and7 rows from SQL with LIMIT 500 and SETTINGS; escaped multiline string preservation; live JSON export larger than 128 MiB validated with a streaming JSON reader; unit 70 MiB compressed expansion verifies bounded windows.

| Lens | Score | Remaining evidence boundary |
|---|---:|---|
| L09 statements | 9.7 | Broad SQL/admin execution covered; not every server input/output protocol. |
| L10 transport/session parity | 9.6 | Native modern types and strict readonly=1 startup profiles remain limited. |
| L11 fidelity/errors | 9.4 | Direct native Dynamic/JSON unsupported; now explicit errors, HTTP/workaround available. |
| L12 resources/limits/cancel | 9.7 | Live bounded export/cancel coverage; exhaustive races and old server framing not proven. |

These are distinct review passes over the evolving shared working tree. Final consolidated test results are recorded in iteration 10; no 9.8 target claim is made.
