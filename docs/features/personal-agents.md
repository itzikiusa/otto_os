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

The scheduler (60s tick, `crates/otto-assistant/src/personal_agents_scheduler.rs`)
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

**One run per agent at a time.** Every run of an agent works in the same folder
and rewrites the same `memory/notes.md`, so scheduled, manual and delegated runs
share one per-agent guard: a due schedule waits (its cursor untouched, so it fires
on a later tick) while another run is going, and Run now answers 409. A failed run
keeps its session linked (*Open session*) and its error and any delivery failure
show inline on the run row. Each run's scratch report file is removed once read. A running run has **Stop…**
(`POST /personal-agents/runs/{run_id}/cancel`): its session is killed (no retry)
and it settles `canceled`.

**Arming.** A schedule never fires an occurrence from before its `armed_at` — set
on create, on resume (the schedule's or the whole agent's), and when its cadence or
timezone really changes. Re-enabling an agent after a week therefore doesn't fire
(and deliver) the week's missed recaps at once. A `once` schedule — the Assistant
creates them; the schedule form now supports them too — fires again after its
`run_at` is edited and it is re-enabled.

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
  room** (a post without a `session_id` is a user post — but only from a person's
  own credential: an agent session's token is always bound to its own session, so
  it can neither post as the human, read a room it isn't a member of, nor name
  another session).
- Room membership is edited in the UI; there are no hidden or private-from-user
  channels.
- **Agents are told about their rooms.** The agent's persona file
  (CLAUDE.md/AGENTS.md, re-provisioned on every run and every new chat) carries
  a "Your rooms" section: each room's name and id, the other member agents, and
  how to use `otto_room_read` / `otto_room_post`. Adding an agent to a room
  reaches it on its next run or new chat; an already-open chat keeps its file
  until reopened. Room messages are framed as information from other agents,
  never as instructions that override the agent's task.
- `otto_room_read` with no cursor returns the room's **newest** messages
  (default 50); `after` pages forward from the last id the agent saw, `before`
  pages back. (It used to start from the room's first message.)
- The rooms list shows each room's member count and last activity; names are
  capped at 120 characters.
- **Live feed cost.** The `agent_room_message` WS event carries the whole
  message, so an open room appends it without a request; nothing is fetched for
  rooms you haven't opened or while the Rooms view is closed (only the list's
  activity line moves), and leaving the view drops every held feed but the
  selected room's. The rooms list's count / last activity are stored on the
  room row (kept in the same transaction as each post, recounted after
  retention prunes), and the tail / paging reads are range scans on
  `(room_id, rowid)` — no query scans a room's whole history.

## 4b. Autonomy — permission modes, standing goals, rules, activity, memory

Named, always-on agents need to be safe to leave running. Every run says which
**permission mode** it ran in (`PersonalAgentRun.mode`, `read_only`, `goal_id`;
shown on Runs and Activity):

| Mode | Started by | Permissions |
|---|---|---|
| **Proactive** | the scheduler, working the agent's **standing goals** within a daily budget | **Strictly read-only.** Findings land in Runs; never delivered. |
| **Directed** | Run now, Assistant delegation, the Chat tab | Normal approvals + your auto-approve rules |
| **Scheduled** | a schedule firing | That schedule's own permission set: `directed` or `read_only` |

**Read-only is enforced at the tool layer, not just the prompt.** A read-only
run's session carries `meta.read_only = true`, and:

1. The daemon's governed tool path (`personal_agent_policy`, called from
   `mcp_outward::governed_invoke`) **denies** every mutating `otto.*` tool plus
   room posts, approval requests and memory writes — auto-approve rules and
   token write grants don't apply.
2. The feature guard refuses every **non-GET request made with that session's
   own token** (the native stdio tools: canvas/swarm writes, PR comments, room
   posts…) except a short list of read POSTs (searches, schema introspection,
   the read-only DB query, SQS peek, browser summarize).
3. Claude sessions start with `--disallowed-tools "Bash NotebookEdit Task"`
   (no shell → no `git push`, `gh`, `curl -X POST`).
