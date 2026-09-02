# Otto

**An agentic development environment.** Otto is a macOS desktop app that runs
coding-agent CLIs (Claude Code, Codex, and others) as first-class, openable
sessions — and wires them into the rest of your workflow: git & pull requests,
multi-agent code review, Jira/Confluence product workflows, SSH/database
connections, an HTTP API client, real token-usage tracking, and Slack/Telegram
bridges so an agent can work a ticket from a chat thread.

> Status: early / actively evolving. macOS-only for now. Expect rough edges.

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
- **Agent sessions** — run `claude`, `codex`, `agy`, a plain shell, or **any
  custom agent CLI you register** (custom providers are first-class on every
  agent surface, with per-surface provider/model pickers) in real PTY-backed
  terminals you can watch, split, and type into. Every session can **pin its
  own model** — chosen from a live per-provider **model catalog** (refreshed
  hourly from the providers' docs/CLIs, no API keys needed) — and the pin
  applies to that session only, so switching models never leaks into the next
  session; custom providers declare their model flag via a template
  (`--model {model}`, `-m {model}`, …). Sessions survive restarts
  (resumable), idle-suspend to save memory, and closing a tab asks what you
  mean — keep it running in the background or **archive** it (stops the
  process, keeps the full history, resumable later) — with bulk close/archive,
  reopen-closed-tab (⌘⇧T), and an opt-in auto-archive for long-idle sessions.
  Sessions can be confined by an optional macOS **Seatbelt sandbox**
  (`otto-sandbox`) and auto-trust their workspace folder so they never stall
  on a permission prompt. By default agents
  launch in a **skip-permissions** mode (`--dangerously-skip-permissions`, or
  codex's `--dangerously-bypass-approvals-and-sandbox`) so tool use never blocks;
  a single **Settings → Providers → Permissions** checkbox opts out and falls back
  to each CLI's own ask / auto permission mode (tool use then prompts in the
  session terminal). Applies to new sessions.
- **Git & Pull Requests** — a GitKraken-style **commit graph** with a WIP row +
  staging panel, per-file diffs, folder-level stage/discard, an interactive
  **conflict resolver** (A/B line picking with live output) that covers merge,
  **rebase, cherry-pick, revert and stash-pop conflicts** alike — conflicted
  files get their own WIP section with ours/theirs quick-resolves, and a dirty
  tree blocking a pull or branch switch offers a one-click
  **stash → pull/switch → restore** instead of a raw git error — branch checkout
  straight from the graph (stash · pull · pop), **worktrees & submodules** as
  first-class tabs, auto-fetch, and a **Focus tab** (your PRs across repos +
  your Jira work). **Create PRs** (draft toggle, reviewers at creation) with an
  **agent-drafted title + description** (it reads your branch diff — watch
  the drafting agent live inside the PR dialog), pushing the branch
  automatically; PR review threads support resolve/reply/reopen.
- **AI code review** — fan out several review agents (one per provider/lens)
  over a PR *or* your local working tree. Each runs as an openable session with
  live progress, per-agent findings, retry, and a configurable grace period.
  Findings become **tracked items** — a review-findings workflow with statuses
  and fix / verify / open-Jira / false-positive / regression-test actions — and
  can be **ingested into a Proof Pack** or saved to workspace memory.
- **Proof Packs — the trust layer** — an evidence layer so "done" means *proven*,
  not asserted. Each unit of work collects **artifacts** — diffs, recognized test
  output, **CI status**, **screenshots/video**, **API/DB/Kafka read** samples,
  review findings, self-review, and human approvals — and pure rules derive its
  **status, risk, badges, and an explainable "done contract" score** purely from
  the evidence (the agent never sets it). On PR creation it auto-captures CI and
  **checks the PR description against the actual change**; it enforces optional
  **per-repo policy** ("a passing test / green CI required"), freezes
  **immutable, tamper-evident snapshots**, exports a self-contained
  **Markdown/HTML report**, and records an **accountable human waiver** (approver
  + reason). Artifacts are redacted and size-capped, and a **gate** can block a PR
  over an unproven pack (with an audited override).
- **Product (Jira / Confluence)** — import a Jira issue or Confluence page
  (search by project or space — no key prefix needed) and run a product-owner
  workflow over it: multi-agent, multi-provider **analysis** with a summarizer
  and **open questions** you can post back as comments; a suggested **rewrite**;
  **test-case** generation with approval published to a linked Confluence page;
  **discovery** drafts (start blank, drop in ideas or call transcripts, then
  publish as an RFC or a Jira story); plus versioning, sectioned history, tags,
  a **Plan/Tasks** breakdown, and a recurring-patterns **learnings** base. A
  background watcher polls for new comments/updates, and you can inject a story's
  full refined context into any running agent. Story titles/descriptions are
  editable inline, and Otto reads **and writes Confluence pages** directly
  (rich HTML publishing — tables, task lists, panels, full-width layout).
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
- **Connections** — open SSH / MySQL / PostgreSQL / Redis / MongoDB / ClickHouse
  sessions side-by-side with agents. The SSH username is optional everywhere
  (terminals, DB tunnels, Kafka tunnels): leave it empty and ssh resolves it the
  way your terminal does — `~/.ssh/config` `User`, else your local login name.
- **Database Explorer** — a TablePlus-class browser for MySQL, PostgreSQL, Redis,
  MongoDB, and ClickHouse over plaintext, TLS/SSL, or SSH tunnels: a lazy schema tree,
  per-engine autocomplete, multiple query tabs, a virtualized results grid
  (client-side filter/sort + approval-gated inline editing), a Navicat-style
  visual JOIN builder, Superset-style dashboards/widgets for ClickHouse, and
  "examine this schema with an agent". Read queries get an automatic row `LIMIT`
  so a huge table is never fully scanned, any running query is cancelable, and
  queries **survive navigation** (detached runs you can re-attach to). Closing a
  connection tab **actually disconnects** — the daemon cancels its in-flight
  queries, closes the pooled backend connections, and drops the SSH tunnel — and
  a closed tab stays closed across restarts. The JOIN builder, table designer,
  and index builder generate **engine-correct SQL** per dialect (Postgres
  quoting/ALTERs, ClickHouse `ADD INDEX`/`MODIFY COLUMN`), the grid and schema
  tree are fully keyboard-navigable, and dashboards/widgets are editable in
  place with cancelable streaming export/import. Also: NL→SQL assistance, full
  **mongosh scripts** in the Mongo query tab, index management
  (view/create/edit/drop from the Structure view), foreign-key navigation, JSON
  cell/document editing, file import, and streaming CSV/format export.
- **Message Brokers (Kafka)** — connect Kafka clusters (incl. **AWS MSK over an
  SSH bastion**) to browse topics, **peek/produce** messages, inspect
  consumer-group lag, edit topic configs, and view a Schema Registry, with an
  Overview of brokers/partitions/throughput. Supports PLAINTEXT/TLS and SASL
  (PLAIN/SCRAM) auth, prod/read-only guards, and an in-process Kafka-aware proxy
  so a private cluster is reachable through a single SSH tunnel (librdkafka can't
  SOCKS, so Otto rewrites the advertised broker addresses on the fly).
- **AWS console** — S3, SQS, EC2, Athena and EKS in one module, driven through
  the official `aws` CLI v2 (installed on demand — Homebrew or a direct
  download — never a bundled SDK), so SSO/MFA/assume-role profiles just work.
  Accounts are configured in the UI (an existing `~/.aws` profile or access
  keys kept in the Keychain; Otto never writes `~/.aws/*`), each with a
  per-service permission probe and a one-click "Sign in" when an SSO session
  expires. S3 is read-only (browse, preview, download); SQS peek/send/purge/
  redrive; EC2 start/stop/reboot with typed confirms; a DB-Explorer-style
  Athena workbench (catalog tree, results grid, history, scanned-bytes cost);
  EKS clusters importable straight into the Kubernetes console. Six RBAC keys
  (`aws` for accounts, plus `aws_s3`, `aws_sqs`, `aws_ec2`, `aws_athena`,
  `aws_eks`) gate it per service, and agents get read tools plus Edit-gated
  `aws_athena_query` / `aws_sqs_send` over MCP.
- **Kubernetes console** — a k9s-like cluster workspace over `kubectl` (installed
  on demand; kubeconfigs picked from `~/.kube/config`, pasted, or imported from
  EKS into Otto-owned files — your kubeconfig is never modified). Namespace
  filter, health-colored resource tables with live CPU/MEM when metrics-server
  exists, a detail drawer with manifest (secrets redacted) / describe / events /
  streaming **logs** / an inline **exec** terminal, and row actions — restart,
  scale, rollout undo/pause/resume, **Argo Rollouts** promote/abort/retry and
  **ArgoCD** sync/refresh/redeploy, all via the CRDs (no extra CLIs). One-click
  **k9s** tab. Gated by the `kubernetes` RBAC key (View/Edit/Admin); agents get
  `k8s_get_resources` / `k8s_describe` / `k8s_logs` / `k8s_top` and the
  Edit-gated `k8s_action` over MCP.
- **Browser** — a workspace-scoped web browser inside Otto: reader tabs fetch a
  URL and render it as clean markdown (JS-rendered pages via a
  [Lightpanda](https://github.com/lightpanda-io/browser) sidecar, beta software,
  with a transparent plain-fetch fallback when it's unavailable), DOM
  annotations you can send into a live session or save as a Vault note, a
  CSS-selector query, one-turn page summarize, and Keychain-backed **Site
  Credentials** an agent can use to sign in only when you've explicitly opted
  the credential in. Five `browser_*` MCP tools give agents the same
  fetch/query/summarize/login pipeline. Every fetch is netguard-checked
  (loopback/private/metadata addresses refused). In the desktop app, tabs can
  flip to **Live** — a real embedded native webview with its own click-to-mark
  overlay — instead of the fetched reader view.
- **Vault — the docs home** — register a local folder of markdown files (even a
  live **Obsidian vault**) and get Obsidian-parity docs in Otto: file tree,
  editor ⇄ reading view with wikilinks/backlinks/tags, full-text search, quick
  switcher, and a graph view built to scale (with focus filters by service,
  type, tag, and hops) — with **OKF** as the documentation standard and every
  note readable/writable by agents over MCP. **Docs agents** can write the docs
  home *for* you: multi-agent create/refine runs over a repo, with a
  summarizer, iterative reviewer rounds, actionable findings, and persistent
  run history. Files stay the source of truth; Otto keeps only a derived index
  (`otto-vault`).
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
- **Skills Lab** — a three-tab workbench over your skills: a **viewer/editor**
  (multi-file CRUD, zip import, provider skills from `.claude`/`.codex`/`.agy`),
  a **multi-agent skills review** (with an apply-fixes agent and static checks
  for dead citations/paths), and an **evaluator** that benchmarks a skill —
  run **implement → validate → score → improve** across multiple iterations and
  providers, read a per-run report, and compare runs side-by-side to see what
  actually got better.
- **Insights** — scheduled, multi-provider "catch-up" reports that turn recent
  activity into action-first summaries, generated on demand and cached.
- **Usage & cost** — an embedded ClickHouse engine records real per-turn token
  usage and cost by tailing Claude and Codex transcripts (no manual
  instrumentation), with per-provider / day / session rollups and configurable
  retention.
- **API client** — a built-in REST workbench (collections, environments, history),
  with import/export (Postman / OpenAPI / HAR) and an SSRF-guarded executor.
- **MCP Control Plane** — two-way Model Context Protocol. **Inbound:** every agent
  session gets a first-party `otto` MCP server (`ottod mcp-tools`) with read-only
  tools over Otto's own data — including your **database connections**:
  `otto_list_connections`, `otto_db_schema`/`_children`/`_object` (full table
  structure), and `otto_db_query` to **run read-only queries and get rows**. Writes
  and DDL are refused server-side (independent of the connection's write-guard);
  results are row-capped, PII-masked, and audited. On by default; attached to Claude
  via `.mcp.json` and to Codex via per-spawn `-c` overrides. **Outbound:** every MCP
  tool your agents call passes a governance pipeline (allowlist → policy →
  single-use approval → dry-run → fail-closed audit → stats). **Outward:** Otto
  exposes its own `otto.*` tools — across every feature (codebase search, context
  packets, workflows, git/PRs, issues, brokers, swarm, vault, read-only DB, proof
  packs, scheduled tasks, …) — to external MCP clients **over HTTP, not only
  locally**: a **Streamable-HTTP** endpoint (`POST /api/v1/mcp/http`) a client
  connects to with a bearer token, no local subprocess (the `ottod mcp-server`
  stdio bridge still works too). You can mint **multiple scoped tokens** — each
  owned by an Otto **user** and carrying its own permission set (which tools,
  **read-only vs writes**, an optional workspace pin), enforced at the one
  governed choke point — so **different users get different access**. Served on
  loopback always, and reachable from another machine over the opt-in **TLS
  network listener** (or fronted by a tunnel). Manage tokens and the HTTP/network
  settings in **MCP → Otto Server**.
- **Workflows** — a visual workflow engine that chains steps (agent prompts, HTTP
  requests, DB queries, broker peeks, channel notifications, human approvals,
  swarm tasks, …) into runnable graphs, with per-workflow instructions +
  `prompt.md` context files, per-step retry / re-run-from-here, a stall
  watchdog, and a concurrency cap with queueing. Runs **survive daemon
  restarts**: finished steps are adopted from the persisted per-node state and
  the interrupted step resumes (side-effect steps are never blindly re-fired;
  a per-workflow toggle opts out). Manual, webhook, **chat**
  (run a workflow from a bound Slack/Telegram channel), and event triggers fire
  today; scheduled triggers and a few Product/Review nodes are still being wired.
- **Scheduled Tasks** — recurring agent jobs on an **interval / daily / weekly**
  schedule. Each run executes a prompt, writes a **Markdown report**, and
  **delivers** it to Slack, Telegram, email, or a webhook (secrets redacted) —
  with a run history and a set of `otto.*` MCP tools to manage jobs.
- **Personal Agents** — grok-bot-style **preset personal agents**: each one is a
  named persona (soul) with a **pinned provider + model**, its own working
  folder and **memory notes**, one **or more** schedules (e.g. a daily recap at
  09:00 *plus* a 15-minute "needs attention" sweep on the same agent), optional
  **browser use** (Playwright MCP), and per-agent delivery to Slack/Telegram/
  email/webhook. Every run is a fresh, watchable session; each agent also has a
  **chat-anytime** session pinned to the same persona/model. Agents can talk to
  each other in **rooms** (group chats) that are **always visible to you** —
  every message is persisted, streamed live to the UI, and you can post into
  any room; rooms are the only agent-to-agent transport, so nothing happens
  behind your back. Ships with editable example agents (personal assistant,
  daily recap, casino reviewers).
- **Custom plugins** — extend Otto at runtime with out-of-process **sidecar
  plugins** (any language) you install/enable/remove **without rebuilding**: the
  daemon supervises each plugin process, reverse-proxies its HTTP/UI into an
  iframe panel, exposes a small **scoped host API**, and gates each by slug-keyed
  RBAC. Two real example plugins ship in `examples/plugins/` — **Team
  Performance** (git-primary delivery analytics: per-dev/team reports,
  AI scope estimates, goals) and **DORA metrics** (see
  `docs/plugins/AUTHORING.md`).
- **Multi-window** — open any number of Otto windows (**⌘⇧N**), each an
  independent workspace surface with its own module, tabs, and split panes;
  quitting snapshots the whole window set (frames, screens, fullscreen) and the
  next launch restores it. Sessions live in the daemon — windows only hold
  references, so nothing restarts or duplicates.
- **Snipping tool** — one-gesture screenshots for agent work: a system-wide
  shortcut (default ⌘⌃⇧2) → native region select → an annotation editor
  (boxes, arrows, text, pixelate, badges) — the image is on the clipboard at
  every step, ready to paste into a session.

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
  `otto-browser` (in-app browser: reader/live tabs, annotations, Lightpanda
  sidecar engine), `otto-swarm` (Agent Swarm), `otto-vault` (the docs home — file-backed
  markdown vaults + OKF), `otto-memory` (workspace memory layer),
  `otto-product` (Jira/Confluence story workflows), `otto-improve`
  (self-improvement), `otto-usage` (ClickHouse usage/metrics), `otto-skills`
  (bundled skill library), `otto-context`, `otto-rbac`, `otto-netguard`
  (SSRF guard), `otto-sandbox` (macOS Seatbelt confinement for spawned
  agent/shell sessions), `otto-keychain` (macOS Keychain secret storage),
  `otto-canvas` (Canvas scenes), `otto-mcp` (MCP control plane + governance),
  `otto-workgraph` (Mission Control work graph), `otto-server` (routes),
  `ottod` (binary).

## Download (prebuilt DMG)

A GitHub Actions workflow builds the Apple-Silicon `.dmg` **every Sunday
morning** (and on demand) and publishes it to the rolling
[`weekly` release](../../releases/tag/weekly) — the newest build is always at
the same URL: `releases/download/weekly/Otto-macos-arm64.dmg`. Download, open,
drag **Otto** to `/Applications`; to **update**, download it again and replace
the app (the bundled daemon self-deploys on launch). The CI build isn't
notarized, so on first launch use **right-click → Open** or run
`xattr -cr /Applications/Otto.app`.

## Prerequisites

- **macOS** (Apple Silicon or Intel). Otto uses launchd, the macOS Keychain, and
  codesigning, so it is macOS-only today.
- **Rust** (stable) — <https://rustup.rs>
- **Node.js 20+** and npm
- **CMake** — compiles the bundled `librdkafka` (the Kafka/Message Brokers
  driver) from source: `brew install cmake` (macOS) / `apt-get install cmake`
  (Linux). Build-from-source only — installing the prebuilt `.dmg` does **not**
  need it (librdkafka is already compiled into the shipped binary).
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
cd ui && npm ci && npm run build && cd ..

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
cargo build --workspace            # Rust build
cargo test --workspace             # Rust tests
cargo clippy --workspace --all-targets -- -D warnings   # lints (CI-enforced)
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
touch-readable.

## Configuration & secrets

- **First run** prompts you to set a root password (local accounts), then add
  workspaces (folders), agent defaults, and git/issue/channel accounts in
  **Settings**.
- **Secrets never live in the repo or in plain files.** Tokens (git, Slack,
  Telegram, Jira, connection passwords) are stored in the **macOS Keychain** via
  `otto-keychain`; the daemon DB only stores opaque key references.
- The daemon listens on **loopback only** unless you explicitly enable a network
  listener in settings (it then also binds `0.0.0.0` over **TLS**, self-signed) —
  which is also how the MCP HTTP transport becomes reachable from another machine.

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
| [`marketing/videos/`](./marketing/videos/) | The Remotion source for the in-app **Walkthroughs** (rendered to `marketing/videos/out/`, published to the `walkthroughs` GitHub release by `packaging/publish-walkthroughs.sh` — not bundled). |

The architecture overview lives in the [Architecture](#architecture) section
above and in [`AGENTS.md`](./AGENTS.md); the feature guides and
`docs/contracts/` are the sources of truth.

## Contributing

Issues and PRs welcome. Please run `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cd ui && npm run check`, and
(for UI changes) `cd ui && npm run test:e2e` before opening a PR. The Rust API in
`docs/contracts/` is authoritative — keep the TypeScript types in
`ui/src/lib/api/types.ts` in lockstep.

## License

Released under the [MIT License](./LICENSE).
