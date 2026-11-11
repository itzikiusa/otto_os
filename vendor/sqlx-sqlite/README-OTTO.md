# Otto patch: allow libsqlite3-sys 0.39

Vendored from the crates.io `sqlx-sqlite` **0.9.0** release, package checksum
`488e99c397a62007e4229aec669a179816339afc6d2620ca6fa420dbee2e982c`.
Upstream repository: https://github.com/launchbadge/sqlx, subdirectory `sqlx-sqlite`.
The upstream MIT and Apache-2.0 licenses are preserved alongside this file.
`Cargo.toml` is the registry-normalized manifest. Registry cache markers are not
vendored. This is a local patch, not a claim of upstream acceptance.

## Why

RUSTSEC-2026-0459 (C SQLite CVE-2026-119375) is fixed in libsqlite3-sys 0.39.
sqlx-sqlite 0.9.0, the newest release, declares `libsqlite3-sys >=0.30.1, <0.38.0`,
so cargo cannot resolve the fixed version.

## Patch

1. `[dependencies.libsqlite3-sys] version` is `>=0.39.1, <0.40.0`.
2. `src/types/time.rs` `mod formats`: the `time` 0.3.48+ replacements for the
   deprecated `Year` / `Month` / `Hour` components and byte `Literal`s
   (`CalendarYearFullExtendedRange`, `MonthNumerical`, `Hour24`, `StringLiteral`)
   — each is exactly what `time` converts the deprecated form to, so parsing is
   unchanged. The deprecated forms built ~40 warnings on every build with
   time 0.3.55. `tests::new_formats_parse_like_the_deprecated_ones` parses the
   same samples with the old and new descriptions and requires identical results.

Remove this vendor directory and its `[patch.crates-io]` entry once an sqlx
release accepts libsqlite3-sys 0.39 and the new `time` API.
