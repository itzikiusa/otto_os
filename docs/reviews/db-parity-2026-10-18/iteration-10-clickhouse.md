# ClickHouse parity — iteration 10: final integration review

The 9.8-per-lens target is **not achieved**. The supported HTTP/native paths improved materially, but direct native decoding still depends on klickhouse 0.15.3's type model. In particular, native JSON and Dynamic were reproduced as unsupported. Such failures now return explicit HTTP/toJSONString guidance rather than successful empty results. Other modern types not recognized by that dependency share this limitation; a full native protocol implementation was not introduced.

Final review traced operation ownership, batch-only native session reuse, native stream closure, HTTP capability negotiation before user SQL, typed Map conversion, FORMAT replacement, capped versus uncapped export routes, compressed-window delivery, and error completion. The newly extracted `drivers/clickhouse/tests.rs` keeps the implementation below the repository's 4,000-line gate without changing its tests. No common classifier, service, contracts, UI, dependency, or other driver's source was changed by this work.

## Verification

- `cargo test -p otto-dbviewer --lib drivers::clickhouse`: **40 passed, one optional benchmark ignored**.
- `OTTO_DBV_PARITY=1 cargo test -p otto-dbviewer --test clickhouse_e2e clickhouse_parity -- --ignored --nocapture`: **11 passed**, 3.06 seconds, isolated ClickHouse 26.3 container on HTTP 28123/native 29000.
- Live cases cover FORMAT/settings; typed integers/enums/maps; SET/USE and timezone; selected/default database isolation; DDL/INSERT and explicit readonly; BACKUP operation id/status; native export; row/zero caps; export limits with existing LIMIT/SETTINGS; escaped multiline values; JSON export over 128 MiB; unsupported native type errors and explicit workaround; concurrent native tabs; real native cancellation/deadline and bounded asynchronous cleanup; both-transport midstream export failure; bad-credential isolation without password disclosure.
- The cleanup regression initially assumed KILL ASYNC acknowledged full termination. With independent native operations, verification no longer queued behind the cancelled query. The corrected test checks actual removal from `system.processes` within three seconds and retains the real deadline assertion; it does not assume an asynchronous acknowledgement is a join.
- `cargo test -p otto-dbviewer --lib`: **519 passed, two failed, seven ignored** on the then-current shared tree. Failures were the shared Mongo comment classifier test at `types.rs:2003` and a Redis transport fixture handshake timeout at `tests/unit/redis_transport.rs:586`; both were reported to root/owners. This is not a green full-suite claim.
- Owned Rust files passed rustfmt check and git diff whitespace check. Full clippy and release gates remain root's coordinated final gate; the Cargo slot was released to MySQL after these results. The final post-test edit only removed an unnecessary reference in a slice call.

## Final scores and limits

| Lens | Score | Evidence boundary |
|---|---:|---|
| L09 statement/result coverage | 9.7 | SELECT/SHOW/DESCRIBE/EXPLAIN plus DDL/DML/admin rowsets routed; BACKUP status live verified. Real RESTORE, all admin engines and clickhouse-client file/inline-input protocols are not exhaustively covered. |
| L10 transport/session parity | 9.6 | Isolated native operations and batch sessions; HTTP batch state, FORMAT/settings and selected DB verified. Native type support and strict server readonly=1 startup profiles remain limitations. Real SSH/TLS and IPv6 establishment were source/unit reviewed, not live fixture tested. |
| L11 typed results/errors | 9.4 | Duplicate/nullable/typed-empty Maps preserved, wide integers precision-safe, native enum/timezone handling, no silent empty result on protocol failure. Native JSON/Dynamic remain unsupported directly; quoted HTTP integers inside JSON differ from explicit native toJSONString numeric serialization by design. |
| L12 limits/resources/cancellation | 9.7 | Native block export, capped HTTP row streaming, 128 KiB decompression windows, over-128-MiB JSON export, concurrent native operations and asynchronous cleanup verified. Exhaustive cancellation interleavings, very large single cells and old-server raw exception framing remain outside live evidence. |

Reports 04–09 contain distinct review passes and concrete fixes, not repeated score-only reports. Root owns the overall 24-lens decision and publication/deployment; no commit or push was made by this worker.

## Post-integration follow-up: strict native read-only startup

The previously listed readonly=1 startup limitation is now repaired. A live
SELECT-only fixture first failed before reaching Otto's setup: klickhouse0.15.3
unconditionally issued SET date_time_input_format='best_effort', rejected with
server error164. A narrowly documented vendored startup patch accepts this
specific rejection only after the same connection confirms readonly=1. Otto's
own setup similarly verifies an already-active matching timezone and preserves
readonly1 instead of lowering it to readonly2 for enforced reads. Other errors
and timezone mismatches remain errors; user SQL is never retried.

`clickhouse_parity_native_strict_readonly_profile` failed red in
/tmp/otto-ch-strict-readonly-red.log, then passed after both startup layers were
fixed (/tmp/otto-ch-strict-readonly-green.log). It checks ordinary and enforced
reads retain readonly1, SET readonly0 is rejected, and a different requested
timezone fails. Unique test-user creation/removal uses the isolated
`otto-parity-ch-20261018` fixture, not the user's embedded usage server.

The full live parity filter now passes12/12 (3.03s), and focused units40/40 with
one optional benchmark ignored. Logs: /tmp/otto-ch-parity-readonly.log and
/tmp/otto-ch-units-readonly.log. L10's specific startup limitation is closed;
its broader score is not automatically raised: live SSH/TLS and older-server
coverage remain separate. Native JSON/Dynamic/Variant decoding is still absent.

A further native-format probe on local26.3.1.715 confirmed
output_format_native_write_json_as_string=1 still emits JSON/Dynamic/Variant
column type names, so that setting alone cannot repair klickhouse's type parser.
The exact byte dump is /tmp/otto-ch-native-types.bin. Upstream NativeWriter writes
the type name independently from serialization:
https://github.com/ClickHouse/ClickHouse/blob/master/src/Formats/NativeWriter.cpp
This hypothesis was rejected; no implicit format-setting workaround was enabled.

Follow-up all-target `cargo clippy -p otto-dbviewer --all-targets -- -D warnings`
passed (18.77s), /tmp/otto-ch-readonly-clippy.log. Final combined consumers pending.
