# ClickHouse parity — iteration 04: Map fidelity and unsupported native types

Confirmed major: ordinary serde JSON objects discard duplicate Map keys. `clickhouse.rs:239` now preserves duplicate objects in both JSON decoders, and modern servers negotiate Map tuple-array output before user execution. Both HTTP request paths disable named-tuple objects, which live testing proved otherwise produced `{keys,values}` objects. Native typed maps use ordered pairs, including Enum keys and nullable values.

Confirmed major: klickhouse drops the result channel on unsupported native type decoding without forwarding the failure. `clickhouse.rs:626` now checks connection completion instead of returning successful empty results. `native_closed_error` gives HTTP/toJSONString guidance and explicitly does not replay a possibly executed statement. Native JSON's compatibility serialization setting was tested and rejected as a solution because the protocol still announces type JSON; no ineffective setting remains.

Verification: duplicate Map decoder unit regression; isolated live duplicate, nullable and typed-empty Maps; HTTP JSON and explicit native toJSONString(JSON); native Dynamic failure followed by successful workaround. Native Dynamic/JSON remain unsupported directly, rather than silently misreported.

| Lens | Score | Remaining evidence boundary |
|---|---:|---|
| L09 statements | 9.7 | Broad SQL/admin execution covered; not every server input/output protocol. |
| L10 transport/session parity | 9.6 | Native modern types and strict readonly=1 startup profiles remain limited. |
| L11 fidelity/errors | 9.4 | Direct native Dynamic/JSON unsupported; now explicit errors, HTTP/workaround available. |
| L12 resources/limits/cancel | 9.7 | Live bounded export/cancel coverage; exhaustive races and old server framing not proven. |

These are distinct review passes over the evolving shared working tree. Final consolidated test results are recorded in iteration 10; no 9.8 target claim is made.
