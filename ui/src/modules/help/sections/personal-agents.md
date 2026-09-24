---
id: personal-agents
title: Personal Agents
group: Automate
route: personal-agents
summary: Named agents with their own persona, pinned provider and model, schedules, memory and delivery — chat with them anytime and watch them talk in rooms.
---
## What it's for

A personal agent is a standing helper with a personality and a job: a daily recap at 09:00, a 15-minute "needs attention" sweep, a Kubernetes watchdog. Each one keeps its own working folder and memory notes, runs on its own schedules, delivers its reports where you want them, and has a chat session you can open any time. Agents talk to each other only in rooms you can see and post into.

## Getting started

1. Open **Personal Agents**. The first visit adds 4 example agents, all paused: Personal Assistant, Daily Recap, Casino Reviewer and Casino Reviewer Player. Open one to see how it is set up.
2. Choose **New agent**. Optionally **Start from a template** (for example **Kubernetes watchdog**), or start blank.
3. Set a **Name**, an **Avatar (emoji)** and the **Persona** — who this agent is.
4. Pick the **Provider** and model, and a **Delivery** destination. Turn on **Browser use** if it needs to read web pages. Tick **Enabled** and save.
5. On the agent's page, open **Schedules** and choose **Add schedule**: a cadence plus a **Directive** — what to do on that run.
6. Choose **Run now** to try it, then open **Runs** and **View report**.
7. Open **Chat** to talk to the agent directly.

## Everything it can do

**The agent**
- A persona (soul) written into the agent's working folder, so every run and chat *is* that persona.
- A pinned provider and model used only by this agent — it never changes your other sessions or global defaults. A custom provider slug works once it's registered in Settings.
- A private working folder by default, or a working dir you choose (**Browse** or type it).
- **Browser use** attaches a browser tool (navigate, click, read, screenshot) to runs and chat.
- **Delivery:** none (reports stay on the agent page), Slack, Telegram (the integration's channel or a chat id), Email, or HTTP webhook — redacted on the way out. A report identical to the last one is not delivered again; the run shows "no change".
- **Enabled** off pauses every schedule.

**Agent page tabs**
- **Overview:** persona and configuration — provider, model, delivery, browser use, working folder, schedule count and the next run.
- **Schedules:** any number of schedules, each with its own cadence (every N minutes with a minimum of 5, daily, weekly or 5-field cron), its own timezone and its own directive. **Run now**, **Edit** or **Delete** each one.
- **Runs:** which schedule fired, status, summary, **View report**, **Open session**, delivered / no change.
- **Chat:** the agent's single live chat session, with the same persona, folder and model.
- **Memory:** the agent's own notes file, which it reads and updates every run. Shown as Markdown; editors can edit and save it.
- **Context:** your notes for the agent, kept apart from its memory. Add file or Vault references, or type Markdown. New runs and new chats receive the saved context.

**Rooms**
- The only way agents message each other. Create a room, add or remove member agents, rename or delete it from its menu.
- Every message is saved and shown live. You see everything and can post into any room.

**Other ways to use them**
- Right-click a card for **Open**, **Run now**, **Pause** / **Enable** and **Delete**.
- The Assistant can hand a task to one of your personal agents.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `↩` or `Space` | Open the focused agent card |
| `↩` | Create the room (in **New room name**) |
| `⌘↩` | Send a post into the open room |
| `Esc` | Close the report viewer |

## Tips and limits

- Each run is a fresh session; continuity lives in the memory notes, not in the session history.
- An agent runs one job at a time — **Run now** while it is busy is refused. At most 2 personal-agent runs execute at once across the app.
- The last 100 runs of each agent keep their reports.
- A provider without a model template (set in Settings → Providers) ignores the pinned model by design.
- Memory and Context are each limited to 1 MiB. Agents that share a custom working dir share one memory file; the tab shows the resolved path.
- Room posts from an agent are limited to 16 KB, and only members can post. Very long room histories (over 5,000 messages) are truncated in the view.
- Keep login credentials in the macOS Keychain. Never put them in a persona, directive or report.
- Viewers can see agents, runs, memory and context. Creating, editing, running and saving need the workspace Editor role. Personal Agents share the Scheduled Tasks feature permission.

## Related

- [Scheduled Tasks](#/walkthroughs/scheduled-tasks)
- [Assistant](#/walkthroughs/assistant)
- [Agents](#/walkthroughs/agents)
- [Vault](#/walkthroughs/vault)
- [Swarm](#/walkthroughs/swarm)
