# Scheduled Tasks

Recurring, workspace-scoped **agent jobs**. A scheduled task runs an agent with a
configurable **prompt** on a **cadence** (every *N* minutes, daily at a time, or
weekly on a day), turns the agent's final reply into a self-contained **Markdown
report**, stores it, and **delivers** it to an optional destination — Slack,
Telegram, email, or an HTTP webhook — with the delivered copy **redacted** on the
way out. Tasks are created and managed from the UI, the REST API, and — so an agent
can be asked *"add me a scheduled job"* — over **MCP**.

It is the generic engine behind examples like an hourly *"go over every ticket
updated in the last 24h, re-analyze it, check for new comments, and produce a
follow-up report"* — shipped as the built-in **`ticket-followup-review` preset**.

## What's new in v2

Scheduled Tasks v2 lifts every v1 limit (claude-only, no cron, UTC-only, headless
runs, `cwd` not a sandbox, one-agent-run-per-task):

- **Any provider** — `provider` is `claude | codex | agy | shell | <custom slug>`.
  Agent runs are provider-agnostic (the agent writes its report to a file we read);
  `shell` runs the prompt as a command and captures stdout/stderr/exit-code.
- **Local timezone** — a per-task IANA `timezone` (the create form defaults to your
  browser's). Daily/weekly/cron times are interpreted there, DST-correctly.
- **Cron** — `schedule = {cadence:"cron", expr:"0 9 * * 1"}` (standard 5-field cron,
  Vixie day-of-month/day-of-week semantics), evaluated in the task timezone.
- **Visible session per run** — every agent run is a **real, openable session**
  (`run.session_id`); Open it from the run row to watch or unblock it live.
- **Per-task sandbox** — `sandbox:"worktree"` runs in a **fresh isolated git
  worktree** off the repo's current branch (left for inspection), so a task that
  touches files can't disturb your working tree.
- **Workflow handoff** — `kind:"workflow"` launches a workflow run (`workflow_id`)
  instead of an agent and reports its node-by-node outcome.
- **Retry policy** — `max_retries` (0..5); a failed/stuck agent session is killed
  and retried with backoff. `run.attempts` records how many it took.
- **Only notify on change** — `notify_on_change` delivers only when the report's
  normalized hash differs from the last successful run; otherwise the run is marked
  `skipped_delivery` (the report is still stored).
- **Attach a proof pack** — `attach_proof` builds a proof pack per run
  (`run.proof_pack_id`) with the report + run metadata as evidence.
- **Recurring PR / review / security scans** — built-in presets
  (`weekly-security-scan`, `weekly-code-review`, `weekly-dependency-scan`) that pair
  a review/security prompt with a worktree sandbox + weekly cron.

All v2 fields default to backward-compatible values (provider unchanged,
`timezone=UTC`, `sandbox=none`, `max_retries=0`, no notify-gate, no proof), so every
pre-v2 task keeps behaving exactly as before. Schema: migration
`0085_scheduled_tasks_v2.sql`. Implementation note: agent runs go through the shared
`agent_run` session runner (the same one PR-review uses), not the headless
`run_agent` — except under `OTTO_E2E`, which keeps the deterministic stub.

This is the definitive end-user + operator guide. It documents what the code in
`crates/otto-server/src/scheduled_tasks_engine.rs`,
`crates/otto-server/src/scheduled_tasks_scheduler.rs`,
`crates/otto-server/src/cadence.rs`,
`crates/otto-server/src/routes/scheduled_tasks.rs`,
`crates/otto-state/migrations/0084_scheduled_tasks.sql`, and
`ui/src/modules/scheduled-tasks/` actually does — the real cadence kinds, the
report/delivery flow, the MCP tool names, the routes, and the WS event.

> Related docs: **[Self-improvement](./self-improvement.md)** (uses the same
> headless `Orchestrator::run_agent` primitive), **[MCP Control Plane](./mcp-control-plane.md)**
> (the outward `otto.*` surface that exposes the seven scheduled-task tools), the
> **[daemon HTTP API](./daemon-http-api.md)** (auth + how to call the routes), and
> the internal **[design doc](../design/scheduled-tasks-design.md)**. The API shape
> is specified in `docs/contracts/api.md` (Scheduled Tasks, #135–#143) and
> `docs/contracts/ws.md` (`scheduled_task_run_updated`), which remain authoritative.

---

## 1. Summary

| | |
|---|---|
| **What it is** | Recurring agent jobs: a prompt + cadence → a Markdown report → an optional delivery. |
| **Task kind (v1)** | `agent_prompt` — runs a **claude** agent headlessly via `Orchestrator::run_agent`. Other kinds are reserved. |
| **Cadence** | `interval` (`every_min`, floor **5**), `daily` (`at:"HH:MM"` UTC), or `weekly` (`at` + `weekday` 0=Mon…6=Sun). |
| **Report** | The agent's final message, saved verbatim as Markdown; a short `summary` is extracted for the list + delivery. |
| **Destinations** | `none` (store only), `slack`, `telegram`, `email`, `webhook`. Delivered text + attachment are **redacted**. |
| **Driven by** | The Scheduled Tasks page, REST (`/scheduled-tasks/*`), and 7 outward `otto.*` MCP tools. |
| **Scheduler** | A 60-second tick in `ottod` that fires every enabled, due task (the `cli_update` `is_due`/`run` pattern). |
| **Persistence** | SQLite tables `scheduled_tasks` + `scheduled_task_runs` (migration **0084**). Reports on disk under `<data_dir>/scheduled/`. |
| **RBAC** | `Feature::ScheduledTasks` (snake `scheduled_tasks`): **View** for reads, **Edit** for writes; plus a per-workspace role check on every route. |
| **WS event** | `scheduled_task_run_updated` (`running` / `ok` / `error`), workspace-scoped. |

---

## 2. Where it lives & how the pieces fit

Open **Scheduled Tasks** in the sidebar (`{ id: 'scheduled-tasks', icon: 'clock',
label: 'Scheduled Tasks', feature: 'scheduled_tasks' }`). The page subtitle states
the purpose: *"Recurring agent jobs — run a prompt on a cadence, produce a report,
and deliver it to Slack, email, or a webhook. Also driveable over MCP."*

| Layer | File | Responsibility |
|---|---|---|
| **Cadence** | `crates/otto-server/src/cadence.rs` | Pure, unit-tested `is_due` / `next_run` / `validate` for the three cadences. |
| **Engine** | `crates/otto-server/src/scheduled_tasks_engine.rs` | `run_task`: open a run row → run the agent → extract summary → write report → deliver → advance cursor. |
| **Scheduler** | `crates/otto-server/src/scheduled_tasks_scheduler.rs` | The 60-s supervisor tick + per-task in-flight guard + startup reaper. |
| **HTTP routes** | `crates/otto-server/src/routes/scheduled_tasks.rs` | The `/scheduled-tasks/*` endpoints + the preset list + the report server. |
| **MCP surface** | `crates/otto-server/src/mcp_outward.rs` | The 7 `otto.*` scheduled-task tools (read + write). |
| **Persistence** | `crates/otto-state/migrations/0084_scheduled_tasks.sql` + repo | `scheduled_tasks` + `scheduled_task_runs` tables. |
| **Domain types** | `otto-core` | `ScheduledTask`, `ScheduledTaskRun`, `ScheduledTaskPreset`, `Feature::ScheduledTasks`, `Event::ScheduledTaskRunUpdated`. |
| **UI** | `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte` | The list, the create/edit form, the runs drill-down, and the report modal. |

### 2.1 Data flow

```
Scheduler (60s tick, ottod)              REST  POST /scheduled-tasks/{id}/run  (trigger=manual)
   └─ enabled + due tasks ───────┐          └────────────────────┐
MCP otto.create/run/… ──▶ REST endpoints                         │
                                            run_task(task, trigger):
                                              create scheduled_task_runs(running) → emit ws "running"
                                              run agent (Orchestrator::run_agent, claude, OTTO_E2E stub in tests)
                                              extract_summary(report) + write report.md
                                              deliver to destination (redacted) → record delivered/err
                                              finish_run(ok) ; if trigger==schedule: advance last_run_at + next_run_at
                                              prune to 100 newest runs → emit ws "ok"
                                            (any failure ⇒ finish_run(error), emit ws "error")
```

- **Execution** uses `Orchestrator::run_agent(prompt, cwd, model_opt, no_progress)`
  — the same headless primitive the self-improvement engine uses — so a task
  "triggers an agent" exactly like self-improvement, and inherits its built-in
  `OTTO_E2E` stub for deterministic Playwright/CI runs (the prompt-wrap embeds an
  `OTTO_TASK: scheduled_task` sentinel that the stub recognizes). **v1 runs
  claude**; an optional `skill` is inlined into the prompt
  (`resolve_skill_inline` + `compose_draft_prompt`). A process-wide semaphore
  (`OTTO_SCHEDULED_MAX_CONCURRENT`, default **2**) bounds concurrent agent runs.
- **Cadence** is computed in pure code (see §5). The scheduler advances the
  `last_run_at` cursor only **on run completion** and only for `trigger=="schedule"`,
  and claims a per-task in-flight guard **first**, so an overlapping or
  crash-interrupted occurrence is retried (at-least-once), never silently dropped.
- **Report** = the agent's final message (see §6).

---

## 3. Setup

There is no per-feature install step beyond the daemon being up. To use it
productively:

1. **An agent CLI on `PATH`.** Tasks run **claude** headlessly (the daemon's
   `claude` environment). The same `~/.claude` config a normal Otto claude session
   uses applies — including any MCP servers configured there (this is what makes
   the ticket-review preset work; see §8).
2. **A workspace.** Tasks are workspace-scoped; you create them on the workspace
   you have an **Editor** role in.
3. **A destination (optional).** To deliver beyond local storage you need the
   matching integration configured (see §7): a Slack/Telegram integration on the
   workspace, a **verified Gmail sender** (Settings → Email sender) for email, or a
   reachable, non-private URL for a webhook.

The scheduler is started once at daemon boot alongside Otto's other supervisors
(swarm, workflow-triggers, CLI auto-update). On startup it also **reaps** any run
rows left `running` by a previous daemon life, marking them `error` (the in-flight
guard is in-memory and resets empty across restarts).

---

## 4. Creating a task (UI walkthrough)

Open **Scheduled Tasks** → **New task**. The form (heading *"New scheduled task"*,
or *"Edit scheduled task"* when editing) has these fields, in order:

1. **Start from a preset** — a `select` (shown only when creating and presets
   exist) with a blank `— blank —` option and each built-in preset. Picking one
   pre-fills the prompt + cadence (the `ticket-followup-review` preset; see §8).
2. **Name** — e.g. *"Nightly ticket review"*. Required.
3. **Prompt (the agent's instructions)** — a 6-row textarea; the agent's full
   instructions. Placeholder: *"Go over every ticket updated in the last 24h…"*.
4. **Cadence** — `Interval` / `Daily` / `Weekly`, which reveals the matching fields:
   - **Interval** → **Every (minutes, min 5)** (a number input, `min=5`).
   - **Daily / Weekly** → **At (HH:MM UTC)** (placeholder `03:00`).
   - **Weekly** also → **Weekday** (`Mon`…`Sun`).
5. **Destination** — `None (store only)` / `Slack` / `Telegram` / `Email` /
   `HTTP webhook`, which reveals:
   - **Slack / Telegram** → **Chat / channel id (optional)** (defaults to the
     integration's channel).
   - **Email** → **Send to (email)**.
   - **Webhook** → **Webhook URL** (`https://…`).
6. **Skill (optional, inlined)** — a skill slug (e.g. `db-mysql`) whose body is
   inlined into the prompt.
7. **Working dir (optional)** — a repo path. The placeholder is explicit:
   *"repo path — not a sandbox"* (see §9 — `cwd` is **not** a security boundary).
8. **Enabled** — a checkbox.

**Save** (or **Saving…**) / **Cancel**. After saving, the task appears in the list
with its cadence label (*"every X min"* / *"daily at HH:MM UTC"* / *"weekly DAY at
HH:MM UTC"*), destination badge, last-status pill (`ok` / `error` / `running`), and
a *paused* pill when disabled.

Each task row has **Run now**, **Runs** / **Hide runs**, **Pause** / **Enable**,
**Edit**, and **Delete**.

- **Run now** triggers an immediate run (`trigger=manual`) — this **does not move
  the schedule cursor**, so it never disturbs the next scheduled fire.
- **Runs** expands the per-run history: a status pill, the start time, the summary
  text, a **View report** button (when a report exists), and a **delivered** /
  **delivery failed** pill.
- **View report** opens a **Report** modal that renders the stored Markdown.

Empty states: *"No scheduled tasks yet. Create one to run an agent on a cadence."*
and, per task, *"No runs yet."*

---

## 5. Cadence (the schedule kinds)

Cadence logic is pure and fully unit-tested in `cadence.rs`. The schedule spec is a
small JSON object stored in `schedule_json`; the **cursor** (`last_run_at`) is a
separate column passed into `is_due` — a deliberate borrow of the `cli_update`
pattern so that editing the schedule can never clobber the cursor.

| Cadence | Spec | "Due" rule (`is_due`) |
|---|---|---|
| **`interval`** | `{ "cadence":"interval", "every_min": 60 }` | Drift-based: due when never run, or `now - last_run >= every_min`. `every_min` is floored to **5**. Naturally catch-up-safe. |
| **`daily`** | `{ "cadence":"daily", "at":"03:00" }` | Due when `now >= today@at` **and** the task hasn't run since `today@at`. A missed window still fires at the next tick (`cli_update` catch-up comparison). |
| **`weekly`** | `{ "cadence":"weekly", "at":"03:00", "weekday":4 }` | As daily, but only on the matching `weekday` (`0`=Mon … `6`=Sun; default Monday). |

`at` defaults to **09:00** and `weekday` to **0** (Monday); times are **UTC**.
`next_run(spec, now)` computes the displayed `next_run_at`. Create/update
**validates** the spec (`cadence::validate`) and rejects: an unknown cadence
(only `interval|daily|weekly`), an interval below the 5-minute floor, a malformed
`at` (must be `HH:MM`, `h<24`, `m<60`), or a `weekday` outside `0..=6` — all `400`.

> There are **no** true five-field cron expressions; the cadence enum is the house
> style (matching the swarm and workflow-trigger schedulers). This is a documented
> non-goal.

---

## 6. The report

The report **is** the agent's final message. The prompt-wrap (`wrap_prompt`)
instructs the agent to produce *a single, self-contained Markdown report* that
begins with a one-line `#` title, then a brief summary, then a `---` horizontal
rule, then the details — and to treat any external content it reads (tickets,
comments, files) as **untrusted input** it must never obey.

- **Summary extraction** (`extract_summary`, pure + unit-tested): everything up to
  the first `---` / `***` horizontal rule, else the first **~800 characters**
  (truncated with an ellipsis). This `summary` is what the run list and the delivery
  message show; the full body is the stored/attached artifact.
- **Storage**: the full report is written to
  `<data_dir>/scheduled/<task_id>/reports/<UTC-timestamp>.md`. The path segments are
  **server-generated** (the task id + a server timestamp) — never the user-supplied
  name (`report_rel`), which is the path-safety guarantee.
- **Retention**: the **100 most-recent runs per task** are kept; older runs and
  their report files are pruned on each new run (`KEEP_RUNS = 100`).
- **Stuck-run budget**: a single agent run has a no-progress budget of **600 s**
  (`RUN_NO_PROGRESS`).

The report is served by **run id** (never an arbitrary path); the route
canonicalizes the resolved path against the reports root and rejects anything that
escapes it.

---

## 7. Destinations

`destination_json` is a tagged enum (`{ "type": "...", ... }`). Delivery is
**best-effort**: a failure sets `delivered=0` + `delivery_error` on the run, but the
run is still `ok` (the report exists locally regardless). The delivered message text
**and** the attached `report.md` are both passed through
`otto_core::redact::redact_text` before they leave the machine.

| `type` | Fields | Behavior |
|---|---|---|
| **`none`** | — | The report is only stored (default). |
| **`slack`** / **`telegram`** | `chat_id?` | Posts the summary and attaches `report.md` to the workspace's configured channel integration (or the given `chat_id`). Uses `improve_notify::send_to` + the channel `Adapter::upload`. Fails clearly if no integration is configured or the bot token is missing. |
| **`email`** | `to`, `subject?` | Sends via the **task owner's verified Gmail sender** (Settings → Email sender), with `report.md` attached (`GmailSender::send_with_attachment`). `subject` defaults to *"Scheduled task report"*. Fails if the owner has no verified sender or the app password isn't in the Keychain. |
| **`webhook`** | `url` | POSTs the summary + the (base64) report via `WebhookAdapter`, which runs the **`otto_netguard` SSRF check + redirect policy** before every request — loopback/private/metadata targets are refused. |

The delivery message is *`*<task name>*` + blank line + the summary*.

---

## 8. The ticket follow-up preset (the motivating example)

The single built-in preset, **`ticket-followup-review`** (*"Processed-ticket
follow-up review"*), asks the agent to: go over every ticket updated in the last
24 h, re-analyze each using the **Jira/Atlassian tools available to it**, check for
new post-triage comments, and produce a Markdown report whose summary lists
*Reviewed / New post-triage comments / Improvements found / a Terminal-no-refetch
count*, then the per-ticket details after a `---` rule. Its cadence is hourly
(`{ "cadence":"interval", "every_min":60 }`).

**Prerequisite for this to work:** the engine is **generic** — it does not fetch
tickets itself. The agent uses whatever tools its environment provides, so the
**daemon's `claude` environment must have the Atlassian/Jira MCP server configured
and authenticated** (the same `~/.claude` MCP config a normal claude session uses),
and the task's **Working dir** should point at the workspace/repo so its `CLAUDE.md`
and project skills are in scope. Without that MCP server, the agent will report that
it has no ticket tools — the task still runs and produces a report, just an empty
one.

Presets are pure data returned by `GET /scheduled-tasks/presets`
(`ScheduledTaskPreset { id, name, description, kind, prompt, schedule,
suggested_destination, skill }`); there is no preset-specific code path.

---

## 9. MCP surface (the 7 `otto.*` tools)

The outward `otto.*` MCP surface (managed in **MCP Control Plane → Otto Server**,
served by `ottod mcp-server`) exposes seven scheduled-task tools so an external
agent can inspect and — once enabled — manage jobs:

| Tool | Mutating | DANGEROUS | Default-enabled | Backing endpoint |
|---|---|---|---|---|
| `otto.list_scheduled_tasks` | no | — | **yes** | `GET /workspaces/{ws}/scheduled-tasks` |
| `otto.list_scheduled_task_runs` | no | — | **yes** | `GET /scheduled-tasks/{id}/runs` |
| `otto.create_scheduled_task` | yes | **yes** | no | `POST /workspaces/{ws}/scheduled-tasks` |
| `otto.update_scheduled_task` | yes | **yes** | no | `PATCH /scheduled-tasks/{id}` |
| `otto.set_scheduled_task_enabled` | yes | **yes** | no | `PATCH /scheduled-tasks/{id}` |
| `otto.run_scheduled_task` | yes | **yes** | no | `POST /scheduled-tasks/{id}/run` |
| `otto.delete_scheduled_task` | yes | **yes** | no | `DELETE /scheduled-tasks/{id}` |

- **Reads are enabled by default** so an agent can immediately inspect existing
  jobs. The five **write** tools are marked `DANGEROUS` and are **off by default** —
  an admin enables them once in the Otto Server tab.
- Each dangerous call is **human-approval-gated** when
  `mcp_require_approval_dangerous` is on (the default). For `create`/`update`, the
  approval prompt surfaces a one-line summary of *the recurring job's name, cadence,
  and destination + the first ~160 chars of the prompt* (`dangerous_detail`) so the
  approver sees exactly what autonomous capability they are granting.
- Every tool **self-calls its REST endpoint** with a short-lived ephemeral token, so
  the endpoint's native RBAC + audit still apply — the MCP path grants no extra
  privilege. See the **[MCP Control Plane guide](./mcp-control-plane.md)** for the
  governance pipeline these calls funnel through.

This is how requirement *"an agent can be asked to add me a scheduled job"* is met:
the admin enables `create_scheduled_task` once; thereafter an agent can register
jobs (each subject to the approval gate).

---

## 10. REST API & WebSocket reference

All routes are under `/api/v1`. Authoritative contract: `docs/contracts/api.md`
(Scheduled Tasks, #135–#143) and `docs/contracts/ws.md`
(`scheduled_task_run_updated`).

| # | Method & path | Purpose | RBAC |
|---|---|---|---|
| 135 | `GET /workspaces/{id}/scheduled-tasks` | List a workspace's tasks → `ScheduledTask[]` | ws Viewer · `scheduled_tasks` View |
| 136 | `POST /workspaces/{id}/scheduled-tasks` | Create a task → `ScheduledTask` | ws Editor · Edit |
| 137 | `GET /scheduled-tasks/presets` | Built-in preset templates → `ScheduledTaskPreset[]` | `scheduled_tasks` View |
| 138 | `GET /scheduled-tasks/{id}` | Get one task → `ScheduledTask` | ws Viewer · View |
| 139 | `PATCH /scheduled-tasks/{id}` | Update a task → `ScheduledTask` | ws Editor · Edit |
| 140 | `DELETE /scheduled-tasks/{id}` | Delete a task (+ its runs) → `{ ok:true }` | ws Editor · Edit |
| 141 | `POST /scheduled-tasks/{id}/run` | Run now (manual; cursor untouched) → `ScheduledTaskRun` | ws Editor · Edit |
| 142 | `GET /scheduled-tasks/{id}/runs` | Run history (newest first, ≤100) → `ScheduledTaskRun[]` | ws Viewer · View |
| 143 | `GET /scheduled-tasks/runs/{run_id}/report` | The stored report → `text/markdown` | ws Viewer · View |
| 144 | `POST /scheduled-tasks/{id}/convert-to-workflow` | Materialize the task as a Workflow + schedule trigger → `{workflow_id, trigger_id?}` | ws Editor · Edit |

**Convert to workflow (#144).** A one-shot bridge from the single-step
Scheduled-Tasks surface to the multi-step **Workflows** engine. It creates a
workflow graph `manual_trigger → agent_prompt [→ channel_notify]` — the
`agent_prompt` carries the task's prompt + provider, and the `channel_notify` node
is added only when the task delivers to Slack/Telegram (it forwards the agent's
`{reply}`) — plus a `schedule` **workflow trigger** mirroring the task's cadence and
timezone. Pass `{ "disable_task": true }` to pause the source task after
converting. Use this once you outgrow a single agent run and want branching, loops,
review/PR steps, or human approval (see `./workflows.md`).

**Create / update body** (all optional except `name` on create): `name`, `prompt`,
`kind`, `skill`, `provider` (claude or blank only — others are rejected `400`),
`model` (blank ⇒ provider default), `cwd`, `schedule` (the cadence spec),
`destination`, `enabled`. On update, `skill` distinguishes *absent* (leave unchanged)
from *present-and-null* (clear). `report_path` is **output-only** — never settable.

### 10.1 WebSocket event

| Event | When | Payload |
|---|---|---|
| `scheduled_task_run_updated` | A run starts (`running`), finishes (`ok`), or errors (`error`). | `{ "type":"scheduled_task_run_updated", "workspace_id":"<id>", "task_id":"<id>", "run_id":"<id>", "status":"running\|ok\|error" }` |

The event is **workspace-scoped** (delivered to members with Viewer+ on the
workspace). The Scheduled Tasks page re-fetches the affected task's run history on a
matching tick instead of polling.

---

## 11. Capabilities & limitations

**Capabilities**

- Recurring agent jobs on three cadences with **catch-up** semantics (a missed
  daily/weekly window still fires; intervals are drift-based).
- Every run produces a stored Markdown report + an extracted summary; the 100 newest
  runs per task are retained with their report files.
- Four delivery channels (Slack, Telegram, email, webhook) reusing Otto's existing,
  tested integrations — with **redaction on delivery** and SSRF-guarded webhooks.
- Full management from UI, REST, and 7 governed `otto.*` MCP tools.
- At-least-once execution with overlap protection (per-task in-flight guard) and a
  startup reaper for interrupted runs.
- A built-in preset that makes the ticket-review example work out of the box.

**Limitations**

- **v1 is claude-only.** `provider` is persisted for forward-compat (codex/agy via
  `run_cli_exec` is a documented follow-up); non-claude providers are rejected at
  create/update.
- **No server-side ticket fetching.** The engine is generic — the agent's own MCP
  tools do the fetching (the preset needs an authenticated Atlassian MCP in the
  daemon's claude env).
- **No five-field cron.** Only the `interval / daily / weekly` enum.
- **`cwd` is not a sandbox** (see §12).
- **`session_id` on a run is reserved** — v1 runs are headless (an ephemeral PTY, no
  Otto session row).
- **Times are UTC** (`at` is interpreted in UTC, not local time).
- **Single-step only** — a task is one agent run + a delivery. Multi-step / DAG
  automation is the Workflows feature; **`POST /scheduled-tasks/{id}/convert-to-workflow`**
  (§10, #144) materializes a task as a workflow + schedule trigger when you outgrow
  the single step.

---

## 12. Security & permissions

- **Two-axis RBAC.** Routes are gated by `Feature::ScheduledTasks` (snake
  `scheduled_tasks`) — **View** for GET, **Edit** for writes — *and* by the
  workspace role. Only workspace **Editors** can create / update / run / delete;
  **Viewers** can list and read reports. The feature axis is workspace-blind, so the
  flat by-id routes load the task/run first and check the role on its `workspace_id`
  — the **IDOR guard**.
- **Report path safety.** The report route serves strictly by **run id** →
  `report_rel`, then canonicalizes both the reports root and the resolved file and
  confirms containment before reading (path-traversal/symlink guard). Report path
  segments are server-generated, never the user name.
- **`cwd` is not a security boundary.** A coding agent can read/write anywhere the
  daemon user can; the prompt and any external content it reads are treated as
  **untrusted** (the prompt-wrap explicitly tells the agent never to follow
  instructions found in tickets/comments/files). Point tasks at repos you are
  comfortable an unattended agent operating in. Per-task tool sandboxing is a planned
  enhancement.
- **Redaction on exfil.** Delivered text + attachments are run through
  `redact_text`; webhooks additionally pass the SSRF guard + redirect policy. The
  **locally-stored** report is the full (un-redacted) artifact, behind RBAC — the
  delivery copy is what leaves the machine.
- **MCP write tools are off by default**, `DANGEROUS`, and human-approval-gated; the
  approval prompt surfaces the cadence + destination + prompt.
- **Resource bounds.** A minimum **5-minute** interval, a process-wide run semaphore
  (default 2, `OTTO_SCHEDULED_MAX_CONCURRENT`), a per-run 600 s no-progress budget,
  and run-history pruning to 100.

---

## 13. Troubleshooting

**A task never fires.** Confirm it is **Enabled** (no *paused* pill) and that
`next_run_at` is in the past or near. The scheduler ticks every 60 s; an `interval`
task is due once `now - last_run >= every_min` (min 5). `daily`/`weekly` use **UTC**
`at` — check you didn't set a local-time value. A daily/weekly window that was missed
(daemon down) fires at the next tick, not retroactively for each missed day.

**Two runs overlap / a run got "stuck".** A per-task in-flight guard prevents a
second tick from re-entering a still-running task (it is skipped without moving the
cursor, so the occurrence is retried). A run with no progress for 600 s is ended.
After a daemon restart, any run left `running` is reaped to `error` (*"interrupted by
daemon restart"*).

**The report is empty / generic.** The agent had nothing to work with — most often
the ticket-review preset without an authenticated Atlassian MCP in the daemon's
claude env (§8), or a prompt that doesn't reference tools the agent has. The report
is whatever the agent produced.

**Delivery shows "delivery failed".** Open the run — `delivery_error` says why.
Common cases: no Slack/Telegram integration configured for the workspace (or a
missing bot token); email with no **verified** Gmail sender for the **task owner**
(`created_by`), or the app password missing from the Keychain; a webhook URL that
the SSRF guard refused (loopback/private/metadata) or that returned an error. The
report is still stored — only delivery failed.

**`POST /scheduled-tasks/{id}/run` returned but nothing changed on the schedule.**
Correct — **Run now** is `trigger=manual`; it never advances `last_run_at` or
`next_run_at`. Use it freely to test without disturbing the cadence.

**Create rejected with a `400`.** The schedule spec failed validation — an interval
below 5 minutes, a bad `at` (`HH:MM`, `h<24`, `m<60`), a `weekday` outside `0..=6`,
an unknown cadence, or a non-claude `provider`.

**An agent's `otto.create_scheduled_task` keeps returning "pending approval".** The
write tools are `DANGEROUS` and approval-gated by default. Approve the request in
**MCP Control Plane → Approvals** (or set `mcp_require_approval_dangerous` off). The
tool itself must also be enabled in the Otto Server tab.

---

## 14. Related docs

- **[MCP Control Plane](./mcp-control-plane.md)** — the outward `otto.*` server that
  exposes the 7 scheduled-task tools, and the governance/approval pipeline they run
  through.
- **[Workflows](./workflows.md)** — the multi-step DAG engine; `convert-to-workflow`
  (#144) turns a scheduled task into a workflow + schedule trigger, and Workflows
  share this feature's cadence engine (cron + timezone).
- **[Self-improvement](./self-improvement.md)** — the other consumer of the headless
  `Orchestrator::run_agent` primitive.
- **[Daemon HTTP API](./daemon-http-api.md)** — auth, tokens, and calling the
  `/scheduled-tasks/*` routes yourself.
- **Design (internal):** [`docs/design/scheduled-tasks-design.md`](../design/scheduled-tasks-design.md).
- **Contracts (authoritative):** `docs/contracts/api.md` (Scheduled Tasks, #135–#143)
  and `docs/contracts/ws.md` (`scheduled_task_run_updated`).
- **Source:** `crates/otto-server/src/{scheduled_tasks_engine,scheduled_tasks_scheduler,cadence}.rs`,
  `crates/otto-server/src/routes/scheduled_tasks.rs`,
  `crates/otto-server/src/mcp_outward.rs`,
  `crates/otto-state/migrations/0084_scheduled_tasks.sql`,
  `ui/src/modules/scheduled-tasks/`.
