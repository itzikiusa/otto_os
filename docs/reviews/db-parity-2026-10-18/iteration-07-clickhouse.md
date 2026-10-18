# ClickHouse parity — iteration 07: Native concurrency and cancellation ownership

Confirmed major by upstream client trace: shared standalone native clients queue operations internally. A timed-out queued request can miss KILL and later start after the caller exits; an unrelated slow tab also delays a quick query. `clickhouse.rs:626` now gives every standalone native operation its own connection. Explicit sequential batches retain their dedicated session, so SET/USE still carries within a batch.

Rechecked fresh unpredictable markers, same-user KILL predicates, independent control connections, timeout cleanup, partial-result cleanup, and batch cache removal. Added a regression that starts a long native sleep, requires another query on the same profile to finish within 2 seconds, then cancels only the slow operation. The existing real 1 second deadline and process cleanup regression remains. A cancellation before query registration is still coordinated by shared service lifecycle; this driver review does not claim a proof over every scheduler interleaving.

| Lens | Score | Remaining evidence boundary |
|---|---:|---|
| L09 statements | 9.7 | Broad SQL/admin execution covered; not every server input/output protocol. |
| L10 transport/session parity | 9.6 | Native modern types and strict readonly=1 startup profiles remain limited. |
| L11 fidelity/errors | 9.4 | Direct native Dynamic/JSON unsupported; now explicit errors, HTTP/workaround available. |
| L12 resources/limits/cancel | 9.7 | Live bounded export/cancel coverage; exhaustive races and old server framing not proven. |

These are distinct review passes over the evolving shared working tree. Final consolidated test results are recorded in iteration 10; no 9.8 target claim is made.
