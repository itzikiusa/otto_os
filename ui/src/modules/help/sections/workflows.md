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

1. Open **Workflows**. It opens on **Runs**: what needs you, what is running, and recent runs, each in words — what the run is doing, whose it is and what it is about.
2. Choose the **Workflows** tab, then **New workflow**. Describe the flow ("Ask an agent to summarize the repo, then POST the summary to our webhook") and choose **Generate**, or pick a template, or choose **Start blank**. The new workflow opens in the editor.
3. Choose **+ Step** to add steps; drag from a card's bottom port onto another card to connect them. Select a step to set it up in the inspector on the right, then choose **Save** (`⌘S`).
4. Choose **Run…**, fill in the form (pull request, ticket, message…) and choose **Run**. The run opens on its own page.
5. On the run page, select a step to see its conversation, output, log and files beside the list. A failed run leads with the failing step's own error and **Retry step**.
6. To run it automatically, open the editor's **⋯ → Triggers** and add a schedule, webhook, event or chat binding.

## Everything it can do

**Creating**
- Generate a graph from a plain-language description. If generation fails you still get a runnable Start → Agent graph seeded with your description.
- Start blank, duplicate an existing workflow, or start from a template:
  - **Write tests for a story**, **Implement a feature from a story**, **UI test authoring**, **API acceptance test authoring** — prepare → implement → a fix/review loop → a PR opened automatically once the review passes.
  - **PO discovery → RFC/Jira** — discovery draft → diagram → approval → rewrite → approval → publish (dry run).
  - 3 game pipelines: **Slots game**, **Crash game (Aviator style)** and **Scratch card**.
- Rename, duplicate or delete from a workflow's row menu in the **Workflows** tab, or from the editor's **⋯**.

**Node types (25)**
- **Triggers:** Manual Trigger — the entry point; emits the run input.
- **AI:**
  - **Agent** — an agent turn with a prompt; provider, model and effort through the same picker as Threads (empty means Default); optional skills; it can continue the conversation of an earlier step with the same provider.
  - **Prepare relevant data** — fetches a referenced Jira ticket into the run's files, then optionally runs an analysis agent.
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
  - **Repeat (Until)** — repeats inner steps until an expression holds, 1–10 iterations. Loops can't be nested.
- **Integrations:** Channel Notify — posts to a configured Slack or Telegram integration.
- **Product:** Product Analyze, Product Rewrite, Product Plan, Product Publish (Confluence RFC or Jira issue, dry run by default), and Canvas Diagram (Mermaid or Excalidraw).
- **Game:** Game Engine and Verifier — scaffolds that produce a template spec, not a certified build.

**Editing**
- The canvas lays the steps out top to bottom and fits them on open. Pan by dragging the background or scrolling with 2 fingers; pinch to zoom (0.3×–2×). The **+**, **−** and **Fit** buttons do the same.
- Edge conditions: an edge runs only when its expression is true, so a Condition step plus conditional edges gives you if/else branches. A branch not taken is skipped without failing the run.
- A safe expression language for conditions, loops and `{{ … }}` templating (comparisons, `contains`, `in`, `len`, `lower`, `default`, `has` and more).
- Per-step retry: attempts (up to 5 extra) and backoff, under **Advanced**.
- **⋯** holds **Runs of this workflow**, **Optimize…**, **Triggers**, **Instructions** (standing rules every step follows, saved and versioned with the graph), **Versions** (every save is a snapshot; **Restore** any of them, **Show older** for more), **Validate**, **Rename…**, **Duplicate** and **Delete workflow…**.
- **Validate** and **Save** list problems in one line above the canvas: duplicate ids, missing endpoints, cycles, unknown kinds, missing required settings, skills that aren't installed and bad expressions.
- **Optimize…** drafts a faster version from rules (steps side by side, built-in steps for patterns Otto recognises), shows the time it expects to save from this workflow's own runs, what it checked and Before | After. Nothing changes until you choose **Apply as version N…**; the old version stays in Versions.
- On a phone the editor is a read-only outline of the steps; tap one to change its settings in a sheet.

