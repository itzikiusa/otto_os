# AGENTS.md

Guidance for AI coding agents (and humans) working in this repository. Read this
before making changes. It is the single source of truth for build/test commands,
the crate/module map, and the rules for not damaging user work.

> Otto is a macOS desktop app (Tauri 2 + Rust daemon + Svelte 5 UI) that runs
> coding-agent CLIs (Claude Code, Codex, …) as first-class sessions and wires
> them into git/PRs, code review, Jira/Confluence, SSH/DB connections, an HTTP
> API client, usage tracking, and Slack/Telegram bridges. See `README.md` for the
> full feature tour.

## Architecture

```
Otto.app (Tauri / otto-desktop)
  ├─ Svelte UI (ui/, webview) ──HTTP+WS──▶ ottod (sidecar, 127.0.0.1:7700)
  └─ ottod spawns claude / codex / shell (PTY), git, providers
```

- **`ottod`** — the daemon: an Axum HTTP+WebSocket server on `127.0.0.1:7700`
  (loopback only by default). Owns sessions, PTYs, git, reviews, channels, and
  state (SQLite). Under `launchd` when installed; bundled as a Tauri sidecar.
- **`ui/`** — Svelte 5 + Vite + TypeScript frontend, embedded into the app at
  build time.
- **`docs/contracts/` is authoritative.** The Rust API (`api.md`, `ws.md`) is the
  source of truth; the TypeScript types in `ui/src/lib/api/types.ts` mirror it.
  Change the contract and the types together.
- **Per-feature guides live in [`docs/features/`](./docs/features/README.md)** —
  setup, full walkthrough, the relevant API/WS surface, capabilities & limits,
  and troubleshooting for every feature (one doc per feature). These are
  code-grounded explainers; `docs/contracts/` remains the API source of truth.

### Rust workspace (`crates/`)

| Crate | Responsibility |
|-------|----------------|
| `otto-core` | Domain types + the API surface |
| `otto-state` | SQLite persistence + migrations (`crates/otto-state/migrations/`) |
| `otto-rbac` | Auth, roles, API tokens |
| `otto-keychain` | macOS Keychain secret storage |
| `otto-netguard` | Outbound SSRF guard (blocks loopback/private/metadata) |
| `otto-pty` | PTY plumbing |
| `otto-sessions` | Session manager + PTY + trust + prompt-guard |
| `otto-connections` | SSH / MySQL / Redis / MongoDB / ClickHouse sessions |
| `otto-ssh` | Shared SSH-tunnel helper (`-L`/SOCKS5 `-D`, SFTP, Kafka-aware proxy) |
| `otto-dbviewer` | Database Explorer engine |
| `otto-brokers` | Message Brokers (Kafka viewer) |
| `otto-orchestrator` | Claude-PTY agent runner + ⌘K plan parsing (summaries, PR/commit drafts) |
| `otto-git` | Repos, diffs, commits, PRs |
| `otto-issues` | Jira / Confluence integration |
| `otto-channels` | Slack / Telegram bridges |
| `otto-improve` | Self-improvement engine |
| `otto-context` | Context assembly |
| `otto-memory` | Vault knowledge store (keyword + vector hybrid recall) |
| `otto-usage` | Embedded ClickHouse usage/metrics |
| `otto-skills` | Bundled, versioned skill library |
| `otto-product` | Jira/Confluence product workflows |
| `otto-swarm` | Agent Swarm (role agents, org tree, coordinator) |
| `otto-server` | Axum routes wiring the crates together; also hosts the multi-agent code-review engine, swarm runtime, workflow engine & plugin supervisor |
| `ottod` | The daemon binary |

> The Tauri desktop shell lives in `apps/desktop/src-tauri` and is a **separate,
> standalone Cargo workspace** (note the `[workspace]` in its `Cargo.toml`). It is
> macOS-only and is **not** part of the root workspace — `cargo build --workspace`
> from the repo root does not build it.

### UI module areas (`ui/src/modules/`)