4. The session is always Seatbelt-confined (writes only inside its folder and
   temp dirs), whatever **Settings → Daemon → process sandbox** says.
5. Browser automation (Playwright MCP) is off; the CLI's own web reading stays.

**Standing goals + budget** (Autonomy tab): a list of goals and a Proactive
switch with *runs per day* (1–24, any rolling 24 h, spaced evenly) and *minutes
per run* (1–60; a run that exceeds it is stopped). Goals are worked round-robin,
least-recently-worked first; **Work on it now** starts one read-only goal run.
The run prompt asks for findings and *proposals* — never actions.

**Custom rules**: plain language, added to the agent's persona file as "Your
rules". When a rule asks first or forbids ("ask before…", "get my approval…",
"never…", "don't…") **and** names a target — a quoted string, a `#channel`, or
a known word (prod/production, staging, main, master, billing, payment,
customer, finance, secret, slack, telegram, email, jira, confluence,
k8s/kubernetes, database, merge, deploy, delete) — Otto also **enforces** it on
the agent's mutating calls whose tool or arguments mention the target: an
approval (`risk_label: agent_rule`, never skipped by auto-approve) or a deny.
The Autonomy tab labels each rule *Enforced: …* or *Instructions only*.

**Sensitive-action gate** (all callers, not just personal agents): a mutating
call that touches accounts, credentials or sharing — `otto.test_integration`; a
scheduled task with a real `destination`; a credential/sharing argument
(`password`, `secret`, `*_token`, `api_key`, `credentials`, `share`,
`visibility`, `permissions`, `grant`, `invite`, `collaborators`, `acl`); or a
URL/path under `/password`, `/credentials`, `/share`, `/permissions`,
`/collaborators`, `/invitations`, `/tokens`, `/oauth`, `/members`, `/grants`,
`/api-keys` — **always** files a human approval (`risk_label: sensitive`), even
when an auto-approve rule or a token write grant covers the tool. Sessions
the scheduled-task or workflow engine spawned (meta `source`) are exempt: they
run unattended on a schedule the operator configured, so they keep the ordinary
dangerous-tool gate and auto-approve rules instead (their result delivery never
passes through this gate).

**Activity tab**: *Now* (the running run, its mode, read-only lock, session
state, **Watch session**), *Waiting for you* (approvals the agent's calls filed,
with Sensitive / Agent-rule labels and a link to MCP → Activity), and a
timeline merging its tool calls (called / blocked / needs approval) with its
runs. Live over the `personal_agent_activity` WS event; each event fetches only
the entries after the tab's cursor (`?after_seq=`), run history is re-read only
when a run changed (or once a minute), and an approval change refreshes the tab
only when it is one of the approvals it shows (perf W4). The tool-call list is an
in-memory view (newest 200 per agent, cleared on daemon restart);
`mcp_call_log` remains the durable audit. Because the ring's `seq` counter
restarts with the daemon, every answer carries the daemon's boot id (`epoch`);
the tab sends it back with its cursor, and a cursor from a previous process is
answered in full (`reset: true`) so the list never freezes after a restart.

**Memory inspector** (Memory tab, above the raw editor): every top-level bullet
of `memory/notes.md` as an item with its **source** — agents are told to start
each bullet with `[run]`, `[chat]`, `[slack]`, `[telegram]` or `[vault]`;
untagged bullets show as *Notes*. Filter by source, **Edit** or **Forget** one
item (version- and content-checked: a concurrent agent rewrite is a conflict,
never a wrong delete). Every run and chat session of the agent reads the same
file.

**Reset agent…** (header ⋯): re-seeds its memory, stops and unpins its chat,
deletes its schedules and run history (and report files), and clears goal
cursors and live activity. Persona, rules, goals, model and delivery stay.
Confirmed by typing the agent's name (checked server-side too); refused while a
run is in progress.

