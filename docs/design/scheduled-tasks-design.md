# Scheduled Tasks — Design

**Status:** design (pre-review) · **Branch:** `feat/scheduled-tasks` · **Date:** 2026-06-26

> **Superseded by v2 (2026-06).** This is the original v1 design. Scheduled Tasks
> v2 lifted the v1 limitations recorded below: any provider
> (`claude`/`codex`/`agy`/`shell`/custom) runs as a **real, openable session**,
> plus 5-field **cron**, a per-task **timezone**, a per-task **worktree sandbox**,
> **workflow handoff**, a **retry policy**, **notify-on-change**, and **proof-pack**
> attachment. The end-user/operator guide
> [`docs/features/scheduled-tasks.md`](../features/scheduled-tasks.md) and the
> contract `docs/contracts/api.md` (Scheduled Tasks) are the current source of
> truth; passages below describing "claude-only", "cron is a non-goal", "headless
> runs", or "UTC-only" are historical.

## 1. Problem & goal

Users want **recurring, autonomous jobs** in Otto. A job runs on a cadence
(e.g. *once an hour*), does work — typically by **triggering an agent** (exactly
like the self-improvement engine) — produces a **report / full summary**, and
**delivers** it to a destination (Slack, email, or an HTTP call). The whole thing
must be **driveable over MCP** so an agent can be asked *"add me a scheduled job"*
and it registers one automatically.

The motivating example the user gave (an hourly *"go over all tickets updated in
the last 24h, re-analyze them, check for new comments, and produce a report"* with
a `Processed-ticket follow-up review` header) turns out to be the user's **external
Hermes cron**, pasted as an *example of the shape*. So we are **not** porting
ticket-specific logic. We build a **generic scheduled-task engine** whose primary
task kind runs an agent with a **configurable prompt**, and we ship the
ticket-review case as a **built-in preset** (just a default prompt + cadence) so
the example works out of the box without hard-coding tickets.

### Requirements traceability

| # | Requirement (from the user) | Where met |
|---|---|---|
| R1 | Create a task that runs on a schedule (e.g. once an hour) | `scheduled_tasks` table + cadence `interval/daily/weekly`; scheduler §6 |
| R2 | Example: hourly re-analyze tickets / new comments → report | `agent_prompt` kind + **ticket-review preset** (§4, §10) |
| R3 | Produce a report / full summary at the end (with that header shape + attach it) | run captures agent's final text → `report.md` on disk + extracted `summary`; delivered with attachment §5, §7 |
| R4 | Destination: Slack message, mail, or HTTP call | `destination` enum: `slack`/`telegram`/`email`/`webhook`/`none` §7 |
| R5 | Part of the MCP operations | 7 new `otto.*` tools in surface A (`mcp_outward.rs`) §8 |
| R6 (clarif.) | Agent can ask "add me a scheduled job" → done automatically | `otto.create_scheduled_task` MCP write tool §8 |
| R7 (clarif.) | The job may trigger agents (like self-improvement) | execution = `Orchestrator::run_agent`, the same primitive otto-improve uses §5 |
| R8 (clarif.) | Full summary at the end | `summary` extraction + run record + delivery message §5 |

## 2. Non-goals (v1)

- True 5-field cron expressions (no cron crate in-tree; the house style is the
  `interval/daily/weekly` cadence enum — we follow it). Documented as a non-goal.
- Refactoring the existing duplicated `is_due` out of `swarm_scheduler` /
  `workflow_trigger_scheduler` into one shared helper. It is a real structural
  win, but with **5 active worktrees** touching those files it is a merge-risk; we
  ship a **self-contained, well-tested cadence module** and leave a `// dedup
  opportunity` note. (Recorded as follow-up.)
- Multi-step / DAG tasks (that is the Workflows feature). A scheduled task is a
  single agent run + delivery.

## 3. Architecture (data flow)

