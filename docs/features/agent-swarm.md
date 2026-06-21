# Agent Swarm

> A team of role-specialized AI agents — organized as an org chart (CEO → CTO →
> VP → Team Lead → Devs) — that autonomously works **projects** broken into
> **tasks**. A built-in **recruiter** drafts each agent (role, persona, skills,
> schedule); a per-swarm **coordinator** schedules ready work onto agents within a
> parallel-session cap, has leaders delegate to their reports, and routes
> hand-offs and reviews; a shared **board** lets the team coordinate live. Every
> agent runs as a normal, openable Otto session. Five preset swarms ship in the
> box.

This document is the end-user and operator reference for Agent Swarm: the
concepts, a step-by-step "set up your company" how-to, a full walkthrough of
every view and control, the real REST/WS contract, capabilities and limits, the
token-efficiency model, security, and troubleshooting. It describes what the
code actually does (crate `otto-swarm`, the runtime in `otto-server`, the UI in
`ui/src/modules/swarm/`, contracts in `docs/contracts/{api.md,ws.md}`).

> **Terminology note.** Otto calls these teams **swarms**, never "companies."
> "Setting up your company" below is shorthand for *building the org* — the swarm,
> its agents, and its hierarchy. The product surface is labelled **Swarms**.

---

## 1. Overview

Agent Swarm turns Otto from a single-agent terminal into a small autonomous
organization. You create a **swarm** (a named team inside a workspace), give it
an **org hierarchy** of role agents, define **projects** with goals, break those
goals into **tasks** (by hand or with the AI planner), and **start** the swarm.
From then on a background **coordinator** keeps ready tasks flowing onto the
right agents — running them as real agent sessions — until the work is done, the
budget is hit, or you pause it.

### Where it lives

