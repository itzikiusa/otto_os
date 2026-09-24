---
id: scheduled-tasks
title: Scheduled Tasks
group: Automate
route: scheduled-tasks
summary: Run an agent, a shell command or a workflow on a cadence, keep a Markdown report of each run, and deliver it to Slack, Telegram, email or a webhook.
---
## What it's for

A scheduled task is a recurring job: "every hour, review the tickets updated in the last day and report what changed". Each run produces a Markdown report that Otto stores and, if you choose, delivers. Agent runs are real sessions, so you can open one to watch or unblock it.

## Getting started

1. Open **Scheduled Tasks** and choose **New task**.
2. Optionally pick **Start from a preset** to fill in a ready-made job.
3. Give it a **Name** and pick a **Type**: **Run an agent** or **Hand off to a workflow**.
4. For an agent, pick the **Provider** and model, then write the **Prompt** — what the agent should do and report.
5. Pick a **Cadence** and, for daily, weekly and cron, a **Timezone**.
6. Pick a **Destination**, or leave **None (store only)**.
7. Choose **Save**, then **Run now** to try it straight away. Choose **Runs** to see the result and **View report**.

## Everything it can do

**What runs**
- **Run an agent:** any installed provider (Claude, Codex, Antigravity…) or a custom provider slug registered in Settings, with an optional model.
- **Shell:** choose the `shell` provider and the prompt becomes a command; stdout, stderr and the exit code are captured.
- **Hand off to a workflow:** launches a workflow run each time and reports its step-by-step outcome.
- **Skill (optional, inlined):** a skill whose instructions are added to the prompt.
- **Working dir:** where the agent or command runs.
- **Sandbox:** run in the working dir, or in a fresh **isolated git worktree** per run (kept for inspection).

**Cadence**
- **Interval:** every N minutes (minimum 5).
- **Daily** at `HH:MM`, **Weekly** on a weekday at `HH:MM`, or **Cron** (5 fields, for example `0 9 * * 1`).
- Daily, weekly and cron times use the task's timezone, including daylight-saving changes. New tasks default to your Mac's timezone.
- A missed daily or weekly window still fires at the next check.

**Reports and delivery**
- Each run's report is stored in full; the run list shows its summary.
- **Destination:** **Slack** or **Telegram** (the integration's channel, or a chat/channel id you enter), **Email** (sent from your verified Gmail sender), or **HTTP webhook**.
- Delivered copies are redacted before they leave your Mac. Webhook targets are checked so private and loopback addresses are refused.
- **Only notify on meaningful change:** skips delivery when the report matches the last one; the run shows "no change".
- **Attach a proof pack to each run.**
- **Retries on failure (0–5):** a failed or stuck agent or shell run is stopped and tried again.

**Presets**
- **Processed-ticket follow-up review** — hourly ticket re-review.
- **Weekly security scan**, **Weekly code-quality review**, **Weekly dependency / PR scan** — each fills in a worktree sandbox; the security and dependency presets turn on "only notify on change", the code-quality preset attaches a proof pack.

**Managing tasks**
- Per task: **Run now**, **Runs**, **Pause** / **Enable**, **Edit**, **To workflow**, and delete.
- **To workflow** turns a task into a multi-step workflow with a matching schedule trigger.
- Run rows show status, start time, summary, attempts, delivered / delivery failed / no change, and links to **View report**, **Open session**, the proof pack and the workflow run.
- The last 100 runs of each task are kept.

**From other agents**
- 8 `otto.*` MCP tools let an agent list, read and see runs of tasks. The 5 that create, change, enable, run or delete are off until an admin turns them on in MCP Control Plane, and each call asks for approval by default.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `Esc` | Close the report viewer |

## Tips and limits

- Otto doesn't fetch tickets or pages itself — the agent uses the tools it has. The ticket preset needs a Jira/Atlassian MCP server set up for the provider, and a working dir that points at the right project.
- A custom provider must be registered in Settings first, or the run fails with "unknown provider".
- The working dir and the worktree sandbox are not a security boundary. An unattended agent can reach anything your user account can, so point tasks only at repos you trust it in.
- A run with no progress for 10 minutes is treated as stuck.
- At most 2 scheduled-task runs execute at once; a task never overlaps itself.
- Retries apply to agent and shell runs, not to workflow hand-offs.
- Delivery failures don't fail the run — the report is still stored, and the run shows "delivery failed".
- Email delivery needs a verified sender in Settings; Slack and Telegram need a configured integration.
- Viewers can list tasks and read reports. Creating, editing, running and deleting need the workspace Editor role and the Scheduled Tasks feature.

## Related

- [Workflows](#/walkthroughs/workflows)
- [Personal Agents](#/walkthroughs/personal-agents)
- [Proof](#/walkthroughs/proof)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Agents](#/walkthroughs/agents)
