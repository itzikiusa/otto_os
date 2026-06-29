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

- **Run with Otto** — the flagship **one-button** flow. Point Otto at a single
  source item — a **Jira** story, a **Confluence** page, a **GitHub** issue or PR,
  a **Slack/Telegram** thread, a **Product** task, a **review finding**, a
  **failing test**, or a **scheduled-task report** — and it runs a fixed pipeline
  end to end: *normalize the source → build a Context Packet → cut an isolated
  branch/worktree → do the work (a single agent **or** a full Goal Loop) →
  assemble a Proof Pack → run AI review → **pause for human approval** → draft the
  PR*. It chains the subsystems below behind one entity (`OttoRun`) and one
  trigger so it feels like **one button, not eight modules**, and projects into
  Mission Control. Launch it from the UI, a Slack/Telegram `/run <ref>`, a REST
  call, or a key-guarded webhook (which can POST the result back to a
  `callback_url` at the gate + each terminal state). It never opens a PR without
  human approval **and** a passing/waived Proof Pack.
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
  Findings become **tracked items** — a review-findings workflow with statuses
  and fix / verify / open-Jira / false-positive / regression-test actions — and
  can be **ingested into a Proof Pack** or saved to the Vault.
- **Proof Packs** — an evidence layer so "done" means *proven*. Each unit of work
  collects **artifacts** (test output, diffs, PR links, screenshots, logs) into a
  pack, and pure rules derive a **status, risk and badges** from them. Artifacts
  are redacted and size-capped, and optional **gates** can require a PR, a Goal
  Loop, or passing tests before work is allowed to close.
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
- **Canvas** — think visually. Each canvas scene is **file-backed** in one of two
  modes: **Excalidraw** (`canvas.json`, freeform shapes & arrows) or **Mermaid**
  (`canvas.mermaid`, diagram-as-code). An agent edits the underlying file while
  you converse in an embedded terminal, so a diagram becomes something an agent
  builds *with* you.
- **Channels** — bridge a Slack or Telegram thread to an agent session: messages
  (and file attachments) are relayed in, the agent's reply (and any file) is
  relayed back. One agent per ticket, auto-archived when idle.
- **Broadcast** — send one literal message to many live agent sessions at once
  (no AI in the loop) — e.g. tell every working agent to "wrap up and commit."
- **Agent Swarm** — assemble a *swarm*: a team of role-specialized agents with an
  org hierarchy (CEO → CTO → VP → Team Lead → Devs) that autonomously works
  **projects** broken into **tasks**. A built-in **recruiter** drafts each agent
  (role, "soul"/persona, skills, schedule); a per-swarm **Coordinator** schedules
  ready work onto agents within a **parallel-session cap**, delegates leader→reports,
  and routes hand-offs/reviews. Agents coordinate on a **shared board** you watch
  live; every agent runs as a normal openable session. Watch progress in an **org
  tree**, a **run-graph (DAG)**, and per-project **Kanban + filterable runs list**;
  set agents to **scheduled runs** (e.g. a daily trend researcher); **pause / abort
  all / resume** any time. Five preset swarms ship in the box. It's far leaner on
  tokens than API-driven equivalents: sessions are **persistent and resumed** (no
  whole-history re-feed each turn) and agents' outputs are read from transcripts/
  files (zero model tokens).
- **Goal Loops** — give a **goal**, machine-checkable acceptance criteria, and a
  **budget** (max iterations + active time). A team of agents iterates
  **Plan → Execute → Evaluate → Digest** on an isolated `goal-loop/<id>` branch,
  repeating until the criteria pass or the budget runs out — with live
  phase/iteration monitoring and openable executor sessions.
- **Mission Control** — one **unified work graph** over everything your agents are
  doing across all eight kinds (sessions, swarms, goal loops, reviews, product
  stories, workflows, PRs, external triggers). A projector builds it from the
  daemon's event bus, so every workstream shows up as a node with a live status —
  click one to jump straight to the work behind it.
- **Connections** — open SSH / MySQL / Redis / MongoDB / ClickHouse sessions
  side-by-side with agents.
