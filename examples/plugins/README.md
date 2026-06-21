# Example Otto plugins

Reference **runtime, out-of-process** plugins. Each is a self-contained sidecar
Otto installs at runtime (no app rebuild) — see `docs/plugins/AUTHORING.md` and
`docs/superpowers/specs/2026-06-21-runtime-plugins-design.md`.

| Plugin | Language | What it shows |
|---|---|---|
| [`team-performance`](team-performance) | Node (zero deps) | Jira stories-only by assignee vs. git delivery (done = last merge to `develop`), AI-era estimation-improvement target + met/missed + concurrency flags. Pure Node builtins — runs immediately. |
| [`dora-metrics`](dora-metrics) | Rust | DORA metrics from git tags (`*deployed*`) + branch-merge classification (hotfix/release/feature → develop), with agent bottleneck analysis. Compiles on first enable via `cargo run --release`. |

## Install

In the app: **Settings → Plugins → Install** with a local path, e.g.
`~/otto_os/examples/plugins/team-performance` (Otto copies it into `~/otto-plugins`),
then **Enable**. Or `POST /api/v1/plugin-admin/install {"source":"<path-or-git-url>"}`
then `POST /api/v1/plugin-admin/team-performance/enable`.

As root you'll see the section immediately; grant other users in **Settings → Users**.

Both consume Otto's scoped host API for repos / Jira credentials / agent runs, and
serve an iframe UI from their `ui/` dir. The `dora-metrics` Rust sidecar's first
enable compiles the binary (needs `cargo`); subsequent spawns are instant.
