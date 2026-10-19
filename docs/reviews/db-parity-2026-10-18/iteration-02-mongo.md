# Mongo parity — iteration 02

Iteration 01 fixes remain intact. This pass addressed observed gaps in JSON/native safety, BSON fidelity, results, SQL convenience translation and connection parity.

## Implemented findings

- Native `deleteMany()`/`deleteOne()` previously supplied `{}` when the required filter was omitted. JSON writes also accepted absent/null filters. All now require explicit filter objects. Malformed JSON `explain:'true'`, numeric `op`, wrong-type `pipeline`, unknown options, and invalid limits now error rather than silently widening/changing the operation. An unterminated block comment could also make invalid delete text look valid; the literal parser now rejects it.
- Explicit native `.limit(2)` now requests exactly two rows when within the explorer cap and does not incorrectly label the complete result truncated. Explorer caps still use a probe.
- JSON/NDJSON exports now retain every document's sparse keys and BSON type sentinels. CSV/TSV retain their documented first-document column schema.
- Canonical Extended JSON handles non-finite doubles, min/max keys, undefined, symbol, JavaScript code and scope, and legacy DBPointer values. Legacy UUID subtype 3 exports as `$binary`, preserving subtype instead of rewriting it as subtype 4 via `$uuid`.
- Shell connection URI now brackets IPv6 hosts and encodes authSource/replicaSet/database values.
- SQL field aliases now map to Mongo projection expressions. COUNT(field) excludes null/missing fields. Unsupported aggregate DISTINCT, window/filter/order modifiers and non-field aggregate forms fail clearly instead of being silently changed. SQL translation remains a documented subset, not a full relational engine.
- Missing-mongosh errors now explain both scripts and unsupported native expressions, with an installation/PATH remedy.

## Verification

Regression tests were added before production edits. Unlike iteration 01, RED execution for this batch was deferred at the coordinator's instruction because other workers held the shared Cargo slot; defects were hand-traced before changes. Root's intervening complete dbviewer library run passed 502 tests, including the then-current changes. Additional edge regressions added afterward still require the final coordinated gate; no unexecuted test is represented as passed here.

Isolated Mongo 8 fixture: loopback 27039, dedicated container `otto-parity-mongo-20261018`. Seeded customers/orders/profiles only inside this disposable container, enabling the full `mongodb_e2e` suite with `OTTO_MONGO_PARITY_PORT=27039`.

Official BSON reference: https://www.mongodb.com/docs/manual/reference/mongodb-extended-json/ and https://github.com/mongodb/specifications/blob/master/source/extended-json/extended-json.md (notably `$uuid` means subtype 4).

## Provisional scores pending final execution

| Lens | Score / 10 | Remaining verification |
|---|---:|---|
| L01 native methods | 9.6 | New malformed-write/JSON/exact-limit cases need final gate. |
| L02 parser fidelity | 9.6 | Rare BSON/URI/SQL aliases and aggregate cases need final gate. |
| L03 shell coverage | 9.6 | Expanded live expression/selected DB/output cases need final gate. |
| L04 results/cancellation/timeouts | 9.5 | Full JSON sparse export, shell timeout and repeated-client-id cancellation tests need execution. |

Iteration 03 peer review found additional cancellation/explain/formatting/date edge cases, recorded separately. These provisional scores do not claim the 9.8 target.

## Final follow-up

Iteration 03 completed the deferred gates: 95 focused unit tests, all 13 live Mongo integration tests, and the existing native distinct live test passed. The final independent review scores and remaining capability limits are recorded in iteration-03-mongo.md; the table above intentionally preserves the provisional iteration-02 assessment.
