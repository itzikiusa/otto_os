---
id: workflows
title: Workflows
group: Automate
route: workflows
summary: Chain agents, HTTP calls, queries, reviews, approvals and PRs into a graph, then run it by hand, on a schedule, from a webhook, an event or a chat message.
---
## What it's for

A workflow is a graph of steps you build once and run many times. Each step is a node — an agent turn, an HTTP call, a database query, a code review, a pull request, an approval — and edges decide what runs next. Every agent step runs as a real session you can open while it works, and every run keeps its files, logs and outputs so you can inspect or retry any step.

## Getting started

1. Open **Workflows**.
2. In **Describe the flow**, type what you want ("Ask an agent to summarize the repo, then POST the summary to our webhook") and choose **Generate workflow**. Or choose **Start blank**, or pick one from **Templates**.
3. On the canvas, choose **Node** to add steps, and drag from a node's right port to another node's left port to connect them.
4. Select a node to set its options in the inspector. Choose **Save**.
5. Choose **Validate** to check the graph, then **Run…**, set the run input (or choose **Suggest**) and start it.
6. Watch each step in the run view. Choose **Open session** on any agent step to see its terminal.
7. To run it automatically, choose **Triggers** and add a schedule, webhook, event or chat binding.

## Everything it can do

**Creating**
- Generate a graph from a plain-language description. If generation fails you still get a runnable Start → Agent graph seeded with your description.
- Start blank, duplicate an existing workflow, or start from a template:
  - **Write tests for a story**, **Implement a feature from a story**, **UI test authoring**, **API acceptance test authoring** — prepare → implement → a fix/review loop → a PR opened automatically once the review passes.
  - **PO discovery → RFC/Jira** — discovery draft → diagram → approval → rewrite → approval → publish (dry run).
  - 3 game pipelines: **Slots game**, **Crash game (Aviator style)** and **Scratch card**.
- Rename inline, duplicate or delete from the list.

**Node types (26)**
- **Triggers:** Manual Trigger — the entry point; emits the run input.
- **AI:**
  - **Agent** — an agent turn with a prompt, provider, model and optional skills inlined.
  - **Prepare relevant data** — fetches a referenced Jira ticket into the run's files, then optionally runs an analysis agent.
  - **Swarm Task** — adds a task to a running swarm project.
  - **Review Run** — the multi-provider × multi-lens code review; outputs findings, a 0–100 score and pass/fail. Fan-out mode (one agent per lens × provider) or orchestrator mode (one agent per provider running every lens).
  - **Self-Improve (offer)** — reflects on recent sessions and offers skill/memory edits for approval. Nothing is applied automatically.
- **Network:**
  - **HTTP Request** — calls a URL.
  - **API Run** — sends a request through the API client, so environments and auth apply.
  - **Git PR** — drafts a PR with an agent-written title and description. Opens it on the remote only when `open` is on.
- **Data:**
  - **Set / Transform** — merges static JSON into the data.
  - **DB Query** — a read-only query on a saved database connection (100 rows by default).
  - **Broker Peek** — reads up to 50 recent Kafka messages.
- **Flow:**
  - **Delay** (up to 10 s) and **Log**.
  - **Budget Gate** — stops the run when a provider spend cap is blocked.
  - **Human Approval** — pauses until someone chooses **Approve** or **Reject**.
  - **Condition** — evaluates an expression.
  - **Loop (Until)** — repeats inner steps until an expression holds, 1–10 iterations. Loops can't be nested.
- **Integrations:** Channel Notify — posts to a configured Slack or Telegram integration.
- **Product:** Product Analyze, Product Rewrite, Product Plan, Product Publish (Confluence RFC or Jira issue, dry run by default), and Canvas Diagram (Mermaid or Excalidraw).
- **Game:** Game Engine and Verifier — scaffolds that produce a template spec, not a certified build.

