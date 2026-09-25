# Agent UI control ("the agent drives Otto, visibly")

An agent running in an Otto session (Claude, Codex, …) can **operate Otto's own
modules in front of you**. It can open a query tab in the Database Explorer, run a
query and page through the grid. It can open a Kubernetes workspace and read a
pod's logs, peek a Kafka topic, open a vault note, or start a workflow. Every
action happens in **your** window, usually in the side pane next to the agent's
terminal, so you see what it does as it does it. The agent reads back the same
data the page shows.

It is off until you allow it for a session. Reads and navigation then just
happen. Anything that changes something asks you first, and anything that leaves
your Mac always asks.

> The contract is authoritative. The tools are listed in
> [`docs/contracts/ui-commands.json`](../contracts/ui-commands.json), the one
> catalog that both the daemon and the UI load. The HTTP routes are in
> [`api.md`](../contracts/api.md) and the socket frames in [`ws.md`](../contracts/ws.md).
> This guide describes how it behaves.

---

## 1. Setup

Nothing to install. The `otto.ui_*` tools (stdio name `otto_ui_*`) sit in the
**MCP Control Plane** catalog under the **UI control** category and are
**enabled by default**. They also work when the outward "Otto MCP server" master
switch is off, because they only ever act for an agent running in one of your
own Otto sessions. To take a single tool away from every session, disable it in
**MCP Control Plane → Otto tools**.

What an agent needs:

- to run **inside an Otto session**. The tools bind to the calling session (its
  session token), never to an argument, so an external MCP client can't use
  them;
- your **grant** for that session (§2);
- an **Otto window** open on the device that started the session. Some reads
  work without one (§6).

## 2. Allowing it: the per-session grant

The first time an agent calls a `ui_*` tool in a session, Otto asks you **next to
that session**: *"‹Claude› wants to drive Otto beside this session"* with
**Allow for this session** and **Deny**. The call waits about 15 s for your
answer. If you don't answer in time the agent gets `pending_grant` ("they were
asked in Otto; retry after they allow it"), and it can simply call again once
you've clicked Allow.

- The grant covers **every module**, for **this session only**. It is saved on
  the session (`session.meta.ui_control`), so it survives a reload, and it ends
  with the session.
- You can flip it any time from the session toolbar's **⋯ → Allow UI control**
  (checked while on).
- **Deny** stops the prompt from coming back for a minute. After that, a new
  call asks again.
- Only you (the session's owner) or an admin can grant it, and only from the
  Otto UI. The route refuses session and MCP tokens, so an agent can't grant
  itself.

## 3. What you see while it drives

- **Where it opens.** If no pane shows the module the agent needs, Otto opens it
  in the **side pane** next to the agent's session. It never takes over a main
  pane that shows something else. If a pane already shows that module, the
  command goes there.
- **The driving bar.** Under the page's toolbar: *"‹Claude› · ‹session title› is
  driving ‹Module›"*, with the current step, a **Recent** list (the last 20
  agent actions in this window) and **Stop**. It stays while commands run and
  for a minute after the last one.
- **Highlights.** The element the agent just touched (the table row, the opened
  drawer, the produce form, the note editor, …) is outlined and scrolled into
  view. The outline doesn't animate if you have reduced motion on.
- **Attribution.** Agent-created DB tabs carry an agent chip. Confirms and
  toasts name the agent ("Claude · ‹session›").
- **Stop** turns UI control off for that session and cancels anything that was
  running. The agent's next call asks you again.

## 4. Risk tiers and confirms

Every command has a fixed `risk` in the catalog:

| Risk | Examples | What happens |
|------|----------|--------------|
| `read` | list clusters, table rows, pod logs, topic peek, note text | Runs once granted. |
| `navigate` | open a workspace, select a row, switch a tab | Runs once granted. |
| `local_write` | k8s restart / scale, Kafka produce, save a vault note, pause a scheduled task, run a swarm task, start a goal loop, add a Home box, git commit | An **attributed confirm** every time. It has an **"Allow writes on ‹target› for this session"** checkbox, and once you tick it, later writes to the same target in the same session don't ask again. |
| `outward` | SQS send, run a workflow, API send, git push | **Always** confirms (where it goes, what is sent, who sees it). It is **never remembered**. |

On top of the tiers, some targets always ask regardless of the checkbox:

- **Prod / guarded targets never use the remembered choice.** A prod or guarded
  DB connection needs the typed confirm. A prod Kubernetes cluster always needs
  the resource's name typed. A guarded Kafka cluster (prod or read-only) asks
  again with the page's own danger confirm.
