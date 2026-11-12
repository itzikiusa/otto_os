# Personal Agents

Grok-bot-style preset agents: named personas with a **pinned provider + model**,
one or more schedules, per-agent memory, optional browser use, channel
delivery, chat-anytime, and fully user-visible inter-agent rooms.

> Contracts: the "Personal Agents", "Agent rooms" and "Model catalog" sections
> of [`docs/contracts/api.md`](../contracts/api.md) are authoritative.

## 0. The home — one conversation per agent

`#/personal-agents` opens the **main agent's conversation** in the Focus layout
(`PersonalAgentsPage.svelte`). An explicit agent link selects that agent instead.

- **Agent switcher** — click the agent's name in the single toolbar to switch.
  The main agent comes first; live working, approval and paused states appear
  beside agent names. **New agent…** starts with name and purpose; tone, avatar
  and templates are optional details. Scheduled tasks and Agent channels stay
  available in the toolbar's **More** menu.
- **Conversation** (`AgentConversation.svelte`) — one continuous chat per agent.
  Before the first message, an intro and ready composer explain how to start.
  The latest completed result appears here when available, with its author,
  time and report link. Suggested prompts only fill the composer; Send starts the work. Live hand-offs
  show who is working and link to their thread. A paused agent explains that
  routines will not run and offers Resume; chat remains available.
- **Routines** — a labelled toolbar button opens a shared sheet containing the
  existing routines, their next run and timezone, pause/edit/run controls, live
  work, and pending instruction proposals. Add routine keeps the current
  permission and delivery review. Saving follows the existing routine form's
  resume semantics; opening the sheet never resumes an agent.
- **History** — a labelled toolbar button opens the complete run history, with
  results, reports, failures, Stop and retry/run actions. It remains one click
  away even while the routines sheet is closed.
- **Settings** — the gear opens settings over the conversation. About, Runs and
  Memory retain personality, provider/model, working folder, delivery, autonomy,
  owner restrictions, memory and background documents. Existing deep links
  (`#/personal-agents/<id>/<tab>`) open the matching sheet; routines/schedules
  open Routines, runs/history/activity open History, and other settings aliases
  keep their existing section mapping. Escape closes a sheet and returns focus;
  opening or closing it preserves the conversation and unsent chat draft.

Avatars are in-repo character art (`ui/src/lib/assets/agents`, one per
personality; stored as `avatar: "char:<id>"`; a glyph or initials still work)
and the Otto mascot (`ui/src/lib/assets/mascot`) in the empty and idle states.
They are emitted as files by Vite (never inlined) and load lazily.

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

A new workspace seeds one agent on first list: the **Personal Assistant**,
your main agent — **active**, owned by the Editor whose listing seeded it
(a Viewer's listing seeds it ownerless; the first Editor to change it, or
whose chat sets up a routine on it, adopts it). Other agents come from the
templates (`+ New agent`) or from the main agent's chat.

**Routines only fire on an active agent.** Setting one up — from chat
(`otto.routine_create`) or the Routines form — resumes a paused agent, since a
routine on a paused agent would show a next run and never fire (migration
`0204` resumes a paused main agent that already has an active routine). A
routine's times are in its timezone; omitted, that is this Mac's timezone
(`/etc/localtime`), never UTC.

## 2. Runs

The scheduler (60s tick, `crates/otto-assistant/src/personal_agents_scheduler.rs`)
fires due schedules; the engine (`personal_agents_engine.rs`) runs each as a
**fresh Threads thread** on the structured runtime when the agent's provider is
one Threads drives (`claude`, `codex`, `gemini`, `grok`, `opencode`, `pi` —
`personal_agent_threads.rs`): the thread is created in the agent's cwd with the
pinned model and the run's server-owned meta (`personal_agent = <id>`,
`run_id`, `read_only`, `browser`, `agent_mode` — the same confinement a
terminal run gets: forced Seatbelt + read-only tool list when read-only), sent
the prompt (persona note + directive + memory instructions + report-file
instruction) as one turn, and its report is the file the agent wrote — or the
turn's final answer. No progress for 10 min = `stuck`. The thread closes when
the run settles and stays in History (*Open thread* on the run). Any other
provider (`agy`, custom CLIs) — or a workspace that routes agents through a
network profile — runs as a **fresh terminal session** instead
(`personal_agent_session.rs`: pasted prompt, report-file watch, retries).
Either way the engine records a `PersonalAgentRun` (summary, report, delivery
state, session / thread id). Concurrency
is capped (`OTTO_PERSONAL_MAX_CONCURRENT`, default 2). Reports are kept for the
last 100 runs; run updates stream over WS. Notify-on-change compares the
last non-proactive successful run only when delivery succeeded or that run
intentionally skipped an unchanged report. A delivery failure does not suppress
the next identical report; proactive feed-only reports do not count as delivered.
The comparison also includes the destination. Older hashes without destination
identity trigger one fresh delivery after upgrade. Schedule-load failures keep
the last known cadence visible and offer Retry in Schedules.