| Layer | Location |
|-------|----------|
| Domain types + API router (CRUD/board) | `crates/otto-swarm/` (`types.rs`, `service.rs`, `http.rs`, `recruiter.rs`, `presets.rs`) |
| Preset org charts (5 YAML templates) | `crates/otto-swarm/assets/presets/*.yaml` |
| Persistence (rows, queries, migrations) | `crates/otto-state/src/swarm.rs`; migrations `0029_agent_swarm.sql`, `0034_swarm_budgets.sql`, `0035_swarm_project_story_link.sql`, `0037_swarm_project_story_unique.sql` |
| Coordinator runtime + lifecycle + recruit/plan | `crates/otto-server/src/swarm_runtime.rs` |
| One agent turn (spawn/resume, brief, watch, parse) | `crates/otto-server/src/swarm_run.rs` |
| Per-agent cwd + identity + `otto-post` helper | `crates/otto-server/src/swarm_workspace.rs` |
| Scheduled runs (daily/weekly/interval) | `crates/otto-server/src/swarm_scheduler.rs` |
| Board ingest (agent → board) | `crates/otto-server/src/routes/swarm_ingest.rs` |
| Product story → swarm project bridge | `crates/otto-server/src/product_swarm.rs` |
| UI section + views | `ui/src/modules/swarm/` |
| UI store (REST + WS state) | `ui/src/lib/stores/swarm.svelte.ts` |
| Contracts (authoritative) | `docs/contracts/api.md` (#59–#86, "Swarm lifecycle"), `docs/contracts/ws.md` (Agent Swarm events) |

Open it from the app's **Swarms** section.

---

## 2. Concepts & org model

A **swarm** is the team. It has a name, an optional mission (`description`),
a `status` (`paused` | `active` | `aborted`), a `config` JSON (provider default,
`max_parallel_sessions`, `cwd_mode`, `auto_submit`), and a set of budget
guardrails. It belongs to exactly one **workspace**.

```
Swarm  (the team, e.g. "Startup Pod")
 ├─ Agents       — the org chart (reports_to forms the tree; NULL = top, e.g. CEO)
 │   CEO
 │    └─ CTO
 │        ├─ Full-stack Dev
 │        ├─ Full-stack Dev
 │        ├─ UI/UX Designer
 │        └─ QA
 ├─ Projects     — units of work, each with its OWN Kanban board
 │   └─ Tasks    — cards; assignable; may depend on each other (DAG edges)
 ├─ Runs         — one agent "turn" per scheduled/manual/scheduled-cadence execution
 └─ Board        — the shared message stream the team posts to (you watch it live)
```

**Agent.** A member of the swarm: `name` (human name), `title` (role label —
"CTO", "Backend Dev"), `reports_to` (its manager's agent id; `NULL` = top of the
org), `provider` (which CLI runs it — `claude`, `codex`, …), optional `model`,
a `soul_md` persona (background + traits) and/or a library `soul_name`, a
`specialization`, a `scope_md` (what it owns / boundaries), a `skills` list
(`[{name, must_use}]`), an optional `schedule`, a `cwd_mode`, an `avatar`
(emoji), and a `status` (`active` | `paused`). An agent with reports is a
**leader**: when assigned a task it *delegates* rather than doing the hands-on
work.

**Project.** A named work area inside a swarm with its own Kanban board. Has a
`description`, an optional `repo_path` (for code work / git worktrees), and a
`goal_md` (the high-level goal — the input the planner breaks into tasks). May
carry a `story_id` back-link when it was created from a Product story.

**Task.** A Kanban card belonging to a project. Has `title`, `description`
(acceptance / "what done looks like"), an optional `assignee_agent_id`,
`status`, `priority` (`low|medium|high|urgent`), `depends_on` (task ids that must
finish first — the DAG edges), an optional `parent_task_id` (delegation/handoff
ancestry), `labels`, a `delegated` flag, an `attempts` counter, and a
`result_ref`. Task `status` is one of:

`backlog` → `todo` → `in_progress` → `in_review` → `done`
(plus `blocked` and `cancelled`).

Only **`todo`** tasks with **all dependencies `done`** are *ready* and picked up
by the coordinator (`SwarmRepo::ready_tasks`). `backlog` is a parking lot — move
a card to **To do** to make it schedulable.

**Run.** One agent *turn*: a single execution of an agent against a task (or a
delegation, a review, or a scheduled directive). A run has a `kind`
(`task` | `planning` | `scheduled`), a `trigger` (`coordinator` | `manual` |
`scheduled`), a `status` (`queued` → `running` → `waiting`/`done`/`error`/
`stopped`), a `session_id` (the real Otto session it ran in — openable), a parsed
`result`, a `summary`, optional `tokens_input`/`tokens_output`/`cost_usd`, and
timestamps. The Coordinator keeps **one active run per agent at a time**.

**Board.** A per-swarm message stream. Agents post to it with the `otto-post`
helper as they work; humans post from the Feed. Message `kind`s: `message`,
`idea`, `review_request`, `review`, `decision`, `status`, `concern`,
`escalation`, `handoff`, `system`. Messages can be addressed to a specific agent
(`to_agent_id`) and tagged with a project/task/run.

---

## 3. Setting up your company (step-by-step)

This is the practical how-to: stand up a swarm, build the org, give it work,
and run it.

### Step 0 — Open Swarms

Go to the **Swarms** section. With no swarms yet you see the empty state:

> **Build an agent swarm** — *A team of role-specialized agents that work
> projects together — pick a preset or start blank.*

Click **New swarm**.

### Step 1 — Create the swarm (preset or blank)

The **New swarm** dialog asks for:

- **Name** (required) — e.g. *"Platform Team"* (placeholder: *"e.g. Platform Team"*).
- **Start from a preset** — pick one of the five shipped templates (each shown as
  *"{name} · {N} agents"* with its description), or **Blank** (*"Start empty and
  recruit your own agents."*).

Click **Create**. A preset instantiates the whole org (agents wired by
`reports_to`, projects, schedules) and maps each agent's provider to an installed
CLI, falling back to the workspace default — so a preset never creates an agent
on a provider you don't have. A blank swarm is created empty, ready for you to
recruit into. Either way, the new swarm opens to the **Org** view, `paused`.

> Budgets and the parallel cap come from the preset (or sensible defaults for a
> blank swarm) — see §8. You can change them later.

### Step 2 — Build the org hierarchy (agents)

There are two ways to add agents; you can mix them.

**a) Recruit (AI-drafted).** Click **Recruit**. The two-step **Recruit an agent**
wizard opens:

1. *"What role do you want to hire?"* (e.g. *"CTO", "Backend Dev", "UX
   Researcher"*) plus an optional *"Any specifics? (optional)"* context box.
   The recruiter *"proposes a soul, skills, provider and schedule — you can edit
   everything before hiring."* Click **Propose agent** (shows *"Recruiting…"*).
2. The recruiter returns a complete draft — **Name, Title, Provider, Reports to,
   Specialization, Soul, Scope, Skills** (with ★ = must-use), and an optional
   suggested **schedule**. Edit anything, then click **Hire** (or **Back** to
   refine the role/context).

The recruiter only proposes skills that exist in your installed library (invented
skills are dropped) and only providers you actually have installed.

**b) Add agent manually.** In the **Org** view, click **Add agent** (top level) or
the **+** on a row to add a direct report. The **Agent editor** opens with every
field editable:

- **Name** (required), **Title / role** (*"e.g. CTO"*), **Provider**
  (e.g. claude / codex / …), **Reports to** (dropdown; default *"— top of
  org —"*), **Avatar (emoji)**.
- **Specialization**, **Soul (background + traits)**, **Scope (what they own)**.
- **Skills (must-use toggle)** — type a skill name (autocompletes from your
  library), **Add**, then toggle the ★/☆ star to mark it must-use.
- **Scheduled runs** — enable a cadence (*every N minutes* / *daily* / *weekly*),
  set the time/weekday, and a *"Standing directive (what to do each run)…"*.

Click **Hire** (new) or **Save** (editing).

**Shape the tree.** In the Org view you can **drag** an agent onto another to make
it that agent's report (cycles are prevented), or drag to the root drop zone
(*"Drop here to make top-level"*) to promote it. Right-click a row for **Edit
agent**, **Run a task…**, **Add direct report**, **Move to top level**, or
**Delete agent**.

> **The hierarchy is the delegation graph.** An agent with at least one report is
> treated as a **leader**: when the coordinator assigns it a task it runs a
> *planning* turn that breaks the task into subtasks for its reports, rather than
> doing the work itself. Leaf agents (no reports) do the hands-on work.

### Step 3 — Create a project

Click **Project** (header). The **New project** dialog asks for **Name**,
**Repo path (optional)** (a local git repo — required for `worktree`/`repo`
cwd modes and code work), and **Goal (optional)** (the high-level goal the
planner will break into tasks). Click **Create**. The view switches to the
project's Kanban **Board**.

### Step 4 — Plan the project into tasks

On the project's Kanban board you have two options:

- **Plan from goal** (tooltip: *"Break the project goal into tasks"*) — runs the
  AI planner against the project's `goal_md`. It produces a focused task
  breakdown, assigns each task to the best-matching role by title, and wires
  `depends_on` so independent work can run in parallel. (The project must have a
  goal; an empty goal returns an error.)
- **Add task** — type a **Task title…** and click **Add**. A hand-added task
  lands as **To do** (immediately schedulable). Right-click any card to **Run
  now**, **Move to** a column, **Assign to** an agent, or **Delete**.

Tasks you want the swarm to pick up must be in **To do** with their dependencies
**Done** (drag from **Backlog** to **To do** when ready).

### Step 5 — Set the pace (optional)

In the header, set **parallel** (the `max_parallel_sessions` cap — how many
agent turns run concurrently; min 1). Review the **run** and **cost** budget
bars. Adjust budgets later via the swarm patch / **Raise budget & resume**.

### Step 6 — Start it

Click **Start**. Status flips to **active**; the coordinator begins scheduling
ready tasks onto agents, leaders delegate, hand-offs and reviews spawn follow-up
tasks, and the team posts to the **Feed** as it works. Watch progress in **Org**
(live status dots + active-run badges), **Graph** (the DAG), **Board** (Kanban),
**Runs** (every turn), and **Feed** (the board).

Pause, abort, or resume at any time (§5.6). Open any agent's session to watch it
work in a real terminal (§5.3).

> **Shortcut: from a Product story.** If you use the Product module, a refined
> story can be pushed straight into a swarm as a project with its tasks already
> seeded (Plan → Swarm). See §5.8 and `./product.md`.

---

## 4. The coordinator (how autonomous work actually runs)

When a swarm is **active**, `start_coordinator` spawns a per-swarm background loop
(`swarm_runtime::coordinator_loop`) that **ticks every 5 seconds** (with
responsive 500 ms cancel slices). Each tick:

1. **Re-reads the swarm.** If it isn't `active`, the tick no-ops.
2. **Checks budgets first.** If any per-swarm budget is exhausted
   (`budget_exceeded`), it **auto-pauses** the swarm with a human-facing
   `pause_reason`, suspends idle swarm sessions, posts a `system` note to the
   board, and raises a warning notice — instead of scheduling more work (§8).
3. **Computes free capacity.** `budget = max_parallel_sessions − active_runs`
   (active = `queued|running|waiting`). If 0, it waits for the next tick.
4. **Selects ready tasks** (`todo` with all `depends_on` done) and, for each,
   within the remaining capacity and the projected `max_total_runs` ceiling:
   - **Picks the agent** (`pick_agent`): the task's explicit assignee if active;
     else the best-fit active agent by title/specialization keyword overlap with
     the task title+description; else any active agent.
   - **Skips** the agent if it already has an active run (one turn per agent).
   - **Bumps the task's attempt counter**, claims the task to `in_progress` so it
     isn't re-selected, and **creates a run**: `kind = "planning"` if the agent is
     a leader and the task isn't yet `delegated`, else `"task"`.
   - **Spawns the turn** (`swarm_run::run_turn`) on a background task; when it
     returns, `route_result` applies the outcome.

### Routing a finished turn (`route_result`)

The agent writes a structured JSON result (schema below). The coordinator routes
it:

- **Concerns** → posted to the board as `concern` + a warning notice ("the plan/
  timeline looks wrong").
- **Delegation (a `planning` turn)** → the leader's returned `subtasks` become
  real `todo` tasks for its reports (matched by `assignee_role` → title); the
  parent task is marked `delegated`. If a leader returns nothing to delegate, it
  is allowed to act as an individual contributor next time.
- **Hand-offs** → each becomes a new `todo` task assigned to the named role,
  labelled `handoff`.
- **Status applied to the task:**
  - `done` with reviews requested → task → **`in_review`** and a review task is
    created for each named reviewer (labelled `review`, high priority).
  - `done` (no review) → task → **`done`**; if all of a parent's children are
    done, the parent rolls up to done too (recursively).
  - `needs_review` → **`in_review`** + review tasks.
  - `blocked` → **`blocked`**.
  - `in_progress`/unknown → re-queued to `todo` for another turn **unless** the
    task hit its attempt ceiling, in which case it is **`blocked`** with an
    `escalation` board post + notice (§8).
- A non-empty `summary` is posted to the board as a `status` message.

### The per-turn result schema

Each agent is told (in its brief) to write a single JSON object to a given file
path as the last thing it does:

```json
{
  "status": "done | blocked | needs_review | in_progress",
  "summary": "one or two sentences on what you did",
  "artifacts": [{"type":"file|pr|doc|url","path":"abs path or null","url":"url or null","label":"short"}],
  "handoffs": [{"to_role":"a teammate's title","brief":"what they should do"}],
  "reviews_requested": [{"of":"artifact label/path","reviewer_role":"a teammate's title"}],
  "subtasks": [{"title":"...","description":"...","assignee_role":"title or null","priority":"low|medium|high","depends_on_titles":[]}],
  "concerns": [{"severity":"low|medium|high","text":"if the plan/timeline looks wrong"}]
}
```

---

## 5. Full feature walkthrough

### 5.1 The Swarm page layout

`SwarmPage.svelte` is the section shell:

- **Swarms rail** (left; a collapsible accordion on phones): the list of swarms
  with a status dot each, a **+** (**New swarm**) button, and the empty state
  *"No swarms yet."* Click a swarm to open it.
- **Header** (when a swarm is open): the swarm name, a status pill
  (**active** / **paused** / **aborted**), a counts line — *"{agents} agents ·
  {projects} projects · {running} running · {queued} queued"* — the **run** and
  **cost** budget bars (turn red past ~80%), the **parallel** cap input, the
  lifecycle buttons, **Recruit**, **Project**, and a trash icon (delete swarm,
  with confirmation). A paused-by-budget swarm also shows *"Paused: {reason}"*.
- **View tabs** (horizontally scrollable on phones): **Org**, **Graph**,
  **Board**, **Runs**, **Feed**.
- **Body**: the selected view, with the **session panel** opening alongside it
  (right on desktop, stacked below on phone) whenever you open an agent/run
  session.

### 5.2 Org Tree (`OrgTree.svelte`)

The recursive org chart. Each row shows the avatar, name + title, a **scheduled**
clock badge if the agent has a schedule, an active-runs badge (*"{n}●"*), and a
status dot (working / idle / paused-exited). Expand a row (chevron) to see its
direct reports and its open sessions nested beneath. Click an agent's session row
to open it in the session panel. Drag to reparent; right-click for the agent
menu. Empty state: *"No agents yet. Use **Recruit** to add one."*

### 5.3 Agents run as normal sessions

Every agent turn runs in a real Otto **Agent session** (PTY running the agent's
provider CLI). The session is created on the agent's first turn and **reused**
across later turns — the run row carries its `session_id`, and you can **open** it
live from the Org tree (session rows), the **Runs** list (**Open**), the **Run
Graph** (a node tagged *"· open"*), or the **Run Inspector** (**Open session**).
Opening it gives you the same terminal you'd get for any agent session — see
`./agent-sessions.md`.

### 5.4 Run Graph (`RunGraph.svelte`)

A live left-to-right DAG of task/run nodes laid out by dependency depth, with
edges for `depends` (solid), `handoff`, and `review` (dashed). Node cards show a
status badge, label, and agent. Pan by dragging the background; zoom with the
wheel or the **+ / − / fit** controls. Click a node to **inspect** its latest run
(opens the Run Inspector) or **open** its session. Empty state: *"No work yet —
Tasks and their dependencies show up here as a live graph."*

### 5.5 Kanban + Runs

**Kanban (`KanbanBoard.svelte`).** The per-project board. Columns in order:
**Backlog**, **To do**, **In progress**, **In review**, **Blocked**, **Done**,
**Cancelled**. Drag cards between columns to change status; **Add task**; **Plan
from goal**; right-click a card to **Run now**, **Move to**, **Assign to**, or
**Delete**. If multiple projects exist, pick one from the toolbar dropdown. If the
project came from a Product story, a **From story** card (`StoryLinkCard.svelte`)
appears with the story title, its stage, and a **View story** button. Empty
state: *"No project selected — Create a project to start a board."*

**Runs (`RunsList.svelte`).** A filterable table of every run: columns Agent,
Work (kind + summary), Status, Started, Tokens (`in/out`), Actions. Filter by
assignee, project, and status chips (`all`, `queued`, `running`, `waiting`,
`done`, `error`, `stopped`). Per-row: **Inspect run** (eye), **Open** (the
session, if any), **Stop** (an active run). Empty state: *"No runs — Runs appear
here as agents work tasks."*

**Run Inspector (`RunInspector.svelte`).** A drawer for one run showing: the
header (agent · kind · status, **Open session**), enqueued/started/finished
times, any error, **Input / Output / Cost** stats (or *"No usage recorded for
this run (usage tracking off, or not flushed yet)."*), **Findings** (concerns),
**Summary**, **Working directory** (the run's `cwd`, copyable), **Artifacts**
(clickable file/PR/url rows), **Brief sent** (the exact prompt, copyable),
**Board posts** tagged with this run, and the **Raw result** JSON. It is a pure
client view — no extra endpoint.

### 5.6 The shared Board / Feed (`BoardFeed.svelte`)

The live team stream. Each message shows a colored **kind** badge, the author
(agent name, *"you"*, or *"system"*), an optional *"→ {agent}"* direction, a
relative time, and the body. Filter chips: *all / idea / review / decision /
concern / status*. To post yourself: pick a **kind** from the dropdown (default
*message*), type into *"Post to the team board…"*, and click **Post** (or Enter).
Empty state: *"Quiet board — Agents post ideas, reviews and decisions here as
they work."*

Agents post via a materialized **`otto-post`** shell helper placed in each
agent's working directory:

```sh
./otto-post --kind idea "..."                      # float an idea
./otto-post --kind review_request --to <agent-id> "..."   # ask for a review
./otto-post --kind review "..."                    # post a review of someone's work
./otto-post --kind decision "..."                  # record a decision
./otto-post --kind concern "..."                   # flag a wrong plan/timeline
```

It posts to `POST /api/v1/ingest/swarm/board`, gated by the session's ingest
token (§9).

### 5.7 Scheduled runs (`swarm_scheduler.rs`)

Agents can carry a **schedule** so they run on a cadence with a *standing
directive*, independent of the task board — e.g. a **daily trend researcher** or a
**periodic PM status report**. The scheduler scans every 60 s and fires an agent
when due, **only if** its swarm is `active`, the parallel cap has room, and the
agent has no active run. Cadences (times are **UTC**):

- `interval` — `every_min` minutes.
- `daily` — `at` "HH:MM".
- `weekly` — `weekday` (0 = Monday) at `at` "HH:MM".

The scheduler advances the agent's `schedule.last_run` cursor *before* firing so a
slow run can't double-fire, and creates a `kind="scheduled"` run executed with the
agent's directive. (Preset examples: Product Studio's researcher posts a daily 09:00
digest; the Engineering Squad PM reports every 240 minutes; Security & Audit's
compliance reporter runs weekly Monday 09:00.)

### 5.8 Product → Swarm (Plan hand-off)

`POST /api/v1/product/stories/{sid}/to-swarm` turns a refined Product story into a
runnable swarm project: it resolves (or auto-creates) a target swarm, sets the
project `goal_md` from the story's refined body, **seeds tasks** by parsing the
story's existing implementation plan (`### Task N:` headings) — or, if there's no
plan, by running the swarm planner over the goal — and creates the project with a
`story_id` back-link (idempotent: re-sending returns the existing project). The
reverse link is `GET /api/v1/swarm/tasks/{tid}/story` (the **From story** card).
See `./product.md`.

### 5.9 Lifecycle (start / pause / abort / resume)

From the header (`SwarmPage`) or the API (§6):

- **Start** — set status `active`, anchor the wall-clock runtime budget
  (`run_started_at`), clear any prior `pause_reason`, and (re)start the
  Coordinator. Gated by the workspace usage budget (blocked if over cap).
- **Pause** — status `paused`; stop scheduling new turns and **suspend idle swarm
  sessions** (frees RAM, resume-friendly). In-flight turns finish.
- **Abort all** — status `aborted`; cancel queued/running runs (mark `stopped`)
  and **kill** the swarm's sessions.
- **Resume** — back to `active`; restart the Coordinator (re-anchors the runtime
  clock). Also gated by the workspace usage budget.
- **Raise budget & resume** — shown when a swarm was auto-paused by a budget; bumps
  `max_total_runs` (and optionally cost) and resumes in one step.

**Crash recovery.** On daemon start, swarm runs left `queued`/`running`/`waiting`
by a previous process are marked `error` (their background task died with the
process) before any coordinator is restored — so they don't permanently consume
the parallel cap or block an agent.

### 5.10 The five preset swarms

Shipped as YAML org charts in `crates/otto-swarm/assets/presets/` and listed by
`GET /api/v1/swarm/presets`:

| Preset (slug) | Cap | Org (agents → who they report to) |
|---|---|---|
| **Startup Pod** (`startup-pod`) | 3 | CEO (Dana) → CTO (Ravi) → 2 Full-stack Devs (Mei/claude, Theo/codex), UI/UX Designer (Lina), QA (Sam). Project: **MVP**. |
| **Engineering Squad** (`engineering-squad`) | 4 | VP R&D (Olek) → Team Lead (Bea) → Backend Devs (Hugo/claude, Yuki/codex), Frontend Dev (Pat), DevOps/SRE (Sol), Code Reviewer (Ada); Project Manager (Quinn → VP, scheduled every 240 min). Project: **Delivery**. |
| **Product Studio** (`product-studio`) | 3 | Head of Product (Priya) → UX Lead (Noah) → 2 UI Designers (Iris/claude, Kenji/codex, cross-review), Frontend Dev (Ana); Product Researcher (Tom → Priya, **daily 09:00 trends digest**); Tech Writer (Gabe). Project: **Redesign**. |
| **Research Lab** (`research-lab`) | 3 | Research Director (Wren) → Trend Researcher (Kai, **daily 08:30**), Market Analyst (Rosa), Data Analyst (Idris), Prototyper (Vee), Writer (Eli). Project: **Exploration**. |
| **Security & Audit** (`security-audit`) | 3 | CISO (Mara) → Security Lead (Ivo) → 2 Code Auditors (Nina/claude, Drew/codex, cross-review), Threat Modeler (Sole); Compliance Reporter (Bo → CISO, **weekly Mon 09:00**). Project: **Audit**. |

Each preset agent ships with a hand-written soul, scope, avatar, and (where it
makes sense) a schedule. Each preset also carries a parallel cap and budget
defaults (§8). Provider assignments in the YAML (`claude`/`codex`) are remapped to
your installed CLIs at instantiation, with a fallback to the workspace default.

---

## 6. API / contract reference

All swarm REST routes are under `/api/v1`; JSON is snake_case; ids are ULIDs;
timestamps are RFC3339; errors are `Problem{code,message}`. Reads require
workspace **viewer**; mutations and lifecycle require workspace **editor**. The
authoritative spec is `docs/contracts/api.md` (#59–#86 + "Swarm lifecycle") and
`docs/contracts/ws.md`.

### CRUD + board (router in `otto-swarm/src/http.rs`)

| # | Method & path | Auth | Body → Response |
|---|---|---|---|
| 59 | `GET /workspaces/{id}/swarm/swarms` | viewer | → `Swarm[]` |
| 60 | `POST /workspaces/{id}/swarm/swarms` | editor | `CreateSwarmReq` → `SwarmDetail` |
| 61 | `GET /swarm/swarms/{sid}` | viewer | → `SwarmDetail` |
| 62 | `PATCH /swarm/swarms/{sid}` | editor | `UpdateSwarmReq` → `Swarm` |
| 63 | `DELETE /swarm/swarms/{sid}` | editor | → 204 |
| 64 | `GET /swarm/presets` | member | → `SwarmPreset[]` |
| 65 | `GET /swarm/swarms/{sid}/agents` | viewer | → `SwarmAgent[]` |
| 66 | `POST /swarm/swarms/{sid}/agents` | editor | `CreateAgentReq` → `SwarmAgent` |
| 67 | `PATCH /swarm/agents/{aid}` | editor | `UpdateAgentReq` → `SwarmAgent` |
| 68 | `DELETE /swarm/agents/{aid}` | editor | → 204 |
| 70 | `GET /swarm/swarms/{sid}/projects` | viewer | → `SwarmProject[]` |
| 71 | `POST /swarm/swarms/{sid}/projects` | editor | `CreateProjectReq` → `SwarmProject` |
| 72 | `PATCH /swarm/projects/{pid}` | editor | `UpdateProjectReq` → `SwarmProject` |
| 73 | `DELETE /swarm/projects/{pid}` | editor | → 204 |
| 75 | `GET /swarm/projects/{pid}/tasks` | viewer | → `SwarmTask[]` |
| 76 | `POST /swarm/projects/{pid}/tasks` | editor | `CreateTaskReq` → `SwarmTask` |
| 77 | `PATCH /swarm/tasks/{tid}` | editor | `UpdateTaskReq` → `SwarmTask` |
| 78 | `DELETE /swarm/tasks/{tid}` | editor | → 204 |
| 80 | `GET /workspaces/{id}/swarm/runs?swarm_id=&project_id=&agent_id=&status=` | viewer | → `SwarmRun[]` |
| 81 | `GET /swarm/runs/{rid}` | viewer | → `SwarmRun` |
| 83 | `GET /swarm/swarms/{sid}/graph` | viewer | → `SwarmGraph` |
| 85 | `GET /swarm/swarms/{sid}/board?project_id=&task_id=` | viewer | → `SwarmMessage[]` (latest 300) |
| 86 | `POST /swarm/swarms/{sid}/board` | editor | `PostMessageReq` → `SwarmMessage` |
| — | `GET /swarm/tasks/{tid}/story` | viewer | → `TaskStoryLink` (always 200; `story:null` if none) |

### Runtime endpoints (in `otto-server/src/swarm_runtime.rs`)

| # | Method & path | Auth | Body → Response |
|---|---|---|---|
| 69 | `POST /workspaces/{id}/swarm/recruit` | editor | `RecruitReq{role,context?,swarm_id?}` → `RecruitedAgent` |
| 74 | `POST /workspaces/{id}/swarm/projects/{pid}/plan` | editor | `PlanReq` → `SwarmTask[]` |
| 79 | `POST /swarm/tasks/{tid}/run` | editor | — → `SwarmRun` (run a task now) |
| 82 | `POST /swarm/runs/{rid}/stop` | editor | — → `SwarmRun` |
| 84 | `POST /workspaces/{id}/swarm/swarms/{sid}/start` | editor | — → `Swarm` |
| 84 | `POST /workspaces/{id}/swarm/swarms/{sid}/pause` | editor | — → `Swarm` |
| 84 | `POST /workspaces/{id}/swarm/swarms/{sid}/abort` | editor | — → `Swarm` |
| 84 | `POST /workspaces/{id}/swarm/swarms/{sid}/resume` | editor | — → `Swarm` |

> The four lifecycle actions are one combined row (#84) in the frozen contract but
> are four distinct routes; each takes no body and returns the updated `Swarm`.

### Board ingest (agent → board)

`POST /api/v1/ingest/swarm/board` — **unauthenticated by bearer**, but gated by
the per-session **ingest token**: the agent sends `X-Otto-Session` +
`X-Otto-Token` (via the `otto-post` helper). Body `{kind?, to_agent_id?, body}`.
Always 204 (fire-and-forget). The session's `meta` supplies `swarm_id`/`agent_id`
(and the current `project_id`/`task_id`/`run_id`).

### Key request/response shapes

- **`CreateSwarmReq`**: `{name, description?, preset_slug?, config?, max_total_runs?,
  max_cost_usd?, max_runtime_secs?, max_attempts?}`. (The UI sends just
  `{name, preset_slug}`.)
- **`UpdateSwarmReq`**: same fields, all optional; the three numeric budgets use a
  double-`Option` so a JSON `null` *clears* the limit (→ unlimited) while an absent
  key leaves it untouched.
- **`CreateAgentReq`**: `{name, provider, title?, reports_to?, model?, soul_name?,
  soul_md?, specialization?, scope_md?, skills?, schedule?, cwd_mode?, avatar?,
  order_idx?}`.
- **`CreateProjectReq`**: `{name, description?, repo_path?, goal_md?}`.
- **`CreateTaskReq`**: `{title, description?, assignee_agent_id?, status?, priority?,
  depends_on?, labels?, order_idx?}` (defaults: status `todo`, priority `medium`).
- **`PostMessageReq`**: `{project_id?, task_id?, to_agent_id?, kind?, body}`.
- **`SwarmDetail`** = the `Swarm` row + `agents[]` + `projects[]` + `counts`
  (`{agents, projects, tasks, running_runs, total_runs, cost_usd}`).
- **`SwarmGraph`** = `{nodes:[{id,kind,label,status,agent_id,session_id,project_id}],
  edges:[{from,to,kind}]}` (edge kind: `depends|handoff|review`).

### WebSocket events (`/ws/events`, member with viewer+ on the workspace)

```json
{"type":"swarm_status","workspace_id":"…","swarm_id":"…","status":"active|paused|aborted"}
{"type":"swarm_run_updated","workspace_id":"…","swarm_id":"…","run":{…SwarmRun…}}
{"type":"swarm_task_updated","workspace_id":"…","swarm_id":"…","project_id":"…","task":{…SwarmTask…}}
{"type":"swarm_message_posted","workspace_id":"…","swarm_id":"…","message":{…SwarmMessage…}}
```

The UI routes these into the swarm store, updating the org tree, run graph,
Kanban, runs list and board live. (`run`/`task`/`message` payloads travel as
serialized JSON.)

---

## 7. Capabilities & limitations

**You can:**

- Stand up a swarm from a preset or blank, and build an arbitrary org tree via
  `reports_to` (drag-to-reparent in the UI; cycles prevented).
- Have the **recruiter** AI-draft a complete agent (soul, scope, skills,
  provider, schedule) for any role you name — then edit everything before hiring.
- Run multiple **projects** per swarm, each with its own Kanban board and DAG.
- **Plan a project from its goal** with AI, or build tasks by hand with explicit
  dependencies, priorities, and assignees.
- Run the swarm autonomously: leaders **delegate** to reports, work **hands off**
  between roles, completed work gets **reviewed**, parents **roll up** when their
  children finish.
- **Schedule** agents to run on a daily/weekly/interval cadence with a standing
  directive (UTC).
- **Open any agent as a live terminal session** at any time; **inspect** any run's
  brief, artifacts, tokens/cost, board posts, and raw result.
- **Watch** the team coordinate on a live shared **board**, and **post** to it
  yourself.
- **Pause / abort / resume** the whole swarm; **stop** an individual run; **run a
  task now** manually.
- Set per-swarm **budgets** (runs / cost / runtime) and a per-task **attempt
  ceiling** to bound autonomous spend.
- Push a refined **Product story** into a swarm as a tasked project (Plan →
  Swarm) and follow the back-link.

**You cannot (today):**

- Span a swarm across workspaces — a swarm is workspace-scoped, and all of its
  agents/projects/tasks/runs/board carry its `workspace_id`.
- Run an agent on a provider you don't have installed — preset/recruiter
  selections are mapped to installed CLIs (fallback to the workspace default).
- Rely on schedule times being local — **all schedule times are interpreted in
  UTC**.
- Have a leader and one of its reports run at the same instant on the same task —
  the coordinator enforces **one active run per agent**, and ready selection is
  serialized per tick.
- Treat `max_cost_usd` as a hard real-time guarantee — cost is a **soft, best-
  effort** cap backfilled from the usage store and checked once per tick; it
  defaults to **unlimited** for blank swarms (it would otherwise read 0 and never
  trip, or trip spuriously, until usage attribution lands). Run-count and runtime
  budgets *are* enforced with non-null defaults. See §8.
- Edit a project's Product `story_id` back-link via the project PATCH endpoint —
  it's an internal Plan → Swarm link, left untouched by `UpdateProjectReq`.

---

## 8. Token-efficiency model

Agent Swarm is designed to run a whole team without re-paying for context every
turn.

- **Persistent + resumed sessions.** Each agent's session is created on its first
  turn and **reused** on subsequent turns (`find_agent_session` → `ensure_live`).
  A resumed turn does **not** re-feed the whole history — the brief injects *only*
  the new task/directive plus a short slice of recent board context. This is the
  core token win.
- **Identity materialized to disk, not tokens.** The agent's role, soul, scope,
  org position, current project/task, skills, and board instructions are written
  into its working directory as `CLAUDE.md`/`AGENTS.md` and `.claude/skills`
  (`swarm_workspace::provision_agent`), so the per-turn prompt stays small.
- **Bounded recruiter prompt.** The recruiter injects at most
  `RECRUITER_SKILL_CAP = 40` skills, ranked by relevance to the role, instead of
  dumping the whole library — then validates the reply against the *full* library
  (invented skills dropped).
- **Outputs read from files/transcripts (0 model tokens).** Results are collected
  from a result file the agent writes (and the provider transcript), not by asking
  the model to repeat its work — reading them costs no model tokens.
- **Per-turn usage attribution.** Token/cost backfill is bounded to events at/after
  the turn's start (`session_usage(..., turn_started_at)`), so a reused session
  reports only *this* turn's usage — never the lifetime sum. Values stay `null`
  (not a misleading `0`) when usage tracking is off or the transcript hasn't
  flushed yet. Runs are tagged with a `WorkRef` (swarm task id, originating story
  id, `origin:"swarm"`) so spend rolls up to the right work.

### Budget guardrails

Four nullable per-swarm limits (migration `0034_swarm_budgets.sql`); each `null` =
unlimited:

| Limit | Meaning | Blank-swarm default |
|---|---|---|
| `max_total_runs` | Lifetime run count for the swarm | **300** |
| `max_runtime_secs` | Wall-clock since the last start (`run_started_at`) | **4 h** (`14400`) |
| `max_cost_usd` | Accumulated backfilled spend, USD (**soft**) | **unlimited** (`null`) |
| `max_attempts` | Per-task attempt ceiling before `blocked` | **3** |

On every tick the coordinator checks runs-so-far, accumulated cost, and wall-clock
elapsed; when any is exceeded it **auto-pauses** the swarm (status `paused`, a
human-facing `pause_reason`, idle sessions suspended) instead of spawning more,
posts a `system` board note, and raises a warning notice. Raise the budget and
**resume** to continue. A run-count tick also can't *overshoot* `max_total_runs`
by the concurrency cap — the projected total is tracked as it schedules. Lifecycle
**start**/**resume** are additionally gated by the workspace-level usage budget.

The **attempt ceiling** stops runaway re-runs: a task that keeps returning a
non-terminal status (or whose turn fails) is re-queued only until `attempts`
reaches `max_attempts`, after which it's marked `blocked` with an `escalation`
post + notice rather than re-run forever.

> Historical note: an earlier fix doc (`docs/fixes-2026-06-19/batch3-d3-swarm-budget.md`)
> references migration `0032_swarm_budget.sql` and "runtime measured from
> `created_at`." The shipped behavior is the one above — migration
> **`0034_swarm_budgets.sql`**, runtime measured from **`run_started_at`** (the
> last time the swarm went active). Trust the code/contract over that note.

---

## 9. Security & permissions

- **RBAC.** Every read requires workspace **Viewer**; every mutation and lifecycle
  action requires workspace **Editor** (checked per request against the row's
  `workspace_id`). Item routes resolve the workspace from the row before checking.
  See `docs/MULTI-USER-RBAC.md`.
- **Workspace isolation.** Swarms, agents, projects, tasks, runs, and board
  messages all carry `workspace_id`; the listing and graph queries scope to it.
  WS `swarm_*` events are only delivered to members with viewer+ on that
  workspace.
- **Board ingest is token-gated, not open.** `POST /ingest/swarm/board` is not
  bearer-authenticated but is gated by the **per-session ingest token**
  (`X-Otto-Session` + `X-Otto-Token`), the same gate as `/ingest/claude`. Only a
  session whose `meta` names a `swarm_id` + `agent_id` can post; everything else is
  silently dropped (204).
- **Secrets stay in the Keychain.** Provider credentials are never in swarm rows;
  agents run their normal provider CLIs whose auth lives in the macOS Keychain
  (`otto-keychain`).
- **Loopback by default.** The daemon listens on `127.0.0.1:7700` unless you
  explicitly enable a network listener — the swarm runtime doesn't change that.

---

## 10. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| Tasks never start after **Start** | They must be in **To do** with all `depends_on` **Done**. `backlog` is not scheduled — drag to **To do**. Check the swarm is **active** and not at the `max_parallel_sessions` cap. |
| Swarm flips to **paused** on its own | A budget was hit — the **Paused: {reason}** pill / a `system` board post / a notice say which (runs, cost, or runtime). Use **Raise budget & resume**. |
| A task is **blocked** with an escalation post | It hit the per-task `max_attempts` ceiling without reaching a terminal status (or kept failing). It needs a human — fix the task/assignee and move it back to **To do**, or raise `max_attempts`. |
| An agent never picks up work | Its `status` must be `active`; it can only have **one active run at a time**; and the swarm must be under its parallel cap. A leader *delegates* instead of doing hands-on work. |
| Scheduled agent didn't fire | Schedules are **UTC**; the swarm must be **active**, under the cap, and the agent free. The scheduler advances `last_run` *before* firing (no double-fire). |
| Recruiter "returned no usable definition" | The model didn't emit a parseable JSON block. Retry, or add specifics in the context box. Invented skills/providers are dropped/remapped automatically. |
| **Plan from goal** errors | The project has no `goal_md`. Edit the project and add a goal. |
| Run shows no tokens/cost | Usage tracking is off, or the transcript hasn't flushed yet — the Inspector shows *"No usage recorded… (usage tracking off, or not flushed yet)."* Not an error. See `./../features/...` and the usage docs. |
| Preset agent landed on the wrong provider | The YAML provider wasn't installed, so it was remapped to an available CLI (or the workspace default). Install the CLI and edit the agent's **Provider**. |
| Runs stuck `running` after a daemon restart | On boot, orphaned `queued/running/waiting` runs are marked `error` (their process died). Re-run the task or **Start**/**Resume** the swarm. |

---

## 11. Related docs

- `./agent-sessions.md` — how agent sessions (the terminals every swarm agent runs
  in) work: opening, attaching, restarting, tasks, activity.
- `./product.md` — the Product (Jira/Confluence) workflow and the **Plan → Swarm**
  hand-off that seeds a swarm project from a story.
- `./skills-library.md` — the bundled skill library the recruiter assigns from and
  that gets materialized into each agent's working directory.
- `docs/contracts/api.md` (#59–#86, "Swarm lifecycle") and `docs/contracts/ws.md`
  (Agent Swarm events) — the authoritative API/event contract.
- `docs/MULTI-USER-RBAC.md` — roles and per-workspace permission model.
