# Consolidated verification

Review covered 24 named lenses, with specialist and independent passes through
iteration 10. The reports preserve intermediate defects, failures and provisional
scores; later green checks do not erase that history. Lenses are assessment
categories, not 24 simultaneous processes. Native specialists and Ronaldo/Pele
worked in parallel, with serialized Cargo builds on the shared worktree.

## Current targeted checks

| Area | Executed result |
|---|---|
| Mongo | 98 focused units, 10 script-policy tests (overlapping), 13 live cases, 1 separate native distinct live case passed |
| MySQL | 32 live cases, 1 multi-case LOCAL INFILE test, 14 focused units, 27 splitter tests and 2 then-current policy tests passed |
| ClickHouse | 40 focused units and 11 live cases passed |
| Redis | 36 focused units, 1 shared-classifier test and 1 live parity scenario passed; live scenario includes 31-second blocking and cancellation cleanup |
| UI | 1996 unit tests, type checks/guards, production build and bundle-size gate passed |
| Session sizing | 4 current named browser cases passed, desktop/phone and light/dark; screenshots in screenshots/ |
| Workspace | Formatting, full clippy and all 5,806 nextest cases passed on Rust 1.99.1; 124 skipped; doc-tests passed (one ignored), nextest filter guard passed |
| Plugin | 492 passed, one skipped; browser suite included |
| Access follow-up | Activation/approved-change owner consistency, workflow cases, legacy MCP and state-read budgets passed in the final nextest run |

## Latest lens assessments

| Lens | Score | Boundary |
|---|---:|---|
| L01 Mongo native methods | 9.8 | Finite native parser plus explicit shell fallback |
| L02 Mongo parser/BSON | 9.8 | Typed values, strict JSON forms, legacy count semantics |
| L03 Mongo shell | 9.8 | Installed mongosh required; arbitrary JavaScript remains conservatively classified |
| L04 Mongo results/cancel | 9.8 | Cursorless distinct cap, bounded native cancel; find/aggregate streaming export |
| L05 MySQL native coverage | 9.8 | MySQL 9.7.1 live; not a multi-vendor certification |
| L06 MySQL batches/results | 9.8 | Native result boundaries and SQL-mode regression verified |
| L07 MySQL values/export | 9.8 | Typed values and empty metadata; LOCAL regular files capped 100 MiB |
| L08 MySQL session/cancel | 9.8 | Operation state isolation; possible LOCAL requests pay dedicated connection setup |
| L09 ClickHouse commands | 9.7 | Administrative/file protocol matrix not exhaustive |
| L10 ClickHouse transport | 9.6 | Live HTTP/native; real SSH/TLS and older-server matrix not exercised |
| L11 ClickHouse values | 9.4 | Native JSON/Dynamic/Variant unsupported; HTTP or explicit conversion required |
| L12 ClickHouse resources | 9.7 | Live large streaming, limits and cancellation; all race interleavings not proved |
| L13 Redis commands | 9.8 | Completion/classifier coverage; continuous push commands use terminal |
| L14 Redis parser/batches | 9.8 | Quote/binary semantics and native batch behavior verified |
| L15 Redis responses | 9.8 | Nested errors preserved; Redis Stack module server matrix not run |
| L16 Redis state/cancel | 9.8 | Live 31s blocking and bounded cancellation cleanup; no universal latency guarantee |
| L17 messaging | 9.7 | Existing root/owner/admin behavior preserved; real sibling messaging delivered |
| L18 workflow folders | 9.6 | Human-root exception and negative cases passed; unchanged node/trigger paths remain outside this exception |
| L19 execution policy | 9.7 | Owner/account consistency, activation/preflight and corrected legacy/read-budget checks passed |
| L20 authorization compatibility | 9.5 | Existing identity, profile, read-only and approval boundaries retained |
| L21 editor routing | 9.8 | Current UI/native integration regressions passed |
| L22 session sizing | 9.8 | Four current browser cases; not packaged WKWebView automation |
| L23 contracts/guidance | 9.8 | Current API, feature and in-app help reconciliation |
| L24 regression/release | Pending | Required CI, merge and installed build not yet verified |

The requested 9.8 target was not reached for every lens after ten passes.
In particular, native ClickHouse type coverage remains a concrete limitation.
No claim of universal Mongo/MySQL/ClickHouse/Redis parity is made.

## Release

One PR is authorized. Merge only after non-advisory checks pass, then synchronize
main and rebuild/reinstall using the receipted deployment script. No release has
been performed at this report snapshot.

## Full-gate failures and corrections

The first standard-Cargo workspace run exposed a gRPC test's process-global
counter interference and LOCAL permission preflight regressions: ordinary
queries gained an extra state read, and legacy callers received an unrelated
feature check. The gRPC test now asserts its own fixture's accepted connections.
LOCAL preflight uses a conservative byte check for the literal keyword (not
SQL-mode-dependent splitting) before loading host authority. Ordinary requests
retain their state-read budget and pool; the driver still validates each actual
LOAD and exact filename. The complete nextest rerun passed all 5,806 cases (124 skipped), including all six prior failures.

## CI fixture follow-up

The first Linux live-driver run passed 92 of 95 cases. Two shell tests failed
because mongosh was absent from the runner PATH; the Redis parity case targeted
the isolated local-review port instead of CI's seeded server. The workflow now
installs checksum-pinned mongosh 2.5.9 (smoke-verified on Ubuntu 24.04) and passes
the seeded Redis port/password explicitly. No driver behavior or assertions were
weakened. The corrected CI run remains pending at this snapshot.
