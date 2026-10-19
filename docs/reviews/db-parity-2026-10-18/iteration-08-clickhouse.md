# ClickHouse parity — iteration 08: Endpoint, authentication and session boundary review

Confirmed endpoint bug: unbracketed IPv6 profile hosts generated invalid HTTP URLs/native endpoints. `clickhouse.rs:1093` now brackets IPv6 literals once for both transports; tests cover raw and already-bracketed IPv6 and DNS hosts.

Traced HTTP header authentication (no password in URL), profile cache keys, selected database, server-enforced explicit readonly, original tunnel hostname for TLS SNI, custom CA and verify policy, and independent native control authentication. Live bad-credential regression checks that the password is absent from the error and a good configuration still succeeds. IPv6 connection establishment, real SSH tunnels and externally issued TLS certificates were not exercised against the fixture, so these are source/unit checks rather than live parity claims.

| Lens | Score | Remaining evidence boundary |
|---|---:|---|
| L09 statements | 9.7 | Broad SQL/admin execution covered; not every server input/output protocol. |
| L10 transport/session parity | 9.6 | Native modern types and strict readonly=1 startup profiles remain limited. |
| L11 fidelity/errors | 9.4 | Direct native Dynamic/JSON unsupported; now explicit errors, HTTP/workaround available. |
| L12 resources/limits/cancel | 9.7 | Live bounded export/cancel coverage; exhaustive races and old server framing not proven. |

These are distinct review passes over the evolving shared working tree. Final consolidated test results are recorded in iteration 10; no 9.8 target claim is made.
