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
