# Mongo parity — iteration 01

Scope: L01 native methods, L02 parser fidelity, L03 mongosh coverage, L04 result/cancellation/timeouts. Existing distinct implementation and tests were preserved. No commit/push/deploy performed.

## Findings and changes

- **L01 major, confirmed RED:** `parse_shorthand` silently dropped the third argument to `find`, `updateOne`, `replaceOne`, and accepted incompatible chains. Upsert/collation could be lost. Native argument arity and chaining now reject unsupported forms; `run_one` executes those through real mongosh before any native operation starts. Negative limits use shell single-batch semantics; zero means no explicit limit. `mongodb.rs:937,2430`; regression tests `tests/unit/mongodb.rs:985,1006`.
- **L02 major, confirmed RED:** dotted collection names split at the first dot; `db.system.profile.find` failed. Split now occurs at the last dot before the first call. `NaN`, `Infinity`, `undefined`, bare `Date()`, multiargument constructors and surrogate escapes were being changed into null, dates, or replacement characters. These now require the shell instead of changing values. `mongo_parse.rs:313,441`; regressions `tests/unit/mongodb.rs:1011,1017`.
- **L02 major, confirmed RED:** SQL translation silently ignored DISTINCT/OFFSET and LIMIT 0 had Mongo's opposite meaning. These unsupported forms now error clearly instead of returning incorrect rows. `mongo_sql.rs:46`; regression `tests/unit/mongodb.rs:1034`. Full SQL parity is not claimed.
- **L03 major, confirmed trace + live test:** valid `findOne`, `skip`, count options, administrative methods and arbitrary method options were unavailable. Parse failures now use the existing credential-safe, timeout-bounded mongosh runner. An async expression wrapper emits canonical Extended JSON for scalars/documents/arrays/cursors and closes cursors after the row-limit probe. Existing detected scripts retain their original body. `mongodb.rs:1688,1844`.
- **L04:** unchanged saved read-only/approval classification uses the same strict parser, so unsupported expressions remain conservatively classified as writes. Native execution errors never trigger shell retries (preventing repeated writes). Explicit explain requests do not execute an unsupported expression as a query. Shell child retains kill-on-drop, output byte bounds and timeout behavior. SSH SOCKS URI and TLS-file behavior reuse existing tested runner.

## Evidence

- RED: 4 new native regression groups failed with NaN-to-null, dotted collection error, limit 0 stored explicitly, and dropped find options. SQL regression separately failed on ignored DISTINCT using the already-built test binary. One concurrent MySQL compilation failure was reported to its owner and resolved there.
- GREEN: `cargo test -p otto-dbviewer --lib drivers::mongo`: **85 passed, 5 ignored**.
- Live: `OTTO_DBV_E2E=1 OTTO_MONGO_PARITY_PORT=27039 cargo test -p otto-dbviewer --test mongodb_e2e mongo_shell_fallback_preserves_options_and_returns_values -- --ignored --nocapture`: **1 passed**. Covers shell upsert, findOne projection, native count, sorted skip cursor + truncation, countDocuments options, admin ping, raw script prints, selected DB.
- Fixture: dedicated `otto-parity-mongo-20261018` Mongo 8 container on loopback 27039. No real user database used. Test collections cleaned after success; container retained temporarily for subsequent iterations.
- Direct mongosh wrapper test also verified a 2+1 cursor probe and close. Installed mongosh rejected top-level `await (async...)`; final wrapper returns the async IIFE, which mongosh awaits successfully.
- Official references: https://www.mongodb.com/docs/manual/reference/method/db.collection.find/ and https://www.mongodb.com/docs/manual/reference/method/db.collection.findOne/ (third options argument; findOne document result).

## Scores after iteration 01

| Lens | Score / 10 | Evidence and remaining gap |
|---|---:|---|
| L01 native methods | 9.0 | Native common reads + shell options verified; fallback requires installed mongosh and is conservatively gated on saved read-only connections. |
| L02 parser fidelity | 8.8 | Five regression groups pass; JSON command unknown fields/BSON sentinel completeness and more SQL unsupported-clause paths still need audit. |
| L03 shell coverage | 9.1 | Live options/admin/cursor/scalar/raw script/selected DB verified; detected getSiblingDB raw expressions still require explicit print, shell-only interactive helper syntax not proven. |
| L04 result/cancellation/timeouts | 9.0 | Existing bounded output/timeouts/cancel tests pass and cursor truncation live-tested; native explicit limit probe still deserves correction, new fallback cancellation/timeout live cases pending. |

No lens is represented as 9.8; further iteration is warranted.
