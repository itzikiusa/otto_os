# Build and verification performance

The build-performance work preserves the full Rust/UI checks and security scan
coverage. CI scheduling and tool caching already landed in `6876a07f`; this
follow-up integrates UI asset ownership and deployment cache preservation.
CodeQL remains an inactive experiment, with no repository settings change or
runner purchase. The measurements below distinguish the September 27 prototype
from integration checks against September 28 main (`20d570da`).

## What changes

- Both deployment entrypoints preserve Cargo caches by default. Cargo does not
  refresh the modification time of a reused dependency, so old does not mean
  unused. `PRUNE=1` explicitly trades cache reuse for disk space. The standalone
  `packaging/prune-target.sh` previews by default; `--apply` evicts older hashed
  dependency variants, never build-script outputs, fingerprints, incremental
  state, bundles or build receipts. Its timestamp heuristic can still evict a
  live dependency: run explicit cleanup only when builds are idle.
- CI checks free disk space before removing unused preinstalled runner tools;
  the threshold is conservative, and disk reports are retained for measurement.
  CI caches the pinned cargo-audit executable but fetches current advisories and
  performs a new audit each time.
- Embedded UI assets now belong to `ottod`, while `otto-server` receives an
  asset lookup function. `cargo build -p ottod --features embed-ui` keeps the
  same interface. UI-only edits no longer invalidate the large server crate.
  API/WS misses, MIME types, deep links and unembedded development mode retain
  their behavior. The server's existing `build_router` constructor remains
  available to its callers.
- CI uses pinned Nextest 0.9.146 to schedule tests across binaries, with bounded
  external-resource groups and a separate workspace doc-test gate.
- `AGENTS.md` distinguishes affected-package checks during editing from the full
  integration gates. Vite remains the UI development path; normal edits do not
  require packaging a desktop app.

## GitHub baseline