- **Destructive Kubernetes actions** (delete pod, rollout undo, scale to 0, sync
  with prune) always need the resource's name typed, and the daemon also
  refuses them without it.
- **A scheduled task that delivers** to Slack, Telegram, email or a webhook is
  outward when run now, so it always asks.
- **Stopping a goal loop** can't be undone, so it always asks.
- **Vault edits stay drafts.** The new text is put into the note's editor with
  autosave held, and nothing is written until you confirm. If you decline, the
  original text comes back. An agent never overwrites a note while it has your
  unsaved edits.
- **Database queries run read-only.** A write or DDL statement asks you first
  (see [Database Explorer](./database-explorer.md)).

If you decline, the agent gets `cancelled_by_user`. If you close the dialog, or
Stop / the deadline fires while it is open, the dialog is dismissed as a cancel.

## 5. Command reference

The tool is `otto.ui_<name>` (stdio `otto_ui_<name>`). Arguments that name a
cluster, account, workspace object, etc. take its **id or its name** (a
case-insensitive exact match). If two have the same name, the command asks for
the id. Lists come back capped at 200 rows. Every result includes
`ui_visible: true` (or `false` when it ran headless), and the daemon redacts
secrets from results before the agent sees them.

### Shell and navigation

| Command | Risk | What it does |
|---------|------|--------------|
| `state` | read | What each open Otto document shows (main pane / side pane, route, module, focus) plus the current module's view state. Works with no window (headless). |
| `open` | navigate | Open a module / route, in the side pane by default (`placement: side|main`). |
| `focus` | navigate | Bring the window forward and focus a pane. |

### Database Explorer, API client, Git, Browser

These are covered in their own guides' "Agent UI control" notes and in the
catalog. They are `db_*` (connections, tabs, run/page/explain/stop, views,
export: you pick the file), `api_*` (requests, curl import, environments,
send = outward), `git_*` (repos, status, diff, stage/commit/pull = local
write, push = outward) and `browser_*` (tabs, navigate, reader/live, page
text, annotate, summarize).

### Kubernetes: `kubernetes` (route `#/kubernetes`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `k8s_list_clusters` | read | none | The console's cluster list. |
| `k8s_open` | navigate | `cluster`, `kind?`, `namespace?` | The cluster workspace on that kind / namespace. |
| `k8s_list_resources` | read | `cluster?`, `kind?`, `namespace?`, `filter?`, `limit?` | The resource table, with the filter typed into its filter box. Returns rows (status, ready, restarts, age, node, health, kind columns). |
| `k8s_select` | navigate | `cluster?`, `kind`, `namespace?`, `name`, `tab?` | The row selected and its drawer open on `overview`/`manifest`/`describe`/`events`/`logs`/`metrics`/`pods`. |
| `k8s_describe` | read | `cluster?`, `kind`, `namespace?`, `name` | The Describe tab. Returns the describe text and recent events. |
| `k8s_logs` | read | `cluster?`, `namespace`, `pod`, `container?`, `tail_lines?`, `previous?` | The Logs tab. Returns the last lines (≤5000 lines, ≤60 KB). |
| `k8s_action` | local_write | `cluster?`, `kind`, `namespace?`, `name`, `action`, `params?` | The drawer on the target, then the confirm (§4). Actions come from the same per-kind registry as the row menu. |

### AWS: `aws` (route `#/aws`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `aws_list_accounts` | read | none | The accounts page. |
| `aws_open` | navigate | `account`, `service?` | The account on `s3`/`sqs`/`ec2`/`athena`/`eks`/`rds`. |
| `aws_s3_browse` | read | `account`, `bucket?`, `prefix?` | The bucket list, or that bucket/folder (deep-linked). |
| `aws_s3_preview` | read | `account`, `bucket`, `key` | The object's folder. Returns a text preview (binary: content type only). |
| `aws_sqs_list_queues` | read | `account`, `prefix?` | The SQS view. Returns queues with approximate depth. |
| `aws_ec2_list` | read | `account`, `region?`, `state?`, `query?` | The EC2 view. Returns instances. |
| `aws_sqs_send` | **outward** | `account`, `queue`, `body`, `delay_seconds?`, `group_id?`, `dedup_id?` | Where / what / who confirm, then a toast. |

