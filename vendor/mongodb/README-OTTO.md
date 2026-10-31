# Otto patch: zstd 0.14

Vendored from crates.io mongodb **3.9.1**, checksum
`af84443ae9878f59a6818686f339064479f42cdf293c4c637d8fc9c0a8c5eed6`
(the latest release when vendored, 2026-10-31). Registry-normalized
Cargo.toml and Cargo.toml.orig are retained; registry cache markers are
omitted. Licensed Apache-2.0 (as published upstream:
https://github.com/mongodb/mongo-rust-driver).

## Patch

Only the `zstd` dependency changes: `0.13.3` → `0.14.3` in Cargo.toml and
Cargo.toml.orig. No source file changes — the driver's zstd use
(`stream::copy_decode`, `Encoder`, `compression_level_range`,
`DEFAULT_COMPRESSION_LEVEL`) is unchanged between the two versions.

Why: `cargo audit` reports zstd 0.13 / zstd-safe 7.3 vulnerabilities
(RUSTSEC-2026-0391, -0398, -0403) and unsound-API notices fixed only in zstd
0.14 / zstd-safe 8.2. Otto keeps the driver's `zstd-compression` feature
(otto-dbviewer: large Mongo reads go ~54 s → ~1.5 s with it).

Drop this vendor copy (and its `[patch.crates-io]` line in the root
Cargo.toml) once an upstream mongodb release depends on zstd ≥ 0.14.
