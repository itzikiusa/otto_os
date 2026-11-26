# Workflows & Automations

> **Maturity: a real orchestrator.** Otto's Workflows feature is a working
> node-graph engine: you build a directed graph of steps, run it (whole graph, or
> "from here" / "only this"), and follow the run on its own page — what it is
> doing in words, what it waits on, and each agent step's **conversation**.
> Agent steps run as **threads on the structured runtime** (the runtime behind
> Threads) by default; the terminal runner stays behind a switch (§3).
> **Independent steps run side by side**, and each keeps the step number the
> one-at-a-time engine gave it. Every node kind executes against a real Otto
> subsystem: agents, the PR-review engine (0–100 score), product stories, git,
> HTTP, databases, brokers and channels, plus four **steps that need no agent**
> (`git_checkout`, `script`, `post_review_comments`, `check`). The engine has
> **branching** (edge conditions + a `condition` node, also on the run's
> repositories), **bounded loops** (`loop`, with an optional active-time
> budget), **retry/backoff**, **typed outputs** (warn-only validation),
> **versioning** (graph snapshot history), a per-run **Proof Pack** link,
> **standing `instructions`** every step follows, and **Goals**: a goal is a
> workflow of one fixed shape — work, check the criteria, repeat within a
> budget — that replaced the separate Goal Loops feature (§6,
> [Goals](./goal-loops.md)). **All trigger kinds fire unattended** — webhook,
> event, schedule (cron + IANA timezone) and a **`chat`-kind trigger** (channel
> bindings, evaluated live by the channels Bridge). A structured
> `Action: Workflow` chat message, the **`run <name>: <prompt>`** command, or a
> channel binding starts a run by name; the run then keeps **one live status
> message** in that thread (edited in place), still posts every step's file,
> and delivers the result there (§5). A run that waits on a person — an
> approval, a question, a check only you can make, a usage limit — is listed
> under **Needs you**; a usage limit with a known reset parks the step until
> the reset and continues by itself. **Optimize** drafts a faster version of a
> stored workflow from rules alone (§6). The separate **API-client
> "Automations"** surface (a multi-step saved-request runner) is also fully
> functional. A couple of honest caveats remain (the game
> `game_engine`/`verifier` are scaffolds) — stated inline; do not assume full
> parity with mature n8n/Zapier engines.

The doc is grounded in the code in `crates/otto-server/src/workflow_engine.rs`
and `workflow_engine/` (`run/schedule.rs` — the scheduler driver,
`step_thread.rs` / `step_pty.rs` — the two step runners, `run/chat.rs`,
`run/hold.rs`, `nodes/`), `crates/otto-agent-protocol/src/step_runner.rs` and
`runtime/{unattended,retention}.rs` (a step on a thread),
`crates/otto-workflows/src/` (`schedule.rs` — the start rule, `context.rs` —
run context files, `chat.rs` / `chat_status.rs` — chat triggers and the status
message, `optimize.rs`, `goal.rs` / `criteria.rs` — goals and the `check`
step, `prepare.rs`, the node catalog, validation, retry policy, loop
checkpoints and event triggers),
`crates/otto-server/src/routes/workflows.rs` (+ `workflows_ext.rs`,
`workflow_progress.rs`, `workflow_run_files.rs`, `goals.rs`),
`crates/otto-server/src/workflow_trigger_scheduler.rs`,
`crates/otto-core/src/expr.rs`, `ui/src/modules/workflows/`,
`ui/src/modules/api/AutomationsView.svelte`, the migrations under
`crates/otto-state/migrations/` (incl. **0089** — versioning + run→proof link,
**0096** — standing `instructions`, **0233–0235** — run overview, holds,
goals, lanes) and the authoritative contracts `docs/contracts/api.md`
(§ "Workflow engine", § "Goals", "Steps side by side and the step runner",
"Engine threads", § "API client") and `docs/contracts/ws.md` (Workflow run
progress).

---

## 1. Two different things called "automations"

Otto has **two unrelated automation surfaces**. Keep them straight:

| Surface | What it is | Where | Model | Backed by |
|---|---|---|---|---|
| **Workflow engine** | A visual node-graph you build and run, with branching, loops, retry, and versioning; nodes call agents, HTTP, DB, brokers, channels, swarm, product, review, git, etc. | `#/workflows` page | `Workflow` / `WorkflowRun` / `WorkflowGraph` / `WorkflowVersion` | `workflows`, `workflow_runs`, `workflow_triggers`, `workflow_versions` tables |
| **API-client "Automations"** | A *collection runner*: an ordered list of saved API-client requests with per-step assertions + variable extraction (a tiny test/regression runner). | API client (`#/api`) → "Automations" view | `ApiAutomation` / `ApiAutomationStep` / `ApiRunResult` | `api_automations` table |

This document covers **both**, but the bulk is the workflow engine. The
API-client automations are a thin, self-contained feature covered in §8.

---

## 2. Overview & where it lives

```
ui/src/modules/workflows/WorkflowsPage.svelte   — the route switch: Runs · Workflows tabs, run page, editor, Optimize; the start form, ⌘K, the agent UI port
ui/src/modules/workflows/runs/                  — the Runs inbox (RunsInbox, RunRow), the run page (RunPage, RunOverview, StateCards, StepList, StepPane, StepConversation, StepFiles, RunOrbit) and the start form (StartRunModal, startForm.ts); words in runWords.ts
ui/src/modules/workflows/list/                  — the Workflows tab (WorkflowList), New workflow (NewWorkflowModal), Duplicate / Rename / Delete (workflowActions.ts)
ui/src/modules/workflows/editor/                — the editor (EditorPage, EditorCanvas — auto-laid-out top to bottom, PhoneOutline)
ui/src/modules/workflows/inspector/             — the step inspector (StepInspector, per-kind forms, StepModel = provider · model · effort)
ui/src/modules/workflows/optimize/              — the Optimize review page (OptimizePage, optimizeView.ts)
ui/src/modules/workflows/runSync.svelte.ts      — RunSync (run state): live run sync (WS fast path + safety poll), approvals, final output
ui/src/modules/workflows/RunFinalOutput.svelte  — the run's final deliverable (one view for mobile and desktop)
ui/src/modules/workflows/TriggersPanel.svelte   — list/add/edit/preview/toggle/delete schedule|webhook|event|chat triggers
ui/src/modules/api/AutomationsView.svelte       — API-client collection runner (the *other* "automations")

crates/otto-server/src/workflow_engine.rs              — the executor: thin run driver, per-node exec, branching/retry, proof
crates/otto-server/src/workflow_engine/run/           — run phases: admission.rs (graph, scope, identity, input, repo isolation, Running CAS), schedule.rs (the scheduler driver: steps side by side, attempts, outcome), board.rs (node states + publish), chat.rs (chat style, needs-you post), hold.rs (paused / blocked / out-of-budget runs), finalize.rs
crates/otto-server/src/workflow_engine/step_thread.rs  — an agent step on a runtime thread: the runner switch, phases, thread record, close + retention
crates/otto-server/src/workflow_engine/step_pty.rs     — the terminal (PTY) step runner, behind the switch
crates/otto-server/src/workflow_engine/nodes/         — one file per large node kind (review_run.rs keeps its pure gate: review_score / review_verdict / aggregate; git_checkout.rs, script.rs, post_review_comments.rs, check.rs are glue over otto-workflows)
crates/otto-agent-protocol/src/step_runner.rs          — ThreadStepRunner: machine place, thread, turn to settle, handoff nudge, usage-limit wait
crates/otto-agent-protocol/src/runtime/                — unattended.rs (a thread has "settled"), retention.rs (closed engine threads are deleted after their date)
crates/otto-workflows/src/                             — server-free pieces: node catalog, validation, retry, checkpoints, run context files, schedule.rs (which step starts, with which number), overview.rs (the run's state and headline), chat_status.rs (the live status message), optimize.rs, goal.rs + criteria.rs + check_run.rs (goals and the check step), checkout*.rs / script_run.rs / post_comments.rs (the steps that need no agent), attachments.rs, steering.rs, event-trigger listener (`WorkflowCtx`)
crates/otto-server/src/routes/workflows.rs             — HTTP handlers: CRUD, generate, run, versions, triggers, webhook, approve, templates
crates/otto-server/src/routes/workflows_ext.rs         — skip a step, Optimize (draft + apply)
crates/otto-server/src/routes/goals.rs                 — goals, pause / resume / steer / verify, the older goal loops' history
crates/otto-server/src/workflow_trigger_scheduler.rs   — schedule scheduler + boot glue for the event-bus listener (both spawned at boot)
crates/otto-workflows/src/chat.rs                      — `Action: Workflow` chat-message parser + WorkflowChatTrigger impl
crates/otto-core/src/expr.rs                           — the safe expression language (edge conditions, condition/loop, {{ }} templating)
crates/otto-core (otto_core::workflows)                — the Workflow/WorkflowRun/Node/Edge/Version domain types
crates/otto-state/migrations/0020_workflows.sql        — workflows + workflow_runs
crates/otto-state/migrations/0051_workflow_node_cache.sql — historical (the per-node cache was removed; the table is dormant)
crates/otto-state/migrations/0058_workflow_triggers.sql   — workflow_triggers + run approval columns
crates/otto-state/migrations/0089_workflow_orchestrator.sql — workflow versioning + run→proof-pack link
crates/otto-state/migrations/0014_api_client.sql          — API client base
crates/otto-state/migrations/0015_api_automations.sql     — API-client automations
```

The Svelte UI talks only HTTP+WS to `ottod` on `127.0.0.1:7700`. The TypeScript
types in `ui/src/lib/api/types.ts` mirror `otto_core::workflows`.

---

## 3. The workflow model: nodes, edges, triggers

A **`Workflow`** is a named, workspace-scoped, directed graph plus run history.

```ts
interface Workflow {
  id; workspace_id; name; description;
  instructions: string;       // standing free-text guidance every run/step follows (§ Standing instructions)
  graph: WorkflowGraph;       // { nodes: WorkflowNode[]; edges: WorkflowEdge[] }
  created_by; created_at; updated_at;
  version: number;            // monotonic; bumped + snapshotted on every graph- OR instructions-changing edit
}
interface WorkflowNode {
  id; kind; name; x; y; params: unknown;          // x/y are canvas layout
  retry?: { max_attempts; backoff_ms; factor };   // optional per-node retry policy
}
interface WorkflowEdge {
  id; source; target;          // source/target are node ids
  condition?: string;          // optional expr; the edge is active only when truthy
}
```

- **Nodes** carry a `kind` (the node type, e.g. `agent_prompt`), a free-form
  `params` object (kind-specific config), `x`/`y` canvas coordinates, and an
  optional `retry` policy (see *Retry/backoff* below).
- **Edges** are directed `source → target` connections between node ids, with an
  optional `condition` (an expression — see below); an edge with no condition is
  always active (the legacy behavior).
- The graph is executed in **run order** (`topo_order`), and steps whose
  inputs have settled run **side by side** (see *Scheduling* below). A cycle
  makes the whole run fail immediately with an error (no nodes execute).

### Standing instructions (`workflow.instructions`)
A `Workflow` carries a second free-text field alongside `description`:
`instructions` — standing guidance every run/step should follow "by the letter"
(e.g. spec-framework conventions, house style, things to never do), distinct
from `description` (a one-line summary shown in lists). It is:
- **Edited in the UI** via the editor's ⋯ → **Instructions** (it opens in the
  inspector column, with its own **Save**).
- **Versioned like the graph.** An instructions-only `PATCH /workflows/{id}`
  bumps `version` and snapshots exactly like a graph-changing edit (note
  `"edited"`); restoring an older `WorkflowVersion` restores its `instructions`
  alongside its `graph` (name/description are not restored — they only live in
  the snapshot).
- **Inherited by templates.** `POST .../workflows/from-template` copies the
  template's `instructions` verbatim (the `ui-test-authoring` and
  `api-acceptance-test-authoring` templates ship real standing instructions —
  see §6). An AI-generated workflow (`POST .../generate`) always gets `""` —
  the generation prompt IS `description`.
- **Written to the run's context dir** as `instructions.md`, verbatim, when
  non-empty — the first file every agent-backed step is told to read (see
  *Run context files* in §7).