### Message Brokers (Kafka): `connections` (route `#/brokers`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `brokers_list_clusters` | read | none | The Brokers page. |
| `brokers_open` | navigate | `cluster`, `view?` | The cluster tab on `overview`/`topics`/`groups`/`schema`/`replay`/`alerts`. |
| `brokers_list_topics` | read | `cluster`, `query?`, `include_internal?` | The Topics tab. |
| `brokers_list_groups` | read | `cluster` | The Consumer Groups tab. |
| `brokers_open_topic` | navigate | `cluster`, `topic` | The topic's detail. |
| `brokers_peek` | read | `cluster`, `topic`, `start?`, `partition?`, `limit?`, `key_filter?`, `value_filter?` | The agent's options filled into the Messages form, and the peek run in your grid. |
| `brokers_produce` | local_write | `cluster`, `topic`, `value`, `key?`, `partition?`, `headers?` | The Produce form prefilled with the message while you confirm. |

### Vault: `vault` (route `#/vault`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `vault_list` | read | none | The Vault page. Returns vaults, open tabs and the open note. |
| `vault_open_note` | navigate | `vault?`, `path`, `edit?` | The note open (in a tab). Returns its markdown (≤60 KB) and links. |
| `vault_search` | read | `vault?`, `query` | The search panel with the query run (`tag:<name>` works). |
| `vault_write_note` | local_write | `vault?`, `path`, `content` | A new note, or the new text staged in the editor until you confirm (§4). |

### Workflows: `workflows` (route `#/workflows`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `wf_list` | read | none | The workflow list, plus the runs in progress. |
| `wf_open` | navigate | `workflow` | The workflow on the canvas. If the open one has unsaved edits, you're asked whether to discard them. |
| `wf_list_runs` | read | `workflow` | Returns its recent runs. |
| `wf_open_run` | navigate | `workflow`, `run_id` | The run in the inspector. Returns per-step status and last logs (call again to follow). |
| `wf_run` | **outward** | `workflow`, `input?`, `prompt?` | Where / what / who confirm, then the same validate → save → run as the Run button. |
| `wf_cancel_run` | local_write | `run_id` | Confirm, then the run halts after its current step. |

### Scheduled Tasks: `scheduled-tasks` (route `#/scheduled-tasks`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `sched_list` | read | none | The task list. Returns tasks with their schedule and delivery. |
| `sched_show_runs` | read | `task` | That task's Runs expanded. |
| `sched_run_now` | local_write* | `task` | Confirm, then the run starts and its Runs open. *It's an outward confirm when the task delivers somewhere. |
| `sched_set_enabled` | local_write | `task`, `enabled` | Confirm, then the task is paused / resumed. |

### Home: `home` (route `#/home`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `home_state` | read | none | Home. Returns views, boxes, the zoomed box and the kinds you may add. |
| `home_go_to_view` | navigate | `view` (1-based index, id or name) | The view slides in. |
| `home_zoom_box` | navigate | `box_id`, `zoom?` | The box fills the page (or returns). |
| `home_add_box` | local_write | `kind`, `view?`, `config?` | Confirm, then the new box appears (only kinds you may view). |
| `home_remove_box` | local_write | `box_id` | Confirm, then the box is removed. |

### Agent Swarm: `swarm` (route `#/swarm`) · Goal Loops: `loops` (route `#/loops`)

| Command | Risk | Args | What you see |
|---------|------|------|--------------|
| `swarm_list` | read | none | The swarm list. |
| `swarm_open` | navigate | `swarm`, `view?`, `project?` | The swarm on `tree`/`graph`/`kanban`/`runs`/`board`. |
| `swarm_list_tasks` | read | `swarm?`, `project?`, `status?` | The Kanban board. |
| `swarm_list_runs` | read | `swarm?`, `status?` | The Runs view. |
| `swarm_run_task` | local_write | `swarm?`, `task_id` | Confirm, then the task's agent starts. |
| `swarm_stop_run` | local_write | `swarm?`, `run_id` | Confirm, then the run stops. |
| `loops_list` | read | none | The Goal Loops list. |
| `loops_open` | navigate | `loop` | The loop's detail. Returns the definition and the last 10 iterations. |
| `loops_control` | local_write | `loop`, `action` (`start`/`pause`/`resume`/`stop`) | Confirm (Stop always asks), then the loop changes state. |

## 6. No Otto window, other devices

- Commands go only to Otto windows on **the device that started the session**,
  and only to documents signed in as you. An agent can't reach another
  person's window, or yours on another machine.
- With **no window open**, a `read` command that has a daemon-side twin runs
  **headless** and returns `ui_visible:false` with a note. Today those are
  `state`, `db_list_connections` and `db_run_query` (read-only). Every other
  command returns `no_ui_client`, and the agent can ask you to open Otto.
- Commands go only to a window that **implements** them (the window's
  `hello.capabilities`). An older UI build never gets a command it doesn't know.