```
Scheduler (60s tick, ottod startup)
  └─ due tasks → advance cursor → run_task(trigger=schedule)
REST POST /scheduled-tasks/{id}/run → run_task(trigger=manual)
MCP otto.* tools → self-call the REST endpoints (governed, ephemeral token)

run_task(ctx, task, trigger):
  1. insert scheduled_task_runs(status=running)         → emit ws event
  2. resolve cwd (task.cwd | per-task scratch dir)
  3. prompt = compose(skill_inline?, wrap_report(task.prompt))
  4. text = Orchestrator::run_agent(prompt, cwd, model_opt, timeout)   // OTTO_E2E stub ⇒ deterministic
  5. summary = extract_summary(text);  write report.md under data_dir/scheduled/<task>/reports/<ts>.md
  6. update run(status=ok, summary, report_path); update task(last_status,next_run_at)
  7. deliver(destination, name, summary, report.md)  (best-effort; records delivered/err)
  8. emit ws event (status=ok|error); prune old runs
  (any failure ⇒ run(status=error, error=…), task last_status=error, emit)
```

## 4. Task model & kinds

A task is **workspace-scoped**. `kind` is an enum for forward-compat; v1 ships one:

- **`agent_prompt`** — run a headless agent with `prompt` (+ optional `skill`,
  `provider`, `model`, `cwd`). The agent's final message is the report. This is
  the generic, powerful case and covers R2/R6/R7. The ticket-review example is an
  `agent_prompt` task whose prompt instructs the agent to query tickets and write
  the report in the desired format.

**Presets** are pure data (no code path): `GET /scheduled-tasks/presets` returns
templates `{ id, name, kind, prompt, schedule, suggested_destination }`. v1 ships:
`ticket-followup-review` (hourly interval, prompt = re-analyze tickets updated in
the last 24h, check for new comments, emit a `Processed-ticket follow-up review`
markdown report with Reviewed/New-comments/Improvements/Terminal counts).

## 5. Agent execution (R7, R8)

Use **`Orchestrator::run_agent(prompt, cwd, model_opt, no_progress) -> Result<String>`**
(`crates/otto-orchestrator/src/lib.rs:120`) — the exact primitive otto-improve's
producer uses. Why this over `run_session_turn`:

- It has a **built-in `OTTO_E2E` stub** (`e2e_stub::canned_reply`) → the run-now
  flow is deterministic in Playwright/CI without a real LLM.
- Headless, no acting-user/session-row needed; mirrors self-improvement.
- Skills are injected **provider-agnostically** by **prompt-inlining**
  (`resolve_skill_inline` + `compose_draft_prompt`), so we don't need the
  PreSpawn materialization hook.

`model` is routed through `model_opt()` (empty ⇒ `None`) to avoid the
`--model claude` gotcha. v1 provider = `claude` (run_agent); `provider` field is
persisted for forward-compat (codex via `run_cli_exec` is a documented follow-up).
Timeout: a `no_progress` budget (default 600s, configurable per task later).

**Summary extraction** (pure, unit-tested): the report body is the agent's final
text. `summary` = the text up to the first `---`/`***` horizontal rule, else the
first ~800 chars, trimmed. The run-list and the delivery message use `summary`;
the full body is the stored/attached report.

Prompt wrapping (pure): we prepend a short instruction — *"Produce your reply as a
self-contained Markdown report; it is saved verbatim and delivered. Begin with a
one-line title, then a brief summary, then details."* — so any task yields a clean
artifact. The user's `prompt` follows.

## 6. Scheduler (R1)

`crates/otto-server/src/scheduled_tasks_scheduler.rs`, modeled on
`workflow_trigger_scheduler.rs` + `cli_update.rs`:

- `start(ctx) -> Arc<AtomicBool>`, 60s tick polled in 500ms slices (responsive
  cancel), spawned in `ottod/src/main.rs` next to the other schedulers.
- Each tick: list **enabled** tasks; for each compute `is_due(cadence, now,
  last_run_at)`; if due **advance `last_run_at = now` first** (cursor write before
  run ⇒ no double-fire), then spawn `run_task(trigger=schedule)`.