### The expression language (`otto_core::expr`)
A tiny, **safe** (pure; no I/O, no `eval`) expression language evaluates against a
JSON context. It powers **edge conditions**, the `condition` / `loop` nodes, and
`{{ … }}` templating. Grammar supports `|| && == != < <= > >=`, `+ - * / %`, the
infix `contains` / `in`, unary `! -`, parentheses, dotted/indexed paths
(`output.result`, `input.rows[0]`), literals, and functions `len, lower, upper,
default, has, int, float, str, bool, not`. Missing path segments resolve to `null`
(never an error). For edge conditions the context is
`{ output, input, node:{id,kind,name}, run:{input, repos, prs} }` (`run.repos`
/ `run.prs` are the run's `repos.json` and `prs.json` entries — §7); a
condition that fails to parse or evaluate is treated as **not taken** (and
logged), never a crash.
Nesting is bounded: more than 64 levels of `!`/`-`, parentheses, call arguments or
`[ ]` indexes is a parse error (*"expression nests too deeply"*), and so is a chain
of more than 1,024 binary operators (*"expression is too long"*) — the parser and
evaluator recurse per level, so an unbounded `!!!…` condition used to overflow the
stack and abort the daemon on save.

### Retry/backoff (`WorkflowNode.retry`)
`{ max_attempts (extra attempts after the first, clamped ≤5), backoff_ms (initial
sleep, clamped ≤60000), factor (multiplier, default 2.0) }`. An absent policy gives
`agent_prompt` and `prepare_context` with a nonempty prompt two retries; other kinds default to zero.
An explicit zero always means one attempt, even for transient failures. The UI
keeps zero distinct from **Use default**. The policy can also be
supplied as a `params.retry` object. `human_approval` and `manual_trigger` are
never retried, and a `loop` retries its inner steps through its own checkpoints
(§7), not as a whole. `NodeRunState.attempts` records how many attempts ran; a
retry shows on the step as a `queued` phase noted "attempt 2 of 3 — <reason>".
A step stopped by a **plan usage limit** is never retried by backoff (below).

### How an agent step runs
An agent-backed step (`agent_prompt`, `prepare_context`'s analysis phase, the
`product_*` / `canvas` / `git_pr` turns, a `check`'s checker, a goal's Work
step) runs as **a thread on the structured runtime** — the runtime Threads
uses — unless the runner switch says otherwise. The engine opens a thread
titled `WF <workflow> · <step> · run <short id>`, sends the step's prompt
(preamble + prompt + the completion protocol) as one turn and waits until the
thread has **settled** (`ThreadStepRunner` in
`crates/otto-agent-protocol/src/step_runner.rs`):

1. **A valid pick.** A model or effort the provider no longer offers fails the
   step before anything starts, and the error says which.
2. **A machine place.** The step waits its turn at the machine's agent
   admission gate; while it waits its phase is `waiting` and the step says why
   (*Scheduling* below).
3. **The turn, to settle.** Settled means: no turn running, no background
   work, nothing queued, no request waiting on a person, and no busy
   sub-thread the agent opened, at any depth (`runtime/unattended.rs`).
   After background work ends the agent gets 10 s to start its own follow-up
   turn. Background work may keep a finished turn open for at most 45 minutes;
   past that the step fails ("background work was still running at the
   limit").
4. **The handoff file.** When the thread settles and the step's `.md` (§7) is
   missing, the engine sends one more turn — "You did not write <path>. Write
   it now; nothing else." — and then takes what is there: the file, else the
   agent's final answer (the engine writes it to the step file).

There is no idle confirm and no transcript polling on this runner: the
runtime reports the turn's end and what is still pending. The step logs
`⏳ starting <provider> thread` and `✓ step complete`; where its time went is
kept as **phases** on the step (`queued` · `waiting` · `starting` · `working`
· `needs_you` · `finishing`), which the run page draws as a bar. The step's
output is `{ reply, thread_id }` (`session_id` on the terminal runner), and
the run keeps the thread's record on the step (`NodeRunState.thread`:
provider, the model and effort that ran, turns, token usage, a digest).

**Model and effort.** Each agent step picks **provider · model · effort**
with the same picker as Threads (the inspector's **Agent** chips). Empty means
**Default**: the provider's own setting, as in Threads. The model is set when
the thread is created and the effort rides the first turn; a step that
continues an earlier thread keeps that thread's model unless it names its own.
Reviewers and the summarizer of a review step, and a goal's Work and Check,
are picked the same way.

**Time limit.** `timeout_s` (the inspector's "Fail the step if it hasn't
finished after (minutes)", 1–600 min; none by default) is the step's hard
budget, time waiting on a person included. A step past it fails with class
`timeout` (the run page says **Timed out**); the step's retry policy applies
to it like any other failure, each attempt with the full limit again. Without
a limit a thread step is bounded by the run's 10-hour active-time budget and
the 45-minute background limit. The no-progress trip (`wf_step_stall_secs`)
belongs to the terminal runner only.

**Waiting on a person.** A provider approval, a governed Otto MCP call that
needs approval, or a question the agent asks puts the step in `needs_you`
with what it asks (`attention`); the run moves to **Needs you** (§7). The
answer is given on the run page and the same turn goes on. An engine thread
cannot ask for a secret: `otto.request_secret` is refused at once.

**Usage limits.** When the provider's plan limit stops the turn, the runner
reads the reset time from the CLI's own message, else from Otto's usage meter
for the step's account. **Known reset:** the step parks in `needs_you`
(`attention.kind: usage_limit`, `resets_at`; the run page says **Limited**
and "Otto continues it at 14:02, when the limit resets."), its agent process
is released, and at the reset the runner sends "Continue the step where you
stopped." on the same thread. **No known reset**, or a limit that comes back
right after its reset: the step fails with class `usage_limit` and **Retry
step** is the way on. A wait that would outlast the step's time limit fails
it as `timeout` instead. A plan limit is never retried by backoff. A per-minute rate limit (`rate_limit`) is
an ordinary retry class.

**Error classes.** A failed step carries `error_class`: from the runner
`usage_limit` · `rate_limit` · `auth` (**Signed out**) · `provider` ·
`stalled` · `timeout` (**Timed out**) · `spawn` · `server_restart` ·
`canceled` (**Stopped**); from the engine `script` and `outcome_unknown`.

**Sub-agents are allowed.** The step preamble (`WF_STEP_RULES`) is a
*completion protocol*, not a ban: delegate to sub-agents / background tasks
freely, but fold every result back in, write the handoff **last** (never while
something you launched is still running), never end with "waiting for …", and
stop every background process you started. On a thread, a provider's own
sub-agents and the threads the agent opens with `otto.thread_open` both count:
the step is not settled while one of them works. At most **4** sub-threads of
one step are open at once (the fifth open answers 409 "a workflow step may
have 4 open sub-threads at once …"), and none opens while the machine is
critically short of memory; the agent closes one with
`otto.thread_close`, and `remove: true` also deletes a sub-thread that agent
opened once it has answered (refused while it works or has open sub-threads
of its own). Sub-threads show as rows under the step, their usage is added to
the step's, and they close with it.

#### The runner switch (`wf_step_runner`)
| Where | Values | Notes |
|---|---|---|
| Daemon setting `wf_step_runner` | `threads` (default) · `pty` | Set with `PUT /api/v1/settings` `{"wf_step_runner": "pty"}` — the root user, on a person's credential. Any other value is refused. There is no control for it in Settings. |
| Environment `OTTO_WF_STEP_RUNNER` | `pty` (anything else = threads) | Read by the daemon; wins over the setting. |

The switch is read **once per run execution**: every agent step of that
execution — and the reviewers and summarizer of its review steps — uses the
runner the run started with, so flipping the switch never changes a run in
flight. A retry, a Resume or a restart resume is a new execution and reads it
again; a step that was already on a thread when the daemon restarted goes on
with that thread. A provider the structured runtime has no driver for always
runs on the terminal runner. Reviews started from the Git page read the same
switch. The terminal runner is planned to be removed one release after the
thread runner has run without it.

#### The terminal runner (behind the switch)
With `pty`, a step is a real terminal session (openable in Agents) and is
**not** done when the CLI ends a turn. A claude parent ends a turn every time
a sub-agent reports back, and it ends one the instant it has *launched* them —
so the engine runs a **turn oracle**
(`crates/otto-agent-run/src/turn_oracle.rs`) over the provider's own
transcript. A step completes only when **all three** hold:

1. **The turn ended natively.** *claude:* the last message-bearing line is an
   assistant `end_turn`, **nothing it launched is still pending** (async `Agent`
   launches, `run_in_background` bashes and `SendMessage` resumes are tracked by
   id and cleared only by the harness's
   `<task-notification>…<task-id>…</task-id>` line, whatever the `<status>`),
   and no notification is enqueued-but-undelivered. *codex:* a `task_complete`
   carrying the **latest** `task_started` of this turn.
2. **The handoff file exists** and is non-empty — the step's `.md` (§7) is the
   agent's explicit done signal.
3. **A 20 s idle confirm** passes: no new *message* line in the transcript (the
   file's mtime alone does not count — claude appends attachment/queue
   bookkeeping after an end_turn) and no `subagents/*.jsonl` mtime moves.

Only then does the step log `✓ step complete (handoff + idle turn)`
(`✓ step complete (codex task_complete + handoff)` for codex) and finish. Every
fallback is **bounded**, so a missing signal can never hang a run:

| Situation | Bound | Outcome |
|---|---|---|
| Turn ended, **no** handoff file | 90 s grace, with one "write your handoff file now" nudge at +10 s | accepts the agent's final reply — `⚠ handoff file missing — accepted the agent's final reply after 90s idle` |
| Handoff written while tasks are still pending | 15 min from the handoff's mtime (`⚠ handoff written but 3 tasks still pending — waiting (up to 15m)` on entry) | completes anyway — `⚠ handoff written; 1 sub-agent never reported after 15m — moving on` |
| Parent idle with only background (`Bash`) tasks left | 15 min from the moment the parent went idle — **not** from the task's launch, so a long test run the agent is still working alongside is never cut short. The clock is keyed to the pending ids, not to the tail: a user ping (or a peer's notification) that wakes the parent for one more turn does **not** restart it when it goes idle again with the same task pending — only a change in what is pending does. When a handoff also exists, its own 15 min (from the file's mtime) applies here too, so ending the turn never buys the straggler a second cap | completes and stops the session (which hangs up the task) — `⚠ background task b5gvqf675 never finished after 15m — moving on` |
| A background task the agent **stopped** (`TaskStop`) | none — the harness never sends a `<task-notification>` for a stopped task, so the stop result itself clears the id (it shows as `stopped` in the sub-agent rows) | nothing to wait for; the turn completes on the normal path |
| A notification enqueued but never delivered (`remove`d) | 60 s with no new message line | treated as delivered; the tail counts as `end_turn` again |
| Provider with no transcript artifact (agy / custom) | explicit final handoff, subject to the stall and absolute deadlines | completes only after the nonempty final handoff is written; silence never counts as success |
| Anything | `TURN_TIMEOUT` (10 h) | the turn is abandoned |

**Evaluation order vs the stall trip.** Each 1 s tick the oracle's verdict is
computed **first**. The **no-real-progress trip** (`wf_step_stall_secs` setting;
default **300 s**, `0` disables) is consulted **only** while the step is in the
`working` or `sub-agents` phase — never during the idle confirm, the
missing-handoff grace, the handoff linger or the background-task linger, which
carry their own bounds above. In those two phases "progress" is measured on
signals a stuck agent can't fake: the newest mtime across the main transcript,
`subagents/*.jsonl` **and** the harness's `tasks/*.output` files, or the
session's child processes burning CPU (a long quiet build/test). PTY output
deliberately does **not** count — a hung TUI keeps repainting its spinner
forever. When the trip does fire the step fails with a retryable "stuck" error
(`✗ step made no progress for 5m (agent looks stuck; 0 tasks pending)`) and the
per-node retry policy re-runs it in a fresh session.

**Sessions are stopped when the step succeeds.** The engine **suspends** the
step's session (kills the PTY, leaves the row `Reconnectable` so **Open in
Agents** on a finished step still resumes the transcript), or kills it
outright for providers that can't resume. Only sessions belonging to *this*
run's step are touched. `keep_session: true` on the node's params leaves it
running — on this runner only; `review_run` is exempt (its reviewer sessions
belong to the review, which tears them down itself when it reaches
`done`/error).

### A step's thread: closed, kept, removed
- **Not in your Threads lists.** A step's thread (and a reviewer's, the
  summarizer's, and every sub-thread they open) is an *engine thread*: it is
  left out of every Threads list and is read from the run page, read-only.
- **Closed when the step settles** — its agent process stops — unless a later
  step that has not settled yet continues it (below); then the last continuer
  closes it. A step skipped while it runs closes its thread at once.
- **Closed and dated when the run ends.** At the run's end (also when a run
  is paused or held) every thread of the run is closed, and all of them —
  sub-threads and reviewers' threads included — are kept for **24 hours**
  after a successful run and **7 days** after a failed, canceled or held one
  (settings `workflow_thread_retention_success_hours`, default 24, and
  `workflow_thread_retention_failed_hours`, default 168; `0` success hours =
  a step's thread goes as soon as the step succeeds). Every 10 minutes the
  daemon deletes closed engine threads past their date (at most 200 a pass,
  skipped while the machine is busy). A person's thread is never touched.
- **What stays after removal.** The step keeps its answer, its step file, its
  usage and a digest (turns, tool calls, files changed, the last answer's
  excerpt, errors), and the newest 50 timeline items as
  `<context>/.threads/<node id>-<attempt>.json`. The run page then shows "The
  conversation was removed after the run ended. The answer, files and usage
  are kept — summary below."
- **`continue_from`** (the inspector's **Conversation → Continue the thread of
  an earlier step**) makes an agent step send its turn into an earlier agent
  step's thread instead of a new one, so the agent keeps what it read. The
  editor offers earlier agent steps on the same provider. Between the two
  steps the thread holds no process unless the continuer is the very next
  step. A source thread that retention already removed starts a fresh one
  (log: `thread of <step> was removed — started a fresh one`); a provider that
  cannot resume the conversation fails the step ("could not resume the
  conversation of the earlier step") rather than going on with an agent that
  remembers nothing. Inside a loop, `steps[].same_thread` continues the
  step's own thread from the previous iteration.

**Retry classes.** On top of the node's own `retry` policy the engine classifies
the error string: `529`, "overloaded", rate limits, file-descriptor exhaustion
("Too many open files" / "dup of fd") and spawn failures get a **20 s floor plus
0–5 s jitter** and up to **5 attempts** (the ≤5 clamp is unchanged), logged as
`↻ retry 2/5 in 23s (provider overloaded: 529)`. Every other error keeps the
node's policy verbatim. A retry opens a fresh thread (the failed attempt's is
closed and kept for the failed period).

### Retrying steps of a finished run (`POST /workflow-runs/{id}/retry-node`)
A **finished** run can be re-entered in place without re-running the earlier
(possibly hours-long) steps: the run reopens, out-of-scope nodes keep their
prior state/output/thread record, in-scope nodes re-execute — against the same
run context dir and the same provisioned `otto-wf/<run_id>` worktree/branch —
and the run's final status is recomputed. On the run page:
- **Retry step** (a failed run's primary button) — re-runs the failed step and
  everything after it.
- **Retry only this step** (the run's ⋯ and a failed step's ⋯) — re-runs just
  that step. If steps after it never ran because it failed, they run too;
  otherwise the run would end `success` with its later steps never run.
- **Retry from here** (a settled step's ⋯) — re-runs that step and everything
  downstream of it.

⚠ This is different from a fresh **Run…** with `start_node`
(`RunWorkflowReq.start_node`): that mints a **fresh run** — new context dir,
new worktree cut from base — so files produced by the original run's steps are
NOT there. To redo part of a run that produced files (implementations, test
changes), use the run page's retry actions, which keep the original worktree.

A retry is refused (409, *"still stopping"*) while a just-canceled run's engine
driver is still winding down — it would otherwise run a second driver in the same
worktree. A cancel also wakes a step sleeping in its retry backoff at once, and
the stale driver's cancel finalize never overwrites a retry that reopened the
run. A retry starts a fresh restart-resume budget and a fresh thread for each
agent step it re-runs.

### How node inputs flow (`assemble_input`)
Each node's input is assembled from its **predecessors' outputs**:
- **0 predecessors that produced output** → the node receives the **run input**
  (the JSON body you pass to `/run`, or `{"trigger": "..."}` for trigger-started
  runs). This is what makes a `manual_trigger` (or a "run from here" entry node)
  get the run input.
- **1 predecessor** → it receives that predecessor's output verbatim.
- **N predecessors** → it receives an **object keyed by source node id**, e.g.
  `{ "nodeA": <outA>, "nodeB": <outB> }`.

Beyond input assembly, the [expression language](#the-expression-language-otto_coreexpr)
handles conditions and `{{ }}` templating; `channel_notify` and `swarm_task` also
do simple `{key}` substitution from the incoming object (see §4).

### Scheduling: steps side by side
- **The start rule** (`otto_workflows::schedule`). The engine walks the steps
  in run order. A step starts as soon as every step it depends on has settled
  **and** every step before it in run order has started or been skipped. So
  two branches with no path between them run at the same time, while each
  step still gets the **step number** — and the `step<N>-<name>.md` file
  name — it had when steps ran one at a time, whatever order they finish in.
  The number is stored on the step when it is given (`step_no`, `step_file`)
  and shown as "Step 4 of 9".
- **Step slots.** At most `wf_max_parallel_steps` steps of one run hold an
  agent or a process at once (daemon setting, default **4**, 1–8; `1` = one at
  a time). The kinds that take a slot: `agent_prompt`, `review_run`,
  `script`, `git_checkout`, `post_review_comments`, `loop`, `check`,
  `canvas`, `git_pr`, the `product_*` kinds and a `prepare_context` with a
  prompt. Everything else (conditions, transforms, HTTP, DB…) runs without
  one. A step over the cap is **Queued** (phase `queued`, log `waiting for a
  step slot (4 of 4 in use)`).
- **Run order.** A ready step held back only because an earlier one has not
  started yet logs `waiting for <step> to start (run order)`.
- **The machine.** An agent step then waits for a place at the machine's
  admission gate, shared with every background agent Otto runs. While it
  waits it is **Waiting 2m** (phase `waiting`) and its second line is the
  reason: "Waiting for a free agent slot — 3 ahead in line", "Waiting for a
  free agent slot — 5 of 5 running", "Waiting for the machine — load 9.2
  (limit 8.0), 3 Otto agents are running" or "Waiting for memory — 3 Otto
  agents are running". The step's phase bar keeps how long it waited, and a
  finished run whose steps waited a quarter of its time or more says so ("4m
  of it waiting for the machine").
- **What a step is told to read.** A step's `[workflow context]` preamble
  lists the step files of the steps it descends from (and its own earlier
  loop iterations) — never a sibling's that happens to have finished.
- **One fails, one is stopped.** When a step fails, the steps running beside
  it finish; only what depends on the failed step is skipped. **Stop run…**
  stops every running step at once; **Skip this step** (a running step's ⋯,
  or `skip <step name>` in the chat thread) stops just that one and the run
  goes on.
- **Order of outward steps.** Two steps with no path between them may run at
  the same time. Connect steps that post, publish or open a PR one after
  another when their order matters (the bundled templates do; a test guards
  them).
- **Waiting on a person costs no slot.** While every running step of a run
  waits on a person, the run gives its run-queue slot back and its active-time
  budget stops; the first step to work again takes both back first.

### Run-time graph behavior
- **Two kinds of skip (`decide_node`, pure + unit-tested):** the engine now
  distinguishes:
  - **error-skip (poison):** a predecessor on an active path **errored** → the node
    is `skipped` ("skipped (upstream did not succeed)") **and propagates failure**
    (the run ends `error`). This is the legacy failure propagation.
  - **branch-skip (not taken):** the node has in-scope predecessors but **no
    satisfied active edge** — every incoming edge's `condition` was false, or the
    upstream was itself branch-skipped → the node is `skipped` ("skipped (branch not
    taken)") and **does NOT fail the run**. This is what makes if/else branches and
    pruned paths terminate cleanly. A join runs from whichever side stayed active.
- **Edge conditions** are evaluated on a node's outgoing edges **against its
  output** (`eval_outgoing`); a false edge is marked inactive so its target sees no
  satisfied input.
- **Partial runs:** `/run` accepts `start_node` and `only_node`:
  - `only_node: true` → run **only** that one node (everything else `skipped`).
  - `start_node` without `only_node` → run that node **and all descendants**
    reachable via edges; ancestors are `skipped` and contribute no output, so the
    entry node receives the run input.
- **No cross-run node-result cache:** every node executes in its own run. (An
  earlier per-node output cache keyed on `(node, params, upstream JSON)` was
  removed — an agent node's real input includes the run's repos/worktrees and
  external state, so a multi-repo run could replay a prior single-repo run's
  checkout verbatim and review the wrong scope. `workflow_node_cache` survives
  only as a dormant table: migrations are append-only.)
- **Global wall clock:** a run cannot execute forever. The engine holds an
  overall `RUN_WALL_CLOCK_TIMEOUT` (10 hours of **active** time — time while
  every running step waits on a person is not counted); exceeding it marks the
  RUNNING steps `error` ("workflow active time budget exhausted"), the un-run
  steps `skipped`, and fails the run with "run exceeded the 10-hour time
  limit". An agent step can carry its own `timeout_s` (above); a
  `human_approval` waits 24 hours by default (`params.timeout_s`, minimum 60).
- **Typed-output validation (warn-only):** after a node succeeds, its output is
  checked against the kind's declared `output_schema` (`validate_node_output`);
  mismatches are appended to the node log as `⚠ …` lines and **never fail the run**.
- **A conversation per step:** agent / product / canvas / loop-inner steps
  each run on their own thread (or terminal session on the terminal runner).
  The thread id is recorded on `NodeRunState.thread` the moment it exists, so
  the run page shows the step's **Conversation** **while the step is still
  running**; terminal session ids go to `NodeRunState.sessions` (`review_run`
  also surfaces a `review_id` in its output).
- **Run → Proof Pack:** on completion the engine assembles a Proof Pack from the
  run (each node output → a `log` artifact with the node's pass/fail status; each
  `human_approval` → an `approval` artifact) and links it on the run
  (`WorkflowRun.proof_pack_id`). Best-effort.
- **Versioning:** the run records the workflow `version` it executed
  (`WorkflowRun.workflow_version`); see §9.

---

## 4. Node types

The catalog is returned by `GET /workflows/node-types` (`node_catalog()` in
`crates/otto-workflows/src/catalog.rs`). Each descriptor (`NodeTypeSpec`) carries
`kind, label, category, description, inputs, outputs, color, icon` plus a
declared `output_schema` and, for some kinds, a `params_schema`. The editor's
**+ Step** menu is built from this list, and per-kind parameter forms live in
`ui/src/modules/workflows/inspector/`.

**Status legend:** **Real** = executes against a live subsystem · **Real
(scaffold)** = runs and produces real structured output but emits a canned spec
pending an external engine (only `game_engine`/`verifier`). There are **no longer
any "not wired" stub kinds** — the four former product/review stubs are now wired.

| Kind | Label / category | Purpose | Params (UI form) | Status |
|---|---|---|---|---|
| `manual_trigger` | Manual Trigger / Triggers | Entry node ("Start"); emits the run input. `inputs:0`. | The run input's defaults (`msg`, `working_directory`, `repo_id`, `base`, `story_id`, `jira_ticket`, `goals`); `chat_updates` (`per_step` = a message and a file per step in the chat thread; absent = one live message — §5); `inputs[]` (the start form's declared fields, read by the UI only); `lane` (`long` = the long-run queue, §7) | **Real** |
| `agent_prompt` | Agent / AI | Runs an agent turn on **its own thread** (§3); output `{ "reply", "thread_id" }` (`session_id` on the terminal runner). `skill`/`skills` inline a skill body ahead of the prompt (see *Per-step skills*). | `prompt`, `provider?` (empty ⇒ the run's resolved default agent), `model?`, `effort?` (empty ⇒ the provider's default), `skill?`, `skills?`, `timeout_s?` (60–36,000), `continue_from?` (an earlier agent step's id: continue its thread), `keep_session?` (terminal runner only — leave the session running after the step succeeds, §3) | **Real** |
| `prepare_context` | Prepare relevant data / AI | App-side context gathering: resolves + fetches a referenced Jira ticket into `jira-<KEY>.md`, then optionally runs an analysis agent turn over it (see *Prepare relevant data*). | `key?`, `require?`, `account_id?`, `prompt?`, `provider?`, `model?` | **Real** |
| `http_request` | HTTP Request / Network | Calls an HTTP endpoint, captures response `{ status, body }`. | `method`, `url`, `body` (JSON) | **Real** |
| `transform` | Set / Transform / Data | Merges a static JSON object into the data flowing through. | `json` (object) | **Real** |
| `delay` | Delay / Flow | Sleeps `ms` milliseconds, then passes input through. | `ms` (0–10000) | **Real** |
| `log` | Log / Flow | Records the incoming data in the run log; passes it through. | — | **Real** |
| `game_engine` | Game Engine / Game | Assembles a slot/crash/scratch game *spec* (RNG/paytable/reels) from inputs. | `game` (`slots`/`crash`/`scratch`) | **Real (scaffold)** ¹ |
| `verifier` | Verifier / Game | Verifies a built game. In the **game path** (`play_url` present) it does real file-existence/size checks and **errors** if they fail; otherwise it emits a scaffold "passed" report. | — | **Real (scaffold)** ¹ |
| `db_query` | DB Query / Data | Runs a **read-only** SQL/CH query against a saved Database-Explorer connection. `confirm_write` is forced `false`. | `connection_id`, `statement`, `max_rows` (default 100, cap 1000) | **Real** |
| `broker_peek` | Broker Peek / Data | Consumes up to N recent messages from a Kafka topic on a saved broker cluster (read-only). | `cluster_id`, `topic`, `limit` (default 20, cap 50) | **Real** |
| `channel_notify` | Channel Notify / Integrations | Sends a message to a configured Slack/Telegram integration. Supports `{key}` substitution from the incoming object. | `message`, `channel` (`slack`/`telegram`/any) | **Real** |
| `budget_gate` | Budget Gate / Flow | Checks the provider spend cap: **errors the run if blocked**; otherwise passes (`exceeded` is warn-only). | `provider` (whose usage budget; empty ⇒ run's resolved default) | **Real** |
| `human_approval` | Human Approval / Flow | **Pauses the run**; sets `waiting_approval=1` and waits until an operator approves/rejects on the run page or via the approve endpoint (or times out after `timeout_s`, default 24 h). The run is listed under **Needs you** meanwhile. | `prompt`, `timeout_s?` (minimum 60) | **Real** |
| `condition` | Condition / Flow | Evaluates an `expr` on its input; outputs `{ result, value }` merged onto the input. Pair with **edge conditions** to branch. | `expr` (default `true`) | **Real** |
| `loop` | Repeat (Until) / Flow | Bounded **iterate-until**: re-runs inner `steps[]` until `until` holds, an inner `check` passes, or the budget ends. Reuses inner-node execution; threads run-level keys + prev-step output to each step. Output `{ iterations, satisfied, last, history }`. No nested loops. | `max_iterations` (1–10, default 3; up to 50 with `max_active_s`), `max_active_s?` (300–28,800 s of active time), `until` (expr), `steps[]` ({kind,name,params,retry, `same_thread?`}), `continue_on_error`, `stop_on_repeat_failure?`, `hold_when_unmet?` (*Goals* in §6) | **Real** |
| `swarm_task` | Swarm Task / AI | Enqueues a task in a running Agent-Swarm project (`todo` status; coordinator picks it up). | `swarm_id`, `project_id`, `title`, `description` | **Real** |
| `api_run` | API Run / Network | Executes an HTTP request **through the API-client engine** so env-var substitution + auth apply. | `method`, `url`, `headers`, `body` | **Real** |
| `product_analyze` | Product Analyze / Product | Runs a real single-agent turn (the **`grill`** lens) over the story's built context; outputs `{ story_id, analysis, thread_id }` (`session_id` on the terminal runner). | `story_id`, `instruction?` | **Real** ² |
| `product_rewrite` | Product Rewrite / Product | Rewrites the story (**`jira-story-writer`**); outputs `{ story_id, body_md, thread_id }`; `persist:true` saves a `suggested` product version. | `story_id`, `persist?`, `instruction?` | **Real** ² |
| `product_plan` | Product Plan / Product | Breaks the story into a plan (**`story-task-breakdown`**); outputs `{ story_id, plan_md, thread_id }`; `persist:true` saves a `plan` version. | `story_id`, `persist?`, `instruction?` | **Real** ² |
| `product_publish` | Product Publish / Product | Publishes a story as a Confluence **RFC** or a **Jira** issue. **`dry_run` defaults true** and captures a real content/destination preview; configure `story_id`, `account_id` and `project_key`/`space_key` before preview. Connect Preview → Human Approval → live Product Publish; live publication consumes that exact approved snapshot. | `kind` (`rfc`/`jira`), `dry_run`, `account_id`, … | **Real** |
| `review_run` | Review Run / AI | Runs the **PR-review engine** (scope-selected reviewer lenses + an evidence-checking summarizer), waits for it, emits a **0–100 `score`** (`100−20×must-fix−5×should-fix; optional findings deduct 0 by default`), optional `goals` assessment blended in, `passed` requires a complete review, nonempty diff, no open must-fix findings and `score≥threshold`. `require_pass:true` **errors** the step when the review cannot pass. Output also carries `blocking`/`advisory`/`findings`/`providers`/`lenses`. Reviewers and the summarizer run on threads like agent steps and show as named rows under the step. See *`review_run`, gating & auto-PR* below. | `repo_id`, `base` (default `main`), `providers[]`, `lenses[]` (alias `skills[]`), `reviewers[]` (per-lens provider, `model`, `effort`, `picks`), `picks` (a model and effort per provider for the whole review), `summarizer.{provider, model, effort}`, `threshold` (default 80), `require_pass`, `await`, `timeout_s`, `goals[]`, `scope_assessment?` (`configured` = the scope assessment may drop declared lenses, never add one), `chain_iterations?` (in a loop: each review checks the previous iteration's findings), `mode?` (`fan_out`/`orchestrator` — *Execution compatibility* below) | **Real** |
| `canvas` | Canvas Diagram / Product | Asks an agent for a **mermaid/excalidraw** diagram and writes it under the data dir (`workflow-canvas/{run}/{node}.{ext}`); output `{ scene_id, path, diagram, … }`. | `prompt`, `mode` (`mermaid`/`excalidraw`), `provider?`, `model?` | **Real** |
| `self_improve` | Self-Improve (offer) / AI | Runs the **self-improvement engine** in OFFER-ONLY mode (`Autonomy::Propose` — every skill/memory edit is queued for approval, **never** applied) over recent sessions and posts the offered list to the trigger's chat thread; output `{ run_id, summary, offered, edits }`. | `providers[]` or `provider?` (empty ⇒ the workspace's configured Self-Improvement providers) | **Real** |
| `git_pr` | Git PR / Network | **Drafts** a pull request for a repo branch — the title/description are crafted by an agent (the node's `provider`/`model`, empty ⇒ run's resolved default; not the claude-only orchestrator). Default is draft-only (`opened:false`); **`open:true`** actually OPENS the PR on the remote (outward-facing, per-step opt-in) — gate it on the incoming edge (e.g. the review passing). | `repo_id`, `base` (default `main`), `open` (default `false`), `worktree_path?`, `provider?`, `model?` | **Real** |
| `git_checkout` | Check out PR / branch / Network | **No agent.** For each repository of the run: finds the pull request (or branch) by fixed rules, checks its head out detached, and writes `source`, `base` (the merge-base) and `base_branch` to `repos.json`. See *Steps that need no agent* below. | `mode` (`pr` default · `branch`) | **Real** |
| `script` | Run script / Flow | **No agent.** Runs a shell command (once, or once per repository) with a time limit; run values arrive as `OTTO_*` environment variables. Its output file becomes the step's answer. | `command` (required), `timeout_s` (10–7,200, default 900), `foreach` (`repos`), `cwd` (`worktree` · `context`), `output_file` (a bare file name), `ok_exit_codes` (default `[0]`), `allow_failure`, `idempotent` | **Real** |
| `post_review_comments` | Post review comments / Network | **No agent.** Posts a review step's findings on the PR: bugs and warnings inline, notes in one combined comment, a summary with the score — each at most once. | `review_from`, `inline` (default `["bug","warn"]`), `info` (`combined` · `skip`), `summary` (default true), `summary_intro_from`, `summary_files[]`, `max_inline` (default 150) | **Real** |
| `check` | Check criteria / Flow | Checks each criterion and records the evidence: a command's exit status, a rule, an agent that must cite evidence, or a person. Only agent criteria use an agent. See *Goals* in §6 and [Goals](./goal-loops.md). | `criteria[]` (`{id, text, verify: command\|agent\|human\|expr, how?, command?, expr?, timeout_s?}`), `evaluator.{provider, model, effort}`, `instructions`, `final_review` | **Real** |

> **¹ `game_engine` / `verifier` are real but "scaffold" nodes.** They execute
> and produce usable structured output — they do real work, and `verifier`
> genuinely errors on a missing/trivial game file in the game path. But these are
> now the **only** scaffold kinds: `game_engine` returns a **canned spec template**
> with the
> note *"Scaffold build: wire a real game engine here."* and `verifier`'s
> non-game path emits a scaffold *"replace with the real certifier."* report.
> They are intended as a game-pipeline scaffold (the built-in templates use
> `agent_prompt → game_engine → verifier`), pending a real external game engine.
> Treat their output as a scaffold, not a certified production artifact.

> **² The four product/review nodes are now wired (formerly stubs).** They run a
> **real single-agent turn** over the story's built context (`ctx.product.
> build_agent_context`) with the matching product **skill inlined** (`grill` /
> `jira-story-writer` / `story-task-breakdown`), on a thread of the run like
> any agent step; `product_*` can optionally `persist` a product version.
> `review_run` calls the in-process **local-review engine**
> (`run_review_for_branch`), waits for it, and computes a deterministic 0–100
> score from the blocking/advisory finding counts (`review_findings_counts`),
> optionally blending in an agent goals-assessment. They no longer emit a
> `{"stub": true}` marker — older docs/UI hints about "not wired" are obsolete.

> **Heads-up — wired nodes still have prerequisites.** `db_query` (a saved
> DB-Explorer connection), `broker_peek` (a broker cluster), `swarm_task` (a
> running swarm/project), `review_run`/`git_pr` (a registered git repo + a
> `repo_id` in params or run input), and `product_*`/`product_publish` (a
> `story_id`, and an Atlassian `account_id` for a real publish) all depend on
> other features being set up. A missing dependency causes the **node** to error
> (and downstream active-path nodes to skip) — not a silent no-op.

### Prepare relevant data (`prepare_context`)
A dedicated node (`otto-workflows/src/prepare.rs`) for the common "go read the ticket
before you start" step, run **app-side** rather than by the agent (so a slow or
unreliable Jira fetch never eats an agent's context or turn budget):

1. **Resolve a Jira key**, in order: `params.key` → `input.jira_ticket` (both
   trusted verbatim — the caller already knows the exact key) → the first
   Jira-key-shaped token (`[A-Z][A-Z0-9]{1,9}-[0-9]{1,7}`) scanned out of
   `input.prompt`, then `input.msg` (free text — scanned, not trusted blindly).
   No key found ⇒ output `{ jira: { found: false } }`, no fetch attempted.
2. **Fetch it** through the workspace's configured Jira issue account:
   `params.account_id` wins; else the run user's own Jira account; else any
   Jira account configured on the daemon (single-user/admin-configured setups).
   On success, the full issue (description, comments, links, attachments,
   custom fields) is rendered to markdown and written to `jira-<KEY>.md` in the
   run's context dir (see §7). On failure, a **loud** placeholder is written
   instead (*"⚠ Could not fetch \<KEY\>"* + the error + "treat ticket details as
   UNAVAILABLE") and the node still **succeeds** — unless `params.require:
   true`, which errors the step. Either way the output carries
   `{ jira: { found, key?, fetched?, summary?, status?, url?, error? } }`.
3. **Optional analysis phase.** When `params.prompt` is set, the node runs a
   second phase identical to `agent_prompt` (same preamble, skills, provider
   default `claude`) over the gathered context on a thread of the run; its
   `reply`/`thread_id`/`working_directory` merge into the output.

Every run re-fetches (there is no cross-run cache, §3), so a ticket edited since
the last run is always picked up. The `ui-test-authoring` and
`api-acceptance-test-authoring` templates (§6) both lead with this node.

### Per-step skills (`skill` / `skills`)
An `agent_prompt` step can name a skill to run *via prompt*: set `skill` (a string)
and/or `skills` (an array of strings) on the node params. Each named skill's body
(plus its references) is resolved from the bundled skill library and **prepended**
ahead of the step's prompt in the shape `{skill}\n\n---\n\n{prompt}` (names are
de-duplicated, in declared order). So a step can apply a specific method —
e.g. `golang-feature-implementation` on a "implement" step, or a review lens on an
agent step — without hand-copying the skill text. The four `product_*` nodes
already inline their matching method skill **by kind** (`grill` / `jira-story-writer`
/ `story-task-breakdown`), and `review_run` consumes `skills`/`lenses` as *reviewer
lens* skills (below) rather than prompt-prepended text.

### Watching the review summarizer

On the run page a review step lists its reviewers and its **Summarizer** as named
rows under the step, each with its word (Working 6m, Done · 14 findings…); the
step's **Summarizer** tab shows the summarizer's conversation, and a reviewer
opens its own conversation and **Findings**. An empty provider setting defaults
to Claude, and the configured model is preserved.

The run page keeps discovering review records even when a step uses `await:false`
or a summarizer is retried after the workflow finishes. Reloading the run restores
the review association and current summarizer. Reviewer and summarizer threads
are engine threads (§3): closed when their turn ends, kept for the run's
retention period, never in your Threads lists.

Failures, the finding-count timeout (5–30 minutes), and empty summaries
when findings exist can preserve a deterministic summary. This is not independently
verified output: adaptive reviews remain incomplete and cannot pass on fallback
alone. The run labels this
**Deterministic fallback**. Cancelling a review stops its summarizer and leaves
the review cancelled, without creating a fallback summary. Each retry uses a new
session and result file; incomplete JSON is never a completion signal.

### `review_run`, gating & auto-PR
`review_run` drives the **same adaptive engine as a PR review**. It assesses
changed behavior and affected consumers, then selects justified lenses. Configured
`providers[]` form a pool; one provider runs each selected lens by default.
`cross_provider_check:true` opts into additional provider comparisons. `skills[]`
remains an alias for `lenses[]`, but an installed skill is not required.

The verified findings produce a **0–100 `score`**, with `passed`, `threshold`
(default 80), `blocking`, `advisory`, `findings`, `providers` and `lenses` in the
output. A goals assessment can be blended into the score. Default deductions are
20 per must-fix, 5 per should-fix and 0 per optional finding. Explicit weights are
preserved. A pass requires complete reviewer and verifier coverage, a nonempty
diff, no open must-fix findings and a score meeting the threshold. A passing
numeric score cannot override a must-fix finding.

`require_pass:true` errors the step when those conditions are not met; dependent
steps are error-skipped. Empty provider/lens configuration inherits the effective
repository review configuration (custom, named preset or global default).

The output's `outcome` is one of `passed`, `failed`, `incomplete` or `pending`
(`await:false`). **`incomplete` is not a verdict**: the review stored its
reviewers' findings but could not verify them (the verifier failed, a reviewer did
not finish, or the change was only partly covered — see
[`code-review.md`](code-review.md#how-a-review-ends)). Then `score`, `review_score`
and `passed` are not a result: `score` is `null` (never 0), `score_text` reads
`incomplete`, or `partial — N/100` with `partial_score` when the verifier did run
over the lenses that finished, `reason` starts `review incomplete — not a verdict:`
and `verification` gives `state` (`verified`/`partial`/`incomplete`/`none`),
`unverified`, `coverage` and `missing_lenses`. An incomplete review **always**
errors the step, with or without `require_pass`, so no downstream step posts or
gates on a non-verdict. `allow_summary_fallback:true` opts in to scoring the
stored unverified findings when the verifier is the ONLY thing that failed.

**Retry step on an incomplete review.** The step remembers its review. When
**Retry step** re-enters it with the same diff, head and configuration and every
reviewer of that review finished (only the verifier failed, its runner vanished,
or a daemon restart interrupted it), the step re-runs ONLY the summarizer over
the stored findings, waits for it, and recomputes its output, score and chat
message — no new checkout, scope assessment or reviewer fleet. A review with a
failed reviewer, a partial diff, or a changed head starts a new review instead.
A run resumed after a daemon restart (`on_restart: resume`) adopts the review the
same way; if every reviewer had finished, boot recovery has already resumed its
summarizer (at most twice per review) and the step simply waits for it.

#### Execution compatibility

Existing `review_mode` run overrides, `params.mode` and stored `ReviewConfig.mode`
remain accepted for compatibility. The step logs the resolved legacy preference;
adaptive review executes managed fan-out regardless of that preference, so every
selected reviewer participates in the same daemon-wide concurrency limit.
`review_mode` still accepts only `fan_out` or `orchestrator`; an unknown value, or
an override combined with a non-object/non-null run input, is rejected with 400.

Multi-repository steps admit at most two repositories concurrently and preserve
per-target result order. These reviews share the global reviewer queue; a second
repository does not multiply the provider concurrency allowance.


The review's comparison base resolves in order: node `base` param → input
`base` → the run's ambient base → the repo's **detected default branch** — and
a named ref that doesn't exist locally falls back through `origin/<base>` (and a
one-branch fetch of `<base>` from `origin`) to the default instead of failing.

Before any step runs, admission **syncs the run's branches**: for every declared
repo it fetches `origin` (with the repo's git-account credential) and makes the
entry's base and the PR's `pr_branch` resolvable by name — a branch that exists
only on the remote gets a local tracking branch, and a stale local copy that is
checked out nowhere is **fast-forwarded** (compare-and-swap `update-ref`). It never
forces a ref or touches a working tree: a branch checked out behind, or diverged
from `origin`, is left as is and the run's worktree is cut from `origin/<base>`
instead. A failed fetch is logged and the run continues on cached refs.

A run that names its PR only by number (`PR: 15`, or `repo-a#15, repo-b#340` for
several repos — a bare number applies only to a single-repo run) looks the PR up
through the repo's provider as the acting user: the PR's target becomes the
entry's base unless a base was given explicitly, and its head becomes
`pr_branch`. Both are synced as above; the head commit is pinned to
`origin/<head>` when that IS the PR head, else the host's PR head ref is fetched
into `refs/otto/pr-review/*` (fork PRs on GitHub/GitLab; Bitbucket exposes no
such ref, so its fork heads resolve only when the commit is already local). The
result is the run input's `prs[]` (`repo_id`, `number`, `head`, `base`,
`head_sha`, `head_ref`) and the run context file `prs.json`; fetched fork refs
are deleted when the run's worktrees are reaped. The node publishes the **resolved** branch in
its output `base` (what a downstream `git_pr` targets). With multiple `repos[]`
declared on the run and no explicit target, one review runs per entry and the
output aggregates (`score` = min, `passed` = all, per-repo `reviews[]`).

Workflows gate "move forward" on the review through **edge conditions**, not a human
approval: put `output.satisfied == true` (the loop's pass flag) — or rely on
`require_pass` — on the edge from the fix→review loop into the PR step. The `git_pr`
node's **`open:true`** then actually opens the PR on the remote; because the
incoming edge only fires on a passing review, the PR opens automatically *and only*
when the bar is cleared. `open` defaults to `false` (draft-only), so this is per-step
opt-in. The bundled `write-tests`, `implement-feature`, `ui-test-authoring`, and
`api-acceptance-test-authoring` templates all wire exactly this (their old
`human_approval` nodes were removed — the review is the approval); `po-lifecycle`
is unchanged and still uses `human_approval`.

### Branching & loops in practice
- **`condition` + edge conditions** are the if/else primitive: a `condition` node
  emits `{ result, value }`, and you put `output.result == true` / `== false`
  conditions on its two outgoing edges. The false branch is **branch-skipped**
  (clean, not a failure); a downstream join runs from whichever branch stayed
  active. Any edge can carry a condition — you don't strictly need a `condition`
  node if the prior node's output already has the field you want to test.
- **`loop`** runs its inner `steps[]` (each a `{kind, name, params, retry}` object,
  executed by the **same `execute_node`** path as top-level nodes) up to
  `max_iterations`, stopping early when the `until` expression holds against
  `{ iteration, last, steps, input }` or when an inner `check` step finds every
  criterion met. The previous step's output threads into the next step, and
  run-level keys (e.g. `repo_id`, `goals`) flow to every step. The built-in
  flow templates use the loop "Review → fix until passing":
  `review_run → check (rule: the review passes) → fix`, which ends the moment
  a review passes. Each iteration is a child row of the loop step on the run
  page.

### Steps that need no agent
Four kinds do in code what an agent used to do by recipe. They take a step
slot but no agent, no model and no quota, and they finish in seconds.

**Check out PR / branch (`git_checkout`).** For each repository in the run's
`repos.json`:
- **The pull request** (`mode: pr`, the default) is found from, in order:
  the `pr` input (authoritative — a PR that is not open is reported, never
  "corrected"); the open PRs whose source branch is the `pr_branch` input; the
  open PRs whose source branch is `feature/<Jira key>` (or starts with it) or
  whose title names the key. If that does not leave exactly one open PR, the
  repository is left unresolved and the candidates are listed.
- **A branch (no pull request)** (`mode: branch`): the first that exists on
  `origin` of the `pr_branch` input, the repository's declared `source`,
  `feature/<Jira key>`; else the single remote branch naming the key. The
  base is the `base` input, else the repository's default branch.
- It fetches, checks the head out **detached** in the run's worktree, takes
  the merge-base, and rewrites the entry's `source`, `base` (the merge-base
  commit) and `base_branch`; PRs found by rule are added to `prs.json`. An
  unresolved repository keeps its reason in `repos.json`; the step fails only
  when no repository resolved. Branch names from the input are checked before
  git sees them. The step file is the result table.

**Run script (`script`).** One command under `/bin/sh -c`, in its own process
group (killed on timeout, cancel, skip and when the shell exits), with the
daemon's environment scrubbed, `PATH` from your login shell, and the same
Seatbelt confinement as a scheduled shell task.
- **Run in** — each repository's worktree, one after another
  (`foreach: repos`; a repository that did not resolve is left out, and with
  none the step succeeds with "no repositories to run on"); the run's working
  folder, once (default); or the run folder with its step files, once
  (`cwd: context`).
- **Values arrive as environment variables, never pasted into the command**:
  `OTTO_RUN_ID`, `OTTO_CONTEXT_DIR`, and per repository `OTTO_WORKTREE`,
  `OTTO_BASE`, `OTTO_BASE_BRANCH`, `OTTO_SOURCE`, `OTTO_REPO` (the main
  clone's path), `OTTO_REPO_NAME`, `OTTO_REPO_ID`; with an output file,
  `OTTO_OUT` is its name.
- **Output file (optional)** — a bare file name in the run folder
  (`$OTTO_CONTEXT_DIR/$OTTO_OUT`). Its text becomes the step's answer and
  what the next steps read; without one the answer is the command's output.
  All output also goes to `<step>.log` beside the step file. A re-run starts
  the output file afresh.
- **When it exits with an error** — an exit code outside `ok_exit_codes`
  fails the step with the last 20 lines of output ("Fail the step"), or with
  `allow_failure` is recorded and the run continues ("Record it and
  continue"). A timeout fails it: "timed out after 900 s".
- A script caught mid-flight by a daemon restart is re-run only with
  `idempotent: true` (**Safe to run again after a restart**, under the step's
  **Advanced**, beside **Exit codes that count as success**); otherwise its
  outcome is unknown and the run stops for you (§7 *Restart resume*).

**Post review comments (`post_review_comments`).** Posts the findings of the
nearest review step before it (or the one named in `review_from`) on the
run's pull request, taken from `prs.json`: bugs and warnings as inline
comments (at most `max_inline`, default 150 — the rest go into one "More
findings" comment), notes as one combined comment, then a summary comment
with the score (its opening text can be an earlier agent step's answer,
`summary_intro_from`, and it can append run files, `summary_files`). A
comment already on the PR (same file, line and first line) is skipped and
every posted draft is marked, so a retry or a re-run never posts twice. A
local review with no PR posts nothing and says so; an incomplete review posts
nothing. What was posted is on the run as **Delivered** (§7).

**Check criteria (`check`).** Decides each criterion and keeps the evidence:
command criteria run first, one after another, in the run's working directory
(exit 0 = met; the evidence keeps the exit status and the last 8 KB of
output, everything in `<step>.<criterion id>.log`); rule criteria
(`verify: expr`) are evaluated on `{iteration, steps, last, input}`; agent
criteria get one checker turn on a fresh thread, and a "met" without evidence
counts as not met; a criterion **you** check makes the step wait for you
(*Needs you*, §7) once everything else is met. With `final_review`, an
independent review turn must approve with no findings. Output:
`{criteria: [{id, text, verify, met, evidence, verified_by}], all_met, met,
total, progress_pct, verdict: achieved|continue|blocked, feedback, …}`. Inside
a loop a passing check ends the loop.

### Conditions on the run's repositories
Edge conditions and the `condition` node read the run itself through `run`:
`run.input`, `run.repos` (the entries of `repos.json` as they are now — after
a `git_checkout` they carry `source`, `base`, `base_branch` and any
`error`) and `run.prs` (`prs.json`). So a graph can branch on what the run
works on without an agent step: `len(run.repos) > 1` for "more than one
repository", `len(run.prs) == 0` for "no pull request". The `condition`
step's inspector offers this as **Field · operator · value** with the
expression it writes shown under it; **Write an expression instead** keeps
the free form. A step that rewrites `repos.json` and a condition that reads
it should be connected one after the other, not side by side (validation
warns).

---

## 5. Triggers

A workflow runs **manually by default**. You can attach triggers in the
**Triggers** panel (`TriggersPanel.svelte`). Trigger rows live in the
`workflow_triggers` table (migration 0058); the schedule/event firing logic is in
`workflow_trigger_scheduler.rs`.

```ts
type TriggerKind = 'schedule' | 'webhook' | 'event' | 'chat';
interface WorkflowTrigger { id; workflow_id; kind; spec: object; enabled; created_at; armed_at? }
```

| Kind | Spec | Fires when… | Run input | Wired & firing in the daemon? |
|---|---|---|---|---|
| **webhook** | `{ token }` (32-byte URL-safe token auto-generated server-side on create) | An external system calls `POST /workflows/{id}/webhook/{token}`. The token **is** the credential — no bearer auth required. The trigger row's copy button copies the full URL. | The request body (JSON), or `null`. | **Yes** — handler `webhook_trigger` spawns the run. |
| **event** | `{ event_kind, filter_json? }` | A daemon event whose mapped name equals `event_kind` fires on the event bus. | `{ "trigger": "event", "event_kind": "..." }` | **Yes** — `spawn_workflow_event_trigger_listener` is started at daemon boot (`ottod` main, "workflow event-trigger listener started"). |
| **schedule** | `{ cadence, every_min, at, weekday, expr, timezone, last_run, enabled, prompt? }` (the **shared cadence** format — same as Scheduled Tasks; `prompt` is new — see *Prompts & chat bindings*) | Cadence comes due: `interval` (every N min, default 60), `daily` (at `HH:MM`), `weekly` (weekday 0=Mon at `HH:MM`), or **`cron`** (`expr`, 5-field). All interpreted in the spec's IANA **`timezone`** (default UTC). | `{ "trigger": "schedule" }`, plus `"prompt"` when `spec.prompt` is set. | **Yes** — `workflow_trigger_scheduler::start` is started at daemon boot (`ottod` main, "workflow schedule-trigger scheduler started"). |
| **chat** | `{ channel: "slack"\|"telegram", chat: "<id>", thread?: "<ts>", mention_only?: bool, chat_updates?: "live"\|"per_step" }` | Any inbound message that matches the binding (channel/chat exact, thread pinned or open, `@mention` if `mention_only`) — see *Prompts & chat bindings* below. | `{ trigger: "chat", origin_workspace_id, channel, chat, thread, user, prompt, msg, raw }` (+ `chat_updates` when the binding sets it) | **Yes** — evaluated **live** by the channels `Bridge` on every inbound message (not polled, unlike the other three kinds). |

### Editing and previewing triggers

The Triggers panel can add and edit all four trigger kinds. Schedules expose
interval, daily, weekly and five-field cron, IANA timezone and prompt. Events
expose canonical event names and a JSON object filter. Chat bindings expose the
channel, chat, thread and mention setting. Result destinations support Slack or
Telegram chat/thread and an HTTP(S) webhook. Editing retains server tokens,
cursors and extension fields; clearing a destination removes it from use.

**Arming (`armed_at`, migration 0151).** A schedule trigger is armed when it is
created, re-enabled after a pause, or given a different `cadence` / `every_min` /
`at` / `weekday` / `expr` / `timezone` / `run_at`; editing only the prompt or the
destinations doesn't re-arm. The scheduler never looks before
`max(last_run, armed_at)`, so re-enabling a trigger after a week does **not**
fire the run it missed (with its worktrees and chat delivery) within a minute,
and a daily 09:00 trigger enabled at 15:00 waits for tomorrow 09:00. Rows
created before 0151 have `armed_at = null` and keep the old behaviour until
their next resume or re-time.

**Preview / validate** validates the draft without saving or firing anything.
For schedules it displays the next five fire times using the scheduler's cadence
engine and the selected timezone. Invalid timezone, cadence, event filter or
result destination returns an actionable validation error.

### Event-kind mapping (configure by string in the trigger spec)
The event listener maps daemon `Event` variants to stable strings; the UI default
in the add-trigger form is the supported canonical value `review_changed`.
The UI offers the canonical identifiers from this mapping:

| Daemon event | `event_kind` you type |
|---|---|
| `ReviewChanged` | `review_changed` |
| `BudgetExceeded` | `budget_exceeded` |
| `ProductChanged` | `product_changed` |
| `SwarmStatus` | `swarm_status` |
| `ImprovementRunFinished` | `improvement_run_finished` |
| `InsightReady` | `insight_ready` |
| `WorkflowRunUpdated` | `workflow_run_updated` |

Session/metric/notice/trail/task and other high-churn events are deliberately
**excluded** (the listener returns `None` for them) — they are too noisy to be
useful macro triggers.

> **✅ Schedule triggers fire at boot now (closed gap).**
> `workflow_trigger_scheduler::start` — a 60-second supervisor that scans enabled
> `schedule` triggers, checks `is_due`, advances the `last_run` cursor first
> (idempotency: a slow/failing run can't double-fire), and spawns runs — is
> **started in `crates/otto-server/src/boot/tasks.rs`** at boot (log line *"workflow
> schedule-trigger scheduler started"*), alongside the event listener and the
> swarm / scheduled-tasks supervisors. Its `is_due` **delegates to the shared
> `cadence` engine** (the one Scheduled Tasks use), so workflow schedule triggers
> get **cron** (`cadence:"cron"`, `expr`) and **IANA timezone** (`timezone`) parity
> for free; `interval`/`daily`/`weekly` behave exactly as before. All three trigger
> kinds — webhook, event, schedule — now start runs unattended.

### Chat trigger: `Action: Workflow` (Slack / Telegram / webhook)
A structured inbound channel message can **start a workflow run by name** instead of
opening a normal session — wired through the channels `Bridge` via the
`WorkflowChatTrigger` hook (`otto-workflows/src/chat.rs`; `ottod` injects
`WorkflowChatTriggerImpl`, mirroring the swarm/run triggers). The message shape
(field labels case-insensitive; `Goals:` may be a bullet list **or** an inline
comma/semicolon list):

```text
@otto
Action: Workflow
Name: Implement Feature        ← resolved case-insensitively against the workspace's workflows
Msg: please do x y z, follow all relevant rules
Jira ticket: PROJ-1111
Working Directory: ~/repo
Relevant Info: ~/a, ~/b
Goals:
  - 100% test coverage
  - under 2 minutes runtime
```

The parser requires `Action: Workflow` **and** a non-empty `Name:`. On a match it
starts a run whose **input carries every parsed field** (`trigger:"chat"`, `channel`,
`chat`, `thread`, `user`, `name`, `msg`, `jira_ticket`, `working_directory`,
`relevant_info[]`, `goals[]`, `raw`) — so the first node (e.g. a "prepare relevant
info" agent) can consolidate the ticket / working dir / paths into a brief for the
rest of the graph — and replies in-thread with where the run landed:

- a free run slot: *"🚀 Started workflow **X** (run `id`). Working through the
  steps now — goals: … Reply `status` · `skip` · `abort` · `help` in this thread
  to control it."*
- every slot taken (at most `OTTO_WF_MAX_PARALLEL_RUNS`, default 2, runs execute
  at once): *"⏳ Queued workflow **X** (run `id`) behind N run(s). It starts when
  a slot frees (at most N run at once). …"* — and when the run gets its slot the
  same thread hears *"🚀 Started workflow **X** (run `id`) — it was queued for
  25 min."* before any step reports. A run that started at once gets no second
  message.
- a refusal (its workspace can't be loaded, the `Working Directory:` is outside
  every registered repo, the run row can't be created) replies *"⚠️ Can't start
  **X** …: <reason>."* — an explicit trigger never ends in silence.

A name that only matched after normalisation adds *"ℹ️ No workflow is named
exactly "…" — matched **X**."*. The run's page in Otto
(`#/workflows/<workflow id>/runs/<run id>`, **Copy link** in the run header)
says who started it and where ("Started from Slack", or the person and the
channel), and keeps the run input under **Result → Input**.

`Name:` is matched case-insensitively first (`name = ? COLLATE NOCASE`), then
after normalising both sides (dash variants — a typed `-` finds an em dash —
curly quotes, spacing, case). The name is read as the person typed it: the
Slack listener decodes Slack's wire format once (`&amp;`/`&lt;`/`&gt;`,
`<@U…>` → `@name`, `<url|label>` → `label (url)`) before the bridge, and the
workflow parsers decode and strip nothing again (a second pass turned
`Name: A <B>` into `A B`). The decode is
not cosmetic: Slack escapes `&` in message text, so a workflow named
*"Provider — games fetch & review"* arrived as `… fetch &amp; review`, matched
nothing, and `try_start` returned `None` — the trigger became a plain chat
session with **no error reported anywhere**. A name that still matches nothing
now gets a reply naming the closest workflows: *"No workflow named "X". Did
you mean: A, B, C? Reply `help` for the full list."* (or *"… Reply `help` for
the list of workflows."* when nothing is close, *"… — this workspace has no
workflows yet."* when there are none); several workflows that normalise to the
same name reply *""X" matches more than one workflow: A, B. Type the exact
name."*

**Worked example — trigger the "Write tests for a story" template from Slack.**
Instantiate the `write-tests` template (workflow name *"Write tests for a story"*),
then post this where the Otto bot is configured for the workspace (`Name:` must
match the workflow name):

```text
@otto
Action: Workflow
Name: Write tests for a story
Msg: Add tests for the new deposit-limit rule; cover happy path + over-limit + boundary.
Jira ticket: PROJ-1421
Working Directory: ~/code/go_deposit
Relevant Info: ~/code/go_deposit/internal/limits, ~/shared-rules/GO_TESTING_STANDARDS.md
Goals:
  - 100% coverage of the services package
  - all component tests pass
  - suite runs under 2 minutes
```

`implement-feature` and `po-lifecycle` use the same shape (add a `story_id` for the
product steps). Slack carries `msg`/`jira_ticket`/`working_directory`/`goals` to
every node; the `review_run` / `git_pr` steps additionally need a **`repo_id`**,
which the chat message doesn't carry — either run those templates from the
**Run…** start form (it carries `repo_id`/`base`/`goals`) or have the first
"prepare relevant info" node resolve the repo from `working_directory`.

**Updates in the thread (while the run executes).** A run started from a chat
reports back into the same thread (Slack/Telegram). A single pump task posts in
order, so the engine never blocks on chat latency. There are two styles; the
Start step's **Updates in the chat thread** picks one (`chat_updates`, also
accepted in the run input and on a chat binding's spec):

- **One live message, plus each step's file** (the default, `live`). The run
  keeps **one status message** and edits it in place (at most one edit every
  3 s). Its first line reads
  `*<workflow>* · <working|reviewing|…> · 5 of 9 steps · 6m 40s (usually 12m)`,
  then one line per step: `◻ <step>` (not started), `▶ <step> · 2m 10s`
  (running, with what it is doing), `⏳ <step> — <why it waits> since 10:42`,
  `⚠️ <step> — <what it asks>`, `✅ <step> · 14s — <one line of what it
  did>`, `❌ <step> failed — <error>`, `⏭ <step> — skipped (<why>)`,
  `⏹ <step> — stopped`. Past 3,000 characters the finished steps fold into
  `✅ N steps done`. A run still queued for a run slot already has its message
  (`*<workflow>* · queued` / `⏳ Queued — 2nd in line`), and one canceled
  there is edited to `⏹ *<workflow>* — canceled while queued`. The message's
  id is stored on the run, so a restart, a Resume or a retry edits the same
  message. **Every step's file is still posted** to the thread, captioned
  `✅ <step> · <duration>` or `❌ <step> failed — <error>` (loop iterations:
  `› ✅ <step> · iteration K`). **New posts** — the ones that notify — are
  made only when the run needs a person
  (`⚠️ @you *<workflow>* needs you — *<step>* is waiting for an approval — … Approve it in Otto (Workflows → Runs).`;
  a usage limit says "Otto continues it when the limit resets."), when it is
  held (`⏸ *<workflow>* paused — …`, `⚠️ *<workflow>* is blocked — …`,
  `⚠️ *<workflow>* is out of budget — …`) and for the final result.
- **A message and a file per step, as before** (`per_step`). The thread of
  earlier releases, byte for byte: a **run-start** line
  (`🚀 <name> started — N step(s) queued.`, plus `Goals: …`); per
  **meaningful** step `▶ <step> started` then `✅ <step> done (<dur>)` with a
  short summary (or `❌ <step> failed — <err>`) and the step's file; for the
  **fix→review loop** a `🔁 Iteration N/M` header, the inner sub-steps as
  `› ▶ …` / `› ✅ …` / `› ❌ …`, and a review block
  `🔍 Review #N done — score X/100 (pass ≥ T) — ✅ passed` / `⚠️ <reason>`
  followed by the per-lens scores and a bulleted findings list (or *"Findings:
  none 🎉"*, said only of a review that finished with none). A review that did
  not complete posts `🔍 Review #N — ⚠️ Review incomplete — not a verdict`
  instead, with `Why:`, the verification coverage, how many findings are
  stored unverified, the lenses that did not finish, a `Partial score` when
  the verifier ran, and how to finish it (**Retry step**, the review's own
  page, or a new run). With steps running side by side the lines arrive in
  the order things happen.

In both styles structural/plumbing nodes (`log`, `transform`, `delay`,
`condition`, `manual_trigger`) are not listed, everything is **flushed before
the final result is delivered**, and every line, caption and file is redacted.
Manual UI runs and webhook-only triggers **do not report** (there is no chat
target). The target is the trigger origin (`channel`/`chat`/`thread`), or an
explicit `result_chat` (+ `result_channel` / `result_thread`) override; the
integration token is resolved **per-workspace** (from the origin/trigger
workspace).

**Controlling a run from its thread.** Reply in the run's thread: `status`
(`progress`, `?` — in the live style the answer is the status message's
text), `skip` (`next` — skips the running step; with several running it asks
"Several steps are running: A, B — reply `skip <step name>`.", and a name
prefix or part of the name picks one while the others keep running), `abort`
(`cancel`, `stop`), `re-review` (`review again`) and `help`. Approvals,
answers, pausing and resuming are done in Otto, not by reply.

**Files sent with the starting message (Slack).** Files attached to the Slack
message that starts a run reach the run as before — the message text names
where each was saved, so the agents read them — and the run now also keeps
its own copy: each file is copied to `<context>/attachments/<safe name>`
before the run can queue (names reduced to `[A-Za-z0-9._-]`, at most 10 files
and 50 MiB per run) and listed in `attachments.json`. The run page shows them
under **Result → Input → Files sent with the run**, with **Preview** for text
files and **Download** for any; a file that could not be received says why.
Telegram receives no files.

**Result delivery (done/failed + deliverable).** When a run that was started from
a chat **finishes**, the engine replies in the **same channel + thread** it was
triggered from and attaches **one file**: on a successful run that produced
one, that's `final-output.md` — the run's actual deliverable (see *Final
output* in §7); otherwise it's the always-generated `summary.md` (every step,
its status/duration/attempts, and a peek at each output). In the live style
the post says what happened and what was delivered, in words —
`✅ *<workflow>* passed — 92/100 · 12m 05s`,
`⚠️ *<workflow>* done — review 62/100, not passed · …`,
`✅ *<workflow>* done · …`, `❌ *<workflow>* failed at *<step>* — <error>`,
`⏹ *<workflow>* canceled at *<step>* · …` — followed by one line per
deliverable (`• <what>: posted`, `• <what>: not posted (<why>)`, `• <what>: pending approval`) and, for
a failed run on a PR that got nothing, "Nothing was posted on PR #N.". In the
per-step style it is the earlier brief status (`✅/❌` +
`N/M steps ok · failed · skipped`, the review score if any, the proof-pack
id). Both are redacted the same way before they leave the machine. This is
origin-driven (`deliver_run_result` reads `channel`/`chat`/`thread` from the
run input), so manual UI runs don't post anything unless **Send the result
to** in the start form names a chat. To also POST the result to an external
system, include a `result_webhook` (or `callback_url`) in the run input —
it's delivered through the SSRF-guarded webhook path, with the same
final-output/summary choice. Cancellations and time-outs report too (always
with `summary.md` — only a `success` run can have a `final-output.md`).

### Prompts & chat bindings
Two more ways to start a run by chat, both handled by `otto_workflows::chat`
(`WorkflowChatTriggerImpl`) alongside the structured `Action: Workflow` command
above — all three resolve in order (legacy structured → simplified command →
channel binding) against every inbound Slack/Telegram/webhook message, before
normal session routing.

**The `input.prompt` convention.** Regardless of which path starts a run, the
ask ends up in `input.prompt`: a chat message's text, this section's simplified
command tail, the manual Run-dialog's **Prompt** box, a webhook body's `prompt`
field, and a `schedule` trigger's `spec.prompt` (table above) all land there. When
a trigger only set `msg` (the chat paths) and `prompt` is absent/blank, the
engine's `normalize_prompt` copies `msg` into `prompt` before the graph runs — so
every agent-facing step, and `prompt.md` in the run's context dir (§7), can rely
on `input.prompt` without caring which trigger fired.

**Simplified command — `run <name>: <prompt>`.** No `Action: Workflow` scaffolding
needed: post `run <name>: <prompt>`, `workflow <name>: <prompt>`, or
`run workflow <name>: <prompt>` (keywords matched case-insensitively at the start
of the message, tried **longest-first** so `run workflow` isn't shadowed by
`run`). `<name>` is the text up to the first `:` on that line (resolved
case-insensitively against the workspace's workflows); `<prompt>` is the rest of
that line plus every following line.

```text
run Write tests for a story: Add tests for the new deposit-limit rule.
Jira ticket PROJ-1421, cover happy path + over-limit + boundary.
```

An unknown name behaves differently depending on which keyword matched: the
**explicit** `workflow`/`run workflow` keyword replies *"No workflow named
"X". Did you mean: …?"* (the same suggestions as above) without starting a run;
the **bare** `run` keyword silently falls
through to normal chat (bare "run" reads too much like ordinary English to
hijack a message that wasn't meant for it).

**Channel bindings — pin a workflow to a chat.** A `chat`-kind `WorkflowTrigger`
(add one in the **Triggers** panel → **Chat binding**) starts its workflow on
**every** matching inbound message, no keyword required — good for a
workflow that IS the channel's purpose (e.g. a #test-requests channel where
every message is a test-writing ask). Spec: `{channel, chat, thread?,
mention_only?, chat_updates?}` (`chat_updates` is copied into the run input).
`channel` + `chat` must match exactly; an absent `thread` in
the spec matches any thread, a present one requires an exact match (a
thread-pinned binding wins over an unpinned one when both match); `mention_only`
(default `false`) requires the message to @-mention someone (an `@word` —
Slack mentions arrive decoded as `@name`; an e-mail address does not count;
the leading bot mention is already stripped by the listener, so it alone does
not satisfy it). On a match the run input is `{trigger:"chat",
origin_workspace_id, channel, chat, thread, user, prompt, msg, raw}` —
`prompt`/`msg` are the mention-stripped text.

All chat trigger parsing reads the message as the person typed it: Slack's
wire encoding is decoded once by the Slack listener, and nothing is decoded or
stripped again here — `Name: R&D`, `Name: A <B>`, `x < 5 and y > 3` and
`Vec<String>` reach the workflow unchanged, and a pasted Jira link (decoded to
`title (url)`) resolves to the key in its URL.

**Does a trigger work?** Every trigger row carries `last_fired_at` /
`last_outcome` (`started` · `queued` · `rejected`) / `last_detail`, stamped
(one UPDATE) where a schedule, event, chat-binding or webhook trigger fires or
is refused — e.g. *"previous run of this workflow still active"* for a due
schedule whose last run is still going. The Triggers panel shows it under each
trigger: *Last fired 2h ago · started*, *Never fired*, or *Last attempt
rejected: <reason>*.

A sender who is not on the integration's `allowed_users` list is dropped
before any routing — by the channel bridge, or already by the Slack listener
(before it downloads attachments, and when the inbound queue is busy) — and
gets **no reply** (answering would let anyone probe the bot). Every such drop
still lists the sender under *Turned away* in Settings → Channels and hands the
message to the workflow side, which only matches it: the chat binding it would have fired (the
receiving workspace's first match, as for a real start) and/or the chat
bindings on that chat of a workflow it names explicitly (`Action: Workflow`,
`run <name>:` / `workflow <name>:`). Those rows read *Last attempt rejected:
sender U0123ABC is not on the integration's allowed users*; the daemon logs it
at warn with the user id and channel; and the first such rejection of a
trigger each day (UTC) raises a System notice in the bell — *Workflow trigger
blocked: <workflow>* — that opens the workflow. A message that is no workflow
trigger, or names a workflow that has no binding on that chat, stamps
nothing (an explicit one is still logged at warn).

**Queue in the app.** A queued run's word on the Runs tab is its place —
**Queued · 2nd** — and its run page says "Both run slots are busy" (or "All N
run slots are busy"), with **Cancel run**; the place comes from
`GET /workspaces/{wid}/workflow-runs/queue`.

**Loop guard.** Slack drops any event carrying a `bot_id` (including the nested
`message` of a `message_changed` edit) before it reaches the bridge; Telegram's
`getUpdates` long-poll structurally never returns the bot's own sends — so
neither adapter needs bot-detection logic to avoid the bot triggering itself.
The channel-binding path (only, since it has no keyword to require) adds a
**second, defensive** guard on top: it never treats a message starting with the
bot's own ack prefixes (`"🚀 Started workflow"`, `"⏳ Queued workflow"`; the
dequeue announcement reuses the first) as a binding trigger, so a binding can't
retrigger itself off its own replies.

> **Ordering note.** `implement-feature`/`po-lifecycle`-style templates that need
> a `repo_id` still don't get one from a bare chat message or binding (chat
> carries `msg`/`prompt`/`jira_ticket` only) — run those from the **Run…**
> start form, or have the first node resolve the repo from `working_directory`, same
> as the `Action: Workflow` worked example above.

### Human approval is **not** a trigger
The `human_approval` *node* pauses a run mid-flight (it writes `waiting_approval`
to the run row); the operator resumes via `POST /workflow-runs/{id}/approve`. This
is a node, not a trigger (migration 0058 comment makes this explicit).

---

## 6. Building a workflow (UI walkthrough)

Open **Workflows** → the **Workflows** tab (`#/workflows/all`) → **New workflow**
(a modal; not on a phone). It offers three ways to create a workflow, and each
opens the new workflow in the editor.

### Create
1. **Generate from a description (the primary path).** Type a natural-language
   description ("Ask an agent to summarize the repo, then POST the summary to our
   webhook") and click **Generate workflow** (or ⌘/Ctrl-Enter). This calls
   `POST /workspaces/{wid}/workflows/generate`, which prompts a headless agent
   with the full node catalog and asks for a JSON graph. The result is then:
   - **sanitized** — nodes with unknown kinds are dropped; edges referencing
     missing nodes are dropped;
   - **laid out** (the editor lays every graph out itself, top to bottom);
   - and, if the LLM is unavailable or returns junk, replaced by a **fallback
     graph** of `manual_trigger → agent_prompt` (the agent node pre-seeded with
     your description). So generation **never fails** to produce a runnable graph.
2. **Start blank** — creates `manual_trigger` ("Start") only; build by hand.
3. **From a template** (`POST /workspaces/{wid}/workflows/from-template`,
   `GET /workflows/templates`) — built-in examples in two families:
   - **Orchestrator flows** (`flow_templates`) exercising the wired nodes + control
     flow:
     - **`write-tests`** — *Write tests for a story*: prepare brief → write tests →
       a `fix → review_run` **loop** (`until last.passed == true`, lenses
       `correctness-review`/`test-review`) → **`git_pr` with `open:true`**, reached
       only by an edge gated on `output.satisfied == true` — so the PR **opens
       automatically once the loop passes** (the review is the approval; the old
       `human_approval` node was removed).
     - **`implement-feature`** — `product_analyze` → prepare → implement → the same
       review loop (lenses `correctness-review`/`security-review`) → **auto-opened
       `git_pr`** on pass (same `output.satisfied == true` edge gate; no approval
       node).
     - **`po-lifecycle`** — *PO discovery → RFC/Jira*: discovery draft
       → `canvas` diagram → `human_approval` → persisted `product_rewrite`
       → Product Publish **preview** → Human Approval → **publish approved snapshot**.
       Before running, configure the same `story_id` on Refine and Preview, plus an Atlassian
       `account_id` and Confluence `space_key` (or Jira `project_key`/issue type) on Preview.
       The live node inherits that frozen destination; the review banner shows all content before any publication.
       Other repository templates expect the run **input** to carry `repo_id` (and
       optionally `base`, `story_id`, `goals`) — a Slack `Action: Workflow` message
       or the Run dialog supplies these.
     - **`ui-test-authoring`** — *UI test authoring*: `prepare_context` (app-fetched
       Jira context, if a ticket is referenced) → write UI tests → the same
       `fix → review_run` loop → **auto-opened `git_pr`** on pass → a final
       "Final report" `agent_prompt` step that always runs (reachable from both the
       PR step *and* directly from the loop, so it still runs when the loop didn't
       pass) and becomes the run's `final-output.md`. Ships real **standing
       instructions** (Playwright conventions: no sleep-polling, role/test-id
       selectors, run only the changed specs, treat `jira-<KEY>.md` as the
       requirements source of truth). Bind a Slack channel (chat-kind trigger) or
       run it with a Prompt.
     - **`api-acceptance-test-authoring`** — the same shape targeting the API
       acceptance-test framework's two layers (Gateway APIs for player-behavior
       flows, ServiceLocator for internal service features); its standing
       instructions say so explicitly and forbid duplicating setup helpers.
   - **Game pipelines** (`game-slots` / `game-crash` / `game-scratch`), each chaining
     an agent design step into the game engine + verifier scaffold.

### Edit (`editor/EditorPage.svelte`, `#/workflows/<wf>`)
- The canvas lays the graph out **top to bottom** on every render (no dragging
  steps around, no "Tidy") and fits it on open; **+ / − / Fit** in the zoom pill,
  pinch or ⌘-wheel to zoom, drag the background to pan.
- **+ Step** (header) adds a step after the selected one (else the last one),
  wired in. **Connect** by dragging from a card's bottom port onto another card,
  or with the port's "Connect to…" menu. **Delete** removes the selected step or
  connection.
- **The inspector** (right column, 360 px) is the selected step's form: agent
  steps get the prompt, provider · model · **effort** (the Threads model picker;
  empty = Default), skills, "continue the conversation of an earlier step" and
  a time limit; a review step gets per-reviewer model and effort; the
  deterministic steps (check out a PR, run a script, post review comments, a
  data condition) their own forms. Only steps that run an agent show a model.
- **Save** (⌘S) validates first (`POST /workflows/{id}/validate`; problems show
  in one line above the canvas) and saves with `expected_version`; a save made
  elsewhere meanwhile asks **Reload theirs** or **Overwrite with mine**.
- **⋯** holds Runs of this workflow · Optimize… · Triggers · Instructions ·
  Versions (newest first, **Show older**, **Restore**) · Validate · Rename… ·
  Duplicate · Delete workflow…. Triggers, Instructions and Versions open in the
  inspector column.
- On a **phone** the editor is a read-only outline of the steps; tapping one
  opens its settings in a bottom sheet (**Done** saves). Adding, connecting or
  deleting steps needs a larger screen.

### Triggers
⋯ → **Triggers** opens the `TriggersPanel` in the inspector column: add/enable/disable/delete
schedule, webhook, event, or **chat binding** triggers — all four fire
unattended in the running daemon (see §5; the chat kind is evaluated live by
the channels Bridge on every message, not polled like the other three). The
**Chat binding** form (channel, chat id, optional thread, "@mention only")
maps directly onto the `{channel, chat, thread?, mention_only?}` spec.

### Optimize (`#/workflows/<wf>/optimize`)
**Optimize…** (editor ⋯, a Workflows-tab row's menu, ⌘K) drafts a faster
version of the workflow from rules alone — no agent, no quota — and shows it
before anything is saved: the expected saving from the workflow's own
successful runs (no minutes under 3 runs, or when the draft is not clearly
faster on them — the card says which), what was checked (validation, the
step numbers prompts mention, every file a step reads still written before it,
outward steps in their order), the changes grouped as **Same result, faster** ·
**Faster; the output may read differently** · **May change findings** (ticked as
Otto pre-selects them; holding a review to its declared lenses never is), what
was **Not offered** and why, **Before | After**, and what each step reads and
writes. Ticking a change asks the daemon again for that selection's draft.
**Apply as version N…** confirms and saves a new version (the old one stays in
Versions); only a person can apply (an agent credential gets 403); a save made
meanwhile asks you to **Check again**.

### Goals
A **goal** is a workflow of one fixed shape — **Start → "Work toward the
goal" { Work → Check criteria }** — a budgeted loop that works, checks every
criterion with evidence, and repeats until all are met or the budget ends.
Goal Loops, which used to be a feature of its own, is this now. Start one from
**Run… → A goal…**, ⌘K **New goal…**, Insights' **Hand off as a goal…**, the
a review's **Run agent to fix…** in Git, or an older loop's **Continue as a goal**. The
**New goal** form (`ui/src/modules/workflows/goal/`) takes the **Goal**, a
**Title**, **Done when** (each criterion checked by *A command passes*, *An
agent finds evidence* or *I check it*), **Work in** (*A repository*, from a
branch or the current commit, or *Research — no repository*), the **Budget**
("Up to N iterations and 1h of active time") and the **Models** for **Work**
and **Check**. **Start goal** creates the goal's workflow
(`POST /workspaces/{wid}/goals`) and opens its run page: the goal card
("Criteria 2 of 3 met", each criterion's evidence, "Iteration 2 of 5 · 18m of
1h active"), the loop's iterations as rows under the loop step, the
**Criteria** tab, and — when the run is held — **Resume**, **Extend and
resume…**, or **Record** / **Not met…** for a criterion you check. Goal
workflows fold under **Goals** in the Workflows tab. Loops made before this
release are a read-only history behind **Older goal loops** at the foot of
Runs. The full guide: [Goals](./goal-loops.md).

## 7. Running & monitoring

### Run
- **Run…** (the Runs tab, the editor, a Workflows-tab row, ⌘K "Run workflow…")
  opens the **start form**: fields built from the Start step's declared
  `inputs` (else the keys chat runs use — Pull request, Jira ticket, Branch,
  Base branch, Goals, Message to the agents), the review mode when the graph has
  a review step, **This run will** (every outward step in words) and **Send the
  result to** (Only this page, or a configured chat destination). JSON is behind
  **Input as JSON**, two-way. **Run** validates the graph, starts the run
  (`POST /workflows/{id}/run`) and opens its run page; a refusal stays in the
  form.
- **Run again…** (a finished run's header, a run row's menu) opens the same form
  filled from that run's input; the original chat thread is offered as a
  destination, never preselected.
- **Retry step** (a failed run's primary) re-runs the failed step and what
  follows it; **Retry only this step** (⋯) just the step. A running step's ⋯ has
  **Skip this step**.
- **Stop run…** → `POST /workflow-runs/{id}/cancel`. The cancel is announced to the
  engine, which stops **every running step at once** (its agents are stopped;
  files written so far are kept), marks each `skipped` with class `canceled`
  (**Stopped**) and the remaining steps `skipped` (status `canceled`). A
  queued run has **Cancel run** instead (nothing started, no confirmation).
  A running node reads only the run's status as a backstop (first tick, then
  every 30 s) — never the run body — so a long agent step costs no DB reads
  while it works (perf W1). Live step changes reach the open run page as a
  node summary on `workflow_run_updated` and are applied in place, without a
  `/progress` GET, when the event is the next revision (perf W5).
- **Pause** and **Add a note for the next step…** (a running run's ⋯) — see
  *Needs you, holds and notes* below.

A run executes in a background task (`run_workflow`) and persists progress to the
`workflow_runs` row after **every** node transition.

### Run queue (parallel-run cap)

At most **2** runs execute at once, daemon-wide (`OTTO_WF_MAX_PARALLEL_RUNS`
overrides, minimum 1). A single run can fan out dozens of agents — one
`review_run` node launches the entire reviewer fleet — so uncapped parallel
runs exhausted the daemon's file descriptors and starved interactive work.
Every trigger path (manual run, retry-node, webhook, schedule/event trigger,
chat, scheduled task) shares one FIFO gate (`workflow_engine::spawn_run`):

- A run beyond the cap keeps its `pending` status — shown as **Queued · 2nd** —
  and starts automatically, in creation order, when a slot frees.
- The queue is **persistent** with no extra table: the still-`pending`
  `workflow_runs` row *is* the queue entry. On daemon restart, queued runs
  re-enqueue in creation order (`resume_queued_runs`).
- **Cancel run** on a queued run works: the gate re-checks the run's status
  when a slot frees and a canceled run never starts.
- **A run waiting on a person holds no slot.** While every running step of a
  run waits on an approval, an answer or a usage-limit reset, the run hands
  its slot to the next queued run and takes one back, in line, when a step
  works again.
- **The long lane.** A run whose Start step has `lane: "long"` — every goal —
  waits on its own gate (`OTTO_WF_MAX_LONG_RUNS`, default 2, 1–4), so an
  hours-long goal never blocks a quick review, and the other way round. Queue
  places count within the lane.

This cap is about **runs**. Inside a run, steps run side by side up to
`wf_max_parallel_steps`, and every agent still waits its turn at the
machine's admission gate (§3 *Scheduling*).

### Validation, pinned versions and durable loops

Use **Validate** in the editor to check the working graph. Run also validates it
before execution. Diagnostics identify nodes or edges and highlight them on the
canvas: duplicate ids, missing endpoints, cycles, unsupported kinds, missing
required static parameters and malformed expressions. Incomplete drafts can be
saved. Loop steps are checked too; nested loops remain unsupported.

A run pins its definition when queued. Queue delays, workflow edits, restarts and
step retries all keep that same graph, instructions and restart policy. The run
page's facts line shows the version it ran (`· version 3`). Use a fresh
Run to execute the latest saved graph. A missing pinned snapshot fails the run
instead of silently executing a different definition.

Loop attempts have durable checkpoints (migration **0127**). The run page
shows each iteration as a row under the loop step, with its inner steps,
status, attempts, logs and output. Completed steps retain their output through
recovery, and inner failures use their own retry policy. An interrupted
external action has an unknown outcome and stops for inspection; it is never
automatically repeated. **Retry step** acknowledges that outcome, resets
failed attempts and retains successful inner work. **Retry from here**
intentionally clears the selected loop checkpoints and can repeat completed
actions. Old interrupted loops without checkpoints fail safely. Recovered
handoff files retain their contents and their step numbering.

### Restart resume

A daemon restart (or a computer reboot, or an update) does not cancel
executing runs. Per-node progress is persisted after every step, so on startup
a reconciler (`reconcile_interrupted_runs`) re-enters each interrupted run
instead of failing it:

- **Adoption**: steps that already finished keep their status/output/thread
  record and run scope. The run continues all unfinished nodes, including
  pending sibling branches, reusing its context dir and `otto-wf/<run_id>`
  worktrees (the startup worktree sweep runs after reconciliation, so
  resumable runs keep theirs). Several steps may have been running side by
  side: the run re-enters only when **every** one of them is safe to re-enter.
- **An agent step on a thread goes on in the same conversation.** A thread
  whose agent process survived the restart (the runtime holds agent processes
  across a daemon restart) is simply awaited again from where it was. A
  thread whose turn the restart cut gets one turn — "Otto restarted while you
  were working on this step. Check what you already did and finish it." — on
  the same conversation; a restart that came before the prompt went out sends
  the prompt now. A handoff the agent wrote while the daemon was down counts.
  If the provider cannot resume the conversation the step fails (class
  `provider`) rather than starting an agent that remembers nothing. A step on
  the terminal runner whose handoff file proves it finished is **adopted as
  done**; otherwise it re-runs in a fresh session.
- **Side-effect safety**: a step with external side effects (`git_pr`,
  `channel_notify`, `swarm_task`, `product_publish`, `api_run`,
  `http_request`, `self_improve`, a `script` without `idempotent: true`)
  caught **mid-flight** is never replayed — its outcome is unknown, so it's
  marked `error` and the run fails with a pointer at **Retry step**.
  `git_checkout` (its work repeats harmlessly), `post_review_comments` (every
  comment is claimed before it is posted) and `check` re-run automatically,
  as do pure and read-only kinds.
- **Human approval**: a run paused at a `human_approval` node survives the
  restart — it resumes at the approval node and re-parks, re-awaiting the
  operator.
- **Held runs** (paused, blocked, out of budget) are not running, so a
  restart leaves them as they are; **Resume** works after it.
- **Chat**: the live status message's id is stored on the run, so the
  resumed run goes on editing the same message.
- **Opt-out & caps**: a workflow carries `on_restart: 'resume' | 'fail'`
  (`'fail'` restores the old hard-fail). It is set through the API
  (`PATCH /workflows/{id}` `{"on_restart": "fail"}`); the editor has no
  control for it. Automatic resumes are capped at 2 per
  run (`resume_attempts`) — a restart while the run waits at a human approval
  re-parks it without spending the budget, and a user retry resets it; the run
  page's facts line says *"Resumed after Otto restarted"*. A retry-node
  re-entry persists its scope (`resume_scope_json`) so even a restart
  mid-retry resumes with the same scope. The 10h run time budget is per
  execution (fresh run, retry, restart resume) and excludes time waiting on a
  person; crash-loop resumes are bounded by the resume cap. (Migration
  `0108_workflow_resume`.)

### Upgrading and rolling back

- **A run in flight when you update** is resumed by the new build like any
  restart. Each execution reads the runner switch again, so a step the old
  build was running in a terminal session re-runs on a thread, with the same
  step number and step file name.
- **Runs from before this release** open on the run page as they are: steps
  in order, logs, outputs and files. They have no phases and no thread, so
  they carry an **Older run** chip ("phase timings and conversations are not
  available; logs and outputs are") and a step's Conversation lists the
  terminal sessions it ran in with **Open in Agents** — or, when none of them
  is kept any more, "This step ran in a terminal session that is no longer
  kept."
- **A Goal Loop running at the update** is paused once, with one notice
  ("… was paused by the update …"); its work is kept and **Continue as a
  goal** picks it up ([Goals](./goal-loops.md)).
- **Rolling back to the previous build** is safe for stored data: the three
  migrations of this release (0233–0235) only add columns and an index, so
  the previous build boots on the newer database, lists every workflow and
  opens every run — including runs this build wrote. It ignores what it does
  not know: a run's hold (a paused, blocked or out-of-budget run reads there
  as canceled or failed), the overview, and the retention date on threads (it
  deletes none of them).

### Run repos & branches (`repos[]` input)

Declare which repos/branches/worktrees the run operates on — **source and
destination, multiple entries supported** — right in the run input:

```json
{ "repos": [
  { "repo": "otto_os", "type": "branch",   "name": "feat/x", "source": "develop" },
  { "repo": "koala",   "type": "worktree", "name": "~/wt/koala-fix" }
] }
```

- `repo` — a registered repo's **id, name, or path**.
- `type: "branch"` — `name` is the working branch; the engine finds the
  checkout that has it checked out (`git worktree list`), and errors the entry
  if it's checked out nowhere (never silently reviews the wrong branch).
  `source` is the branch the work diffs against and PRs into.
- `type: "worktree"` — `name` is the worktree path; `source` optional.
- Missing `source` ⇒ the repo's **detected default branch** (`origin/HEAD`,
  then `main`/`master`/`develop`/`trunk` probes). The engine never fabricates
  `main`; an unresolvable base fails with the exact candidates tried instead
  of a raw `git exited 128`.

At run start the entries are normalized, written to the run's `repos.json`,
and seeded into the input (first valid entry fills `working_directory`/`base`/
`repo_id` unless explicitly set). Every git-aware step reads the registry:
`review_run` with several declared entries reviews **all** of them (worst
score gates; per-repo detail under `reviews[]`), and `git_pr` drafts/opens one
PR per entry. Steps that publish a repo reference (a review's resolved base, a
loop's worktree) **merge it back into `repos.json`** as the run progresses.

### Run context files (file-based step handoff)

Every run owns `<data_dir>/workflow-context/<run_id>/` (`otto-workflows/src/context.rs`)
— browsable on the run page under **Files** (the run's Result, and each
step's own tab). The tree and the **Result** read it through run-scoped
routes, `GET /workflow-runs/{id}/context/browse?path=` and
`…/context/read?path=` (paths relative to the context dir; `&download=1`
returns the file's bytes), gated at ws **Viewer** on the run's workspace like
the run itself — not through `/fs/*`, which refuses Otto's data dir to every
non-root caller. An absolute path, `..` or a symlink escaping the dir is 400.
Every write here is best-effort — a failure logs a warning and the run
continues; context files never fail a node:

```
instructions.md                 # workflow.instructions, verbatim — only when non-empty
prompt.md                       # this run's ask, verbatim — only when a prompt exists
run-brief.md                    # mission brief: trigger, mission, repos table, planned steps
repos.json                      # live registry of repos/branches — only when repos declared
prs.json                        # the run's pull requests (from the run input and git_checkout)
jira-<KEY>.md                   # a prepare_context node's fetched ticket (or fetch-failed notice)
step1-gather-info.md            # per-step handoff summary
step1-gather-info.output.json   # the step's raw output (5 MiB cap, truncation marker if hit)
step2-go-gates.log              # a script step's full output (a check: <step>.<criterion id>.log)
step3-review-iter2.md           # loop inner steps, per iteration
attachments/  attachments.json  # files sent with the chat message that started the run
steering.json                   # notes waiting for the next agent step
.threads/<node id>-<attempt>.json  # the newest 50 timeline items of a step's thread (kept after the thread is removed)
final-output.md                 # on a successful run: the deliverable (see Final output below)
```

Steps hand context to each other **through files, not truncated prompt
text**: each agent-backed step is pointed at the directory and told to read,
**in order**, `instructions.md` → `prompt.md` → `run-brief.md` → `repos.json`
→ the prior `step*.md` files it needs (each named only when it exists — a run
with no standing instructions never tells a step to go read a file that isn't
there), then write its own `step{N}-{name}.md` summary before finishing. With
steps running side by side, "prior" means the steps this one **descends
from** (and its own earlier loop iterations) — a sibling's file is not listed,
even when it is already there. If the step doesn't write its file, the engine
writes a full-fidelity fallback — an agent's reply lands **untruncated**,
`review_run` files carry the score breakdown + findings, and failed steps
(including failed loop iterations) leave their error in the trail. A retried
step only trusts a summary written during the **winning** attempt (a stale
file left by an earlier failed attempt is replaced). The inline
`[input data]` excerpt in prompts remains as a quick glance; the files are
the complete channel. Steps that run side by side change `repos.json` through
one guarded update, so neither loses the other's entry.

Chat-triggered runs post each meaningful step's `.md` handoff file to the
thread (success *and* failure, loop iterations included), in both chat styles
— captioned in the live style, attached to the step's line in the per-step
style (§5). Posted files are redacted like `summary.md` and capped at 1 MiB
(the full file always remains in the run's context directory).

### Final output

On a run that finishes `success`, the engine tracks — as steps complete — the
**last step that succeeded and wasn't a bookkeeping/control-only kind**
(`manual_trigger`, `log`, `delay`, `channel_notify`, `budget_gate`,
`human_approval` don't count; everything else does, including a `loop`'s own
`.md` and a `prepare_context` with no agent phase). At run completion, that
step's `.md` handoff is copied to `final-output.md` — the run's actual
deliverable, as opposed to `summary.md`'s per-step bookkeeping. A run whose
only successful steps are all utility kinds (e.g. it errored before any real
work ran) produces no `final-output.md`.

The run page's **Result** (the last row of the steps; a sandboxed `<iframe>`,
for a `success` run that has a `context_dir`) fetches
`final-output.md` once per run (`GET /workflow-runs/{id}/context/read?path=final-output.md`) and renders it — it says "This run wrote no final output." when the
file doesn't exist. Chat/webhook delivery prefers it too: see *Result delivery*
in §5. This is why the `ui-test-authoring`/`api-acceptance-test-authoring`
templates' last step is an explicit "Final report" `agent_prompt` (§6) — it's
what ends up as the deliverable both in the UI panel and in the chat/webhook
attachment.

### Statuses, states and words
```ts
RunStatus  = 'pending' | 'running' | 'success' | 'error' | 'canceled';
NodeStatus = 'pending' | 'running' | 'success' | 'error' | 'skipped';
```
The stored statuses are unchanged. A run is `error` if **any** node errored
("one or more nodes failed"). `pending` means the run is **queued** behind the
parallel-run cap; it flips to `running` the moment a gate slot frees.

What a person reads comes from the run's **overview** (`WorkflowRun.overview`,
computed by `otto_workflows::overview::run_overview` on every change and sent
with every event): `state`, a one-sentence `headline`, `steps_done` /
`steps_total`, who waits (`attention[]`), the queue place, the failed step,
`deliverables`, usage, where the time went, and for a goal its progress and
hold.

| `overview.state` | The run's word | When |
|---|---|---|
| `queued` | **Queued · 2nd** | waiting for a run slot |
| `running` | **Working 12m** | at least one step works |
| `waiting` | **Waiting** | every running step waits for a step slot or the machine |
| `needs_you` | **Approval** · **Question** · **Verify** · **Limited** | a step waits on a person, or on a usage-limit reset (the oldest wait names it) |
| `succeeded` | **Done** | |
| `failed` | **Failed** | |
| `canceled` | **Canceled** | stopped by a person |
| `paused` | **Paused** | held by **Pause** |
| `blocked` | **Blocked** | a goal's loop stopped and needs a person |
| `exhausted` | **Out of budget** | a goal's loop used its iterations or active time |

A step's word: **Queued** · **Waiting 2m** · **Starting** · **Working 4m** ·
**Finishing** · **Approval** / **Question** / **Verify** / **Limited** while
it waits on you; then its duration (a condition shows **Yes** / **No**),
**Failed** · **Timed out** · **Signed out**, **Skipped**, **Stopped** (it was
running when the run was stopped), **Not run**. A reviewer or sub-thread row:
**Queued** · **Working** · **Done** · **Failed** · **Skipped** · **Needs
you**.

### Live progress over WebSocket (`workflow_run_updated`)
The engine emits `Event::WorkflowRunUpdated` on the shared event bus at **every
node transition** (start, phase change, finish, skip, thread or session
start), when a step starts or stops **waiting on a person**, on the
**human-approval pause** and the **approve/reject decision**, on **pause**,
**cancel** and **hold**, and at **run completion**:

```json
{ "type": "workflow_run_updated",
  "workspace_id": "<Id>", "run_id": "<Id>",
  "status": "running|success|error|canceled",
  "node_id": "<node_id | null>",
  "rev": 7, "node": { "node_id": "…", "status": "…" },
  "nodes_done": 2, "nodes_total": 5, "waiting_approval": false,
  "overview": { "rev": 7, "state": "needs_you", "headline": "…" } }
```
- `rev` is the run's monotonic revision (also on `WorkflowRun.rev`); `node` is
  the changed node's summary (the shape `…/progress` serves: no log, input or
  output bodies); `overview` is the run's state, headline and waits (≤ 4 KiB).
- The UI (`events.svelte.ts` → `workflowRunBus.apply()` → `RunSync`)
  **merges the change into the viewed run in place** when the event carries the
  node + the contiguous rev — no network at all. On a rev gap, a run-level
  event, or a missing payload it converges with **one single-flight,
  rev-guarded** read. The viewed run object is **never replaced wholesale**,
  so the open step, its tab and scroll positions all survive every update,
  and stale/out-of-order responses can never regress the view.
- A **2.5s fallback poll** (while the viewed run is non-terminal, uncapped)
  keeps the UI live if the WS connection is unavailable. The Runs tab's
  **Needs you** and **Running** rows and the sidebar badge update in place
  from the event's `overview` (a full refetch only for unknown run ids);
  **Recent** refetches only when a run ends.

### Inspect: Runs and the run page
**Workflows opens on Runs** (`#/workflows`), one reading column:
- **Needs you** — runs waiting on a person (an approval, a question, a
  criterion you check, a usage limit) and goals held **Blocked** or **Out of
  budget**, the oldest wait first. The tab shows how many.
- **Running** — working, waiting and paused runs; queued ones last, by place.
- **Recent** — finished runs, unread failures first, with **Search runs…**
  (the workflow's name, the ask, repository, branch, PR, step names and
  errors), a **Failed** chip and a workflow filter ("Runs of this workflow"
  sets it).

Each row says in words what the run is doing, what it is about (the PR or
ticket, else the branch or the ask) and who started it. A row's menu: **Copy
link**, **Open workflow**, **Run again with these inputs…**, **Stop run…**.
The **Workflows** tab (`#/workflows/all`) lists the definitions with their
last run's word and **New workflow** (describe it and **Generate**, a
template, or **Start blank**); goals fold under **Goals**. **Older goal loops
(n)** at the foot of Runs opens the read-only history.

A run has its own page, `#/workflows/<wf>/runs/<run>` (the address notices,
chat links and **Copy link** hand out):
- **The top**: a ring of the steps with the elapsed time, one sentence — the
  engine's headline — and a sub line that ticks ("Usually about 12m · 2
  waiting", "Waiting 4m · the run continues when you decide", "Otto continues
  it at 14:02", "Both run slots are busy", "Finished at 14:20 · 2m faster
  than usual").
- **One card** for the run's state: the failing step's **own** error; an
  approval ("<step> asks to go ahead", where it goes and who sees it,
  **Approve** / **Deny…**); a question (**Answer in the conversation**); a
  usage limit; a criterion to **Record**; a hold (**Resume**, **Extend and
  resume…**); what was **Delivered** (each outward result: Posted · Partly
  posted · Not posted: why · Waiting for approval · Not confirmed); or a
  goal's criteria.
- **The facts** in names: who started it and where, the PR or branch, the
  ticket, when, which version, notes waiting for the next step, "Resumed
  after Otto restarted".
- **The steps in execution order** with their numbers, words and times;
  steps that ran side by side are grouped under **In parallel**; reviewers,
  the summarizer, sub-threads and a loop's iterations are named rows under
  their step; **Result** is the last row.
- **The header**: the run's word, **Copy link**, **Stop run…** while it runs
  (**Cancel run** while queued) and one primary for its state — **Retry
  step**, **Resume** or **Run again…**. ⋯ holds Open workflow · Run again with
  these inputs… · Retry only this step · Re-review · Pause · Add a note for
  the next step… · Stop run….

A step opens beside the list on a wide screen (880 px or more) and as a
pushed screen on a narrow one (`…/runs/<run>/<node>[/<child>]`). Its header
has "Step 4 of 9", the word, **what ran** (provider · model · effort — the
ones the thread actually used) and a bar of **where the step's time went**
("Queued 3s · Waited 40s — Waiting for a free agent slot … · Working 4m ·
With you 2m · Finishing 5s"). Its tabs depend on the kind:

| Step | Tabs |
|---|---|
| An agent step | **Conversation** (the step's thread, read-only; its approval and question cards are answerable there) · **Output** · **Files** |
| A review step | **Summary** (score, counts, reviewers, **Open review**) · **Summarizer** · **Log** · **Files** |
| A reviewer (a row under a review) | **Conversation** · **Findings** · **Files** |
| Another sub-thread | **Conversation** · **Files** |
| A check, a goal's loop | **Criteria** · **Log** · **Output** |
| An approval | **Preview** · **Log** |
| A step with no agent | **Log** · **Output** |
| The run's **Result** | **Result** (`final-output.md`) · **Criteria** (a goal) · **Files** · **Input** (the run input and the files sent with the run) |

Under a conversation the page says how long it is kept: "Read-only: this
conversation belongs to the run. It is kept for 24 hours after the run ends
(7 days if the run fails)." A step's ⋯: **Retry only this step** (a failed
step of a finished run), **Retry from here** (any other step that ran),
**Copy step output**, **Skip this step** (while it runs). **Maximize
conversation** folds the run column away. While the event socket reconnects
the run's word is **Reconnecting…** and the sub line says when the page last
heard of it.

#### Phases and log lines
On a thread, a step's progress is its **phases** (§3) — the bar on the step —
and the run's words; its log holds the engine's own lines: `⏳ starting
<provider> thread`, `✓ step complete`, `waiting for a step slot (4 of 4 in
use)`, `waiting for <step> to start (run order)`, `thread of <step> was
removed — started a fresh one`, the retry, edge and persist lines.

On the **terminal runner** the turn oracle (§3) narrates itself into the
step's log as it goes, and those lines are kept after the step succeeds. The
vocabulary is fixed:

| Line | Means |
|---|---|
| `⏳ starting claude session` | the CLI is booting (the provider's name varies) |
| `✉ prompt accepted` | the prompt landed in the TUI and was submitted |
| `⚙ working` | a turn is open, nothing delegated |
| `🧩 sub-agents: 2 running · 1 done (Sweep diff chunk aa ✓)` | logged on every count change |
| `⏸ agent idle — confirming completion (20s)` | the 20 s idle confirm — logged once, on entry |
| `📄 handoff file written` | the step's `.md` appeared |
| `📄 handoff written — waiting for the turn to end (up to 15m)` | the handoff is on disk and the agent is finishing its turn, with nothing pending — normal |
| `✓ step complete (handoff + idle turn)` / `✓ step complete (codex task_complete + handoff)` | the oracle accepted the step |
| `⚠ …` | a bounded fallback fired — see the table in §3 and §12 |
| `✗ step made no progress for 5m (agent looks stuck; 0 tasks pending)` | the stall trip |

On both runners `↻ retry 2/5 in 23s (provider overloaded: 529)` means the
step failed and is being retried. A step's log list is capped at **200
entries**; the cap evicts only *phase* lines (the `⏳ ✉ ⚙ 🧩 ⏸ 📄` prefixes),
oldest first — `▶`, `✓`, `⚠`, `↻`, `✗`, retry, edge and persist lines are
never dropped.

#### Live sub-agent view (`NodeRunState.activity`)
While an agent step runs it also carries an `activity` snapshot (phase,
`pending_tasks`, and one entry per sub-agent / background task with
`id`, `description`, `status`, `started_at`, `finished_at`), refreshed at most
every 5 s and capped at 40 entries. The run page shows the current phase as
the running step's second line; threads the agent opened are rows under the
step (`NodeRunState.children`), each with its own conversation.

`activity` is **cleared when the step finishes** — a settled step carries no
`activity` key at all, so the phase line only ever shows on a running step.

### Run → Proof Pack
On completion the run links a **Proof Pack** (`WorkflowRun.proof_pack_id`)
assembling each node's output as evidence (a `log` artifact carrying the node's
pass/fail; each `human_approval` becomes an `approval` artifact recording the
approver). The run also records the workflow `version` it executed
(`WorkflowRun.workflow_version`).

### Human-approval pause
When a run hits a `human_approval` node it pauses (`waiting_approval` +
`approval_node_id` ride both the run row and a `workflow_run_updated` pause
event, so the run moves to **Needs you** and the sidebar badge counts it at once); the run page
leads with the approval card (what asks, where it posts, the preview) with
**Approve** / **Deny…** (→ `POST /workflow-runs/{id}/approve` with
`{node_id, approved}`). Approve resumes the run (records `approved_by`; the
step wakes on the decision, with a 30 s safety re-check); reject errors the
node ("rejected — &lt;note&gt;"). Both decisions re-emit the event so open
views drop the card at once. If no decision arrives within the node's
timeout (24 hours unless `timeout_s` says otherwise), the node errors
("timed out waiting for operator decision"). While it waits — and no other
step of the run is working — the run holds no run slot and its active-time
budget does not run.

A cancel while parked is not a decision: the approval step waits for the run's
cancel to stop it. A `skip` (chat or **Skip this step**) is refused for an
approval step **and** for a loop parked at an approval inside it — skipping it
would let the steps after the gate run unapproved.

### Needs you, holds and notes
- **A step waits on you.** Besides the approval step above, an agent step
  waits when its agent asks to run something that needs approval (the
  provider's own request, or a governed Otto MCP call), asks a question, or a
  `check` reaches a criterion only you can check. The run moves to **Needs
  you** with the word **Approval**, **Question** or **Verify**; Otto raises
  one notification per wait ("Workflow “…” needs you"), and a chat-started
  run posts one new message in its thread (§5). Approve or deny on the card;
  answer a question in the step's conversation; for a criterion write what
  you checked and press **Record**, or **Not met…** with what is still wrong
  (the next iteration reads it as the reason). The step goes on from where it
  was; the answer is kept on the step.
- **Limited.** A usage limit with a known reset parks the step (§3): no
  buttons, "Otto continues it at 14:02, when the limit resets." Nothing to do
  but wait — or **Stop run…**.
- **Pause** (a running run's ⋯) stops the running steps and holds the run:
  its word is **Paused**, the reason "Paused by <you>", and its primary is
  **Resume**. The run's worktrees stay. **Resume** re-enters the run: finished steps and finished loop iterations are kept,
  the steps that were in flight start again.
- **Blocked** and **Out of budget** are a goal's holds ([Goals](./goal-loops.md)):
  **Resume** (after **Add a note first**, if you want to tell it something),
  or **Extend and resume…** with new totals.
- **Add a note for the next step…** (a running run's ⋯; **Add a note first**
  on a held one) queues a note — up to 4,000 characters, at most 20 waiting.
  The next agent step that starts reads every waiting note as a `[steering]`
  section of its prompt, and the note is recorded on that step. The facts
  line shows "1 note waiting for the next step" meanwhile.
- **Stop run…** on a held run drops the hold; the run stays as it ended and
  its worktrees are cleaned up.
- A held run is never pruned with old runs, and stays in the Runs tab until
  it is resumed or stopped.

### On a phone
Workflows is one of the six modules of the [phone's light version](./phone-light-version.md)
and works in the full app on a phone too:
- **Runs** is the same inbox in one column; the search and filters sit under
  **Recent**.
- **A run page** is one column: the header keeps the title, the state's one
  primary and ⋯; a step opens as a pushed screen with **Back to the run**;
  the cards' buttons are full-width. **Extend and resume** and the start form
  are bottom sheets.
- **Run…** and **A goal…** work as on the desktop.
- **The editor** is a read-only outline of the steps ("The graph is edited on
  a larger screen. Tap a step to change its settings."); a step's settings
  open in a bottom sheet (**Done** saves). **New workflow** needs a larger
  screen.

---

## 8. API-client "Automations" (the collection runner)

A **completely separate** feature inside the **API client** (`#/api` →
"Automations", `AutomationsView.svelte`). It is **not** the workflow engine and
shares no code with it. Think of it as a lightweight request-sequence / smoke-test
runner.

```ts
interface ApiAutomation { id; workspace_id; name; steps: ApiAutomationStep[]; created_at }
interface ApiAutomationStep { request_id; assertions: ApiAssertion[]; extract: ApiExtract[] }
interface ApiAssertion { kind: 'status'|'json_path'|'duration_ms'; op: 'eq'|'ne'|'contains'|'lt'|'gt'; path?; value }
interface ApiExtract   { path; var }   // JSON path → env var for later steps
```

- An automation is an **ordered list of steps**, each pointing at a **saved
  API-client request** (`request_id`).
- Per step you can add **assertions** (e.g. `status eq 200`, `json_path $.data.id
  contains 42`, `duration_ms lt 500`) and **extracts** (pull a value at a JSON
  path into a `{{var}}` for later steps).
- **Run** → `POST /workspaces/{wid}/api-client/automations/{id}/run`. Each step's
  request executes through the API-client `/execute` engine (so the active
  environment, variables, auth, and cookies all apply). The result is an
  `ApiRunResult`: a per-step pass/fail report (`passed`, status, duration,
  per-assertion `desc`/`passed`, error). A step with **no assertions passes on a
  2xx response**.
- **There is no scheduler for these** — they run **only when you click Run** (or
  call the run endpoint). The api.md "automations" rows (`CreateAutomationReq`,
  `.../automations/{id}/run`) are exactly this surface.

See `./api-client.md` for the request/environment/execution model these steps
build on.

---

## 9. API & contract reference

`docs/contracts/api.md` is authoritative. Workflow-engine routes (api.md
§"Workflow engine"):

| Method & path | Auth | Notes |
|---|---|---|
| `GET /workflows/node-types` | member | node-type catalog (`NodeTypeSpec[]`) |
| `GET /workflows/templates` | member | built-in templates |
| `GET /workspaces/{wid}/workflows` | ws viewer | `Workflow[]` |
| `POST /workspaces/{wid}/workflows` | ws editor | create (`CreateWorkflowReq {name, description?, instructions?, graph?}`) |
| `POST /workspaces/{wid}/workflows/from-template` | ws editor | instantiate a template (inherits the template's `instructions`) |
| `POST /workspaces/{wid}/workflows/generate` | ws editor | AI-generate from `{description, name?}` (`instructions` always `""`) |
| `GET /workflows/{id}` | ws viewer | one workflow |
| `PATCH /workflows/{id}` | ws editor | update (e.g. `{graph}` and/or `{instructions}` — either bumps `version`) |
| `DELETE /workflows/{id}` | ws editor | 204; 409 while a run is pending/running. Pauses the scheduled tasks that run this workflow and records *"workflow deleted – task paused"* on each (see scheduled-tasks.md) |
| `POST /workflows/{id}/run` | ws editor | `RunWorkflowReq?` `{input?, start_node?, only_node?, review_mode?}` → `WorkflowRun`. `review_mode` (`fan_out`/`orchestrator`) is seeded into `input.review_mode` and overrides every `review_run` node of the run (§4 *Execution mode*); any other value, or a non-object `input` alongside it, is a **400** |
| `GET /workflows/{id}/runs` | ws viewer | `WorkflowRun[]` |
| `GET /workflow-runs/{id}` | ws viewer | one run (poll/refresh target) |
| `GET /workflow-runs/{id}/context/browse?path=` | ws viewer (the run's workspace) | `FsBrowse` of the run's context dir; `path` and entry paths are relative to it (empty = its root). 400 absolute / `..` / escaping symlink; 404 unknown run or path |
| `GET /workflow-runs/{id}/context/read?path=` | ws viewer (the run's workspace) | `FsRead` of one context file (~400 KiB bound); the deliverable is `?path=final-output.md` |
| `POST /workflow-runs/{id}/cancel` | ws editor | cancel a run; on a held run it drops the hold |
| `POST /workflow-runs/{id}/retry-node` | ws editor | `{node_id, include_downstream?}` — re-enter a finished run in place (§3) |
| `POST /workflow-runs/{id}/nodes/{node_id}/skip` | ws editor | skip that one running step; 409 when it is not running or is the approval gate |
| `POST /workflow-runs/{id}/pause` · `…/resume` | ws editor | hold a run / re-enter a held one (`{extend?: {iterations?, active_s?}}` = a goal's new totals) |
| `POST /workflow-runs/{id}/steer` | ws editor, a person's credential | `{note}` — queue a note for the next agent step |
| `POST /workflow-runs/{id}/nodes/{node_id}/criteria/{criterion_id}/verify` | ws editor, a person's credential | `{met, evidence}` — record a criterion you check |
| `GET /workflow-runs/{id}/progress?after_rev=` · `…/nodes/{node_id}` | ws viewer | the lightweight run (with `overview`, no bodies) / one step in full (output, logs, the thread's digest) |
| `GET /workspaces/{wid}/workflow-runs` · `…/active` · `…/queue` | ws viewer | run history with search (`q`, `status`, `workflow_id`, `attention=1`) / runs in flight and held runs / queue places |
| `POST /workflows/{id}/validate` | ws viewer | `{graph?}` → `{valid, issues}` (each issue `save` · `run` · `warning`); no execution |
| `POST /workflows/{id}/optimize` · `…/optimize/apply` | ws viewer · ws editor on a person's credential | the Optimize draft / apply it as a new version (409 when the workflow changed meanwhile) |
| `POST /workspaces/{wid}/goals` · `GET …/goals` | ws editor · ws viewer | start a goal / list goal runs and older loops ([Goals](./goal-loops.md)) |
| `GET /workflows/{id}/versions` | ws viewer | `WorkflowVersion[]` (snapshot history, newest first) |
| `GET /workflows/{id}/versions/{v}` | ws viewer | one snapshot (404 if unknown) |
| `POST /workflows/{id}/versions/{v}/restore` | ws editor | `{note?}` → copies `v`'s graph in as a **new** version |

Engine settings (`PUT /api/v1/settings`, the root user on a person's
credential; the first four are range-checked and a bad value is refused) and
environment:

| Key | Default | What it does |
|---|---|---|
| `wf_step_runner` (env `OTTO_WF_STEP_RUNNER` wins) | `threads` | `threads` or `pty` — how agent steps and review agents run (§3) |
| `wf_max_parallel_steps` | `4` (1–8) | steps of one run that hold an agent or a process at once; `1` = one at a time |
| `workflow_thread_retention_success_hours` | `24` | how long a successful run's threads are kept after it ends; `0` = a step's thread goes when the step succeeds |
| `workflow_thread_retention_failed_hours` | `168` | the same for a failed, canceled or held run |
| `wf_step_stall_secs` | `300` | the terminal runner's no-progress trip; `0` disables |
| env `OTTO_WF_MAX_PARALLEL_RUNS` | `2` | runs executing at once, daemon-wide (read at start) |
| env `OTTO_WF_MAX_LONG_RUNS` | `2` (1–4) | the same for the long lane (goals) |

Plus, on the Scheduled-Tasks surface: `POST /scheduled-tasks/{id}/convert-to-workflow`
(`{disable_task?}` → `{workflow_id, trigger_id?}`) materializes a task as a Workflow
+ schedule trigger — see `./scheduled-tasks.md`.

Trigger / webhook / approval routes (api.md Wave-3 additions):

| Method & path | Auth | Notes |
|---|---|---|
| `POST /workflows/{id}/webhook/{token}` | **public-by-token** | run input = body; token matched against `workflow_triggers`; returns `{run_id}` |
| `GET /workflows/{id}/triggers` | ws viewer (Workflows:View) | `WorkflowTrigger[]` |
| `POST /workflows/{id}/triggers/preview` | ws viewer | `{kind, spec}` → `{kind, next_fire_times}`; no save/fire |
| `POST /workflows/{id}/triggers` | ws editor (Workflows:Edit) | `UpsertTriggerReq {kind, spec}` |
| `PATCH /workflow-triggers/{id}` | ws editor (Workflows:Edit) | toggle/enable, update spec |
| `DELETE /workflow-triggers/{id}` | ws editor (Workflows:Edit) | 204 |
| `POST /workflow-runs/{id}/approve` | ws editor (Workflows:Edit) | `{node_id, approved}` → resumed run status |

API-client automations (api.md §"API client"):

| Method & path | Auth |
|---|---|
| `GET /workspaces/{wid}/api-client/automations` | ws viewer |
| `POST /workspaces/{wid}/api-client/automations` | ws editor |
| `PATCH /workspaces/{wid}/api-client/automations/{id}` | ws editor |
| `DELETE /workspaces/{wid}/api-client/automations/{id}` | ws editor |
| `POST /workspaces/{wid}/api-client/automations/{id}/run` | ws editor |

Cross-module search (api.md Wave-4): `GET /workspaces/{id}/search?q=`
returns ranked `SearchHit[]` **across modules including workflows** (alongside
stories, api-requests, swarm, memories, repos, broker-clusters). This lets you
find a workflow by name/description from the global search; it is a discovery
aid, **not** a trigger — searching does not start a run.

WebSocket: `docs/contracts/ws.md` → **Workflow run progress** documents the
`workflow_run_updated` event (see §7). API-client automation runs return their
report synchronously from the run endpoint and have no dedicated WS event.

### Persistence (SQLite, `crates/otto-state/migrations/`)
- **0020** `workflows` (id, workspace_id, name, description, `graph_json`,
  created_by/at, updated_at) + `workflow_runs` (id, workflow_id, workspace_id,
  status CHECK, `input_json`, `nodes_json`, error, started_at, finished_at).
- **0051** `workflow_node_cache` — **historical**: the per-node output cache was
  removed (§3) and nothing reads or writes this table any more. The file stays
  because migrations are append-only.
- **0058** `workflow_triggers` (kind CHECK in `schedule|webhook|event`,
  `spec_json`, enabled) **+ five ALTER columns** on `workflow_runs`:
  `waiting_approval`, `approval_node_id`, `approved_by`, `approval_note`,
  `approved_at` (human-approval pause/resume is tracked on the run row, not a
  trigger).
- **0089** workflow versioning + run→proof link: `workflows.version` (default 1),
  `workflow_runs.workflow_version` + `workflow_runs.proof_pack_id`, and a
  `workflow_versions` history table (`id, workflow_id, version, name, description,
  graph_json, note, created_by, created_at`; `UNIQUE(workflow_id, version)`). The
  migration backfills a `v1` snapshot for every pre-existing workflow.
- **0096** `workflows.instructions` + `workflow_versions.instructions` (both
  `TEXT NOT NULL DEFAULT ''`) — the standing-instructions field (§3).
- **0097** widens `workflow_triggers.kind` CHECK to `schedule|webhook|event|chat`
  (table rebuild — SQLite can't `ALTER` a CHECK) so `chat`-kind channel
  bindings can be persisted.
- **0108** restart resume (`resume_attempts`, `resume_scope_json`, `on_restart`);
  **0127** durable loop checkpoints.
- **0233** `workflow_runs.overview_json`, `attention`, `hold_json`,
  `chat_status_json` (the live status message's id) + a partial index for the
  Needs-you list; **0234** `workflows.origin` / `origin_ref` (a goal and where
  it came from), `workflow_runs.limits_json` (a resumed goal's new totals) and
  `workflow_runs.lane`; **0235** an index on
  `agent_runtime_sessions(parent_id)`. All three only add (§7 *Upgrading and
  rolling back*).
- **0014/0015** `api_client` base + `api_automations` (name, `steps_json`).
- `retry` and edge `condition` need **no migration** — they live inside
  `graph_json`. Run context files (§7) are plain files under
  `<data_dir>/workflow-context/`, not a table.

All ids are ULID strings; timestamps are UTC RFC-3339; rows cascade-delete with
their workspace (and triggers/runs/cache cascade with the workflow). Migrations
are **append-only** — never edit or renumber an existing one.

---

## 10. Capabilities & limitations (be explicit)

**What works today**
- Build graphs by description (AI), template (orchestrator flows + game pipelines),
  or hand; an auto-laid-out canvas editor; a rule-based **Optimize** draft.
- Execution in run order with **independent steps side by side** (step
  numbers as in a one-at-a-time run), failure propagation, partial runs
  (from-here / only), run cancellation, skipping one running step, and a
  global active-time limit.
- **Agent steps on threads** — the structured runtime by default, the
  terminal runner behind a switch; a step finishes when its thread has
  settled (turn ended, nothing it started still running, no request pending),
  with one nudge for a missing handoff file; **provider · model · effort**
  per step, an optional time limit, and continuing an earlier step's thread.
- **Steps that need no agent:** `git_checkout`, `script`,
  `post_review_comments`, `check`, and conditions on the run's repositories.
- **Branching & loops:** edge `condition`s + a `condition` node (if/else, with clean
  branch-skip vs failure-poison semantics), and a bounded `loop` (iterate-until,
  optionally with an active-time budget) that reuses inner-node execution —
  all driven by the safe `otto_core::expr` language (also available as
  `{{ }}` templating).
- **Goals:** a budgeted work → check loop with criteria checked by a command,
  an agent, a rule or a person; Pause, Resume, Extend and notes for the next
  step ([Goals](./goal-loops.md)).
- **Needs you:** approvals, questions and criteria wait on the run page, in a
  notification and in the chat thread; a usage limit with a known reset parks
  the step and continues it at the reset.
- **Retry/backoff** per node (`retry`), and **typed outputs** (warn-only validation
  against each kind's `output_schema`).
- **All node kinds are real:** `manual_trigger`, `agent_prompt`, `prepare_context`
  (app-side Jira fetch + optional analysis agent), `http_request`,
  `transform`, `delay`, `log`, `db_query` (read-only), `broker_peek`,
  `channel_notify`, `budget_gate`, `human_approval`, `condition`, `loop`,
  `swarm_task`, `api_run`, `product_analyze`, `product_rewrite`,
  `product_plan`, `product_publish`, `review_run` (adaptive PR-review engine,
  0–100 scored, `require_pass`), `canvas`, `git_pr` (PR draft, or `open:true`
  to auto-open on a passing review), `self_improve`, `git_checkout`,
  `script`, `post_review_comments`, `check`.
- **A conversation per agent step** (readable while it runs, read-only; a
  summary after the thread is removed) and a **Proof Pack** linked to each
  completed run.
- **Proportionate review** — scope assessment selects relevant lenses; bounded
  fan-out runs one provider per lens by default. Cross-provider comparison is
  explicit; legacy execution-mode fields remain compatible (§4).
- **Versioning:** graph snapshot history with view + restore (append-only),
  covering **standing `instructions`** alongside the graph.
- A **Runs** inbox (Needs you · Running · Recent, with search) and a page per
  run, live over WS, with per-step conversation/output/log/files and the
  run's **Result** (its `final-output.md` deliverable on success).
- **All trigger kinds fire in the running daemon** — webhook, event, and schedule
  (spawned at boot; cron + IANA timezone via the shared cadence engine), plus a
  **`chat`-kind** channel binding evaluated live on every inbound message.
- Three ways to start a run by chat: the structured `Action: Workflow` message,
  the simplified **`run <name>: <prompt>`** command, or a channel binding — the
  run keeps **one live status message** in the thread (or a message per step,
  `chat_updates: per_step`), posts every step's file, then delivers
  `final-output.md` (or `summary.md` if the run made none). Files attached to
  the starting Slack message reach the run and are kept with it.
  **Convert** a scheduled task into a workflow + schedule trigger.
- **Per-step skills** (`skill`/`skills` on an `agent_prompt` step inline a skill body
  ahead of the prompt) and **threshold-gated auto-PR** (`review_run` →
  `git_pr open:true` via an `output.satisfied == true` edge — the review is the
  approval, no human gate).
- Human-approval pause/resume; **restart resume** (a step on a thread goes on
  in the same conversation); usable on a phone.
- API-client automations: ordered request runner with assertions + extracts.

**Limits (still honest about the edges)**
- **The runner switch has no control in the app.** `wf_step_runner` is set
  through the settings API or the daemon's environment (§3), and a run in
  flight keeps the runner it started with.
- **A thread is kept for a time, not for good.** 24 hours after a successful
  run and 7 days after a failed one (both settings), then the conversation is
  removed and the run keeps a summary. Copy what you need, or raise the
  settings. `keep_session` ("Keep this step's thread open after it finishes")
  only affects the terminal runner: a thread step's thread is closed when the
  step settles.
- **A step's time limit is not the end of it.** A step past its `timeout_s`
  fails as **Timed out**, and its retry policy then applies (an
  `agent_prompt` retries twice by default, each attempt with the full limit).
  Set the step's retries to 0 for a hard stop.
- **Answering from chat is not possible.** Approvals, answers, a criterion's
  record, Pause, Resume and notes are given in Otto; the thread accepts
  `status`, `skip`, `abort`, `re-review` and `help` only. Telegram receives no
  files.
- **At most 4 sub-threads** of one step are open at once, and an engine
  thread cannot request a secret.
- **Step slots and the machine gate bound parallelism**, not the graph: four
  steps per run by default, two runs at once, and every agent still waits
  for a machine place — a wide fork on a loaded Mac runs partly one after
  another, and says so on each waiting step.
- **Two steps with no path between them may run together.** Order outward
  steps with an edge.
- **`game_engine` / `verifier` are scaffolds** — real, runnable, useful for the
  game-pipeline templates, but they emit canned specs / scaffold reports awaiting
  a real external game engine + certifier (see footnote ¹ in §4).
- **`product_publish` defaults to a preview** — configure the story, account and destination even
  for previews. Live publishing requires Preview → Human Approval → Product Publish (`dry_run:false`)
  in the same run, using the approved snapshot. A copied review identity or toggling `dry_run` alone
  cannot authorize it. To migrate older graphs, insert a destination-configured preview before the
  publication gate and leave matching live fields or create a new live node with only `dry_run:false`.
  Source/destination changes require a new preview and review; see [publication contract](../contracts/api.md#workflow-publication-migration).
- **`git_pr` defaults to draft-only** (`opened:false`) — set **`open:true`** (gate it
  on the incoming edge so it only fires when the review passed) to actually open the
  PR on the remote; the `write-tests`/`implement-feature` templates do exactly this
  on a passing review→fix loop.
- **Wired nodes have prerequisites** — `db_query`/`broker_peek`/`swarm_task`/
  `review_run`/`git_pr`/`product_*` need their backing connection/cluster/swarm/repo/
  story set up, or the node errors (and downstream active-path nodes skip).
  `git_checkout` and `post_review_comments` need a git account bound to the
  repository; a `script` needs its tools on your login shell's `PATH`.
- **Typed-output validation is warn-only** — schema mismatches log `⚠` but never
  fail a run; a kind's `params_schema` is a UI hint and never gates a run.
- API-client automations have **no scheduler** — run-on-demand only.
- **Terminal runner only:** a silent *grandchild* can still slip past the idle
  confirm (a sub-agent's own sub-agent is recorded only in the child's
  transcript; any movement of the flat `subagents/` files resets the 20 s
  idle confirm, which is the only guard), and agy / custom providers must
  write the final handoff file — silence alone never completes a step there.

---

## 11. Security & permissions

- **RBAC (per-workspace `Workflows` feature grant):** node-types and templates
  are **member**-readable; listing/reading workflows + runs + triggers is **ws
  viewer** (`Workflows:View`); creating/editing/deleting/running workflows, and
  all trigger mutations + run approval, are **ws editor** (`Workflows:Edit`).
  Runs resolve the workspace from the workflow/run row. (See `./daemon-http-api.md`
  and the multi-user RBAC doc for how feature grants and roles compose.)
- **Run identity.** A manual run acts as the person who started it. A trigger,
  schedule, webhook or chat run acts as whichever is LATER of: the author of the
  graph version the run is pinned to (`workflow_versions.created_by` — the last
  person to change that graph) and the `updated_by` of any of the workflow's
  triggers (set to the caller when a trigger is created or its spec changes, with
  the time in `updated_by_at`; migration 0189). The firing trigger is not stored
  on the run, so every trigger counts; that can only pick a more recent changer,
  never an older one. It acts as the
  workflow's creator only when both are unset or are the creator. So an Editor who
  edits someone else's workflow, or adds a trigger to it, cannot make later
  unattended runs use the creator's private connections or root authority — those
  runs act as the Editor.
- **Webhook triggers are public-by-token.** `POST /workflows/{id}/webhook/{token}`
  requires **no bearer auth** — the 32-byte URL-safe token in the path *is* the
  credential, matched against an enabled webhook trigger. Treat the URL as a
  secret; delete the trigger to revoke. Anyone with the URL can start runs (with
  attacker-controlled JSON input).
- **Chat-started runs (`Action: Workflow`, `run <name>: <prompt>`, or a channel
  binding) run on the channel-trust model.** A message in a configured
  Slack/Telegram/webhook channel can start a run by name (the run acts as the
  workflow's latest changer — see *Run identity* above — falling back to a
  synthetic "Workflow" user for system-initiated runs). Anyone who can post to a wired channel can start any
  workflow in that workspace by name — treat channel access as run-start
  capability.
- **`prepare_context` fetches through the run's resolved Jira account** —
  `params.account_id`, else the run user's own account, else any Jira account
  configured on the daemon (§4). The fetched ticket (description, comments,
  attachments list) lands in the run's context dir and, on a chat-started run,
  can end up in the chat thread via a step's `.md` attachment or
  `final-output.md` — treat a bound channel's audience as the ticket's
  effective audience for that run.
- **Engine threads.** A step's thread runs as the run's user in the run's
  working directory, like the terminal session it replaced, and is read from
  the run page by whoever may read that thread (someone who may not sees "You
  can't open this conversation" and the kept summary). It cannot request a
  secret (`otto.request_secret` is refused at once); a governed Otto MCP call
  it makes waits for a person's approval as a person's session would; an
  untrusted read-only reviewer thread gets no MCP servers at all. Sub-threads
  inherit the step's identity and limits.
- **`script` runs your command, confined.** `/bin/sh -c` under the same
  Seatbelt policy as a scheduled shell task, with the daemon's environment
  scrubbed. Run values (branch names, repository names from a chat message)
  reach it only as `OTTO_*` environment variables — never interpolated into
  the command — and file names it is given must be bare names in the run
  folder. `git_checkout` checks every branch name from the input before git
  sees it and passes refs as full `refs/…` names.
- **Only a person changes a graph or steers a run.** Applying an Optimize
  draft, queueing a note (`/steer`) and recording a criterion (`/verify`)
  need a person's credential — an agent credential gets 403. From MCP,
  `otto.control_workflow_run` (pause / resume / extend) is off by default and
  approval-gated ([MCP control plane](./mcp-control-plane.md)).
- **Reserved run-input keys.** `attachments` and a non-boolean
  `research_dir` are set by the engine only and dropped from every trigger's
  input; `chat_updates` is dropped from a webhook body. Files sent with a
  chat message are copied under sanitised names, never through a symlink, and
  served for download as attachments (`nosniff`).
- **`db_query` is read-only by construction** — the engine forces
  `confirm_write = false`, so a workflow can never silently issue DB writes (a
  graph that genuinely needs a write must set the param explicitly).
- **`broker_peek` is consume-only** (peek, not produce).
- **`budget_gate`** lets you hard-stop a run when a provider spend cap is
  exceeded (errors the run if `blocked`).
- **`human_approval`** inserts an explicit human-in-the-loop checkpoint; the
  decision and approver are recorded on the run row.
- The daemon listens on **loopback only** by default; webhook URLs are only
  reachable externally if you deliberately enable a network listener / tunnel
  (see the remote-access runbook). Do not expose webhook tokens over untrusted
  channels.
- **Secrets:** node params are stored as plain JSON in `graph_json`. **Do not put
  raw secrets in node params** (e.g. an `http_request` Authorization header) — use
  `api_run` (which applies API-client env vars / auth) or a `channel_notify`
  integration whose credentials live in the Keychain.

---

## 12. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| A `product_*` / `review_run` node errors "missing story_id / repo_id" | These are **wired** now (§4) and need their target: a `story_id` (product) or `repo_id` (review/PR) in the node params or the run input. Provide it (the Run dialog / `Action: Workflow` message / template input). |
| A `review_run` node "passes" too easily / never passes | `score = 100 − 20×must-fix − 5×should-fix (optional findings deduct 0 by default)` (optionally blended with a goals score), `passed` needs `score ≥ threshold` **and** complete reviewer coverage, a nonempty diff, and no open must-fix findings. Tune `threshold` (default 80) or check the finding counts in the node output. |
| A Slack/Telegram workflow trigger does nothing — no reply, no run | Almost always the integration's `allowed_users` list: the sender is not on it, and the bridge drops the message silently before any parsing (by design; it never answers strangers). The binding's Triggers row reads *Last attempt rejected: sender … is not on the integration's allowed users* and the bell shows *Workflow trigger blocked*; add the sender's id in Settings → Channels (the rejected sender is listed there with an **Allow** action). A trigger with no binding on that chat leaves only a warn log line. |
| A scheduled run never starts | The schedule scheduler **is** spawned at boot now (§5). Check the trigger is **enabled**, the cadence/`timezone` is right, and `last_run` shows it isn't mid-window; for cron, validate the `expr`. |
| A step says **Waiting 6m** — "Waiting for a free agent slot — 5 of 5 running" / "Waiting for the machine — load …" / "Waiting for memory — …" | The machine's agent admission gate (§3 *Scheduling*): other background agents hold the places, or the Mac is under load or memory pressure. The step starts by itself; its phase bar keeps how long it waited. Fewer agents at once (a lower `wf_max_parallel_steps`, fewer reviewers) shortens the wait for the rest. |
| A step says **Queued** — log `waiting for a step slot (4 of 4 in use)` | Four steps of this run already hold an agent or a process (`wf_max_parallel_steps`). It starts when one settles. |
| The run says **Limited** — "Otto continues it at 14:02, when the limit resets." | The provider's plan limit stopped a step and the reset time is known: the step is parked and continues on the same thread at the reset (§3). Nothing to do. |
| A step failed with `usage_limit` | The plan limit hit and no reset time was known (or it came straight back). **Retry step** when the limit has reset, or give the step another provider. |
| A step is **Timed out** | It passed its own time limit (`timeout_s`), waiting on a person included. Its retry policy applies; raise the limit, or answer sooner. |
| A step is **Signed out** | The provider's CLI is not signed in (class `auth`). Sign in on the Mac, then **Retry step**. |
| "The conversation was removed after the run ended. …" | Retention deleted the step's thread (24 h after a successful run, 7 days after a failed one). The answer, files, usage and a summary are kept. Raise `workflow_thread_retention_*_hours` to keep conversations longer. |
| Step log: `thread of <step> was removed — started a fresh one` | A `continue_from` step ran after retention removed the earlier step's thread (a retry days later). It ran on a new thread with the step files as its context. |
| A step failed: "could not resume the conversation of the earlier step" | The provider could not resume the thread this step continues. **Retry from here** on the earlier step so both run again, or switch the step to **New thread**. |
| An agent reports "a workflow step may have 4 open sub-threads at once" | The step's agent tried to open a fifth sub-thread. It closes one (`otto.thread_close`) or does that part itself; raise nothing — the cap is fixed. |
| A step failed: "background work was still running at the limit" | The agent ended its turn with background work that ran past 45 minutes. Have the step stop what it starts, or run that work as a `script` step. |
| A `git_checkout` step left a repository unresolved | It never guesses: no open PR (or more than one) matched the `pr` / `pr_branch` input or the Jira key. The candidates are in the step file and `repos.json`; start the run with `pr` set. |
| A `script` step: "exit code N" / "timed out after 900 s" | The command failed or ran past its time limit. The last lines are in the error, everything in `<step>.log` under **Files**. `command not found` usually means the tool is not on your login shell's `PATH`. |
| `post_review_comments` posted nothing | The review was local (no PR in `prs.json`), incomplete, or every finding was already on the PR. The step's output and the **Delivered** card say which. |
| The chat thread shows one message that keeps changing, not a line per step | That is the default (**One live message, plus each step's file**). Pick **A message and a file per step, as before** on the Start step for the earlier style. |
| *(terminal runner)* Step log: `⚠ handoff file missing — accepted the agent's final reply after 90s idle` | The agent ended its turn but never wrote the step's `.md` handoff (§3), so the engine accepted its final reply after the 90 s grace (a nudge was sent at +10 s). Tighten the step prompt to write the handoff, or accept the reply. |
| *(terminal runner)* Step log: `⚠ handoff written but 3 tasks still pending — waiting (up to 15m)` | The agent wrote its handoff **while** sub-agents / background tasks it launched were still running — against the step protocol (§3). The engine waits up to 15 min for them rather than cutting them off. Fix the prompt so the handoff is written last. |
| *(terminal runner)* Step log: `⚠ handoff written; 1 sub-agent never reported after 15m — moving on` | The 15 min handoff linger expired: a sub-agent never delivered its `<task-notification>`. The step completes on the handoff and its session is stopped. Check that sub-agent's transcript under `subagents/` — it was probably killed or is genuinely stuck. |
| *(terminal runner)* Step log: `⚠ background task b5gvqf675 never finished after 15m — moving on` | The parent went idle with only a `run_in_background` bash left (a dev server, a `--watch`, a `tail`). The engine completes the step after 15 min and suspends the session, which hangs up the task. Add "stop every background process you started" work to the step, or don't background it. |
| *(terminal runner)* agy / custom step is idle without a final handoff | The step remains active until it writes its handoff or reaches its stall/absolute deadline. Silence is not evidence that the work finished (§10). |
| *(terminal runner)* Step log: `↻ retry 2/3 in 2s (codex turn aborted)`, then `✗ codex turn aborted` | codex wrote a `turn_aborted` for the turn we submitted and started no new one within 90 s (usually a dropped upstream connection or a cancelled TUI). The node's retry policy re-runs the step; with no retries left the step errors. |
| Step log: `↻ retry 2/5 in 23s (provider overloaded: 529)` | An overload / rate-limit / fd-exhaustion / spawn error — the engine forces a ≥ 20 s backoff (plus jitter) and up to 5 attempts when retries are enabled (§3). Nothing to do but let it retry; repeated hits mean too many concurrent agents. |
| *(terminal runner)* Step log: `✗ step made no progress for 5m (agent looks stuck; 0 tasks pending)` | The stall trip fired (§3): no transcript, sub-agent or `tasks/*.output` movement for `wf_step_stall_secs`. The step is retried in a fresh session. Raise the setting for genuinely long quiet work, or `0` to disable. |
| A review launches fewer agents than the configured lens/provider combinations | Expected: scope assessment selects applicable lenses and uses one provider per lens by default. Inspect **Review scope**. Enable cross-provider comparison only when the added comparison is useful. |
| A branch I expected to run was `skipped (branch not taken)` | An incoming edge's `condition` evaluated false (or its upstream was branch-skipped). Inspect the `edge → … not taken` log line and the source node's output the condition tested. |
| Run fails immediately, no node ran | The graph has a **cycle** (topo-sort failed) — remove the back-edge. |
| A downstream node is `skipped` ("upstream did not succeed") | A predecessor errored or was skipped; fix/inspect the upstream node first. |
| `db_query` errors "missing connection_id" / connection not found | The `connection_id` must be a saved Database-Explorer connection id; create it there first (`./connections-ssh-sftp.md`). |
| `broker_peek` / `swarm_task` errors | The referenced cluster / swarm + project must exist and be set up. |
| `channel_notify` does nothing | No enabled Slack/Telegram integration, or the selected `channel` isn't configured (`./channels-slack-telegram.md`). |
| Run UI stops updating but isn't finished | WS dropped → the uncapped 2.5s fallback poll keeps the viewed run converging; the run still runs server-side. If the view is genuinely frozen, re-open the run or reload. |
| Webhook returns 401 | Token doesn't match an **enabled** webhook trigger on that workflow id. Re-check the token / re-enable the trigger. |
| Run "exceeded the 10-hour time limit" (running step: "workflow active time budget exhausted") | The global wall-clock fired; the graph (often an `agent_prompt` chain) is too long. Split it or reduce work. |
| **Approve** / **Deny…** does nothing | The run must actually wait at that step, and you need `Workflows:Edit`. A publish gate's **Approve** stays off until its preview has loaded ("Load and review the publication preview before approving."). |
| API-client automation step fails on a 2xx | It has a failing assertion. With **no** assertions a step passes on any 2xx; check the assertion `desc` in the report. |
| `prepare_context` node errors "required Jira fetch failed" | `params.require: true` and the fetch failed (no matching Jira key, no configured Jira account, or the API call errored) — check `jira-<KEY>.md` in the run's context dir for the reason, or drop `require`. |
| A run finished but its **Result** says "This run wrote no final output." | The run isn't `success`, or every successful step was a utility kind (`log`/`delay`/`channel_notify`/`budget_gate`/`human_approval`/`manual_trigger` — §7 *Final output*) — add a content-bearing step (e.g. a closing `agent_prompt` "report" node) if you want a deliverable. |
| A paused, blocked or out-of-budget run does nothing | It is **held**: it waits for **Resume** (or **Extend and resume…**) on its run page, however long that takes; **Stop run…** ends it. |

---

## 13. Related docs

- `./api-client.md` — the HTTP/gRPC API client, environments, saved requests, and
  the request engine that **API-client automations** (§8) and the `api_run` node
  build on.
- `./daemon-http-api.md` — how the daemon's HTTP+WS surface, auth tokens, and
  RBAC feature grants work (the `Workflows:View/Edit` grants used here).
- [Goals](./goal-loops.md) — goals inside Workflows: the Goal form, the check
  step, budgets, holds, and the older goal loops' history.
- [Threads](./threads.md) and
  [the structured runtime](./threads-structured-runtime.md) — the runtime
  agent steps run on; engine threads are read from the run page, not listed
  in Threads.
- `./agent-sessions.md` — terminal agent sessions: what the terminal runner
  uses, and what AI workflow generation runs.
- [Phone version (light)](./phone-light-version.md) — Workflows is one of its
  six modules.
- `./scheduled-tasks.md` — recurring agent jobs; the **convert-to-workflow** bridge
  materializes a task as a workflow + schedule trigger, and Workflows borrow its
  shared cadence engine (cron + timezone).
- `./agent-swarm.md` — Agent Swarm, the target of the `swarm_task` node.
- `./product.md` — product stories that the `product_analyze` / `product_rewrite` /
  `product_plan` / `product_publish` nodes operate on.
- `./channels-slack-telegram.md` — Slack/Telegram integrations used by
  `channel_notify` and the `Action: Workflow` chat trigger.
- `./message-brokers.md` — Kafka clusters used by `broker_peek`.
- `./connections-ssh-sftp.md` / Database Explorer — connections used by `db_query`.
- Contracts: `docs/contracts/api.md` (§"Workflow engine", §"API client",
  Wave-3/Wave-4) and `docs/contracts/ws.md` (Workflow run progress).

## Re-review a PR after fixes

A succeeded run with a review step has **Re-review** in its ⋯ menu. In the
original Slack/Telegram review thread, reply **`re-review`** or **`review again`**.
This creates a new run and retains the old report. The workflow's current
configuration is used, source PR refs are refreshed, and all configured approval,
notification and PR-comment steps execute again. An active workflow refuses a
second launch. Chat re-review requires the exact original thread; it never picks
a different review merely because it ran in the same channel.

The new context includes `previous-review.md` plus each previous step's full
output. Reviewers verify every earlier finding (fixed with evidence, still open,
or not relevant with a reason), then review the current diff and score it again.
Previous conclusions and gate results are historical evidence. A failed source
refresh fails re-review instead of silently reviewing cached refs.

Independent repositories now progress two at a time. All reviewers still share
the existing process-wide agent concurrency limit; adding repositories does not
multiply that limit. Results retain the original repository order and every
required target must complete for a multi-repo review to pass.

### Reuse background context safely

A **read-only context-discovery** `agent_prompt` can opt into
`params.review_context_paths: ["docs/architecture", "README.md"]`. These tracked
paths must cover **all repository dependencies of that step**. A repeat reuses
its reply only when those Git blobs, the Jira context, original request,
substantive upstream inputs and prior handoffs, resolved skill instructions, node parameters and
workflow version match, and every repository is clean. Missing
metadata, changed dependencies, a changed ticket/config, or a dirty checkout
runs the step normally. The first run after opting in records the metadata.

Never put this option on checkout, gates, fixes, verification, notifications or
publishing. When the dependency set is unknown, leave caching off; previous
outputs remain available for selective verification. This is conservative: changed upstream briefs may require rediscovery even when
the declared documentation stayed unchanged. Cache hits are recorded in
the step output (`context_reused`) and log. They do not inherit approval or pass
results, and current `repos.json` remains the authority for checkout paths.

### Upgrade a workflow on another machine

The custom PR Reviewer shown in the examples is machine-local data, so upgrading
the app does not rewrite its prompts. Export it on its owning machine and run:

```bash
python3 scripts/upgrade-pr-review-workflow.py pr-review.json pr-review-upgraded.json
```

Review the output and import it on that machine. The script preserves the graph,
Go-gates prompt, summarizer instructions, thresholds and critical/high weights.
It opts out of duplicated provider-per-lens checks, makes optional observations
non-penalizing, and appends the iteration/evidence policy. Optional
`--context-paths CONTEXT_NODE=docs/architecture,README.md` declares a complete,
read-only context dependency set. It never writes to the running daemon.

The existing Go-gates contract remains: the prescribed `pr_gates.sh` runs once
per changed-Go repository and reviewed HEAD in the iteration. Keep all table
rows including NOT RUN, NEW-at-HEAD standards findings, and the nightly skill
evaluation date/status. No extra manual gates, Docker-backed suites, module
tidy/download, or skill-eval executions are added. Changed HEAD invalidates old
passes; reviewers consume this iteration's gate report instead of repeating it.
