# Otto

**An agentic development environment.** Otto is a macOS desktop app that runs
coding-agent CLIs (Claude Code, Codex, and others) as first-class, openable
sessions — and wires them into the rest of your workflow: git & pull requests,
multi-agent code review, Jira/Confluence product workflows, SSH/database
connections, an HTTP API client, real token-usage tracking, and Slack/Telegram
bridges so an agent can work a ticket from a chat thread.

> Status: early / actively evolving (v0.1). macOS-only for now. Expect rough edges.

> ⚠️ **Vibe coded.** This entire repository was built through conversational AI
> ("vibe coding") — the code, tests, and documentation were largely AI-generated
> and have **not** been independently audited or formally reviewed. Treat it as
> experimental. **Verification is your responsibility**: validate correctness,
> security, and dependency licensing before relying on any of it. Provided
> **as-is, with no warranty** — see [LICENSE](./LICENSE).

---

## Features

- **Agent sessions** — run `claude`, `codex`, `agy` (and a plain shell) in real
  PTY-backed terminals you can watch, split, and type into. Sessions survive
  restarts (resumable), idle-suspend to save memory, and auto-trust their
  workspace folder so they never stall on a permission prompt.
- **Git & Pull Requests** — browse repos, stage/commit/discard, view diffs,
  resolve merge conflicts, and **create PRs** with an **agent-drafted title +
  description** (it reads your branch diff), pushing the branch automatically.
- **AI code review** — fan out several review agents (one per provider/lens)
  over a PR *or* your local working tree. Each runs as an openable session with
  live progress, per-agent findings, retry, and a configurable grace period.
- **Product (Jira / Confluence)** — import a Jira issue or Confluence page
  (search by project or space — no key prefix needed) and run a product-owner
  workflow over it: multi-agent, multi-provider **analysis** with a summarizer
  and **open questions** you can post back as comments; a suggested **rewrite**;
  **test-case** generation with approval published to a linked Confluence page;
  **discovery** drafts (start blank, drop in ideas or call transcripts, then
  publish as an RFC or a Jira story); plus versioning, sectioned history, tags,
  a **Plan/Tasks** breakdown, and a recurring-patterns **learnings** base. A
  background watcher polls for new comments/updates, and you can inject a story's
  full refined context into any running agent.
- **Channels** — bridge a Slack or Telegram thread to an agent session: messages
  (and file attachments) are relayed in, the agent's reply (and any file) is
  relayed back. One agent per ticket, auto-archived when idle.
- **Broadcast** — send one literal message to many live agent sessions at once
  (no AI in the loop) — e.g. tell every working agent to "wrap up and commit."
- **Connections** — open SSH / MySQL / Redis / MongoDB / ClickHouse sessions
  side-by-side with agents.
- **Database Explorer** — a TablePlus-class browser for MySQL, Redis, MongoDB,
  and ClickHouse over plaintext, TLS/SSL, or SSH tunnels: a lazy schema tree,
  per-engine autocomplete, multiple query tabs, a virtualized results grid
  (client-side filter/sort + approval-gated inline editing), a Navicat-style
  visual JOIN builder, Superset-style dashboards/widgets for ClickHouse, and
  "examine this schema with an agent". Read queries get an automatic row `LIMIT`
  so a huge table is never fully scanned, and any running query is cancelable.
- **Self-improvement** — an optional, gated engine that reflects on recent
  sessions and proposes edits to the workspace's skills/memory (tiered autonomy:
  safe edits auto-apply, risky ones queue for approval). Can run on multiple
  providers for varied suggestions.
- **Skills library** — a bundled, versioned skill library (`otto-skills`) you
  can browse and install/update from Settings; skills drive review lenses,
  product analysis, and insights, and the self-improvement engine refines them
  from your sessions.
- **Insights** — scheduled, multi-provider "catch-up" reports that turn recent
  activity into action-first summaries, generated on demand and cached.
- **Usage & cost** — an embedded ClickHouse engine records real per-turn token
  usage and cost by tailing Claude and Codex transcripts (no manual
  instrumentation), with per-provider / day / session rollups and configurable
  retention.
- **API client** — a built-in REST workbench (collections, environments, history).

## Architecture

Otto is a Tauri 2 desktop app with a Rust backend daemon and a Svelte 5 frontend.

```
┌──────────────────────────────────────────────┐
│  Otto.app  (Tauri / otto-desktop)             │
│  ┌───────────────┐      ┌──────────────────┐  │
│  │   Svelte UI   │◀────▶│  ottod (sidecar) │  │
│  │ (ui/, webview)│ HTTP │  127.0.0.1:7700  │  │
│  └───────────────┘  +WS └──────────────────┘  │
└──────────────────────────────────────────────┘
                              │ spawns
                  claude / codex / shell (PTY),  git, providers
```