## 7. API / WS surface (summary)

- **Tools:** one governed tool per catalog entry, `otto.ui_<name>` in the MCP
  Control Plane (category "UI control"). They are audited like every governed
  call (`mcp_call_log`), but they are **not** in the approval (DANGEROUS) set:
  the session grant plus the in-UI confirms take the place of a per-call
  approval.
- **HTTP** ([`api.md`](../contracts/api.md)), all human-credential only:
  - `GET /api/v1/ui/commands/catalog`
  - `POST /api/v1/ui/commands/{id}/result` (header `X-Otto-Ui-Conn`)
  - `POST /api/v1/ui/commands/{id}/progress` (`awaiting_human` extends the
    deadline while you decide)
  - `POST /api/v1/sessions/{id}/ui-control {enabled}`: the only writer of
    `session.meta.ui_control`. `PATCH /sessions/{id}` can't set it.
- **WebSocket** ([`ws.md`](../contracts/ws.md)): each Otto document sends
  `hello` / `presence` on `/ws/events`. The daemon sends `ui_command` /
  `ui_command_cancel` to **that one connection only**, never as a broadcast. The
  owner-scoped `ui_control_requested` event raises the prompt beside the
  session.
- **Adding a command** means adding a catalog entry and a handler in
  `ui/src/lib/uiCommands/<module>.ts`. The unit test
  `ui/unit/uiCommands.test.ts` and the daemon's catalog test keep the two in
  step. Modules whose view state lives in a component (Workflows, the Brokers
  sub-tabs, Scheduled Tasks, Swarm, Goal Loops) expose it through a small
  **page port** (`uiCommands/pagePort.ts`) that the page binds while it's
  mounted.

## 8. Capabilities and limits

- **The agent can never do more than you could.** Each command runs in your
  window with **your** login, so the same RBAC, resource access and endpoint
  checks apply as for your own clicks. The daemon also checks connection access
  before sending a command.
- **One window at a time.** A command drives one document. If you and the agent
  work in the same tab, the last write wins. Agent commands address tabs and
  resources by id, and results echo what is on screen.
- **Not driveable yet:** creating or editing clusters, accounts, Kafka
  clusters, workflows, scheduled tasks, swarms or loops, Athena queries, EC2
  start/stop, SQS purge/redrive, consumer-group offset resets, vault
  rename/delete, and the docs agents. Use the page (or the existing `otto.*` MCP
  tools, where one exists).
- **Brokers is part of the Connections pane.** It shares that pane with the
  Database Explorer, so `ui_state` there describes the Explorer's view.
- **Results are capped**: 200 rows per list, 60 KB of log/describe/note text,
  and the S3 preview reads 256 KB.

## 9. Troubleshooting

| The agent gets… | Meaning / fix |
|-----------------|---------------|
| `pending_grant` | You haven't allowed UI control for this session. Click **Allow for this session** beside it (or **⋯ → Allow UI control**), then let the agent retry. |
| `no_ui_client` | No Otto window is open on the session's device (or none implements that command). Open Otto, or ask for a read that can run headless. |
| `cancelled_by_user` | You declined a confirm, closed it, kept unsaved edits, or pressed **Stop**. |
| `not_found` | The cluster / account / topic / note / task named doesn't exist. The message lists the candidates, so retry with an id. |
| `invalid_args` | A bad kind, action, view or path, or two objects share the name (pass the id). |
| `forbidden` | Your RBAC or the account's IAM doesn't allow it. The agent can't do more than you. |
| `failed` | The page's own call failed (the page shows the same error inline or as a toast), or the view didn't settle in time. |

**The side pane didn't open.** The side-by-side split needs a desktop-width
main window with room for two panes. On a narrow window Otto shows "Not enough
room for two panes": widen the window or collapse the sidebar (⌘1).

**It keeps asking to confirm writes.** Tick **"Allow writes on ‹target› for
this session"** in the confirm. Prod, guarded and outward targets always ask on
purpose.

## Related

- [Agent sessions](./agent-sessions.md), [MCP Control Plane](./mcp-control-plane.md), [Side by side / multi-window](./multi-window.md)
- [Database Explorer](./database-explorer.md), [Kubernetes console](./kubernetes-console.md), [AWS console](./aws-console.md), [Message Brokers](./message-brokers.md), [Vault](./vault.md), [Workflows](./workflows.md), [Scheduled Tasks](./scheduled-tasks.md), [Home](./home.md), [Agent Swarm](./agent-swarm.md), [Goal Loops](./goal-loops.md)