Manual fire: **Run now** on the agent (or a specific schedule) →
`POST /personal-agents/{id}/run`.

**One run per agent at a time.** Every run of an agent works in the same folder
and rewrites the same `memory/notes.md`, so scheduled, manual and delegated runs
share one per-agent guard: a due schedule waits (its cursor untouched, so it fires
on a later tick) while another run is going, and Run now answers 409. A failed run
keeps its session linked (*Open thread*) and its error and any delivery failure
show inline on the run row. Each run's scratch report file is removed once read. A running run has **Stop…**
(`POST /personal-agents/runs/{run_id}/cancel`): its thread is closed / its session killed (no retry)
and it settles `canceled`.

**Arming.** A schedule never fires an occurrence from before its `armed_at` — set
on create, on resume (the schedule's or the whole agent's), and when its cadence or
timezone really changes. Re-enabling an agent after a week therefore doesn't fire
(and deliver) the week's missed recaps at once. A `once` schedule — the Assistant
creates them; the schedule form now supports them too — fires again after its
`run_at` is edited and it is re-enabled.

## 3. Chat anytime — the chat never dies, and everything lands in it

`POST /personal-agents/{id}/chat-session` returns (creating if absent or
closed) the caller's single chat with the agent — same persona cwd, same pinned
provider/model. On a Threads provider it is a **Threads thread** (`runtime:
true`) and the agent page's **Chat** tab renders the Threads conversation view
(streaming answer, work log, approvals inline, the composer) through
`ui/src/lib/runtimeThreadPort.ts`; **Open in Threads** hands it to the Threads
page. Other providers keep an embedded terminal.