- **Database Explorer** — a TablePlus-class browser for MySQL, Redis, MongoDB,
  and ClickHouse over plaintext, TLS/SSL, or SSH tunnels: a lazy schema tree,
  per-engine autocomplete, multiple query tabs, a virtualized results grid
  (client-side filter/sort + approval-gated inline editing), a Navicat-style
  visual JOIN builder, Superset-style dashboards/widgets for ClickHouse, and
  "examine this schema with an agent". Read queries get an automatic row `LIMIT`
  so a huge table is never fully scanned, and any running query is cancelable.
- **Message Brokers (Kafka)** — connect Kafka clusters (incl. **AWS MSK over an
  SSH bastion**) to browse topics, **peek/produce** messages, inspect
  consumer-group lag, edit topic configs, and view a Schema Registry, with an
  Overview of brokers/partitions/throughput. Supports PLAINTEXT/TLS and SASL
  (PLAIN/SCRAM) auth, prod/read-only guards, and an in-process Kafka-aware proxy
  so a private cluster is reachable through a single SSH tunnel (librdkafka can't
  SOCKS, so Otto rewrites the advertised broker addresses on the fly).
- **Vault** — a workspace knowledge store: notes with `[[backlinks]]` and
  keyword + **semantic (vector) hybrid recall** (`otto-memory`). The core is
  domain-agnostic, so other areas (e.g. Product) recall from it instead of
  re-fetching context each turn.
- **Multi-user, RBAC & sharing** — per-feature roles (None < View < Edit <
  Admin), per-session isolation, an admin overview + audited impersonation, and
  **session sharing** via scoped, expiring, revocable links gated by an
  email-OTP access code. Optional **remote/mobile access** (Cloudflare tunnel +
  installable PWA) keeps the daemon loopback-only by default. The shell is fully
  **responsive (phone + iPad, portrait & landscape)** with collapsible,
  independently-scrollable sections, **light/dark + RTL**, and an opt-in
  **per-device session view** (show only sessions started on this device). See
  `docs/MULTI-USER-RBAC.md` and `docs/remote-access-runbook.md`.
- **Self-improvement** — an optional, gated engine that reflects on recent
  sessions and proposes edits to the workspace's skills/memory (tiered autonomy:
  safe edits auto-apply, risky ones queue for approval). Can run on multiple
  providers for varied suggestions.
- **Skills library** — a bundled, versioned skill library (`otto-skills`) you
  can browse and install/update from Settings; skills drive review lenses,
  product analysis, and insights, and the self-improvement engine refines them
  from your sessions.
- **Skills evaluator** — benchmark a skill: run **implement → validate → score →
  improve** across multiple iterations and providers, read a per-run report, and
  compare runs side-by-side to see what actually got better.
- **Insights** — scheduled, multi-provider "catch-up" reports that turn recent
  activity into action-first summaries, generated on demand and cached.
- **Usage & cost** — an embedded ClickHouse engine records real per-turn token
  usage and cost by tailing Claude and Codex transcripts (no manual
  instrumentation), with per-provider / day / session rollups and configurable
  retention.
- **API client** — a built-in REST workbench (collections, environments, history),
  with import/export (Postman / OpenAPI / HAR) and an SSRF-guarded executor.
- **MCP Control Plane** — two-way Model Context Protocol. **Outbound:** every MCP
  tool your agents call passes a governance pipeline (allowlist → policy →
  single-use approval → dry-run → fail-closed audit → stats). **Outward:**
  `ottod mcp-server` exposes a set of `otto.*` tools (codebase search, context
  packets, goal loops, work items, read-only DB, PR drafts, proof packs, human
  approval) to external MCP clients behind a restricted, single-purpose token.
- **Workflows** — a visual workflow engine that chains steps (agent prompts, HTTP
  requests, DB queries, broker peeks, channel notifications, human approvals,
  swarm tasks, …) into runnable graphs. Manual, webhook, and event triggers fire
  today; scheduled triggers and a few Product/Review nodes are still being wired.
- **Scheduled Tasks** — recurring agent jobs on an **interval / daily / weekly**
  schedule. Each run executes a prompt, writes a **Markdown report**, and
  **delivers** it to Slack, Telegram, email, or a webhook (secrets redacted) —
  with a run history and a set of `otto.*` MCP tools to manage jobs.
