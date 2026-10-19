# ClickHouse independent peer review — iteration 03

Reviewer owns Mongo, performed read-only ClickHouse review across L09–L12. No ClickHouse files or tests were edited by this reviewer and no Cargo command was run for this review. The ClickHouse owner handled repairs and tests.

## Confirmed findings

1. **Major L10 — single-statement session mutation escaped batch isolation.** `native_query_scoped` reused the cached profile connection whenever selected DB equaled profile DB. A single `USE other_db` therefore changed the cached connection used by another tab; a later query selecting the original profile DB reused that mutated connection. Single SET similarly leaked settings. Confirmed by tracing `run_tracked` single path → `exec_ch_write` → `native_query_controlled` → cached `native_query_scoped`. Owner added operation-owned connections for `sql_leaves_session_state` outside explicit batch sessions (`clickhouse.rs:541`) and a live regression. Dedicated batch-session preservation remains intact.
2. **Major L09/L10 — FORMAT table identifier mistaken for output clause.** `output_format_sql` accepted a preceding FROM word, so valid `SELECT x FROM format csv` replaced alias `csv` with JSONCompact instead of appending an output clause. Owner verified ClickHouse accepts that grammar (server resolves `format AS csv`), excluded FROM/JOIN contexts (`clickhouse.rs:2224`), and added regression tests.

## Reviewed without new blocker

- Unique HTTP/native batch session identity and eviction: `run_ch_batch` around line 911.
- Native marker-based cancellation on independent control connection, current-user match, five-second control deadline, and partial-result kill: `native_query_controlled` around 624 and `native_kill` around 1835.
- Native typed enum/array/tuple/map/date handling: `typed_value_to_json` around 1860. Native duplicate map keys preserve pairs rather than collapsing values.
- Native export streaming block-by-block with a row cap: `export_native` around 2957.
- HTTP FORMAT replacement, comment-aware rowset classification and wide integer precision settings.

## Scores and evidence boundary

| Lens | Score / 10 | Rationale |
|---|---:|---|
| L09 statement coverage | 9.7 | FORMAT identifier fix addresses confirmed gap; targeted new regression pending at review time. |
| L10 transport/session parity | 9.7 | Single-session pollution fixed in source; owner must execute its live regression. |
| L11 type/error fidelity | 9.5 | Native map duplication addressed; owner documents HTTP duplicate-key Map collapse and newer native types unsupported by current client. |
| L12 resource/control behavior | 9.7 | Traced bounded native collection/control/streaming; owner's five live tests passed before latest peer fixes. HTTP JSON export remains buffered and byte bounded. |

No new unverified suspicion is asserted as a defect. No independent live test success is claimed by this reviewer. Owner's live/type/session evidence is in iteration-02-clickhouse.md; final scores should follow final coordinated tests and documented capability limits.