[CI run 36219379169](https://github.com/itzikiusa/otto_os/actions/runs/36219379169)
on `4ad7475d`, September 26, 2026:

| Work | Time |
| --- | ---: |
| Rust runner disk purge | 2m06 |
| Cargo cache restore (exact hit) | 17s |
| Clippy, workspace/all targets | 1m30 |
| Test compilation | 3m59 |
| Test execution and doc-tests | about 8m05 |
| Whole CI workflow | 16m30 |
| UI job, running concurrently | 1m55 |
| cargo-audit installation / actual audit | 2m53 / 5s |

The existing cache already hit. This work does not claim that adding another
cache alone removes the application compilation cost. The slowest observed
Rust test binary was otto-state (129.11s), then otto-server (35.45s), otto-rbac
(26.85s) and otto-sessions (23.73s). Standard `cargo test` runs binaries in
sequence, with parallel tests inside each binary.

[CodeQL run 36219378962](https://github.com/itzikiusa/otto_os/actions/runs/36219378962)
took 20m34 and already ran concurrently with CI. Rust already used
`build-mode: none`: extraction about 7m32, finalization 1m17 and queries 10m21.
See [CodeQL proposal](codeql-performance.md) for the inactive extraction-cache
experiment and the separate activation boundary. No CodeQL speedup has been
measured by this local change.

## Measurement rules

Compare the same source, features, profile, toolchain and concurrency. Record
cache warmup separately from unchanged reruns and actual edit rebuilds. Build
reports come from `cargo build --timings`; tests report compilation separately
from execution. A cached no-change command is not evidence of fast editing.
Do not run competing Cargo builds in the same target directory or reuse a target
between worktrees. Report other host workload when it prevents an isolated
benchmark. Report failed comparisons rather than excluding the failing tests.

Local host: Apple M5 Pro, 18 logical CPUs, 48 GiB RAM, Rust 1.96.0. Earlier
rooms verification used two build jobs and sequential tests to bound resources;
its 9m34 build and 4m01 Clippy timings are not normal-deploy benchmarks. The
unchanged full Clippy rerun took 0.62s, establishing that warm reuse works.

The release baseline for this pass used
`CARGO_BUILD_JOBS=8 cargo build --release -p ottod --features embed-ui --timings`.
It passed in 637.48s (10m37). Cargo reported 471.79s for otto-server (241.57s
frontend and 230.22s codegen), followed by 43.62s for ottod. Dependencies were
partly cached, and other checkout builds overlapped the early part of this run;
this is neither a clean build nor an isolated UI-edit speedup comparison. The
asset-boundary acceptance is whether a subsequent UI-only edit reuses the server
artifact, with its own observed elapsed time reported separately.

## Prototype verification — September 27

- Disposable deployment/cache fixtures: 14 passed. These reproduce both default
  eviction policies and verify explicit cleanup preserves build outputs.
- Workflow syntax and new runner configuration: validated locally. Subsequent
  GitHub runner results are recorded below.
- SPA asset-provider regression tests: four passed, covering binary bytes/MIME,
  deep links, API/WS misses and absent UI. The tests failed on the missing
  provider API before implementation.
- Independent reviews of the asset refactor, CI, cache cleanup and CodeQL
  template found no remaining confirmed issues.
- Optimized embedded daemon build passed. The controlled UI-only edit rebuilt
  only `ottod`; `otto-server` remained Fresh. Original UI bytes and the final
  embedded binary were restored and verified.
- Clippy passed with warnings denied for `otto-server` and `ottod`, all targets,
  default features (98s), then `ottod`, all targets, with `embed-ui` (7.58s).
  These are verification runs with different compilation inputs, not a
  before/after Clippy benchmark. Focused rustfmt and `git diff --check` passed.

## Test scheduling comparison

The local comparison used the same 93 compiled test binaries and the same
3,686 test identities: 3,620 regular tests and 66 ignored tests. Both runners
passed all regular tests. Neither retries nor test exclusions were added.
Six unrelated server fixtures explicitly disable metrics so they do not start
ClickHouse; this fixture correction was present in **both** measured runs.
The actual ClickHouse integration tests remained enabled and passed.

| Warm command | Observed wall time | Outcome |
| --- | ---: | --- |
| `cargo test --workspace --tests --no-fail-fast -- --test-threads=8` | 692.37s | 3,620 passed, 66 ignored |
| Nextest 0.9.146, frozen binary metadata, eight test processes | 147.96s | 3,620 passed, 66 skipped |
| Separate `cargo test --workspace --doc` | 16.52s | passed; currently zero doc-tests |

Nextest's total includes discovery; its execution-only summary was 121.287s.
The observed command improvement is 78.6% (4.68 times faster), or 76.8% after
adding the same doc-test gate to both totals. Warmup compilation (139.00s) is
excluded from both execution comparisons. Cargo/Nextest inventories were matched
by binary executable path, test name and ignored status, not only total counts.
Source edits for the SPA refactor happened after these binaries were frozen and
are verified separately.

Other agents' compilation overlapped part of the native run. This is one local
comparison, not a controlled multi-run benchmark or a claim about GitHub Linux
runner performance. CI uses four test processes rather than the local eight;
the later Actions observation is recorded below. Resource groups serialize ClickHouse
startup and limit PTY/session/sandbox process pressure. The metrics integrations
are kept; process-local mutexes alone do not provide cross-process exclusion.

## UI-only release rebuild

Command for each measured run:

```sh
CARGO_BUILD_JOBS=8 cargo build --release -p ottod --features embed-ui --timings -v
```

The first build after moving asset ownership passed in 390.38s. Its server unit
still took 357.27s, followed by 32.41s for the daemon. This one-time structural
change necessarily recompiles the server; changes inside that crate still do.
Other checkout compilation overlapped, so the difference from the earlier
637.48s build is not a controlled speedup measurement.

After that warmup, a controlled comment was appended to the generated
`ui/dist/index.html`. No Rust source, manifest, profile or toolchain changed.

| Run | Wall time | Cargo result |
| --- | ---: | --- |
| Unchanged release build | 0.942s | server and daemon Fresh |
| Generated UI asset edit | 38.963s | server Fresh; only daemon compiled |
| Restore original UI bytes | 34.157s | server Fresh; only daemon compiled |

The verbose log explicitly reports `Dirty ottod` because `ui/dist/index.html`
changed and `Fresh otto-server`. The edited binary contained the unique comment;
the restored binary did not. Original and restored UI SHA-256 hashes matched.
This proves updated assets ship without invalidating the large server crate.
It does not measure Vite compilation, desktop bundling, signing or installation,
and is not a promise of a 39-second complete deployment or cold build.

Evidence for this local run: `/tmp/otto-ui-rebuild-results.json`,
`/tmp/otto-ui-rebuild-{unchanged,edited,restored}.log`,
`/tmp/otto-build-asset-boundary.log` and `target/cargo-timings/`.
The source of the embedding macro is now `crates/ottod/src/ui_assets.rs`;
server routing tests use an injected fixture instead of generated UI files.

## Published CI observation — September 28

[Main CI run 36404833398](https://github.com/itzikiusa/otto_os/actions/runs/36404833398)
on `20d570da` passed after the CI-only changes were merged:

| Work | Observed time |
| --- | ---: |
| Whole CI workflow, including queue/start overhead | 8m38 |
| Rust job | 8m35 |
| Disk headroom check (no purge needed) | under 1s |
| Cargo cache restore | 22s |
| Clippy, workspace/all targets | 1m22 |
| Nextest, including compilation | 6m08 |
| Separate doc-test gate | 9s |
| cargo-audit job (cached executable; fresh scan) | 14s |

This is a successful production CI observation, compared with the historical
16m30 workflow and 3m09 audit job. The source and cache state differ, so it is
not a controlled speedup benchmark. All three CI jobs passed; no tests or
security queries were removed for these savings.

## Integration verification — September 28

- Fourteen deployment/cache fixtures passed on the current checkout. They run
  only in disposable directories and cover both deploy entrypoints.
- Shell syntax checks, inactive CodeQL workflow actionlint and diff checks passed.
- All four asset-routing tests passed with
  `cargo test --locked -p otto-server --lib spa::tests -- --nocapture`.
  Compilation took 4m44s while warming this checkout's test configuration;
  test execution took 0.01s. This is verification, not a speedup measurement.
- Independent review found no introduced regressions. The pruner's inherited
  best-effort process detection remains documented; cleanup is opt-in.

## Deliberately deferred experiments

The approved CodeQL deliverable is an inactive proposal and benchmark template.
Activation still requires a coordinated setup switch and coverage comparison;
no CodeQL speedup is claimed. No larger runner or security/test exclusion was
introduced. Deeper server-crate splits remain a subsequent, timing-guided
change: this pass removes proven recompilation and scheduling overhead without
attempting a language rewrite.

## Everyday-loop pass — October 3

Goal: an everyday change checked in about a minute. All numbers below come from
one shared 8-core host while up to ten other agents were building (every Cargo
run went through a 3-job throttle). Absolute times are therefore inflated and
noisy; each comparison is an **A/B of the same command, back to back, in the
same target slot**, and only relative differences are claimed. Two identical
runs of the same five-crate test set differed by up to ±20% per crate (otto-git
121 s vs 152 s of summed test time), so smaller deltas are noise.

### Where the time goes (`cargo nextest run -p otto-server --timings`)

Workspace crates only (dependencies were already built), 410 s total:

| Unit | Time | Note |
| --- | ---: | --- |
| otto-server lib | 314.5 s | 286.9 s frontend (single-threaded), 27.6 s codegen; starts at 64 s, ends at 378 s |
| otto-server lib unit tests | 149.1 s | runs in parallel with the lib |
| 19 otto-server integration binaries | 94.5 s CPU / 31.6 s wall | each links the whole workspace |
| otto-dbviewer | 32.8 s | next-largest crate, on the critical path before the server |
| otto-git, otto-state, otto-core | 15.6 / 14.6 / 11.1 s | |

otto-server is 162k lines (37% of all crate sources) and 77% of the build:
any edit in it, or in anything it depends on, pays its ~5 minute (loaded)
frontend serially. Nothing else comes close.

### Changes and measured effect

| Change | Before | After |
| --- | ---: | ---: |
| otto-server integration tests linked as one binary (`tests/it.rs`) | 19 units, 94.5 s CPU, 31.6 s wall after the lib, ~1.4 GB of binaries | 2 units, 16.9 s CPU, 12.3 s wall, 415 MB |
| ClickHouse: internal DNS cache off (server boot / SIGTERM exit) | 5.5 s / 4.9 s | 0.6 s / 0.2 s |
| ClickHouse: no idle keep-alive connection in the client pool (SIGTERM exit) | 10.1 s | 0.1 s |
| otto-usage e2e (5 server-backed tests, summed) | 32.3 s | 8.9 s |
| otto-server k8s_monitor_clickhouse (3 tests) | 9.4 / 8.1 / 7.2 s | 3.1 / 1.9 / 1.5 s |
| otto-git ARG_MAX stage/unstage/discard (1,800 long paths, not 14,000 short) | 31 s (deps report) | 1.6 s |
| otto-vault revision index at 5k revisions (50k variant now `#[ignore]`d, run by a CI step) | 39 s (deps report) | 4.0 s |
| Dependencies at `opt-level = 1` in dev/test: otto-state tests (summed test time) | 309 s / 272 s (two runs) | 86 s |
| … otto-sessions tests (summed) | 82.5 s / 82.0 s | 29.8 s |
| … slowest otto-sessions test (`lifecycle_mutations_wait_for_in_flight_restart`) | 5.5 s (13 s in the deps report) | 2.0 s |

The dependency `opt-level` change keeps workspace crates at `opt-level = 0`, so
an edit compiles as before; it only changes how fast the dependencies run. Every
test that opens a migrated SQLite database (146 migrations) spends its time in
SQLite (C, built by `libsqlite3-sys` at the package's opt-level), sqlx, tokio
and serde. One-time cost: rebuilding the 284 dependency units of otto-state and
otto-sessions took 358 s of CPU (2m27 wall at 3 jobs); the whole otto-server
tree (615 dependency units) took 2,186 s of CPU (about 12 minutes wall at 3
jobs on the loaded host). They are cached afterwards locally and by CI's
rust-cache, whose key changes once with this profile. On the new profile the
complete otto-server suite (1,311 tests: lib, `it`, `snips`) passed in 65 s of
nextest wall time, and the server lib still compiled at opt-level 0 (216 s).
`debug = "line-tables-only"` / no dependency debuginfo would likely shorten
local links further, but this host's throttle forces `debug = 0`, so it was
not measured and not changed.

The ClickHouse stalls were found by timing the real server: an unresolvable
`<host>.local` costs a 5 s DNS timeout at boot and again on shutdown, and SIGTERM
waits for every open client connection ("Waiting for 1 outstanding
connections"). They also slowed the daemon's own usage-engine start and restart.
The runtime-lag gates now take the best of up to three attempts and run in a
max-one `timing` nextest group, so CPU contention cannot fail them on its own.

### Everyday command

`scripts/check.sh` (documented in `AGENTS.md`) runs CI's gates only for the
changed crates (fmt + clippy) and their reverse dependents (nextest + doc-tests),
plus the UI gates when `ui/` changed. A root build-file change selects the whole
workspace. For an otto-server edit the loop is dominated by the server's own
frontend; for a leaf crate (otto-vault, otto-git, …) it is that crate plus
otto-server and ottod, i.e. still the server.

### Not changed, and why

- **Linker.** macOS uses Apple's ld-prime (ld-27037), already competitive with
  lld; lld is not installed and would add a toolchain prerequisite. Linux CI
  already links with the bundled `rust-lld` (default on x86_64 Linux since Rust
  1.90).
- **`split-debuginfo`** is already `unpacked` by default for macOS dev builds.
- **CI sharding.** The CI Rust job is compile-bound (clippy 1m22, nextest with
  compilation 6m08); two nextest partitions would each repeat the compilation.
  Not a win without a shared build artifact, so it was not added.

### Plan for the big ones (not done in this pass)

1. **Split otto-server (the only lever that reaches "about a minute").** Its
   frontend is single-threaded and serial on every edit. Move self-contained
   domains into their own crates behind a narrow `AppState`-facing trait:
   `workflow_engine` + `run_*` (~12k lines), `mcp_outward` + `mcp_*` (~17k),
   design/vault/canvas assistants (`design_*`, `vault_docs_agent`,
   `canvas_assist`, `mockup_assist`, ~12k), browser + API-client routes (~9k),
   goal loops / swarm runtime (~9k). Keep `state.rs`, auth, policy, error and
   `api_helpers` in a small `otto-server-core` that the domain crates depend on;
   otto-server becomes the router that wires them. Expected effect: an edit in
   one domain recompiles that crate plus the thin router, and the domains'
   frontends run in parallel on a cold build. Start with `workflow_engine`
   (largest, fewest inbound references), measure with `--timings`, continue only
   if the edit-loop time drops as predicted.
2. **Feature-gating heavy dependencies** (rdkafka/CMake, boa_engine, tree-sitter
   grammars) only shortens cold builds; dependencies are cached in every warm
   loop and in CI (rust-cache). Worth it for fresh worktrees, not for the
   everyday loop — do it after the split.
3. **Debuginfo** (`line-tables-only` for workspace crates, none for
   dependencies) for local dev builds: measure link times on an unthrottled
   host before adopting.

## Runtime guard: usage tailer and embedded ClickHouse idle cost

The usage section's idle and memory cost is guarded by unit tests, plus a
scripted probe for the ClickHouse child that tests can't run:

- `cargo test -p ottod --bin ottod unchanged_tree_pass` — on a 2k-Claude +
  1.4k-Codex synthetic tree with 10 transcripts ending in a partial line, an
  unchanged-tree pass (full listing or event-driven) issues **zero** file reads
  and **zero** `sessions` attribution queries; one appended usage line costs
  exactly one read and one query.
- `cargo test -p otto-transcript --lib seen_keys_100k` — the Claude dedup set
  at its 100k-key cap stays under 4 MiB (it was ~20 MB as two `String`
  copies per key).
- `cargo test -p ottod --bin ottod fsevents_watcher` — the FSEvents watcher
  reports a new transcript (the tailer's 20 s polling is now a fallback).
- ClickHouse idle probe (scratch data dir + spare port, never the live one):
  start `clickhouse server --config-file=<generated config.xml>` with
  `CLICKHOUSE_WATCHDOG_ENABLE=0`, wait 20 s, then
  `ps -M <pid> | wc -l` and `top -l 4 -s 5 -pid <pid> -stats pid,cpu,th,mem`.

Measured 2026-10-03 (ClickHouse 26.6.1, 244k-row `usage_events`, machine
shared with other builds): the perf-wave config (memory worker 10 s, capped
IO/parts/table-loader pools, merge selector 30 s → 5 min) idles at
**0.4 % CPU, 71 threads, 135 MB**; the previous config at **0.5–0.6 %, 71
threads, 134 MB** (the capped pools are created lazily, so thread count is
unchanged at idle; the saving is wake-ups).
