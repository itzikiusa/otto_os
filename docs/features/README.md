# Otto Feature Guides

In-depth, **code-grounded** documentation for every Otto feature — one guide per
feature. Each guide is the definitive reference for that area: what it is, how to
set it up (including tokens, accounts, and ready-to-paste manifests), a full
walkthrough of every sub-feature, the relevant REST/WebSocket surface, an explicit
**capabilities & limitations** section, security notes, and troubleshooting.

> **How to read these.** This index is the map; the README's
> [feature tour](../../README.md) is the elevator pitch. The API itself is
> specified in [`docs/contracts/`](../contracts/) — that contract is
> **authoritative** and these guides describe the running implementation against
> it. Where the two diverge, each guide documents the *code's actual behavior* and
> flags the drift; see [Known drift](#known-drift-between-docs-and-code) below.
>
> _Last verified against the codebase: **2026-06-21**._

---

## Agents & automation

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [Agent sessions](./agent-sessions.md) | Running `claude` / `codex` / `agy` / shell as PTY-backed, resumable, idle-suspending sessions — the terminal WS protocol, split/tile, trust & prompt-guard, the activity trail, and driving sessions programmatically. | No setup beyond an agent CLI on `PATH`. |
| [Agent Swarm](./agent-swarm.md) | A team of role-specialized agents in an org hierarchy (CEO → … → Devs) that autonomously work projects → tasks: the recruiter, the per-swarm coordinator, the shared board, org-tree / DAG / Kanban views, scheduled runs, and the five preset swarms. Includes a **"set up your company"** how-to. | Start from a preset or build an org in the Swarm tab. |
| [AI code review](./code-review.md) | Fan out review agents (one per lens × provider) over a **PR** or your **local working tree**; live per-agent findings, retry, grace period, and the PR-review config. Lenses are data-driven from installed `review` skills. | PR mode needs a git token ([git guide](./git.md)). |
| [Goal Loops](./goal-loops.md) | Give a **goal** + a **budget** (max iterations + active time); a team of agents iterates **Plan → Execute → Evaluate → Digest** on an isolated `goal-loop/<id>` branch until the **machine-checked** acceptance criteria are met or a limit is hit. AI-assisted goal definition, live phase/iteration monitoring, openable executor sessions. | No setup beyond an agent CLI on `PATH`. |
| [Workflows](./workflows.md) | The visual workflow engine — nodes + edges + triggers, the topological run loop, `human_approval` pause/resume, and the API-client automation runner. **Honest maturity table**: which nodes/triggers are real vs. stubbed/unwired. | Build in the Workflows tab; webhook/event/manual triggers fire today. |

## Source control & product delivery

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [Git & Pull Requests](./git.md) | GitKraken-style repo tabs, stage/commit/discard, diffs, conflict resolution, and agent-drafted PRs that auto-push the branch. Includes **per-provider git-token setup**. | **GitHub** PAT, **Bitbucket Cloud** app password, or **GitLab** PAT → Settings → Git Accounts. |
| [Jira & Confluence](./jira-confluence.md) | Connecting an Atlassian account and importing issues/pages (search by project/space, no key prefix), reading, and posting comments back. | **Atlassian Cloud API token** (email + token, Basic auth) → Settings → Issue Accounts. One account drives both Jira & Confluence. |
| [Product](./product.md) | The full product-owner workflow on top of Jira/Confluence: multi-lens/multi-provider analysis + summarizer + open questions, rewrite, test-case generation → Confluence publish, the multi-agent **Plan/Tasks** breakdown (+ send-to-swarm), Discovery drafts → RFC/story, the global Learnings base, versioned history, and the background watcher. | Needs a connected Jira/Confluence account first. |

## Team communication

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [Channels — Slack & Telegram](./channels-slack-telegram.md) | Bridge a Slack or Telegram thread to an agent session (messages + files relayed both ways, one agent per ticket, idle auto-archive), plus **Broadcast** (one literal message to many live sessions). | **Slack uses Socket Mode** — includes a **copy-paste app manifest** with the exact scopes/events. **Telegram** via @BotFather (long-poll). |

## Data & infrastructure

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [Connections — SSH & SFTP](./connections-ssh-sftp.md) | Defining connections (SSH / MySQL / Redis / MongoDB / ClickHouse), interactive PTY sessions, **SSH tunnels** (`-L` vs SOCKS5 `-D`, with the MongoDB `+srv` reason and the bastion `AllowTcpForwarding` gotcha), and the **SFTP file browser** over SSH. | Add a connection; secrets go to the Keychain. |
| [Database Explorer](./database-explorer.md) | A TablePlus-class browser: lazy schema tree, per-engine autocomplete, query tabs with an auto row `LIMIT` and cancel, a virtualized results grid with approval-gated inline edits, a visual JOIN builder, ClickHouse dashboards/widgets, "examine schema with an agent", and streaming CSV/format export. | Reuses connection profiles; engines configured here. |
| [Message Brokers (Kafka)](./message-brokers.md) | Connect Kafka clusters (incl. **AWS MSK over an SSH bastion** via the in-process Kafka-aware proxy), browse topics, peek/produce, consumer-group lag, topic configs, Schema Registry, and an Overview with CPU/RAM. PLAINTEXT/TLS + SASL PLAIN/SCRAM, prod/read-only guards. | Add a cluster; MSK path documented step-by-step. |

## Knowledge & quality

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [Vault](./vault.md) | The workspace knowledge store (`otto-memory`): notes with backlinks, collections + graph, and **keyword + vector hybrid recall** that other features (e.g. Product) read from instead of re-fetching context. | Local-first; ships with a dependency-free stub embedder. |
| [Skills library](./skills-library.md) | The bundled, versioned first-party skill catalog you browse and install/update from Settings; how installed skills drive Review lenses, Product analysis, and Insights. | Installs to `<data_dir>/library/skills/`. |
| [Skills evaluator](./skills-evaluator.md) | Benchmark a skill: run implement → validate → score → improve across iterations/providers, view a run report, and compare runs side-by-side. | Settings → Skill Eval for defaults. |
| [Self-improvement](./self-improvement.md) | The optional, gated engine that reflects on recent sessions and proposes edits to **skills (`SKILL.md`) and memory (`*.md`) only** — never repo code — with tiered autonomy (auto-apply vs. approval queue) and optional channel notifications. | Off by default; enable in Settings. |

## API & integration surface

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [API client (REST workbench)](./api-client.md) | The built-in "Postman": collections, request builder (HTTP/SSE/WS/gRPC), environments + `{{vars}}`, response viewer, history, and automations. **Outbound is SSRF-guarded** (localhost/RFC1918/cloud-metadata blocked). | In-app; no external setup. |
| [Daemon HTTP API](./daemon-http-api.md) | Driving Otto **programmatically** over `ottod` (`http://127.0.0.1:7700/api/v1`): authentication (PAT / login / share / ingest tokens), a navigable domain map of the REST surface, the WebSocket terminal & event streams, the async-202 pattern, and an explicit **what-you-can / cannot** list. | Create a PAT → Settings → Personal Access Tokens. |

## Observability

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [Usage & cost](./usage-and-cost.md) | The embedded ClickHouse engine that tails Claude/Codex transcripts (zero instrumentation) for per-turn token usage and cost — input/output/cache-read/cache-write breakdown, provider/day/session rollups, system CPU/RAM, retention TTL, and opt-in budgets. | Install ClickHouse via the in-app one-liner. |
| [Insights](./insights.md) | Scheduled, multi-provider "catch-up" reports (daily/weekly/monthly) that turn recent activity into action-first HTML summaries, generated on demand and cached. | Toggle in Settings → Insights. |

## Access, security & UX

| Guide | What it covers | Quick-start note |
|-------|----------------|------------------|
| [Multi-user, RBAC & sharing](./rbac-multiuser-sharing.md) | Per-feature roles (None < View < Edit < Admin) and the grant matrix, per-session isolation & data ownership, the admin overview + terminate, audited impersonation, API tokens, and the scoped-share + email-OTP model. | First run sets a root password. |
| [Remote / mobile access](./remote-mobile-access.md) | Reaching Otto from a phone/iPad: a Cloudflare tunnel (or opt-in TLS network listener), the installable PWA, scoped/expiring/revocable share links, and the email-OTP access gate (Gmail App Password). Loopback-only by default. | Opt-in; tunnel recommended over the listener. |
| [Using Otto on mobile](./mobile-usage.md) | Task-oriented: install the PWA, what the touch UI looks like (phone/tablet/desktop shells, bottom nav, drawers), running & typing into a session on a touchscreen (DOM-renderer terminal, key accessory bar), per-device session view, and limits. | Make Otto reachable first (see remote access). |
| [Sharing a session](./session-sharing.md) | Task-oriented: mint a scoped viewer/editor link to **one** session for someone with no account, optionally email-OTP gated; what the guest sees; revoke (with immediate eviction). | OTP shares need a verified Gmail sender. |
| [RTL & responsive shell](./rtl-and-responsive.md) | Right-to-left support (incl. the terminal-bidi mode), the responsive phone/iPad shell (breakpoints, collapsible sections, touch terminal), light/dark theming, the per-device session view, and how the E2E suite verifies it all. | Settings → Appearance. |
| [Custom plugins](./plugins.md) | Runtime, out-of-process sidecar plugins (install/enable/remove without rebuild): the supervisor, reverse-proxy + iframe UI, the scoped host API, and slug-keyed RBAC. User/operator companion to the [authoring guide](../plugins/AUTHORING.md). | Settings → Plugins (installed app needs a rebuild to surface). |

---

## Conventions

- **Code-grounded.** Every claim is verified against the crate(s) and UI module(s)
  that implement it. Endpoints, WebSocket events, setting keys, and UI labels are
  the real ones. Where a feature is partial, stubbed, or deferred, the guide says so.
- **`docs/contracts/` is authoritative** for the API shape; these guides describe
  behavior and link back to the contract sections.
- **Secrets** (git/Slack/Telegram/Jira/connection passwords) live in the macOS
  **Keychain** via `otto-keychain`; the SQLite DB stores only opaque key references.
  (The one current exception — API-client environment values — is noted in that guide.)
- **Loopback-only by default.** The daemon does not accept network connections
  unless you explicitly enable remote access.

## Known drift between docs and code

These are discrepancies the guides surfaced between the **frozen contracts /
seed docs** and the **running code** (as of 2026-06-21). Each feature guide
documents the code's real behavior; this list is a maintenance to-do for the
contract owners.

- **`ws.md` review statuses** — lists `queued|running|done|error|cancelled`, but the
  review runner only emits `running|done|error` (no cancel path today). See
  [code-review](./code-review.md).
- **`ws.md` budget `direction`** — says `"recovered"` is reserved, but the code
  already emits it (and the UI handles it). See [usage & cost](./usage-and-cost.md).
- **`api.md` Usage table** — omits `GET /usage/by-kind`, `GET /usage/attribution`,
  and `POST /usage/forecast`, which are all wired. See [usage & cost](./usage-and-cost.md).
- **`api.md` Product draft route** — shown as `POST`; the real method is `PATCH`.
  See [product](./product.md).
- **`docs/contracts/product.md`** — still says analysis agents are "claude-backed
  (v1)", but each lens × provider now runs as a real openable session
  (multi-provider is live). See [product](./product.md).
- **Vault recall** — README/contract frame it as "FTS5 + sqlite-vec hybrid", but
  the shipped path is a SQL `LIKE` prefilter + brute-force cosine over a local,
  dependency-free **stub embedder**; OpenAI/Voyage/`fastembed` are unwired seams.
  See [vault](./vault.md).
- **Skill catalog** — `otto-skills/SKILL_AUTHORING.md` describes a five-category
  catalog, but the crate currently ships only `review` (7) + `insights` (1); the
  seven `product` skills are auto-seeded from `otto-product`. See [skills library](./skills-library.md).
- **Insights auth** — `api.md` labels the endpoints `root`, but the code enforces
  RBAC `Insights` tiers (with config/run effectively root-gated today). See
  [insights](./insights.md).
- **AGENTS.md crate/module map** — previously mislabeled `otto-orchestrator` as the
  review engine (it is the Claude-PTY agent runner / ⌘K planner; the review engine
  is in `otto-server`) and omitted `otto-brokers`, `otto-ssh`, `otto-memory`,
  `otto-netguard` and the `brokers`/`plugins`/`share`/`vault` UI modules. **Fixed**
  alongside these docs.
- **README feature tour** — did not list **Custom plugins** or **Workflows**.
  **Fixed** alongside these docs.