**Editing**
- Pan by dragging the background or scrolling with 2 fingers; pinch to zoom (0.3×–2×). The **+**, **−** and reset buttons do the same.
- Edge conditions: an edge runs only when its expression is true, so a Condition node plus conditional edges gives you if/else branches. A branch not taken is skipped without failing the run.
- A safe expression language for conditions, loops and `{{ … }}` templating (comparisons, `contains`, `in`, `len`, `lower`, `default`, `has` and more).
- Per-node retry: attempts (up to 5 extra) and backoff. Agent steps retry twice by default; **Use default** restores that.
- **Instructions:** standing rules every step follows, saved and versioned with the graph.
- **Tidy** reflows the graph into readable rows. **Dock** moves the node inspector between the bottom and a resizable side panel.
- **Versions:** every saved change is a snapshot; **Restore** any of them.
- **Validate** flags duplicate ids, missing endpoints, cycles, unknown kinds, missing required settings and bad expressions, and highlights them on the canvas.

**Running**
- **Run…** with a JSON input (for example `repo_id`, `story_id`, `goals`, `msg`) and an optional review-mode override for every review step.
- From a selected node: **▶ From here** (this node and everything after it) or **Only this**.
- **Stop** finishes the current step, then halts.
- At most 2 runs execute at once across the app; extra runs wait as **queued** and start in order.
- Runs survive a daemon restart and resume from the step they were on.
- Run view: status per step, attempt counts, logs, the step's work product (copyable), sub-agent rows, a zoomed step view, and paged checkpoints.
- **Retry step** re-runs only a failed step; **Re-run from here** re-runs a step and everything after it. Both keep the run's files and worktree.
- **Final output** shows the run's deliverable. Each run links to the proof pack assembled from it.
- The sidebar shows a count of running workflows; open any from the **Running** list.

**Triggers**
- **Schedule:** every N minutes, daily, weekly or 5-field cron, in any IANA timezone, with an optional run prompt.
- **Webhook:** a secret URL; anything that POSTs to it starts a run with the request body as input.
- **Event:** review changed, budget exceeded, product changed, swarm status, improvement finished, insight ready — with an optional JSON filter.
- **Chat binding:** a Slack or Telegram chat (optionally one thread, optionally @mention only on Slack); matching messages start a run.
- Any trigger can send results to a Slack or Telegram chat/thread and to a result webhook.
- **Preview / validate** checks a trigger without saving and shows the next 5 fire times for a schedule.
- **Trigger from chat:** copy a ready-made `Action: Workflow` message to post in Slack or Telegram. `run <name>: <prompt>` works too. Chat-started runs post live step progress back into the thread; reply `status`, `skip` or `abort` to control them.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `⌘↩` | Generate workflow (in **Describe the flow**) |
| `↩` | Save the new name while renaming a workflow |
| `Esc` | Cancel renaming |
| `⌃` + scroll | Zoom the canvas (same as a trackpad pinch) |

## Tips and limits

- Node settings are stored as plain JSON. Don't put secrets in them — use **API Run** with an API-client environment, or an integration whose credentials live in the Keychain.
- A webhook URL is the credential. Anyone who has it can start runs; delete the trigger to revoke it. The daemon is only reachable from outside your Mac if you enable remote access.
- Anyone who can post in a bound Slack or Telegram channel can start that workspace's workflows by name.
- Several nodes need other features set up first: **DB Query** (a saved connection), **Broker Peek** (a Kafka cluster), **Swarm Task** (a running swarm), **Review Run** and **Git PR** (a registered repository), Product nodes (a story, and a Jira/Confluence account to publish).
- A cycle in the graph fails the run before any step starts. A run stops after 10 hours.
- An agent step that shows no real progress for 5 minutes is treated as stuck and retried in a fresh session. The 5-minute limit is a daemon setting and can be raised or turned off.
- DB Query is always read-only; Broker Peek never produces.
- Viewers can see workflows, runs and triggers. Creating, editing, running, approving and changing triggers need the workspace Editor role and the Workflows feature.

## Related

- [Scheduled Tasks](#/walkthroughs/scheduled-tasks)
- [Swarm](#/walkthroughs/swarm)
- [Goal Loops](#/walkthroughs/loops)
- [Git](#/walkthroughs/git)
- [Proof](#/walkthroughs/proof)
- [Product](#/walkthroughs/product)
- [Mission Control](#/walkthroughs/mission-control)