**Running**
- **Run…** opens a form built from the Start step's inputs (JSON is behind **Input as JSON**), shows what the run will post outside Otto and where the result goes. **Run again…** fills it from an earlier run.
- **Runs** lists **Needs you** (approvals, questions, usage limits), **Running** (queued runs by position) and **Recent** with search, a **Failed** filter and a workflow filter.
- The run page: one sentence that says what is happening now, a ring of the steps, the facts (who, where, the PR or ticket, which version), the steps in the order they ran with parallel steps grouped and reviewers as named rows.
- A step opens its read-only **Conversation**, **Output**, **Log** and **Files**; a review step its **Summary** (**Open review**) and **Summarizer**; a reviewer its **Findings**. Step conversations are kept for a day after a run succeeds (a week after it fails), then a summary stays.
- **Retry step** re-runs the failed step and what follows it; **Retry only this step** (⋯) just the step; **Skip this step** on a running step; **Stop run…** asks first.
- At most 2 runs execute at once across the app; extra runs wait as **Queued** and start in order. Runs survive a daemon restart and resume from the step they were on.
- The sidebar badge counts runs that need you, else the running ones.

**Goals**
- **Run… → A goal…** (or ⌘K **New goal…**) starts a goal: what must be true when it's done (each criterion checked by a command, an agent or you), where it works, a budget of iterations and active time, and the models that work and check. It runs as a workflow you can watch on the run page: criteria met, iterations as rows, budget left.
- A goal that runs out of budget or is blocked waits for you: **Resume** or **Extend and resume…**. **Pause** and **Add a note for the next step…** are in a running goal's ⋯.
- Goals are listed under **Goals** in the Workflows tab. Older goal loops stay readable from the link at the foot of Runs.

**Triggers**
- **Schedule:** every N minutes, daily, weekly or 5-field cron, in any IANA timezone, with an optional run prompt.
- **Webhook:** a secret URL; anything that POSTs to it starts a run with the request body as input.
- **Event:** review changed, budget exceeded, product changed, improvement finished, insight ready — with an optional JSON filter.
- **Chat binding:** a Slack or Telegram chat (optionally one thread, optionally @mention only on Slack); matching messages start a run.
- Any trigger can send results to a Slack or Telegram chat/thread and to a result webhook.
- **Preview / validate** checks a trigger without saving and shows the next 5 fire times for a schedule.
- **Trigger from chat:** copy a ready-made `Action: Workflow` message to post in Slack or Telegram. `run <name>: <prompt>` works too. Chat-started runs post live step progress back into the thread; reply `status`, `skip` or `abort` to control them.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `⌘↩` | Generate (in **New workflow**) |
| `⌘S` | Save the workflow in the editor |
| `⌃` + scroll | Zoom the canvas (same as a trackpad pinch) |

## Tips and limits

- Node settings are stored as plain JSON. Don't put secrets in them — use **API Run** with an API-client environment, or an integration whose credentials live in the Keychain.
- A webhook URL is the credential. Anyone who has it can start runs; delete the trigger to revoke it. The daemon is only reachable from outside your Mac if you enable remote access.
- Anyone who can post in a bound Slack or Telegram channel can start that workspace's workflows by name.
- Several nodes need other features set up first: **DB Query** (a saved connection), **Broker Peek** (a Kafka cluster), **Review Run** and **Git PR** (a registered repository), Product nodes (a story, and a Jira/Confluence account to publish).
- A cycle in the graph fails the run before any step starts. A run stops after 10 hours.
- An agent step that shows no real progress for 5 minutes is treated as stuck and retried in a fresh session. The 5-minute limit is a daemon setting and can be raised or turned off.
- DB Query is always read-only; Broker Peek never produces.
- Viewers can see workflows, runs and triggers. Creating, editing, running, approving and changing triggers need the workspace Editor role and the Workflows feature.

## Related

- [Scheduled Tasks](#/walkthroughs/scheduled-tasks)
- [Older goal loops](#/walkthroughs/loops)
- [Git](#/walkthroughs/git)
- [Product](#/walkthroughs/product)
