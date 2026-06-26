# Scheduled Tasks

Recurring, workspace-scoped jobs. A scheduled task runs an **agent** with a
configurable prompt on a **cadence** (every hour, daily at a time, weekly on a
day), turns the agent's final reply into a **Markdown report**, stores it, and
**delivers** it to an optional destination (Slack, Telegram, email, or an HTTP
webhook). Tasks can be created and managed from the UI, the REST API, and — so an
agent can be asked *"add me a scheduled job"* — over **MCP**.

It is the generic engine behind examples like an hourly *"go over every ticket
updated in the last 24h, re-analyze it, check for new comments, and produce a
follow-up report"* — shipped as the built-in **`ticket-followup-review` preset**.

## How it works

```
Scheduler (60s tick, ottod)         REST  POST /scheduled-tasks/{id}/run
   └─ due tasks ──────────────┐        └──────────────┐
MCP otto.create/run_* ──▶ REST endpoints              │
                                       run_task(task, trigger):
                                         create run row → emit ws event
                                         run agent (Orchestrator::run_agent)
                                         extract summary + write report.md
                                         deliver to destination (redacted)
                                         advance cursor (schedule only) → emit
```

- **Execution** uses `Orchestrator::run_agent` — the same headless primitive the
  self-improvement engine uses — so a task "triggers an agent" exactly like
  self-improvement. v1 runs **claude**; an optional `skill` is inlined into the
  prompt. A process-wide semaphore (`OTTO_SCHEDULED_MAX_CONCURRENT`, default 2)
  bounds concurrent runs.
- **Cadence** is `interval` (`every_min`, floor 5), `daily` (`at:"HH:MM"`), or
  `weekly` (`at` + `weekday` 0=Mon…6=Sun). Daily/weekly use catch-up comparison so
  a missed window still fires at the next opportunity. The scheduler advances the
  `last_run_at` cursor only **on run completion** and claims a per-task in-flight
  guard first, so an overlapping or crash-interrupted occurrence is retried, not
  silently dropped. On startup, interrupted `running` runs are reaped to `error`.
- **Report** = the agent's final message. The first part (up to a `---` rule, else
  ~800 chars) becomes the run `summary` shown in the list and the delivery message;
  the full body is stored at `data_dir/scheduled/<task_id>/reports/<ts>.md` and
  served by run id. The 100 most-recent runs per task are kept.

## Creating a task (UI)

Open **Scheduled Tasks** in the sidebar → **New task**. Pick a **preset** to
pre-fill (or start blank), set the **prompt**, the **cadence**, an optional
**skill**, and a **destination**, then save. Use **Run now** to execute
immediately (this does not change the schedule), then open the run to view or
download the report.

## Destinations

| type | fields | notes |
|---|---|---|
| `none` | — | report is only stored (default) |
| `slack` / `telegram` | `chat_id?` | posts the summary + attaches `report.md` to the workspace's configured channel (or `chat_id`) |
| `email` | `to`, `subject?` | sends via the task owner's **verified** Gmail sender (Settings → Email sender); the report is attached |
| `webhook` | `url` | POSTs the summary + (base64) report; **SSRF-guarded** by `otto_netguard` (loopback/private/metadata blocked) |

Delivered report bodies and attachments are passed through `otto_core::redact`
before they leave the machine.

## The ticket follow-up example (prerequisite)

The `ticket-followup-review` preset asks the agent to query Jira/Atlassian for
tickets updated in the last 24h and report on new comments. For this to work the
**daemon's claude environment must have the Atlassian/Jira MCP server configured
and authenticated** (the same `~/.claude` MCP config a normal claude session
uses), and the task's `cwd` should point at a workspace/repo so its `CLAUDE.md`
and project skills are in scope. The engine stays generic — it does not fetch
tickets itself; the agent uses whatever tools its environment provides.

## MCP

The outward `otto.*` MCP surface (Settings → MCP → **Otto Server**) exposes:
`otto.list_scheduled_tasks` and `otto.list_scheduled_task_runs` (read,
**default-enabled**), and `otto.create_scheduled_task`, `otto.update_scheduled_task`,
`otto.set_scheduled_task_enabled`, `otto.run_scheduled_task`,
`otto.delete_scheduled_task` (write, **DANGEROUS**, off by default,
human-approval-gated). An admin enables the write tools once; thereafter an agent
can create a job, and each dangerous call surfaces the prompt + cadence +
destination to the human approver. Each tool self-calls its REST endpoint with an
ephemeral token, so the endpoint's native RBAC + audit still apply.

## API / WS

REST endpoints #135–#143 and the `otto.*` tools are documented in
`docs/contracts/api.md` (Scheduled Tasks). The live signal
`scheduled_task_run_updated` is in `docs/contracts/ws.md`.

## Security & limits

- **RBAC**: only workspace **Editors** can create/update/run/delete; **Viewers**
  can list and read reports. Flat by-id routes load the task/run and check the
  role on its workspace (IDOR guard). The report route serves by run id and
  canonicalizes the path against the reports root (no traversal).
- **`cwd` is not a security boundary.** A coding agent can read/write anywhere the
  daemon user can; the prompt and any external content it reads are treated as
  untrusted (the preset instructs read-only analysis). Point tasks at repos you're
  comfortable an unattended agent operating in. Per-task tool sandboxing is a
  planned enhancement.
- **Resource bounds**: minimum 5-minute interval; a global run semaphore; a
  per-run no-progress (stuck) timeout; run-history pruning.

## Limitations / follow-ups

- v1 is claude-only (`provider` is reserved for codex/agy via `run_cli_exec`).
- The engine doesn't pre-fetch tickets server-side (the agent's MCP tools do).
- No true 5-field cron expressions (the cadence enum is the house style).
- `session_id` on a run is reserved (runs are headless; no Otto session row).
