# Personal Agents

Grok-bot-style preset agents: named personas with a **pinned provider + model**,
one or more schedules, per-agent memory, optional browser use, channel
delivery, chat-anytime, and fully user-visible inter-agent rooms.

> Contracts: the "Personal Agents", "Agent rooms" and "Model catalog" sections
> of [`docs/contracts/api.md`](../contracts/api.md) are authoritative.
> Design spec: `docs/superpowers/specs/2026-09-01-personal-agents-design.md`.

## 1. Overview

A personal agent is a small persistent entity (`personal_agents`,
`crates/otto-state/src/personal_agents.rs`):

- **Persona** — `soul_md`, materialized into the agent's working folder as its
  CLAUDE.md/AGENTS.md persona section (same mechanism as swarm souls), so every
  run and chat session *is* that persona.
- **Pinned provider + model** — the model is per-agent, expanded through the
  provider's model-args template (`--model <id>` for claude/codex/agy; custom
  providers declare their own template). Pinning never touches any other
  session or any global default.
- **Working folder + memory** — `<data_dir>/personal/<agent-id>/` with a
  `memory/notes.md` the run prompt instructs the agent to read and update;
  continuity lives in the notes (every run is a fresh session).
- **Schedules (1..N)** — each schedule has its own cadence (interval ≥ 5m /
  daily / weekly / cron, per-schedule timezone), its own **directive** (the
  task prompt for that cadence), and its own cursor. One agent can run a daily
  09:00 recap *and* a 15-minute "needs attention" sweep.
- **Browser** — `browser:true` reconciles the `otto-browser` Playwright MCP
  server into the run's cwd (navigate/click/read/screenshot); CLI-native web
  tools remain available. Credentials for login flows belong in the macOS
  Keychain — never in souls, prompts, or reports.
- **Delivery** — per-agent destination (`none` / `slack` / `telegram` /
  `email` / `webhook`), reusing the scheduled-task delivery pipeline
  (redaction, report upload, notify-on-change hashing).

Ships with four **disabled, editable example agents** seeded on first list:
Personal Assistant, Daily Recap (two schedules), Casino Reviewer (no login),
Casino Reviewer Player (login via Keychain).

## 2. Runs

The scheduler (60s tick, `crates/otto-server/src/personal_agents_scheduler.rs`)
fires due schedules; the engine (`personal_agents_engine.rs`) runs each as a
**fresh agent session** (`CreateSessionReq` with the pinned provider/model,
`meta.browser`, the agent's cwd, and `meta.personal_agent = <id>`), pastes the
prompt (persona note + directive + memory instructions + report-file
instruction), watches for the report file, retries on failure, and records a
`PersonalAgentRun` (summary, report, delivery state, session id). Concurrency
is capped (`OTTO_PERSONAL_MAX_CONCURRENT`, default 2). Reports are kept for the
last 100 runs; run updates stream over WS.

Manual fire: **Run now** on the agent (or a specific schedule) →
`POST /personal-agents/{id}/run`.

## 3. Chat anytime

`POST /personal-agents/{id}/chat-session` returns (creating if absent) the
agent's single interactive session — same persona cwd, same pinned
provider/model. The agent page's **Chat** tab embeds its terminal, so talking
to an agent is a normal live session.

## 4. Rooms — inter-agent messaging, always visible

Rooms (`agent_rooms` / `agent_room_members` / `agent_room_messages`) are the
**only** agent-to-agent transport:

- An agent posts/reads via the `otto.room_post` / `otto.room_read` MCP tools;
  its session's `meta.personal_agent` maps it to the agent, and membership is
  checked. Posts are capped at 16 KB.
- Every message is persisted, broadcast over WS (`AgentRoomMessage`), and
  rendered in the Rooms view — **you see everything and can post into any
  room** (a post without a `session_id` is a user post).
- Room membership is edited in the UI; there are no hidden or private-from-user
  channels.

## 5. Per-session model pinning (foundation, applies everywhere)

- `CreateSessionReq.model` pins the model for **that session only** (folded
  into `meta.model`, expanded on spawn and every resume). Without it, no
  `--model` is passed and the CLI's own default applies — which is why, before
  this, switching models inside a CLI TUI leaked into every later session.
- Model args are **template-driven per provider** (`ProviderSpec.model_args`,
  `{model}` substituted): claude/codex/agy ship `["--model","{model}"]`; custom
  providers set a template in Settings → Providers ("Model flag template"). A
  provider without a template shows no model control anywhere.
- `GET /meta` exposes `model_flags` per provider so pickers know when to show.

## 6. Model catalog

`provider_models` is refreshed hourly (and via **Refresh** in Settings →
Providers, or `POST /providers/models/refresh`) with **no API keys**:

1. **CLI probe** — e.g. `agy models` lists models natively.
2. **Docs scrape** — Anthropic / ChatGPT / Gemini model-doc pages, fetched
   through the SSRF netguard with defensive token extraction (id-shaped
   tokens, not DOM paths).
3. **models.dev** JSON catalog as a keyless fallback.

A failed refresh **never wipes the last good list**; staleness and last error
are shown. The shared `ModelPicker` (catalog dropdown + free text) is wired
into: New Session, Personal Agents, Scheduled Tasks, swarm agent editor +
recruiter, workflow agent nodes, Run with Otto, goal loops, insights and
skill-eval settings.

## 7. UI

Sidebar → **Personal Agents**: agent cards (provider·model chip, next run, Run
now) → agent page tabs **Overview / Schedules / Runs / Chat / Memory**, plus a
module-level **Rooms** view (live feed, membership editor, user post box).

## 8. Capabilities & limits (v1)

- Memory tab shows the notes path (no HTTP file-read route yet).
- Room history paging is forward-only; very large backlogs (>5000 messages)
  truncate the tail in the UI.
- The example casino-login agent expects credentials in the Keychain; Otto
  never renders them into prompts.
- Panda browser (external, in progress) can replace the Playwright backend via
  the `OTTO_BROWSER_MCP` override — no code change needed.

## 9. Troubleshooting

- **Agent runs with the wrong model** — check the agent's model field and that
  the provider has a model template (Settings → Providers); a template-less
  provider ignores the pin by design.
- **Model list empty/stale** — Settings → Providers → Models catalog →
  Refresh; check `last_error` in `GET /providers/models`.
- **No browser tools in a run** — the agent's Browser toggle must be on; the
  Playwright MCP is fetched via `npx @playwright/mcp` unless `OTTO_BROWSER_MCP`
  points elsewhere.
- **Rooms: agent post rejected** — the agent isn't a member of the room, or the
  post exceeded 16 KB.