- **`ottod`** — the daemon: an Axum HTTP+WebSocket server on `127.0.0.1:7700`
  (loopback only by default). Owns sessions, PTYs, git, reviews, channels, and
  state (SQLite). Runs under `launchd` when installed; the desktop app bundles
  it as a sidecar.
- **`ui/`** — the Svelte 5 + Vite + TypeScript frontend, embedded into the app
  at build time. The Rust API (`docs/contracts/`) is the source of truth; the
  TS types mirror it.
- **Rust crates** (`crates/`): `otto-core` (domain/API), `otto-state` (SQLite),
  `otto-sessions` (session manager + PTY + trust + prompt-guard), `otto-pty`,
  `otto-orchestrator`, `otto-git`, `otto-issues` (Jira/Confluence),
  `otto-channels`, `otto-connections`, `otto-dbviewer` (Database Explorer),
  `otto-product` (Jira/Confluence story workflows), `otto-improve`
  (self-improvement), `otto-usage` (ClickHouse usage/metrics), `otto-skills`
  (bundled skill library), `otto-context`, `otto-rbac`, `otto-keychain` (macOS
  Keychain secret storage), `otto-server` (routes), `ottod` (binary).

## Prerequisites

- **macOS** (Apple Silicon or Intel). Otto uses launchd, the macOS Keychain, and
  codesigning, so it is macOS-only today.
- **Rust** (stable) — <https://rustup.rs>
- **Node.js 20+** and npm
- **Tauri CLI** — `cargo install tauri-cli` (or `npm i -g @tauri-apps/cli`)
- **git**
- At least one **agent CLI** on your `PATH`, e.g.:
  - [Claude Code](https://docs.anthropic.com/claude/docs/claude-code) (`claude`)
  - Codex (`codex`)

  Otto detects which are installed and lets you pick a default; you don't need
  all of them.

## Build from source

```bash
git clone <your-fork-url> otto && cd otto

# 1. Frontend → ui/dist
cd ui && npm install && npm run build && cd ..

# 2. Daemon (release)
cargo build --release -p ottod

# 3. Bundle the daemon as the app's sidecar (Tauri externalBin)
cp target/release/ottod \
   apps/desktop/src-tauri/binaries/ottod-$(rustc -vV | sed -n 's/host: //p')

# 4. Build the desktop app
cd apps/desktop/src-tauri && tauri build --bundles app
#   → target/release/bundle/macos/Otto.app
```

### Code signing (local / self-signed)

macOS requires the app and its sidecar to be signed. For local use you can use a
self-signed identity:

```bash
packaging/make-cert.sh          # creates a self-signed "Otto Dev Signing" identity (once)
packaging/sign.sh /path/to/Otto.app /path/to/ottod
```

`packaging/dmg.sh` builds a distributable `.dmg`, and
`packaging/com.otto.daemon.plist` is the launchd template for running `ottod`
in the background.

## Development

Run the daemon and the Vite dev server separately for hot-reload:

```bash
# Terminal 1 — daemon on http://127.0.0.1:7700
cargo run -p ottod

# Terminal 2 — UI on http://localhost:5173 (talks to the daemon)
cd ui && npm run dev
```

The frontend's API base defaults to `http://127.0.0.1:7700`; override it in the
browser console with `localStorage.otto_base = 'http://127.0.0.1:7700'` if needed.

Useful checks:

```bash
cargo build && cargo test          # Rust
cd ui && npm run check              # svelte-check + tsc
```

## Configuration & secrets

- **First run** prompts you to set a root password (local accounts), then add
  workspaces (folders), agent defaults, and git/issue/channel accounts in
  **Settings**.
- **Secrets never live in the repo or in plain files.** Tokens (git, Slack,
  Telegram, Jira, connection passwords) are stored in the **macOS Keychain** via
  `otto-keychain`; the daemon DB only stores opaque key references.
- The daemon listens on **loopback only** unless you explicitly enable a network
  listener in settings.

## Project layout

```
crates/         Rust workspace (daemon + libraries)
apps/desktop/   Tauri desktop shell (otto-desktop)
ui/             Svelte 5 + Vite frontend
packaging/      sign.sh, dmg.sh, make-cert.sh, launchd plist
docs/contracts/ API + WebSocket contracts (source of truth for the TS types)
```

## Contributing

Issues and PRs welcome. Please run `cargo test` and `cd ui && npm run check`
before opening a PR. The Rust API in `docs/contracts/` is authoritative — keep
the TypeScript types in `ui/src/lib/api/types.ts` in lockstep.

## License

Released under the [MIT License](./LICENSE).
