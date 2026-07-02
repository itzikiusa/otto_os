# Example Otto plugins

Reference **runtime, out-of-process** plugins. Each is a self-contained sidecar
Otto installs at runtime (no app rebuild) — see `docs/plugins/AUTHORING.md` and
`docs/superpowers/specs/2026-06-21-runtime-plugins-design.md`.

| Plugin | Language | What it shows |
|---|---|---|
| [`team-performance`](team-performance) | Node (zero deps) | Scans **every Jira issue of a project** (paginated + incremental, async job with progress), splits each task's changelog time into **design vs implementation** business days (configurable status→phase map), correlates git delivery (one-pass repo index), builds **statistical baselines** per (type, points) bucket — "how long it should have taken" — with per-task fast/on-track/slow verdicts and evidence drill-down, **predicted timelines for open tasks**, an estimation-guide table, and **per-dev goals** tracked across scans. Pure Node builtins — runs immediately. |
| [`dora-metrics`](dora-metrics) | Rust | All **four DORA keys** from git signals (deploy = `*deploy*` tag, configurable; hotfix/release/feature merges): deployment frequency, median+p90 lead time, change-failure rate, and failed-deployment recovery time — with Elite/High/Medium/Low **tier benchmarks**, weekly **trend charts**, deltas vs the previous window, a deterministic **suggestions engine**, and AI bottleneck analysis. Compiles on first enable via `cargo run --release`. |

## Install

In the app: **Settings → Plugins → Install** with a local path, e.g.
`~/otto_os/examples/plugins/team-performance` (Otto copies it into `~/otto-plugins`),
then **Enable**. Or `POST /api/v1/plugin-admin/install {"source":"<path-or-git-url>"}`
then `POST /api/v1/plugin-admin/team-performance/enable`.

As root you'll see the section immediately; grant other users in **Settings → Users**.

Both consume Otto's scoped host API for repos / Jira credentials / agent runs, and
serve an iframe UI from their `ui/` dir (single-file, inline-SVG dashboards that
follow Otto's theme). The `dora-metrics` Rust sidecar's first enable compiles the
binary (needs `cargo`); subsequent spawns are instant.

## Tests

Each example ships its own test suite (they are reference implementations —
tested like real software, but as **local gates**: the repo's CI workspace
gates intentionally exclude them):

```bash
cd examples/plugins/team-performance && node --test         # unit + sidecar E2E
                                                            # (mock Jira + host API + scripted git repo)
cd examples/plugins/dora-metrics && cargo test              # engine units + fixture-repo integration
```

Otto-level Playwright coverage lives in `ui/e2e/desktop-plugins.spec.ts`
(installs both plugins into the isolated test daemon and drives the dashboards
through the real iframe).
