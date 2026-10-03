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

## Perf gates on WebKit (round 3, r3-10-02)

The Playwright perf specs (`ui/e2e/desktop-*perf*.spec.ts`) used to run only
in the `desktop-browser` project (Chrome). The app renders in WKWebView, where
style, layout and paint dominate, so the gates could not see its regressions.

- **Project.** `desktop-webkit` (`ui/playwright.config.ts`) runs every perf
  spec on Playwright WebKit (Desktop Safari, 1280×800, service worker blocked
  so `page.route` fixtures apply). Specs gate on `isDesktopProject()`;
  WebKit-calibrated timing gates (DB results/editor, terminal flood) on
  `isWebkitProject()`; Chromium-only probes (long tasks, CDP heap) skip
  themselves on WebKit. `desktop-transport-perf` stays in `desktop-browser`:
  it launches both engines itself.
- **After paint.** Timings end after the frame's paint, not at a microtask:
  `watchKeyFrameCosts` (keystroke script + the next frame's rendering work)
  and `scrollFrameWork` (a scroll step to its painted frame), both in
  `ui/e2e/perf.ts`. Neither counts the idle wait for vsync. Budgets are
  measured on WebKit (M-series Mac): URL keystroke + frame p95 2–4 ms (budget
  12), 200 KB JSON body keystroke + frame p95 6–7 ms (24), 100k-row grid scroll
  step to painted frame p95 18–19 ms (40).
- **Budget scale.** `OTTO_PERF_BUDGET_SCALE` multiplies timing budgets
  (`budgetMs()`); DOM and request counts are never scaled.
- **CI.** The `perf-gates` job runs a small subset on the Ubuntu runner:
  `desktop-git-sidebar-perf`, `desktop-docs-orch-perf`, `desktop-infra-perf`
  and `desktop-db-results-perf`, with `OTTO_PERF_BUDGET_SCALE=3`. It is
  advisory (`continue-on-error`) until it has a green history: it is the first
  job to run the daemon and Playwright WebKit on Linux. Promote it by removing
  `continue-on-error`.
- **Run locally** (isolated daemon; pick a free slot/port):

  ```sh
  cargo build -p ottod
  cd ui && OTTO_E2E_BIN=../target/debug/ottod OTTO_E2E_SLOT=7 OTTO_E2E_PORT=7897 \
    OTTO_E2E_PW_PORT=5197 npx playwright test --project=desktop-webkit --workers=1
  ```

- **Known gaps (September 28, 2026).** `desktop-terminal-flood-perf` fails in
  both engines at this commit (`.xterm-rows` never appears in its harness page;
  it failed the same way before the move to `desktop-webkit`).
  `desktop-conversation-perf` is skipped on WebKit: its seeded session never
  reaches the session list there.

The Rust runtime-lag gate (`crates/otto-server/tests/runtime_lag.rs`) also has
a burst case again: 200 × 64 KB events queued at once and drained by a socket
loop modelled on `ws_events`. It measured 208 ms of blocked worker before the
events socket started pacing itself (`ws_fanout::Pacer`), 2–4 ms after, under
the unchanged 20 ms budget. Every case in the file takes the best of up to three
attempts (stopping at the first under budget): the lag is one worst tick gap, so
a single OS preemption on a loaded box failed the paced case at 57 ms, while a
real regression blocks every attempt. Nextest runs the file in its own
`timing` test group (max one at a time).

## Parallel-load CPU/RAM test

`ui/scripts/loadtest/` holds the round-3 load driver (isolated daemon, agent
emulator, Chromium-driven UI). See its README.