- **Custom plugins** — extend Otto at runtime with out-of-process **sidecar
  plugins** (any language) you install/enable/remove **without rebuilding**: the
  daemon supervises each plugin process, reverse-proxies its HTTP/UI into an
  iframe panel, exposes a small **scoped host API**, and gates each by slug-keyed
  RBAC. Node and Rust examples ship in `examples/plugins/` (see
  `docs/plugins/AUTHORING.md`).

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
  `otto-brokers` (Kafka viewer), `otto-ssh` (shared SSH-tunnel helper),
  `otto-swarm` (Agent Swarm), `otto-memory` (Vault knowledge store),
  `otto-product` (Jira/Confluence story workflows), `otto-improve`
  (self-improvement), `otto-usage` (ClickHouse usage/metrics), `otto-skills`
  (bundled skill library), `otto-context`, `otto-rbac`, `otto-netguard`
  (SSRF guard), `otto-keychain` (macOS Keychain secret storage),
  `otto-canvas` (Canvas scenes), `otto-mcp` (MCP control plane + governance),
  `otto-workgraph` (Mission Control work graph), `otto-server` (routes),
  `ottod` (binary).

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
cd ui && npm run check              # svelte-check + tsc (+ the e2e tsconfig)
cd ui && npm run test:e2e          # Playwright mobile/tablet E2E
```

The **E2E suite** (`ui/e2e/`, Playwright) spins up an **isolated throwaway daemon**
(temp data dir + port — it never touches your real sessions/DBs), serves the live
UI via Vite, and drives every page across five device profiles (iPhone & iPad,
**portrait + landscape**, plus a small phone). It asserts real behaviour — pages
fit the width and scroll, sections collapse, and core flows work (DB query →
results, Git commit → diff, terminal output/input) — and runs the same checks in
**light/dark** and **RTL**. The mobile shell is collapsible-section based and
touch-readable; see `docs/superpowers/specs/` for the design notes.

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
docs/features/  Per-feature guides (setup, walkthrough, API, limits)
```

## Documentation

Every feature above has a dedicated, **code-grounded** guide under
**[`docs/features/`](./docs/features/README.md)** — setup (incl. token/account
and Slack-manifest steps), a full walkthrough, the relevant REST/WS surface,
explicit capabilities & limitations, security notes, and troubleshooting. Start
at the **[features index](./docs/features/README.md)**.

Where everything lives:

| Doc | What it is |
|-----|------------|
| **[Feature guides](./docs/features/README.md)** | One in-depth guide per feature — the definitive reference. Start here. |
| [`docs/contracts/`](./docs/contracts/) — [`api.md`](./docs/contracts/api.md), [`ws.md`](./docs/contracts/ws.md), [`product.md`](./docs/contracts/product.md) | The REST + WebSocket contracts. **Authoritative** for the API shape; the TS types in `ui/src/lib/api/types.ts` mirror them. |
| [`docs/MULTI-USER-RBAC.md`](./docs/MULTI-USER-RBAC.md) | Operator runbook: per-feature roles, isolation, impersonation, API tokens. |
| [`docs/remote-access-runbook.md`](./docs/remote-access-runbook.md) | Operator runbook: reaching Otto from a phone/iPad — Cloudflare tunnel, PWA, share links, email-OTP. |
| [`docs/RELEASE.md`](./docs/RELEASE.md) | The macOS packaging flow — sidecar copy, Tauri build, codesigning, DMG. |
| [`docs/plugins/AUTHORING.md`](./docs/plugins/AUTHORING.md) | How to write a custom sidecar plugin (the host API, manifest, examples). |
| [`marketing/videos/`](./marketing/videos/) | The Remotion source for the in-app **Walkthroughs** (rendered to `ui/public/walkthroughs/`). |

Design notes, implementation plans, and research write-ups live under
[`docs/design/`](./docs/design/), [`docs/plans/`](./docs/plans/), and
[`docs/research/`](./docs/research/) — useful background, but the feature guides
and `docs/contracts/` are the sources of truth.

## Contributing

Issues and PRs welcome. Please run `cargo test`, `cd ui && npm run check`, and
(for UI changes) `cd ui && npm run test:e2e` before opening a PR. The Rust API in
`docs/contracts/` is authoritative — keep the TypeScript types in
`ui/src/lib/api/types.ts` in lockstep.

## License

Released under the [MIT License](./LICENSE).
