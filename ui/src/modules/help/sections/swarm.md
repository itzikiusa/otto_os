---
id: swarm
title: Swarm
group: Automate
route: swarm
summary: A team of role agents in an org chart that works projects on a Kanban board — leaders delegate, work is handed off and reviewed, and every agent is a session you can open.
---
## What it's for

A swarm is a small autonomous team: a CTO, a couple of developers, a designer, QA. You give it a project with a goal, break the goal into tasks, and start it. A coordinator keeps ready tasks flowing onto the right agents within a parallel limit and a budget. Leaders split their tasks for their reports, finished work is handed off or sent for review, and the team posts ideas, reviews and decisions to a shared feed you can read and post into.

## Getting started

1. Open **Swarm** and create a swarm. Give it a **Name** and pick a preset, or **Blank**. Choose **Create**.
2. Build the team: choose **Recruit**, name a role ("Backend Dev"), and choose **Propose agent**. Edit the draft and choose **Hire**. Or, in the **Org** view, add an agent by hand.
3. Drag agents onto each other in **Org** to set who reports to whom.
4. Choose **Project**, give it a name, an optional repo path and a goal. Choose **Create**.
5. On the project **Board**, choose **Plan from goal** to break the goal into tasks, or **Add task** to write them yourself.
6. Choose **Start**. Watch the work in **Org**, **Graph**, **Board**, **Runs** and **Feed**.

## Everything it can do

**Swarms and presets**
- 6 presets, each a full org with projects and schedules: **Startup Pod**, **Engineering Squad**, **Product Studio**, **Research Lab**, **Security & Audit**, **Product / PO Team**.
- Preset agents are mapped to providers you have installed, falling back to the workspace default.
- Several swarms per workspace in the left rail, each with a status dot. **Look in my other workspaces** finds swarms elsewhere.

**Agents**
- **Recruit:** an AI recruiter drafts name, title, provider, reports-to, specialization, soul, scope, skills (★ marks must-use) and an optional schedule — only from skills and providers you actually have. Edit everything before **Hire**.
- Agent editor: name, title, provider and model, reports-to, avatar, specialization, soul, scope, skills with a must-use toggle, and scheduled runs (every N minutes, daily or weekly, with a standing directive).
- Row menu: **Edit agent**, **Duplicate agent**, **Run a task…**, **Add direct report**, **Move to top level**, **Delete agent**.
- An agent with reports is a leader: given a task, it plans subtasks for its reports instead of doing the work itself.

**Projects and tasks**
- Several projects per swarm, each with its own board, optional repo path and goal. **Set goal** / **Edit goal** and **Project** settings (name, repo, goal, skills).
- **Plan from goal** runs several planner agents plus a summarizer and wires task dependencies so independent work runs in parallel. **Stop** stops waiting for it.
- Board columns: Backlog, To do, In progress, In review, Blocked, Done, Cancelled. Drag cards to change status.
- Card menu: **Run now**, **Goals…**, **Move to**, **Assign to**, **Delete**.
- Tick several cards to **Move to…**, **Assign…** or **Delete** them together. **Clear board** deletes every task on the board.
- Per-task **goals**: a title, what the verifier checks, an optional metric with a comparator (≤, ≥, =, contains, absent), target and block values, a verify command, max retries, and a blocking flag. The coordinator verifies them before a task counts as done.
- A project created from a Product story shows a **From story** card with **View story**.

**Running**
- **Start**, **Pause**, **Resume**, **Abort all**. Pause lets in-flight turns finish; Abort all stops runs and ends the swarm's sessions.
- **parallel** sets how many agent turns run at once. Each agent runs one turn at a time.
- Budgets: a run-count bar (select it to change the limit, blank = unlimited) and a cost bar. A swarm that hits a budget pauses itself with the reason shown; **Raise budget & resume** continues it.
- Finished turns are routed automatically: handoffs become new tasks for the named role, review requests create review tasks, parents roll up to done when their children finish, and a task that keeps failing is blocked with an escalation.
- Agents on a schedule run their standing directive on their cadence while the swarm is active.

**Views**
- **Org:** the org chart with schedule badges, active-run counts and status dots; expand a row to see its open sessions.
- **Graph:** a live graph of the team with completed-run and to-address counts per agent, and task search.
- **Board:** the Kanban board.
- **Runs:** every agent turn, filterable by assignee, project and status, with tokens in/out. **Inspect run** shows the brief sent, working directory, artifacts, cost, findings, board posts and raw result. **Open** the session or **Stop** a run.
- **Feed:** the team's message stream (ideas, reviews, decisions, concerns, status). Pick a kind and post yourself.
- Opening a session shows it in a resizable panel beside the view.

**Settings**
- **Standing goals:** quality bars verified on every task.
- **Team skills:** added to every agent in the swarm.
- **Triggers:** launch swarm work from a Slack or Telegram message or a webhook, matched by chat and an optional keyword, with an optional repo path.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `↩` | Propose agent (in the Recruit role box) |
| `↩` | Add the task (in **Task title…**) |
| `↩` | Post to the feed |
| `↩` | Add the skill (in a skill box) |
| `↩` or `Space` | Inspect the focused run in **Runs** |
| `⌃` + scroll | Zoom the graph (same as a trackpad pinch) |

## Tips and limits

- Only tasks in **To do** whose dependencies are **Done** get picked up. Backlog is a parking lot — drag a card to To do when it is ready.
- Agent schedules use UTC and fire only while the swarm is active, under its parallel limit, and when the agent is free.
- New swarms default to 3,000 runs, 4 hours of runtime since the last start, 3 attempts per task and no cost limit, unless a preset sets its own. The cost limit is checked once per coordinator tick, so treat it as soft.
- Starting and resuming are also blocked when the workspace usage budget is over its cap.
- After a daemon restart, runs that were in flight are marked as errors; start or resume the swarm to continue.
- A swarm belongs to one workspace. Viewers can watch everything; changing anything, including start and stop, needs the workspace Editor role and the Swarm feature.
- Anyone who can post in a channel with a swarm trigger can start work in that swarm.

## Related

- [Agents](#/walkthroughs/agents)
- [Product](#/walkthroughs/product)
- [Workflows](#/walkthroughs/workflows)
- [Goal Loops](#/walkthroughs/loops)
- [Mission Control](#/walkthroughs/mission-control)
- [Usage](#/walkthroughs/usage)
