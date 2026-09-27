# CI test scheduling and cache improvements

The CI-only changes from `feat/session-rooms-20260927` were applied to the
`perf/db-editor-git-diff-lag` worktree. Rooms, embedded UI ownership, deployment
cache policy and the inactive CodeQL proposal are separate changes.

## Behavior

- CI installs checksum-verified Nextest 0.9.146 and runs the full workspace
  with four test processes. `.config/nextest.toml` serializes real ClickHouse
  integrations and limits PTY/session/sandbox process concurrency to two.
- `cargo test --workspace --doc` remains a separate gate because Nextest does
  not run doc-tests. Ignored tests stay ignored; no retries or exclusions were
  added. Clippy and existing UI/security gates remain in place.
- Five server fixtures unrelated to metrics disable incidental ClickHouse
  startup. Actual metrics integration tests still start ClickHouse.
- CI caches the pinned cargo-audit 0.22.2 executable, while every scan fetches
  current advisories. Saving the executable precedes the advisory scan.
- Runner cleanup is conditional on a conservative 40 GiB free-space floor
  before Cargo cache restore. Disk usage is reported after restore/build.
  The floor is not a measured minimum for every runner image.
- Deployment verification discovers all `packaging/tests/test_*.py` tests.

## Local usage and measurement

With cargo-nextest 0.9.146 installed:

```sh
cargo nextest run --workspace
cargo test --workspace --doc
```

The default profile uses eight test processes; CI uses four. Standard
`cargo test --workspace` remains available without Nextest.

The September 27, 2026 comparison was measured in the source feature worktree,
including its rooms tests, on an Apple M5 Pro (18 logical CPUs, 48 GiB RAM,
Rust 1.96.0). Both runners used exactly the same 93 warmed binaries and test
inventory: 3,620 passed and 66 ignored/skipped. Native Cargo command wall time
was 692.37s; Nextest took 147.96s including discovery (121.287s execution).
Warmup compilation of 139s was excluded. The separate doc-test gate passed
in 16.52s. Another checkout compiled during part of the native run.

These are source-worktree results, not a full-suite result for this worktree
or a GitHub Linux performance guarantee. CI uses fewer test processes, and
actual Actions/cache/disk timing remains to be measured after publication.
Local transfer validation covers workflow syntax, configuration parsing,
the fixture-only diff and this worktree's deployment verification tests.
