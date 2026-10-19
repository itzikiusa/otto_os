# ClickHouse parity — iteration 02 (verification in progress)

The isolated ClickHouse 26.3 server runs in `otto-parity-ch-20261018`, HTTP 28123/native 29000. No pre-existing server was changed.

Implemented follow-up fixes:

- Native query cancellation uses a per-query unpredictable comment marker and an independent native control connection. KILL matches the exact marker prefix and current database user. A running-query test asserts the task is still running before cancelling; timeout test requires a real deadline error after at least 900 ms and checks `system.processes` for orphan work.
- Native interactive reads apply row, cell and response-byte budgets while collecting blocks, then kill partial queries. Native export writes one decoded block at a time on the blocking pool instead of retaining the full result.
- HTTP batches use unique server sessions; native batches use unique client sessions. SET and USE affect subsequent statements without mutating another tab's profile connection. Follow-up HTTP requests do not reset the batch's selected database/timezone.
- Native profile timezone is applied on connection setup. Selected databases use owned scoped connections. Explicit read-only native work uses owned connections with server `readonly=2`; writable work never reuses those sessions.
- Native Enum8/Enum16 labels, nested arrays/tuples and timestamp formatting use wire column types. HTTP explicitly requests quoted 64-bit integers and decimals: direct live testing found the server's JSONCompact default otherwise emitted UInt64 MAX as an unquoted JSON number.

Verification so far: 36 ClickHouse unit tests passed (one optional performance benchmark ignored). Two live parity tests passed for HTTP/native FORMAT, integer/enum arrays, SET/USE state, selected/default database isolation, configured timezone, native cancellation and timeout. Subsequent export/DDL/read-only/row-cap tests are added but awaiting the shared compile gate: unrelated Redis edits temporarily leave helper references undefined. Final scores will be updated after those tests execute; the 9.8 target is not claimed while verification remains incomplete.

## Completed verification and scores

All five live parity tests passed against the isolated server: formats/types/batch-state, real native cancellation/deadline cleanup, DDL/INSERT/read-only isolation, native 5,000-row export, and both-transport row caps/empty metadata. `cargo test -p otto-dbviewer --lib` passed 502 tests, with seven explicitly ignored. A subsequent final typed-result sweep added Map(Enum) labels and duplicate-key preservation plus its unit regression; this last small change awaits the next coordinated test run.

| Lens | Iteration 02 score | Evidence and remaining limits |
|---|---:|---|
| L09 statement coverage | 9.7 | Live CREATE/INSERT VALUES/EXISTS/SELECT/DROP across both transports; existing SHOW/DESCRIBE/EXPLAIN routes preserved. Native new server data types remain limited by klickhouse 0.15. |
| L10 transport/batches/settings | 9.7 | Both transports pass SET→SELECT and USE→SELECT, format plus settings, selected-database isolation, timezone. Native server profiles pinned to readonly=1 may reject the upstream client's startup SET; explicit Otto read-only mode on normal profiles is tested. |
| L11 typed results/errors | 9.5 | Wide integers, enum labels/arrays live verified; typed timestamps and Map(Enum) implemented. HTTP duplicate-key Map objects can still collapse duplicates in serde_json; newer native Dynamic/JSON representations need client support. |
| L12 resource/limits/cancel | 9.7 | Running query cancellation, real deadline cleanup and absence of orphan sleep query verified; native export block streaming and row caps verified. HTTP JSON-array export still takes the buffered fallback under the 128 MiB response cap. |

The score target is not declared achieved. These remaining limitations are concrete follow-up review targets, not grounds to claim all ClickHouse statements/types are supported. No command restrictions were added; permission failures remain server-enforced.