`agents`, `api` (REST client), `brokers` (Kafka viewer), `connections`,
`database` (Database Explorer), `git`, `help`, `insights`, `panels`, `plugins`,
`product`, `settings`, `share` (remote/mobile), `skills-eval`, `swarm`, `usage`,
`vault` (knowledge store), `workflows`. Shared code lives in `ui/src/lib/`
(`api/`, `components/`, `stores/`); the app shell is `ui/src/shell/` +
`ui/src/App.svelte`.

## Build & test commands

The repo has **no Makefile**. Use these directly:

```bash
# Rust (run from repo root)
cargo build --workspace          # build the daemon crates + ottod
cargo test --workspace           # run all Rust tests
cargo fmt --all --check          # formatting (CI: advisory for now — the tree predates rustfmt-in-CI and isn't fully formatted yet; a one-time repo-wide `cargo fmt --all` should land as its own commit before this is promoted to blocking)
cargo clippy --workspace --all-targets -- -D warnings   # lints (CI-enforced)

# UI (run from ui/)
cd ui
npm ci          # install (uses package-lock.json); `npm install` when adding deps
npm run check   # svelte-check + tsc (app + node + e2e tsconfigs) — the type-check gate
npm run build   # production build → ui/dist
npm run dev     # Vite dev server on :5173 (talks to a running ottod)
npm run test:e2e # Playwright mobile/tablet E2E (spins an ISOLATED throwaway daemon
                 # on a temp data dir + port — never touches real sessions/DBs —
                 # serves the live UI via Vite, drives every page across iPhone/iPad
                 # portrait+landscape: fits/scrolls, collapsible sections, real flows
                 # (query→results, commit→diff, terminal I/O), light/dark + RTL).
                 # Slot-isolated via OTTO_E2E_SLOT/OTTO_E2E_PORT/OTTO_E2E_PW_PORT for
                 # parallel per-page runs. Specs: ui/e2e/*.spec.ts.
```

Run the daemon and the UI separately for hot-reload during development:

```bash
cargo run -p ottod          # daemon on http://127.0.0.1:7700
cd ui && npm run dev        # UI on http://localhost:5173
```

CI runs the Rust and UI gates above on every push/PR
(`.github/workflows/ci.yml`). The full desktop-app packaging flow (sidecar copy,
Tauri build, codesigning, DMG) is documented in `docs/RELEASE.md` and is
macOS-only.

## Conventions

- **Match the surrounding code.** Comment density, naming, and idiom in this repo
  are fairly dense and intentional — mirror the file you're editing.
- **Contracts first.** When you touch an endpoint or WS event, update
  `docs/contracts/*.md` and `ui/src/lib/api/types.ts` in lockstep.
- **Migrations are append-only.** Add a new numbered file under
  `crates/otto-state/migrations/`; never edit or renumber an existing migration.
- **Secrets never live in the repo.** Tokens/passwords go through the macOS
  Keychain (`otto-keychain`); the DB stores only opaque key references. Never
  commit `.env`, `*.pem`, `*.key`, `*.p12`, or local DBs (see `.gitignore`).

## Do NOT damage user work

This app manages a user's real sessions, repositories, databases, and local
state. When acting in this repo or driving the running daemon:

- **Never delete or overwrite user data.** Do not drop tables, wipe the SQLite
  state DB, delete a user's local databases, or remove workspace folders.
- **Never run destructive git without explicit, current approval.** No
  `git push --force`, `git reset --hard`, history rewrites, or branch deletion
  unless the user asks for that exact operation in this conversation. Default to
  PRs over the `main` branch (it is protected).
- **Ask before irreversible or outward-facing actions** — anything that publishes
  (opening a PR, posting a Jira/Confluence comment, sending to Slack/Telegram),
  deletes, or touches a remote/production system. Approval in one context does
  not carry to the next.
- **Inspect before you overwrite.** If a file's contents contradict how it was
  described, or you didn't create it, surface that instead of proceeding.
- **Report outcomes faithfully.** If tests fail, say so with the output; if a
  step was skipped, say that. Don't claim work is done until it's verified.
- **Don't weaken security defaults.** The daemon listens on loopback only unless
  the user explicitly enables a network listener; don't change that casually.