- **In-flight guard**: `Arc<Mutex<HashSet<task_id>>>` so a slow run is never
  re-entered (mirrors otto-improve's double-guard).

**Cadence (pure `cadence.rs`, fully unit-tested):**
- `interval`: due iff `last_run.is_none() || now - last >= every_min` (drift-based;
  naturally catch-up-safe). "once an hour" = `{cadence:"interval", every_min:60}`.
- `daily`: `{at:"HH:MM"}` — due iff `now >= scheduled_today && (last < scheduled_today)`
  (the robust `cli_update` catch-up comparison, so a missed day still fires).
- `weekly`: as daily, gated on `weekday == now.weekday()`.
- `next_run(cadence, last)` for display (`next_run_at`).

Schedule **config** lives in `schedule_json` (`cadence/every_min/at/weekday`); the
**cursor** is a dedicated `last_run_at` column passed explicitly into `is_due`
(cli_update style — config edits never clobber the cursor).

## 7. Destinations (R3, R4)

`destination_json` = tagged enum, all reusing existing tested code:

| type | resolve | text | attachment |
|---|---|---|---|
| `none` (default) | — | — | report only stored |
| `slack` / `telegram` | `IntegrationsRepo::get(ws, channel)` + Keychain token | `improve_notify::send_to(secrets,&integ,chat,None,msg)` | `build_adapter(secrets,&integ)?.upload(chat,None,"report.md",&bytes)` |
| `email` | workspace owner's verified `email_senders` row + Keychain app pw | `GmailSender::send(to,subject,body)` | **new**: extend `GmailSender` to a lettre `MultiPart::mixed` with the `.md` attached |
| `webhook` | the URL (SSRF-checked) | `WebhookAdapter::new(Some(url)).send_formatted(...)` | `.upload(...)` (base64 JSON) — `WebhookAdapter` already runs `otto_netguard::check_url` + `redirect_policy` |

`destination` JSON shape: `{ "type":"slack", "chat_id":"C0..." }`,
`{ "type":"email", "to":"x@y.com", "subject":"..." }`,
`{ "type":"webhook", "url":"https://..." }`. Delivery is **best-effort**: failure
sets `delivered=0` + `delivery_error` but the run is still `ok` (the report exists).
The **only net-new sending code** is the email-attachment MultiPart (one small
function on `GmailSender`); everything else is reuse.

## 8. MCP surface (R5, R6)

Extend **surface A** (`crates/otto-server/src/mcp_outward.rs`) — the `ottod
mcp-server` `otto.*` tools — using the proven recipe (spec in `otto_tool_specs()`
+ dispatch arm in `run_tool` self-calling the REST endpoint via the ephemeral
full-token, preserving native RBAC + audit). New tools:

| tool | mutating | DANGEROUS | default-enabled | backing endpoint |
|---|---|---|---|---|
| `otto.list_scheduled_tasks` | false | — | **yes** | `GET /workspaces/{ws}/scheduled-tasks` |
| `otto.list_scheduled_task_runs` | false | — | **yes** | `GET /scheduled-tasks/{id}/runs` |
| `otto.create_scheduled_task` | true | **yes** | no | `POST /workspaces/{ws}/scheduled-tasks` |
| `otto.update_scheduled_task` | true | **yes** | no | `PATCH /scheduled-tasks/{id}` |
| `otto.delete_scheduled_task` | true | **yes** | no | `DELETE /scheduled-tasks/{id}` |
| `otto.run_scheduled_task` | true | **yes** | no | `POST /scheduled-tasks/{id}/run` |
| `otto.set_scheduled_task_enabled` | true | **yes** | no | `PATCH /scheduled-tasks/{id}` |

Write tools are `mutating:true`, added to `DANGEROUS` (human-approval gated when
`mcp_require_approval_dangerous` is on — default on), and **off by default** (admin
flips them on in the **Otto Server** MCP tab; they auto-appear there). Reads are
default-enabled so an agent can immediately inspect jobs. This matches the safety
model of the existing `create_work_item` / `run_goal_loop` DANGEROUS tools and
honors R6 (the admin enables creation once; thereafter agents create jobs).

No feature-guard allow-list or governance-pipeline change is needed — external mcp
tokens always funnel through `POST /mcp/otto-tools/invoke`, which self-calls our
endpoints with an ephemeral full token.

## 9. API, persistence, RBAC, events

**Migration `0084_scheduled_tasks.sql`** (append-only) — two tables:

`scheduled_tasks(id PK, workspace_id→workspaces ON DELETE CASCADE, name, kind
DEFAULT 'agent_prompt', prompt, skill, provider DEFAULT 'claude', model DEFAULT '',
cwd DEFAULT '', schedule_json DEFAULT '{}', destination_json DEFAULT '{}', enabled
INTEGER DEFAULT 1, last_run_at, last_status, next_run_at, created_by, created_at,
updated_at)` + indexes `(workspace_id, enabled)`, `(enabled)`.

`scheduled_task_runs(id PK, task_id→scheduled_tasks ON DELETE CASCADE,
workspace_id, status DEFAULT 'running', trigger DEFAULT 'schedule', started_at,
finished_at, summary, report_path, report_rel, delivered INTEGER DEFAULT 0,
delivery_error, error, session_id, created_at)` + indexes `(task_id, started_at
DESC)`, `(workspace_id, started_at DESC)`.

**Repo** `crates/otto-state/src/scheduled_tasks.rs` (manual row mappers, `convert.rs`
helpers, `dberr`), registered in `lib.rs`. **Domain types** `ScheduledTask`,
`ScheduledTaskRun`, `ScheduledTaskPreset` in `otto-core/src/domain.rs`.

**Endpoints** (`routes/scheduled_tasks.rs`, mounted in `modules.rs`):
- `GET  /workspaces/{id}/scheduled-tasks` — list — ws Viewer · ScheduledTasks View
- `POST /workspaces/{id}/scheduled-tasks` — create — ws Editor · Edit
- `GET  /scheduled-tasks/{id}` — get — ws Viewer · View
- `PATCH /scheduled-tasks/{id}` — update — ws Editor · Edit
- `DELETE /scheduled-tasks/{id}` — delete — ws Editor · Edit
- `POST /scheduled-tasks/{id}/run` — run now — ws Editor · Edit
- `GET  /scheduled-tasks/{id}/runs` — run history — ws Viewer · View
- `GET  /scheduled-tasks/runs/{run_id}/report` — report markdown — ws Viewer · View
- `GET  /scheduled-tasks/presets` — preset templates — ScheduledTasks View

**RBAC two-axis**: `policy.rs` arms gate by `Feature::ScheduledTasks` (View for GET,
Edit for writes); handlers additionally `require_ws_role` on the task's
`workspace_id` (flat by-id routes load the task first ⇒ IDOR guard). `Feature`
touch-points: enum variant + `parse` + `as_str` (domain.rs) + `grants.rs
ALL_FEATURES` + `types.ts` union + the policy arm.

**Event** `Event::ScheduledTaskRunUpdated { workspace_id, task_id, run_id, status }`
— `event.rs` variant + `ws_events.rs::scope_of` → `Scope::Workspace` (exhaustive
match, compiler-enforced) + `types.ts` `OttoEvent` member + `events.svelte.ts`
dispatch.

**Contracts**: `api.md` group starting at #135; `ws.md`
`### scheduled_task_run_updated`; `types.ts` in lockstep.

## 10. UI

`ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte` (+ components):
- **List**: name, cadence summary, destination badge, enabled toggle, last-run
  status pill, next-run time, Run-now, Edit, Delete.
- **Editor**: name, preset picker (pre-fills prompt+cadence — exposes R2),
  prompt (textarea), cadence (interval minutes / daily HH:MM / weekly), optional
  skill, provider/model, destination picker (none/slack/telegram/email/webhook +
  fields), enabled.
- **Runs**: per-task history (status, started, summary, delivered badge) + a
  report viewer (renders the markdown).
- Store `lib/stores/scheduledTasks.svelte.ts`, API `lib/api/scheduledTasks.ts`,
  types in `types.ts`, sidebar entry (`{id:'scheduled-tasks', icon:'clock',
  label:'Scheduled Tasks', feature:'scheduled_tasks'}`), `App.svelte` import +
  render arm + command-palette entry, `events.svelte.ts` dispatch arm.

## 11. Security & robustness

- **SSRF**: webhook delivery uses `WebhookAdapter` (already `check_url` +
  `redirect_policy`); never a raw `reqwest` to a user URL.
- **AuthZ**: only ws Editors create/run; flat by-id routes load the task and
  `require_ws_role` (IDOR). MCP write tools are DANGEROUS + approval-gated +
  off-by-default.
- **Path traversal**: the report route serves by `run_id` → `report_path`, and
  validates the resolved path is **inside `data_dir/scheduled/`** before reading.
- **Resource bounds**: per-task in-flight guard (no overlap); prune to the most
  recent 100 runs per task (delete old rows + report files) on each new run;
  per-run no-progress timeout.
- **Cursor-before-run** prevents double-fire across ticks/restarts.
- **Data leaving the machine**: delivery to Slack/email/webhook is the *explicit
  purpose*; the report body is the intended artifact (not redacted, like insights
  reports). RBAC + DANGEROUS gating bound who can configure it.

## 12. Testing

- **Pure unit (Rust)**: `cadence::is_due` (interval/daily/weekly + catch-up + DST
  skip) and `next_run`; `extract_summary`; prompt wrapping; webhook/email/slack
  **message building** + destination resolution; report filename.
- **Repo tests** (`test_pool`): tasks CRUD; runs insert/list/prune; due-selection
  query.
- **E2E (Playwright)** mirroring `mission-control.spec.ts`: create an
  `agent_prompt` task via API → `POST …/run` (OTTO_E2E stub) → poll `…/runs` until
  `ok` → assert `summary` non-empty → `GET …/report` → assert markdown → open the
  Scheduled Tasks page → see task + run + report; create with `interval` → assert
  `next_run_at` set. Branch backend via `OTTO_E2E_BIN=target/debug/ottod`.
- **Gates**: `cargo build/test/clippy -D`, `npm run check/build`, targeted E2E.

## 13. File touch-list (implementation order)

1. `crates/otto-state/migrations/0084_scheduled_tasks.sql`
2. `crates/otto-core/src/domain.rs` (types + `Feature::ScheduledTasks` ×3), `event.rs`
3. `crates/otto-state/src/scheduled_tasks.rs` + `lib.rs` registration
4. `crates/otto-channels/src/email.rs` (`GmailSender::send_with_attachment`)
5. `crates/otto-server/src/cadence.rs` (pure) — or inline in engine
6. `crates/otto-server/src/scheduled_tasks_engine.rs` (run_task, summary, deliver, report I/O)
7. `crates/otto-server/src/scheduled_tasks_scheduler.rs` (supervisor)
8. `crates/otto-server/src/routes/scheduled_tasks.rs` + `routes/mod.rs` + `modules.rs` mount
9. `crates/otto-server/src/policy.rs` arm + `routes/grants.rs` ALL_FEATURES
10. `crates/otto-server/src/ws_events.rs` `scope_of` arm
11. `crates/otto-server/src/state.rs` field + `crates/ottod/src/main.rs` construct + scheduler spawn
12. `crates/otto-server/src/mcp_outward.rs` (7 tools: specs + dispatch + DANGEROUS)
13. `docs/contracts/api.md`, `docs/contracts/ws.md`, `docs/features/scheduled-tasks.md`
14. `ui/src/lib/api/types.ts`, `lib/api/scheduledTasks.ts`, `lib/stores/scheduledTasks.svelte.ts`
15. `ui/src/modules/scheduled-tasks/*`, `lib/sidebar.ts`, `shell/App.svelte`, `lib/events.svelte.ts`
16. Tests (inline Rust + `ui/e2e/scheduled-tasks.spec.ts` / `desktop-scheduled-tasks.spec.ts`)
17. `crates/otto-orchestrator/src/e2e_stub.rs` (sentinel branch for deterministic E2E — see §15 MAJOR-3)

## 14. Review outcomes (2 independent reviews — both "Approve with fixes", no blockers)

An architecture/requirements review and a security review validated this design against
the real code. Both approve. The fixes below are **folded into the plan** and supersede the
matching prose above.

### Concurrency model (architecture MAJOR-1) — corrected
Use the **`cli_update.rs` ordering**, NOT the "advance-before-run" of workflow_trigger:
- The scheduler tick **claims the per-task in-flight guard FIRST**. If already in-flight →
  **skip and do NOT touch the cursor** (so the occurrence isn't lost).
- `run_task` advances `last_run_at = now` + recomputes `next_run_at` **only on completion**
  and **only for `trigger=schedule`** (manual run-now never moves the cursor). This is
  **at-least-once** with overlap protection + natural catch-up — the right default for a
  report job (the design's earlier "advance-before-run" risked silently dropping an
  occurrence and orphaning a `running` row on crash).
- **Startup reaper**: on scheduler start, mark every `scheduled_task_runs` row still
  `status=running` as `status=error` (`error="interrupted by daemon restart"`) — the
  in-flight guard is in-memory and resets empty on restart.

### R2 ticket example reachability (architecture MAJOR-2)
The `ticket-followup-review` preset sets **`cwd` = the workspace repo path** (so the
agent's `CLAUDE.md`, project skills, and the user's global Atlassian MCP are in scope) and
its prompt instructs the agent to use the available Jira/Atlassian tools with a
configurable JQL/project. `docs/features/scheduled-tasks.md` documents the prerequisite:
the daemon's claude environment must have the Atlassian MCP authenticated. The engine stays
generic; pre-fetching tickets server-side is a noted follow-up.

### E2E determinism (architecture MAJOR-3)
The prompt-wrap embeds an `OTTO_TASK: scheduled_task` sentinel; a matching branch in
`crates/otto-orchestrator/src/e2e_stub.rs` returns a representative markdown report (title +
a `Reviewed/New comments/...` counts header + a `---` rule + details) so the E2E genuinely
exercises `extract_summary` (the `---` split) and the report viewer — not the bare `"OK"`.

### Summary contract alignment (architecture MINOR-6)
The prompt-wrap explicitly instructs the agent to separate the summary from details with a
`---` horizontal rule, matching `extract_summary`'s split key.

### Provider scope (architecture MINOR-4/5)
> **Updated in v2.** Any provider (`claude`/`codex`/`agy`/`shell`/custom) is
> accepted and runs as a **real, openable session** (`run.session_id` is now
> populated); `shell` runs the prompt as a command; a no-owner task is the only
> claude-only path (the headless fallback). The v1 text below is historical.

v1 was **claude-only**: create/update validated-rejected any non-`claude` provider
(400); the UI provider field was fixed to claude. `session_id` was kept but
commented "reserved" (run_agent used an ephemeral PTY, no Otto session row in v1).

### MCP read tools default-enabled (architecture MINOR-7)
Explicitly add `list_scheduled_tasks` + `list_scheduled_task_runs` to `DEFAULT_ENABLED` in
`mcp_outward.rs`. Write tools stay off-by-default + `DANGEROUS`.

### Security must-fixes (security review — all folded in)
- **IDOR**: `GET /scheduled-tasks/runs/{run_id}/report` loads the **run**, takes
  `run.workspace_id`, `require_ws_role(Viewer)` **before** reading. Cross-workspace E2E.
- **Path safety**: report path segments are the **server task id + a server timestamp**
  (never the user `name`); the route `canonicalize`s **both** the base dir and the resolved
  path and `starts_with`-checks (mirror `insights::get_report`); `report_path` is an
  **output-only** column (never settable via create/update).
- **Exfil**: the **delivered** report text + attachment are run through
  `otto_core::redact::redact_text` for external destinations; the locally-stored report
  stays full (behind RBAC).
- **Resource bounds**: a **min-interval floor** (`every_min >= 5`) enforced at
  create/update (400 otherwise); a process-wide **`Arc<Semaphore>`** caps concurrent
  scheduled-task runs (default 2).
- **Untrusted input**: document that `cwd` is **not** a security boundary; the preset is
  read-only analysis; Editor-only + DANGEROUS-approval gating bounds who configures it.
  Per-task tool sandboxing is a documented follow-up.
- **MCP approval clarity**: the dangerous-tool approval `detail` for scheduled-task
  create/update summarizes prompt + cadence + destination (args are also in
  `args_redacted_json`).
- **DNS-rebind TOCTOU** in `otto-netguard` is pre-existing + repo-wide (same guard used
  everywhere); noted, deferred.
