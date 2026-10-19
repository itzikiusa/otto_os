# Mongo parity — iteration 03: independent peer findings and final verification

Independent ClickHouse owner reviewed Mongo L01–L04 without editing it, identified four concrete gaps, and re-read the repairs. Mongo owner implemented and tested them.

## Peer findings repaired

1. **Cancellation isolation:** caller-supplied query IDs were used directly as database comment tags. Two owners/connections could choose the same ID and share a server cancellation selector. `run_tracked` now generates an opaque server ULID for every execution, including requests without a client ID (`mongodb.rs:555,2896`). The request ID still serves the UI/service registry; it cannot select another execution's database operation.
2. **Formatted expressions:** a multiline/comment-prefixed `db.getSiblingDB(...).collection.find(...)` was routed as a raw script without printing its result. The complete property/call-chain recognizer now accepts comments and line wrapping while leaving actual scripts verbatim (`mongodb.rs:1866`).
3. **Explain control:** native explain bypassed timeout/comment propagation. The explain command now receives maxTimeMS and the server-owned cancellation comment; arbitrary scripts with an explain flag are rejected before execution (`mongodb.rs:3721`).
4. **Extreme BSON dates:** dates outside RFC3339 range fell back to a decimal string which the parser could not read back. These now use canonical Extended JSON epoch-millisecond sentinels; both signed i64 extremes round-trip (`mongodb.rs:3238`).

## Executed final gates

- `cargo test -p otto-dbviewer --lib drivers::mongo`: **95 passed, 5 ignored** (the ignored cases are explicit live/performance tests, not failures).
- `OTTO_DBV_E2E=1 OTTO_MONGO_PARITY_PORT=27039 cargo test -p otto-dbviewer --test mongodb_e2e -- --ignored --nocapture`: **13 passed, 0 failed** on isolated Mongo **8.3.13**.
- `cargo test -p otto-dbviewer --lib mongo_distinct_e2e -- --ignored --nocapture`: **1 passed**, using that test's disposable container.
- Added a final live `.find({}).explain()` assertion with timeout/comment settings and reran the affected fallback integration: **1 passed**.
- Scoped rustfmt and `git diff --check`: passed.

The 13-test integration run covers connection/version, schema/detail/indexed completion, native shorthand/JSON reads, import batches, query plan, upsert options, findOne projection, sorted skip cursors, native distinct/count, shell count options, administrative ping, raw prints, multiline sibling DB expressions, selected database, exact limit/truncation, sparse typed JSON/NDJSON export, shell timeout, and concurrent cancellation isolation.

Cancellation isolation test (`tests/mongodb_e2e.rs:625`) starts two real slow Mongo queries with the SAME client ID, waits until both server operations are visible, cancels one, asserts the other remains running, then cancels the second. This proves the boundary rather than just comparing generated strings.

## Final scoped scores

The independent reviewer conditionally assessed all four lenses at 9.8 after re-reading the fixes, subject to these gates. Those gates now passed.

| Lens | Score / 10 | Basis |
|---|---:|---|
| L01 native Mongo methods | 9.8 | Native common methods, strict argument handling, real-shell fallback for unsupported methods/options, no silent dropped options, live distinct/count/findOne/skip/options paths. |
| L02 parser fidelity | 9.8 | Dotted names, chains, BSON/EJSON round trips, malformed scope/filter inputs, zero/negative limit handling and explicit SQL-subset errors covered by regressions. |
| L03 mongosh coverage | 9.8 | Raw scripts preserved, formatted db expression results shown, active database/auth URI maintained, cursor/scalar/admin/options live-tested. |
| L04 results/control | 9.8 | Exact limits, bounded output, typed sparse export, actual shell timeout and server-side cancellation isolation verified; explain settings accepted by the live server. |

These are scoped engineering review scores, not a claim that every Mongo server command/version/type permutation was tested. The native parser remains deliberately finite; unsupported JavaScript expressions require installed mongosh and retain conservative write classification on saved read-only/approval-governed connections. Raw scripts retain their own print behavior. CSV/TSV preserve their documented fixed schema. SQL→Mongo remains a convenience subset, with relational edge semantics outside the native Mongo parity claim. SSH/TLS paths reuse existing runner configuration and unit coverage; no new remote SSH/TLS server was exercised.

No real user database was touched, and this worker performed no commit, push, PR, or deployment. Dedicated fixture container remains available to the coordinator until cleanup.
