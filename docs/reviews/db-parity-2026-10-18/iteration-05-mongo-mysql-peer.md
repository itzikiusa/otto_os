# Iteration 05 — Mongo repair and independent MySQL review

No Cargo commands executed: coordinator owns upgraded-toolchain server compilation. MySQL/vendor/service review was read-only; Mongo and delegated shared script-predicate fixes were authorized by coordinator during this pass.

## Mongo findings and fixes

1. **Legacy count conflation (confirmed):** `count()` / cursor `count()` / JSON `op:count` previously executed Rust `count_documents`, substituting aggregation semantics for the legacy count command. They now execute the native `count` command; `countDocuments` retains its separate variant. An absent legacy filter stays absent, and maxTimeMS plus the server-owned cancellation comment are included. Native execution errors never trigger replay/fallback. Count replies validate nonnegative integral BSON Int32/Int64/Double and use precision-preserving output conversion.
2. **Literal text triggers script mode (confirmed):** substring detection classified `db.c.find({note:'process.env'})` and `db.c.find({note:'db.getSiblingDB('})` as scripts, causing read-only refusal. Both editor/backend now anchor these special expression openers to the comment-stripped line start. Actual declarations/control flow still use token detection. Added trailing-comment and policy-classification negatives.

Official semantic basis: [collection.count](https://www.mongodb.com/docs/v7.1/reference/method/db.collection.count/) describes metadata/sharding distinctions; [countDocuments](https://www.mongodb.com/docs/manual/reference/method/db.collection.countdocuments/) describes aggregation-based count semantics. Direct isolated Mongo 8.3.13 probe: native `count` with `$where`, comment and maxTimeMS returned `n:4`; `countDocuments` with the same `$where` returned `BadValue`. The added driver live regression uses this difference, rather than merely testing equal ordinary counts.

UI test before literal fix: **8 passed, 1 failed** on the process.env literal. After fix: **9 passed, 0 failed**. Rust count/predicate regressions and driver live additions are written but await coordinator Cargo gate. Scoped rustfmt and diff check passed.

## MySQL independent trace

Compared all six changed vendor Rust files directly against local crates.io sqlx-mysql 0.9.0 source: query_result, options/mod, connection/mod, connection/stream, connection/establish, connection/executor. Changes are limited to rowset completion metadata and explicit LOCAL INFILE bytes grants.

- Shared pools default LOCAL_FILES off; the explicit file request gets a cloned request-owned pool preserving authentication/TLS/setup and one connection shared by the complete batch.
- File authorization is cleared/reissued by the service after current root host-file checks, including approved execution. Ordinary server-side INFILE is not classified as host-file access. Saved read-only handling remains separate.
- Driver file loading requires its explicit authorization boolean, parses the statement's filename and enforces the existing size/regular-file bounds. Governed read execution never enters this file loader and uses the capability-disabled pool.
- Vendor grant is taken once before execution; server filename bytes must match exactly. A missing, mismatched, or second grant shuts down the connection and returns error. SQLx never opens a server-selected path. Upload chunks preserve packet sequencing and terminate with an empty packet. No command retry was found.
- Metadata snapshots use Arc cloning plus existing Arc::make_mut before a later rowset, preserving empty-result schema without replay.
- Existing integration test checks same-session temporary table/user-variable LOAD semantics and a protocol proxy substitution, asserting actual substitution and zero uploaded bytes. Service tests check root grant, grant clearing, and owner denial. These tests were reviewed, not executed by this peer.

Remaining semantic edge sent to MySQL owner: escaped or double-quoted filenames require a `SELECT @@session.sql_mode` observer before LOAD, which changes session ROW_COUNT/FOUND_ROWS state used by subsequent expressions. Ordinary single-quoted filenames skip this observer. This limits universal same-session parity; owner assessment/repair remains pending at report time. Vendor README's earlier metadata-only claims should be labeled as that subsection, since a later section adds protocol changes.

## Scoped scores at this checkpoint

| Lens | Score | Basis |
|---|---:|---|
| L01 Mongo methods | 9.7 | Legacy count semantic repair implemented, server difference proven; driver test pending. |
| L02 Mongo parser/BSON | 9.7 | Separate operations and count numeric validation added; unit execution pending. |
| L03 Mongo shell | 9.7 | Literal-trigger repair UI RED/GREEN; backend gate pending. |
| L04 Mongo results/control | 9.8 | Prior control proofs retained; count command explicitly carries bounds/marker. |
| L05 MySQL native commands | 9.7 | LOCAL INFILE protocol/authorization trace sound; new live gate pending. |
| L06 MySQL batch/results | 9.7 | Shared batch connection and empty rowset metadata trace sound; observer semantic edge remains. |
| L07 MySQL values/metadata | 9.8 | Narrow metadata snapshots preserve result boundaries without replay; prior validation retained. |
| L08 MySQL sessions/control | 9.7 | Request isolation/retirement and one-shot grant verified by trace; observer edge and new execution gates pending. |

No universal compatibility or new Cargo execution is claimed. No MySQL/vendor/service source was edited by this reviewer.

## Same-pass MySQL peer follow-up

MySQL owner removed the SQL-mode observer and exposed SQLx's existing `SERVER_STATUS_NO_BACKSLASH_ESCAPES` status bit through a read-only accessor. Re-read confirms filename decoding now reads connection state without executing SQL. Native server still validates double-quote syntax under ANSI_QUOTES. Escaped-filename ROW_COUNT preservation and NO_BACKSLASH_ESCAPES live regressions were added by the owner; README now explicitly scopes the earlier claims to the metadata patch. The confirmed observer concern is repaired in source. L05/L06/L08 can be assessed at 9.8 once the new integration gates pass; this reviewer has not executed them.
