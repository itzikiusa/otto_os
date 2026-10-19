# Otto patch: strict read-only native startup

Vendored from crates.io klickhouse **0.15.3**, checksum
`43df4e6356e65204d50730a3699b49a0ad345a947b725a0aaf7544f691c8e98a`.
Upstream: https://github.com/Protryon/klickhouse at
`0e514f93fb07073dbf86f7b7a180e4656f117244`, subdirectory `klickhouse`.
MIT and Apache-2.0 license files are copied from that exact upstream revision.
Registry-normalized Cargo.toml and Cargo.toml.orig are retained; registry cache
markers and package Cargo.lock are omitted. This patch is not upstream acceptance.

## Patch

Only src/client.rs changes. Connection startup normally runs
`SET date_time_input_format='best_effort'`. A server-enforced readonly=1 profile
rejects that command, so otherwise valid SELECT-only credentials cannot connect.

On that command's server error164 only, query the same connection's readonly
setting. Accept startup only when it is exactly1, preserving server settings.
All other errors propagate. The profile disallows writes already; no permission,
authentication, TLS, protocol decoder or user-query retry behavior changes.
Otto's driver separately validates its requested timezone and preserves readonly1
when enforcing a read, instead of trying to lower it to readonly2.

## Regression / removal

`clickhouse_parity_native_strict_readonly_profile` in otto-dbviewer/tests/clickhouse_e2e.rs
creates a unique SELECT-only user on the isolated parity server, proves normal and
Otto-enforced reads retain readonly1, rejects SET readonly0 and a mismatched
requested timezone, then removes the fixture user. It failed before this patch.
Run with OTTO_DBV_PARITY=1 cargo test -p otto-dbviewer --test clickhouse_e2e
clickhouse_parity_native_strict_readonly_profile -- --ignored --nocapture.

On upgrades, compare this single startup hunk and remove the fork when upstream
supports strict read-only connections. This patch does not add native
JSON/Dynamic/Variant decoding.