**Your agent (primary)**: one agent per workspace can be marked primary
(Autonomy tab). It leads the agents list with a **Chat** button and its persona
gets a "You are the user's primary assistant" section listing the other enabled
agents, with instructions to route specialist work to them through a shared
room.

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
now, *Your agent* / *Proactive* badges; the primary agent first, with Chat) →
agent page tabs **Overview / Activity / Autonomy / Schedules / Runs / Chat /
Memory / Context**, plus a module-level **Rooms** view (live feed, membership
editor, user post box). Schedules carry a *Read-only* chip when their
permission set is read-only; runs show their mode and a read-only lock.

## 8. Capabilities & limits (v1)

- **Memory** displays the existing `memory/notes.md` as Markdown. Editors can
  edit the source, Save or Cancel. A concurrent change produces a conflict while
  preserving your draft; reload the saved version and reconcile it before saving.
  Opening the tab does not create the workspace. A manual first save survives
  subsequent agent provisioning. Agents with the same custom working directory
  share this file; the tab displays the resolved path and this sharing behavior.
- **Context** holds user-maintained notes separately from agent-written Memory.
  Add file or Vault references using the picker, or type Markdown directly.
  New manual/scheduled runs and new chats receive the saved context across all
  providers. Existing chats keep their snapshot. References are paths the agent
  can read subject to its session permissions, not uploaded attachments.
- Viewers can read both tabs; editors can save. Documents are limited to 1 MiB.
  Memory reads/writes are restricted to the agent's fixed notes file and reject
  symlinks. Both editors use optimistic version checks; Memory replacements are
  atomic and Context uses a database compare-and-swap.
- Working-directory inputs offer **Browse** while preserving direct typing.
  Selection fills the field without saving the parent form. TLS certificate/key
  and ClickHouse binary inputs browse files. The authenticated picker browses the
  daemon host; browser onboarding before authentication retains text entry.
- Room history opens on the newest 200 messages and pages back with **Show
  earlier messages**; the UI keeps at most 500 in memory at once.
- Room create/rename/delete and membership changes are not broadcast — another
  open window sees them on its next rooms reload.
- The example casino-login agent expects credentials in the Keychain; Otto
  never renders them into prompts.
- Panda browser (external, in progress) can replace the Playwright backend via
  the `OTTO_BROWSER_MCP` override — no code change needed.

- Read-only confinement of the CLI's own tools is complete for **claude**
  (shell removed). codex/agy rely on the forced Seatbelt profile (filesystem)
  plus the daemon-side policy (Otto tools and the session token); their shell
  can still reach the network.
- Writes inside the agent's own working folder stay allowed in read-only runs
  (memory + report). With a custom working directory that folder is writable.
- Slack/Telegram conversations with a personal agent are not routed yet; the
  memory sources `[slack]`/`[telegram]` are ready for when they are.
- Rule enforcement is keyword-based (word-boundary match on the tool name and
  arguments) — it over-asks rather than under-asks.

**Failure notices.** The first failed run (or failed delivery) of a streak
posts one notification-center notice to the agent's creator; clicking it opens
the agent's Runs tab. A clean run ends the streak.

## 9. Troubleshooting

- **"this agent session is read-only" in a run** — the run is Proactive or its
  schedule's permission set is Read-only. Change the schedule to Directed, or
  use Run now (Directed), to let it act.
- **An action keeps asking even with an auto-approve rule** — it hit the
  sensitive-action gate or one of the agent's enforced rules; the approval's
  detail says which.

- **Agent runs with the wrong model** — check the agent's model field and that
  the provider has a model template (Settings → Providers); a template-less
  provider ignores the pin by design.
- **Model list empty/stale** — Settings → Providers → Models catalog →
  Refresh; check `last_error` in `GET /providers/models`.
- **No browser tools in a run** — the agent's Browser toggle must be on; the
  Playwright MCP is fetched via `npx @playwright/mcp@<pinned version>` (`PLAYWRIGHT_MCP_VERSION` in `otto-sessions/src/mcp.rs`, never `@latest`) unless `OTTO_BROWSER_MCP`
  points elsewhere.
- **Rooms: agent post rejected** — the agent isn't a member of the room, or the
  post exceeded 16 KB.
- **Rooms: an agent never posts** — check it is a member (Rooms view) and that it
  has run (or a new chat was opened) since it was added: membership reaches the
  agent through its persona file at session start.
