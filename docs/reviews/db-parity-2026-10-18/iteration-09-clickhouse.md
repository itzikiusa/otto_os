# ClickHouse parity — iteration 09: Failure completion and exported artifacts

Rechecked native channel-close failure versus genuine empty metadata, stale cached-client recovery, server exception cleanup, gzip/unsupported encoding rejection, JSON duplicate keys, and exporter finish semantics. Added both-transport export failure tests with throwIf after 10,000 rows, using unlimited and client-capped exports; neither may report success. Raw HTTP server failure was also probed directly: ClickHouse 26.3 closes the chunked body incompletely, which reqwest propagates.

The capped decoder rejects incomplete rows and missing metadata and issues best-effort KILL on decode/write failure. Native export rejects a closed connection instead of finalizing partial rows. Export destination cleanup/atomicity remains the shared export/service contract, not reimplemented in this driver. Older ClickHouse raw-format midstream error framing is not covered by this single-version fixture. No unproven compatibility claim is used to raise the score.

| Lens | Score | Remaining evidence boundary |
|---|---:|---|
| L09 statements | 9.7 | Broad SQL/admin execution covered; not every server input/output protocol. |
| L10 transport/session parity | 9.6 | Native modern types and strict readonly=1 startup profiles remain limited. |
| L11 fidelity/errors | 9.4 | Direct native Dynamic/JSON unsupported; now explicit errors, HTTP/workaround available. |
| L12 resources/limits/cancel | 9.7 | Live bounded export/cancel coverage; exhaustive races and old server framing not proven. |

These are distinct review passes over the evolving shared working tree. Final consolidated test results are recorded in iteration 10; no 9.8 target claim is made.