**Permanent.** The caller's own pinned chat that was closed is **reopened in
place** (`AssistantCtx::reopen_thread` → `AgentRuntime::reopen`: same history,
the provider's session resumed) instead of a fresh empty thread; a CLI that
exited respawns on the next prompt. (Until the runtime's reopen lands on the
daemon build, the hook refuses and a fresh chat is opened, as before.)

**Everything reports into it** (`personal_agent_feed.rs`). Every finished run
— scheduled, manual or proactive, ok / failed / stopped — posts an attributed
line into the owner's chat with the agent ("Morning brief ran · Two PRs need
review [otto-run:<run id>]"), as a system notice (`AgentRuntime::notice`); the
chat renders the `[otto-run:…]` marker as a result card with **Open report**.
A scheduled or manual result is then also sent as a framed follow-up turn
(`[Message from Scheduled · Morning brief (<run thread>)]: …` carrying the
report and the current time), so the agent itself knows what its routine found
and adds, in at most two lines, what else matters — a notice alone is UI-only, and the agent used to
answer "what did my brief say?" with "no such routine". Proactive runs never
cost a turn. A busy chat is waited for (≤ 30 min); a closed one is reopened
first.

## 3a. The main agent

One agent per workspace is the **main agent** — your one point of contact
("Task me with anything; I'll make sure it gets to the right agent"). A new
workspace seeds it (active, with the `chief` personality); the
`main-agent` template and the quick create's "Main agent" start make one; any
agent becomes it from Details → *Make main agent*. Its persona lists your
other agents and the team tools:

- `otto.personal_agent_list` — the team (id, name, what each does);
- `otto.personal_agent_create` — when you ask, or a recurring kind of work no
  agent covers: a new enabled agent owned by you, `created_by_agent` = the main
  agent (shown on the agent);
- `otto.personal_agent_delegate` — **non-blocking**: opens the agent as a
  linked thread of the chat (its own persona folder, as itself, never wider
  than the chat), and the main agent ends its turn.

**Never dropped.** Every hand-off is a `personal_agent_delegations` row
(0203) until its outcome is IN the main agent's chat. When the delegated turn
ends, the outcome is posted there — a notice ("Report from Scout — done:
Build is green") **and** the main agent's next turn with the report, told to
relay its actual substance and listing the hand-offs still out (so it never
re-sends or polls them). A failure (including a turn that ended with tool
steps but no written reply), a stop, "still working after 4 h" and "an Otto
restart interrupted it" (boot recovery settles open rows and delivers what is
owed) all reach the chat too — the main agent never silently stops. The
hand-off chips and the Activity tab show each one's live status
(`GET /personal-agents/{id}/delegations`).

What changed from the first version, and why: the main agent had to be set by
hand (none existed by default, so "the main agent" did nothing); its prompt
told it to handle things itself and gave no rules for reporting back, so it
said work was "done" when it was only assigned, and answered "the report came
through" instead of the substance; a delegated failure, an empty reply, a
closed chat or a restart dropped the report silently (an in-memory relay with
`warn!` on every error path); and routines needed the approval-gated admin
tools. Now: a seeded main agent, chief-of-staff prompt rules ("assigned, not
done", relay the substance, offer a retry ≤ 2 and never retry an outward
step), a persisted relay that always ends in the chat, and own-scope routine
tools (§3c).

## 3b. Agent admin — an agent sets agents up for you

Any agent in your own session or thread (not only the main agent) can set up
and manage your personal agents with the **Agent admin** MCP tools — e.g. "make
me a Slack helper from the *Slack daily digest (as me)* template, cron
`0 7 * * 0-5` in my timezone, on claude, make it my main agent, let it act as
me in my Slack workspace, then run it":

- reads (on by default): `otto.list_personal_agent_templates`,
  `otto.list_personal_agents`, `otto.get_personal_agent` (config, schedules,
  autonomy incl. `slack_as_me_teams`, latest runs),
  `otto.list_personal_agent_runs`, `otto.get_personal_agent_report`;
- writes (off by default — MCP → Otto server → **Agent admin** → All):
  `otto.create_personal_agent` (from a `template`, with `schedules`,
  `timezone`, `primary` in ONE call), `update_personal_agent` (`enabled`
  pauses / resumes), `delete_personal_agent`, `add_` / `update_` /
  `remove_personal_agent_schedule`, `set_personal_agent_autonomy`,
  `set_personal_agent_slack_as_me`, `run_personal_agent`.

Every write asks you first; the approval card says exactly what is created or
changed — the persona (summary, or a line diff against the stored / template
persona), each schedule (cadence, timezone, read-only vs may-act), main
agent, delivery. The approved call runs as your session (you own the agent;
a read-only session is refused). **Act as me in Slack** is stricter: it
always asks (no auto-approve rule or token write grant covers it), and only
the call you approved runs — as you; an agent's own token is still refused
by `PUT …/slack-as-me`. Why writes are off by default: the catalog invariant
"mutating ⇒ approval-gated ⇒ off by default" holds for every write tool;
enabling the category is one click, after which every call still needs your
approval. Templates have one source, `docs/contracts/personal-agent-templates.json`
(the New-agent sheet imports it; `GET /personal-agents/templates` serves it).

## 3c. An agent's own scope — routines, personality, learning

Every personal agent manages **its own** routines straight from chat, with
**no approval card** (they change nothing outside the agent, and every change
shows in its Routines at once via `personal_agent_changed`). The main agent
may manage its team's routines too (`agent: "<name>"`).

- `otto.routine_create` — "every weekday at 9 Israel time, send me a Slack
  message with …": the agent turns your words into a schedule (`daily`,
  `weekly`, `days` with `days: "sun-thu"` / `"weekdays"` / a list, `interval`,
  `cron`, `once`), the timezone defaults to yours (this Mac's), and the result
  goes to the chat — or `deliver: {type: "slack", channel, as: "bot"|"me"}`
  (as the Otto bot via the routine's own destination, `delivery_json`; as you
  only with "Can act as me in Slack"). It answers with the next 3 runs.
- `otto.routine_list` / `otto.routine_update` (`enabled: false` pauses) /
  `otto.routine_delete`.
- `otto.personality_set` — "be more blunt" (tone only; applies from its next
  session).
- `otto.instructions_propose` — the agent may PROPOSE new standing
  instructions; Details shows **Update instructions?** with From / To, *Apply*
  / *Edit first* / *Dismiss* (owner only), and *Undo* after applying. Agents
  never rewrite their instructions themselves.

**Personality** — a preset from `docs/contracts/personal-agent-personalities.json`
(chief of staff, warm, blunt, nerdy, cynical, very strict, free-willed,
autodidact, playful, formal, terse) plus your own words, rendered into the
persona as "Your personality". It shapes tone in chat, reports and messages to
other agents — never rules, permissions or approvals. Pick it at create time
or in Details (with a sample answer).

**Learning** — every persona tells the agent to keep `memory/notes.md` small
and curated (one fact per line, rewrite rather than append, ≤ ~60 lines) and
to record your corrections and preferences there; Details → *What it has
learned* lists those items (edit / forget).

All six tools are on by default (Sessions group), refused in read-only runs,
and hidden from narrow review sessions.

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

**Can act as me in Slack** (Autonomy tab, owner only, default off): which of
the owner's connected Slack workspaces (Settings → Channels → *Act as me in
Slack*) the agent may use the as-you Slack tools in — read DMs, search, post
(approval-gated every time) and mark unread. Stored as `slack_as_me_teams` in
the autonomy config, set via `PUT /personal-agents/{id}/slack-as-me` from a
person's credential only (an agent can't grant itself). Without it every
`as=user` Slack call from the agent's sessions is refused (403). The *Slack
daily digest (as me)* template uses it — see
[Channels §7b](./channels-slack-telegram.md#7b-slack-tools-for-agents--as-the-bot-or-as-you).

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

Sidebar → **Personal Agents**: the home (§0) → *All settings* opens the agent
page tabs **Overview / Activity / Autonomy / Schedules / Runs / Chat /
Memory / Context**, plus a module-level **Rooms** view (live feed, membership
editor, user post box).

**In Threads** an agent's threads are grouped under the agent's **name** (from
the thread meta `personal_agent_name`, or the daemon-given "Chat: X" / "Agent:
X" title) — never its folder's id — and each row carries what started it:
**Chat**, **Scheduled · <routine>**, **Manual run**, **Proactive** or
**Delegated by <main agent>** (`ui/src/lib/personalAgentBadges.ts`, shared
with the agent page's run lists). Schedules carry a *Read-only* chip when their
permission set is read-only; runs show their mode and a read-only lock.

An agent with UI control can drive this page visibly with the `otto.ui_pa_*`
commands ([agent-ui-control.md §5](agent-ui-control.md)): list / open an agent
on a tab, fill and save the New-agent sheet (template, persona, provider,
schedule), edit fields, set autonomy, add / edit / pause schedules, Run now and
read a run's report — each write after your attributed confirm. Granting
"Can act as me in Slack" (`pa_set_slack_as_me`) always asks with the outward
where / what / who confirm and is never remembered. The templates come from
`docs/contracts/personal-agent-templates.json` (the sheet and the daemon read
the same file).

## 8. Capabilities & limits (v1)

- The chat's reopen and turn-less notices need the runtime's `reopen` /
  `notice` (otto-agent-protocol); until the daemon build has them the
  `AssistantCtx` hooks refuse, a closed chat is replaced by a fresh one, and a
  run result is posted as a framed follow-up turn instead of a notice.
- The conversation view shows a `[Message from …]` post as a plain user
  bubble with its header (an attributed card and date separators are a
  Threads rendering follow-up).
- Workspace **Scheduled Tasks** are a separate automation and are not linked
  to an agent; Details points to them.

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

- **403 "This agent runs as its owner; only the owner or a workspace admin can
  change …"** — the agent runs with its creator's identity, so its owner-only
  fields (`soul_md`, provider, model, cwd, browser, delivery, its memory and
  context (`PUT …/memory`, `PUT …/context`), its memory items (edit / forget,
  `POST …/memories/edit`), its standing-goal texts and rules (`PUT …/autonomy`
  when the goals added/removed/edited or the rule set changes), and a
  schedule's directive and permission) can be changed only by the owner, a
  workspace Admin or root (`PATCH /personal-agents/{id}`,
  `PUT /personal-agents/{id}/memory`, `PUT /personal-agents/{id}/context`,
  `POST /personal-agents/{id}/memories/edit`, `PUT /personal-agents/{id}/autonomy`,
  `PATCH /personal-agents/schedules/{id}`). The page hides those editors from
  other Editors and says "Runs as its owner; only the owner or a workspace
  admin can change this."
  An ownerless preset is adopted by the first caller who writes it. Any Editor can still rename it,
  change its avatar, enable or pause it, change cadence, Run now (a goal's
  "Work on it now" too — the run acts as the owner), toggle a goal's
  `enabled`, proactive on/budget and `primary`, cancel a run and manage rooms.
- **403 "an agent cannot change its own …"** — the same owner-only fields
  (and applying / undoing an instructions proposal, a personality note) need a
  PERSON's credential: an agent session's own token or MCP credential acts as
  its owner, so it is refused outright. An agent proposes new instructions
  with `otto.instructions_propose`; the person decides on the page.
- **A routine update asks for approval** — `otto.routine_update` (and the REST
  `PATCH /personal-agents/routines/{id}` from an agent token) on a routine that
  ALREADY posts outside the chat is judged against its stored destination:
  any change other than a pause needs the owner's approval card.
- **400 "gemini can't load a persona for one launch …"** — a personal agent
  runs on Claude, Codex, agy, Grok or opencode only: Gemini, Pi and custom CLIs
  have no per-launch context file, so they would run without the persona. The
  editor only offers the supported providers and explains a saved other one.
- **403 "This agent runs as its owner; only the owner or a workspace admin can
  give it a directive."** from Assistant delegation (`POST
  /assistant/threads/{id}/delegate`, the `assistant_delegate` tool) — a
  delegation starts a Directed run of your text as the agent's owner, so it
  needs the owner, an ownerless agent, a workspace Admin or root (the same
  rule as the agent's directives, goals and rules). No run is created. When
  two agents share the name, delegation picks your own; a name that matches
  nothing is a 404.
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
