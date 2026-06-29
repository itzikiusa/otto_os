# Workflows & Automations

> **Maturity: a real orchestrator.** Otto's Workflows feature is a working
> node-graph engine: you build a directed graph of nodes, run it (whole graph, or
> "from here" / "only this"), and watch live per-node progress — including the
> openable agent **sessions** each step spawns — over WebSocket. Every node kind
> now executes against a real Otto subsystem: the four formerly-stub
> product/review kinds (`product_analyze`, `product_rewrite`, `product_plan`,
> `review_run`) are **wired** (real single-agent turns + the local-review engine
> with a 0–100 score), and the engine gained **branching** (edge conditions + a
> `condition` node), **bounded loops** (`loop`, iterate-until), **retry/backoff**,
> **typed outputs** (warn-only validation), **versioning** (graph snapshot
> history), a per-run **Proof Pack** link, plus `canvas`, `git_pr`, and
> `product_publish` nodes. **All three trigger kinds fire unattended now** —
> webhook, event, *and* schedule (the cadence scheduler is spawned at daemon boot,
> with cron + IANA-timezone parity). A structured `Action: Workflow` chat message
> can start a run by name. The separate **API-client "Automations"** surface (a
> multi-step saved-request runner) is also fully functional. A couple of honest
> caveats remain (the game `game_engine`/`verifier` are scaffolds; agent output is
> cached) — stated inline; do not assume full parity with mature n8n/Zapier engines.

The doc is grounded in the code in `crates/otto-server/src/workflow_engine.rs`,
`crates/otto-server/src/routes/workflows.rs`,
`crates/otto-server/src/workflow_trigger_scheduler.rs`,
`crates/otto-server/src/workflow_chat.rs`, `crates/otto-core/src/expr.rs`,
`ui/src/modules/workflows/`, `ui/src/modules/api/AutomationsView.svelte`, the
migrations under `crates/otto-state/migrations/` (incl. **0089** — versioning +
run→proof link), and the authoritative contracts `docs/contracts/api.md`
(§ "Workflow engine", § "API client", Wave-3/Wave-4 routes) and
`docs/contracts/ws.md` (Workflow run progress).

---

## 1. Two different things called "automations"

Otto has **two unrelated automation surfaces**. Keep them straight:

| Surface | What it is | Where | Model | Backed by |
|---|---|---|---|---|
| **Workflow engine** | A visual node-graph you build and run, with branching, loops, retry, and versioning; nodes call agents, HTTP, DB, brokers, channels, swarm, product, review, git, etc. | `#/workflows` page | `Workflow` / `WorkflowRun` / `WorkflowGraph` / `WorkflowVersion` | `workflows`, `workflow_runs`, `workflow_node_cache`, `workflow_triggers`, `workflow_versions` tables |
| **API-client "Automations"** | A *collection runner*: an ordered list of saved API-client requests with per-step assertions + variable extraction (a tiny test/regression runner). | API client (`#/api`) → "Automations" view | `ApiAutomation` / `ApiAutomationStep` / `ApiRunResult` | `api_automations` table |

This document covers **both**, but the bulk is the workflow engine. The
API-client automations are a thin, self-contained feature covered in §8.

---

## 2. Overview & where it lives

```
ui/src/modules/workflows/WorkflowsPage.svelte   — the page: generate / list / edit / run / inspect
ui/src/modules/workflows/WorkflowCanvas.svelte  — n8n-style pan/zoom node-graph canvas (SVG + cards)
ui/src/modules/workflows/RunSteps.svelte        — per-step run detail (status, logs, "work product")
ui/src/modules/workflows/TriggersPanel.svelte   — list/add/toggle/delete schedule|webhook|event triggers
ui/src/modules/api/AutomationsView.svelte       — API-client collection runner (the *other* "automations")

crates/otto-server/src/workflow_engine.rs              — the executor: node catalog, run loop, per-node exec, branching/retry, proof
crates/otto-server/src/routes/workflows.rs             — HTTP handlers: CRUD, generate, run, versions, triggers, webhook, approve, templates
crates/otto-server/src/workflow_trigger_scheduler.rs   — schedule scheduler + event-bus listener (both spawned at boot)
crates/otto-server/src/workflow_chat.rs                — `Action: Workflow` chat-message parser + WorkflowChatTrigger impl
crates/otto-core/src/expr.rs                           — the safe expression language (edge conditions, condition/loop, {{ }} templating)
crates/otto-core (otto_core::workflows)                — the Workflow/WorkflowRun/Node/Edge/Version domain types
crates/otto-state/migrations/0020_workflows.sql        — workflows + workflow_runs
crates/otto-state/migrations/0051_workflow_node_cache.sql — per-node output cache
crates/otto-state/migrations/0058_workflow_triggers.sql   — workflow_triggers + run approval columns
crates/otto-state/migrations/0089_workflow_orchestrator.sql — workflow versioning + run→proof-pack link
crates/otto-state/migrations/0014_api_client.sql          — API client base
crates/otto-state/migrations/0015_api_automations.sql     — API-client automations
```

The Svelte UI talks only HTTP+WS to `ottod` on `127.0.0.1:7700`. The TypeScript
types in `ui/src/lib/api/types.ts` mirror `otto_core::workflows`.

> **README note:** as of this writing the `README.md` feature tour does **not**
> mention Workflows or the workflow engine by name (it lists git/PRs, review,
> product, connections, API client, usage, channels). This doc and
> `docs/contracts/api.md` are the source of truth for the feature.

---

## 3. The workflow model: nodes, edges, triggers

A **`Workflow`** is a named, workspace-scoped, directed graph plus run history.

```ts
interface Workflow {
  id; workspace_id; name; description;
  graph: WorkflowGraph;       // { nodes: WorkflowNode[]; edges: WorkflowEdge[] }
  created_by; created_at; updated_at;
  version: number;            // monotonic; bumped + snapshotted on every graph-changing edit
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
- The graph is executed in **topological order** (`topo_order`). A cycle makes
  the whole run fail immediately with an error (no nodes execute).

### The expression language (`otto_core::expr`)
A tiny, **safe** (pure; no I/O, no `eval`) expression language evaluates against a
JSON context. It powers **edge conditions**, the `condition` / `loop` nodes, and
`{{ … }}` templating. Grammar supports `|| && == != < <= > >=`, `+ - * / %`, the
infix `contains` / `in`, unary `! -`, parentheses, dotted/indexed paths
(`output.result`, `input.rows[0]`), literals, and functions `len, lower, upper,
default, has, int, float, str, bool, not`. Missing path segments resolve to `null`
(never an error). For edge conditions the context is
`{ output, input, node:{id,kind,name}, run:{input} }`; a condition that fails to
parse or evaluate is treated as **not taken** (and logged), never a crash.

### Retry/backoff (`WorkflowNode.retry`)
`{ max_attempts (extra attempts after the first, clamped ≤5), backoff_ms (initial
sleep, clamped ≤60000), factor (multiplier, default 2.0) }`. Default is **no
retry** (single attempt), so existing graphs are unchanged. The policy can also be
supplied as a `params.retry` object. `human_approval` and `manual_trigger` are
never retried. `NodeRunState.attempts` records how many attempts ran (`0` = a cache
hit).

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
    reachable via edges; ancestors are `skipped` but their **cached** outputs (if
    any) still feed the entry node.
- **Per-node output cache** (`workflow_node_cache`): keyed by
  `(workflow_id, node_id, sha256(params), sha256(assembled_input))`. On a re-run,
  a node whose params + input are unchanged is **skipped and its stored output
  surfaced as "Success (cached)"** (duration `0ms`). **All** node kinds
  participate in the cache — including `agent_prompt`, even though agent output is
  non-deterministic — so "run from here" can skip expensive unchanged upstream
  work. (This is the `finish/cached` transition referenced in `ws.md`.)
- **Global wall clock:** a run cannot execute forever. At each node boundary the
  engine checks an overall `RUN_WALL_CLOCK_TIMEOUT`; exceeding it marks all
  un-run nodes `skipped` and fails the run with "run exceeded the N-minute time
  limit". Individual agent/approval nodes also have their own `NODE_AGENT_TIMEOUT`.
- **Typed-output validation (warn-only):** after a node succeeds, its output is
  checked against the kind's declared `output_schema` (`validate_node_output`);
  mismatches are appended to the node log as `⚠ …` lines and **never fail the run**.
- **Visible sessions per step:** agent / product / canvas / loop-inner nodes run as
  **real, openable Otto sessions** (not the headless PTY). Each session id is
  reported the moment it's created and recorded on `NodeRunState.sessions`, so the
  run view can open it **while the step is still running** (`review_run` also
  surfaces a `review_id` in its output).
- **Run → Proof Pack:** on completion the engine assembles a Proof Pack from the
  run (each node output → a `log` artifact with the node's pass/fail status; each
  `human_approval` → an `approval` artifact) and links it on the run
  (`WorkflowRun.proof_pack_id`). Best-effort.
- **Versioning:** the run records the workflow `version` it executed
  (`WorkflowRun.workflow_version`); see §9.

---

## 4. Node types

The catalog is returned by `GET /workflows/node-types` (`node_catalog()` in
`workflow_engine.rs`). Each descriptor (`NodeTypeSpec`) carries
`kind, label, category, description, inputs, outputs, color, icon`. The UI's
"+ Node" palette is built directly from this list, and per-kind parameter forms
live in `WorkflowsPage.svelte`.

**Status legend:** **Real** = executes against a live subsystem · **Real
(scaffold)** = runs and produces real structured output but emits a canned spec
pending an external engine (only `game_engine`/`verifier`). There are **no longer
any "not wired" stub kinds** — the four former product/review stubs are now wired.

| Kind | Label / category | Purpose | Params (UI form) | Status |
|---|---|---|---|---|
| `manual_trigger` | Manual Trigger / Triggers | Entry node; emits the run input. `inputs:0`. | — | **Real** |
| `agent_prompt` | Agent / AI | Runs an agent turn as a **real, openable session**; output `{ "reply", "session_id" }`. | `prompt`, `provider` (default `claude`) | **Real** |
| `http_request` | HTTP Request / Network | Calls an HTTP endpoint, captures response `{ status, body }`. | `method`, `url`, `body` (JSON) | **Real** |
| `transform` | Set / Transform / Data | Merges a static JSON object into the data flowing through. | `json` (object) | **Real** |
| `delay` | Delay / Flow | Sleeps `ms` milliseconds, then passes input through. | `ms` (0–10000) | **Real** |
| `log` | Log / Flow | Records the incoming data in the run log; passes it through. | — | **Real** |
| `game_engine` | Game Engine / Game | Assembles a slot/crash/scratch game *spec* (RNG/paytable/reels) from inputs. | `game` (`slots`/`crash`/`scratch`) | **Real (scaffold)** ¹ |
| `verifier` | Verifier / Game | Verifies a built game. In the **game path** (`play_url` present) it does real file-existence/size checks and **errors** if they fail; otherwise it emits a scaffold "passed" report. | — | **Real (scaffold)** ¹ |
| `db_query` | DB Query / Data | Runs a **read-only** SQL/CH query against a saved Database-Explorer connection. `confirm_write` is forced `false`. | `connection_id`, `statement`, `max_rows` (default 100, cap 1000) | **Real** |
| `broker_peek` | Broker Peek / Data | Consumes up to N recent messages from a Kafka topic on a saved broker cluster (read-only). | `cluster_id`, `topic`, `limit` (default 20, cap 50) | **Real** |
| `channel_notify` | Channel Notify / Integrations | Sends a message to a configured Slack/Telegram integration. Supports `{key}` substitution from the incoming object. | `message`, `channel` (`slack`/`telegram`/any) | **Real** |
| `budget_gate` | Budget Gate / Flow | Checks the provider spend cap: **errors the run if blocked**; otherwise passes (`exceeded` is warn-only). | `provider` (`claude`/`codex`) | **Real** |
| `human_approval` | Human Approval / Flow | **Pauses the run**; sets `waiting_approval=1` and polls until an operator approves/rejects via the approve endpoint (or times out at `NODE_AGENT_TIMEOUT`). | `prompt` | **Real** |
| `condition` | Condition / Flow | Evaluates an `expr` on its input; outputs `{ result, value }` merged onto the input. Pair with **edge conditions** to branch. | `expr` (default `true`) | **Real** |
| `loop` | Loop (Until) / Flow | Bounded **iterate-until**: re-runs inner `steps[]` until `until` holds or `max_iterations`. Reuses inner-node execution; threads run-level keys + prev-step output to each step. Output `{ iterations, satisfied, last, history }`. No nested loops. | `max_iterations` (1–10, default 3), `until` (expr), `steps[]` ({kind,name,params,retry}), `continue_on_error` | **Real** |
| `swarm_task` | Swarm Task / AI | Enqueues a task in a running Agent-Swarm project (`todo` status; coordinator picks it up). | `swarm_id`, `project_id`, `title`, `description` | **Real** |
| `api_run` | API Run / Network | Executes an HTTP request **through the API-client engine** so env-var substitution + auth apply. | `method`, `url`, `headers`, `body` | **Real** |
| `product_analyze` | Product Analyze / Product | Runs a real single-agent turn (the **`grill`** lens) over the story's built context; outputs `{ story_id, analysis, session_id }`. | `story_id`, `instruction?` | **Real** ² |
| `product_rewrite` | Product Rewrite / Product | Rewrites the story (**`jira-story-writer`**); outputs `{ story_id, body_md, session_id }`; `persist:true` saves a `suggested` product version. | `story_id`, `persist?`, `instruction?` | **Real** ² |
| `product_plan` | Product Plan / Product | Breaks the story into a plan (**`story-task-breakdown`**); outputs `{ story_id, plan_md, session_id }`; `persist:true` saves a `plan` version. | `story_id`, `persist?`, `instruction?` | **Real** ² |
| `product_publish` | Product Publish / Product | Publishes a story as a Confluence **RFC** or a **Jira** issue. **`dry_run` defaults true** (no-op note); a real publish needs `account_id` (+ `project_key`/`space_key`). | `kind` (`rfc`/`jira`), `dry_run`, `account_id`, … | **Real** |
| `review_run` | Review Run / AI | Runs the **local-review engine** (`run_review_for_branch`), polls to completion, emits a **0–100 `score`** (`100−20×blocking−5×advisory`), optional `goals` assessment blended in, `passed = score≥threshold && status==done`. | `repo_id`, `base` (default `main`), `threshold` (default 80), `await`, `timeout_s`, `goals[]` | **Real** |
| `canvas` | Canvas Diagram / Product | Asks an agent for a **mermaid/excalidraw** diagram and writes it under the data dir (`workflow-canvas/{run}/{node}.{ext}`); output `{ scene_id, path, diagram, … }`. | `prompt`, `mode` (`mermaid`/`excalidraw`) | **Real** |
| `git_pr` | Git PR / Network | **Drafts** a pull request for a repo branch (title/description via `draft_pr_core`); **`opened:false`** — opening is left to the Git tab. | `repo_id`, `base` (default `main`), `worktree_path?` | **Real** |

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
> `jira-story-writer` / `story-task-breakdown`), as a visible session; `product_*`
> can optionally `persist` a product version. `review_run` calls the in-process
> **local-review engine** (`run_review_for_branch`), polls it to completion, and
> computes a deterministic 0–100 score from the blocking/advisory finding counts
> (`review_findings_counts`), optionally blending in an agent goals-assessment.
> They no longer emit a `{"stub": true}` marker — older docs/UI hints about "not
> wired" are obsolete.

> **Heads-up — wired nodes still have prerequisites.** `db_query` (a saved
> DB-Explorer connection), `broker_peek` (a broker cluster), `swarm_task` (a
> running swarm/project), `review_run`/`git_pr` (a registered git repo + a
> `repo_id` in params or run input), and `product_*`/`product_publish` (a
> `story_id`, and an Atlassian `account_id` for a real publish) all depend on
> other features being set up. A missing dependency causes the **node** to error
> (and downstream active-path nodes to skip) — not a silent no-op.

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
  `{ iteration, last, steps, input }`. The previous step's output threads into the
  next step, and run-level keys (e.g. `repo_id`, `goals`) flow to every step. The
  built-in flow templates use a `fix → review_run until last.passed == true` loop.

---

## 5. Triggers

A workflow runs **manually by default**. You can attach triggers in the
**Triggers** panel (`TriggersPanel.svelte`). Trigger rows live in the
`workflow_triggers` table (migration 0058); the schedule/event firing logic is in
`workflow_trigger_scheduler.rs`.

```ts
type TriggerKind = 'schedule' | 'webhook' | 'event';
interface WorkflowTrigger { id; workflow_id; kind; spec: object; enabled; created_at }
```

| Kind | Spec | Fires when… | Run input | Wired & firing in the daemon? |
|---|---|---|---|---|
| **webhook** | `{ token }` (32-byte URL-safe token auto-generated server-side on create) | An external system calls `POST /workflows/{id}/webhook/{token}`. The token **is** the credential — no bearer auth required. | The request body (JSON), or `null`. | **Yes** — handler `webhook_trigger` spawns the run. |
| **event** | `{ event_kind, filter_json? }` | A daemon event whose mapped name equals `event_kind` fires on the event bus. | `{ "trigger": "event", "event_kind": "..." }` | **Yes** — `spawn_workflow_event_trigger_listener` is started at daemon boot (`ottod` main, "workflow event-trigger listener started"). |
| **schedule** | `{ cadence, every_min, at, weekday, expr, timezone, last_run, enabled }` (the **shared cadence** format — same as Scheduled Tasks) | Cadence comes due: `interval` (every N min, default 60), `daily` (at `HH:MM`), `weekly` (weekday 0=Mon at `HH:MM`), or **`cron`** (`expr`, 5-field). All interpreted in the spec's IANA **`timezone`** (default UTC). | `{ "trigger": "schedule" }` | **Yes** — `workflow_trigger_scheduler::start` is started at daemon boot (`ottod` main, "workflow schedule-trigger scheduler started"). |

### Event-kind mapping (configure by string in the trigger spec)
The event listener maps daemon `Event` variants to stable strings; the UI default
in the add-trigger form is `ReviewChanged` (which maps to `review_changed`):

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
> **started in `crates/ottod/src/main.rs`** at boot (log line *"workflow
> schedule-trigger scheduler started"*), alongside the event listener and the
> swarm / scheduled-tasks supervisors. Its `is_due` **delegates to the shared
> `cadence` engine** (the one Scheduled Tasks use), so workflow schedule triggers
> get **cron** (`cadence:"cron"`, `expr`) and **IANA timezone** (`timezone`) parity
> for free; `interval`/`daily`/`weekly` behave exactly as before. All three trigger
> kinds — webhook, event, schedule — now start runs unattended.

### Chat trigger: `Action: Workflow` (Slack / Telegram / webhook)
A structured inbound channel message can **start a workflow run by name** instead of
opening a normal session — wired through the channels `Bridge` via the
`WorkflowChatTrigger` hook (`workflow_chat.rs`; `ottod` injects
`WorkflowChatTriggerImpl`, mirroring the swarm/run triggers). The message shape
(field labels case-insensitive; `Goals:` may be a bullet list **or** an inline
comma/semicolon list):

```text
@otto
Action: Workflow
Name: Implement Feature        ← resolved case-insensitively against the workspace's workflows
Msg: please do x y z, follow all relevant rules
Jira ticket: GS-1111
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
rest of the graph — and replies in-thread (*"🚀 Started workflow … (run `…`)"*).

### Human approval is **not** a trigger
The `human_approval` *node* pauses a run mid-flight (it writes `waiting_approval`
to the run row); the operator resumes via `POST /workflow-runs/{id}/approve`. This
is a node, not a trigger (migration 0058 comment makes this explicit).

---

## 6. Building a workflow (UI walkthrough)

Open the **Workflows** page (`#/workflows`). The left sidebar offers three ways
to create a workflow; the center is the canvas editor.

### Create
1. **Generate from a description (the primary path).** Type a natural-language
   description ("Ask an agent to summarize the repo, then POST the summary to our
   webhook") and click **Generate workflow** (or ⌘/Ctrl-Enter). This calls
   `POST /workspaces/{wid}/workflows/generate`, which prompts a headless agent
   with the full node catalog and asks for a JSON graph. The result is then:
   - **sanitized** — nodes with unknown kinds are dropped; edges referencing
     missing nodes are dropped;
   - **laid out** left-to-right by topological layer;
   - and, if the LLM is unavailable or returns junk, replaced by a **fallback
     graph** of `manual_trigger → agent_prompt` (the agent node pre-seeded with
     your description). So generation **never fails** to produce a runnable graph.
2. **Start blank** — creates `manual_trigger` ("Start") only; build by hand.
3. **From a template** (`POST /workspaces/{wid}/workflows/from-template`,
   `GET /workflows/templates`) — built-in examples in two families:
   - **Orchestrator flows** (`flow_templates`) exercising the wired nodes + control
     flow:
     - **`write-tests`** — *Write tests for a story*: prepare brief → write tests →
       a `fix → review_run` **loop** (`until last.passed == true`) → `human_approval`
       → `git_pr` draft.
     - **`implement-feature`** — `product_analyze` → prepare → implement → the same
       review loop → approval → PR draft.
     - **`po-lifecycle`** — *PO discovery → RFC/Jira*: discovery draft → `canvas`
       diagram → approval → `product_rewrite` → approval → `product_publish`
       (RFC, dry-run). They expect the run **input** to carry `repo_id` (and
       optionally `base`, `story_id`, `goals`) — a Slack `Action: Workflow` message
       or the Run dialog supplies these.
   - **Game pipelines** (`game-slots` / `game-crash` / `game-scratch`), each chaining
     an agent design step into the game engine + verifier scaffold.

### Edit on the canvas (`WorkflowCanvas.svelte`)
- **Pan**: drag the background. **Zoom**: mouse wheel (0.3×–2×), or the HUD
  `+ / − / reset` controls.
- **Add a node**: top-bar **+ Node** opens the palette (built from
  `node-types`); click a kind to drop it.
- **Connect**: drag from a node's **right (output) port** to another node's
  **left (input) port**. Click an edge to delete it.
- **Move/select**: drag a node to reposition; click to select (opens the
  inspector). **Trash** removes the selected node (and its edges).
- **Configure params**: the bottom **inspector** shows a per-kind form for the
  selected node (e.g. `agent_prompt` → prompt + provider; `http_request` →
  method/url/body; `review_run` → repo_id/base/threshold/goals). Unrecognized or
  future kinds fall back to a **raw-JSON params editor**.
- **Save**: edits set an "unsaved" badge; **Save** (`PATCH /workflows/{id}` with
  the new `graph`) persists. Running while dirty auto-saves first.

### Triggers
Top-bar **Triggers** toggles the `TriggersPanel`: add/enable/disable/delete
schedule, webhook, or event triggers — all three fire unattended in the running
daemon (see §5).

---

## 7. Running & monitoring

### Run
- **Run** (top bar) → `POST /workflows/{id}/run` with `{}` → runs the whole graph.
- From the inspector of a selected node: **▶ From here**
  (`{ start_node, only_node:false }`) or **Only this**
  (`{ start_node, only_node:true }`).
- **Stop** → `POST /workflow-runs/{id}/cancel`. Cancellation is checked at each
  node boundary: the **current node finishes**, then the run halts and remaining
  nodes are marked `skipped` (status `canceled`).

A run executes in a background task (`run_workflow`) and persists progress to the
`workflow_runs` row after **every** node transition.

### Statuses
```ts
RunStatus  = 'pending' | 'running' | 'success' | 'error' | 'canceled';
NodeStatus = 'pending' | 'running' | 'success' | 'error' | 'skipped';
```
A run is `error` if **any** node errored ("one or more nodes failed"). Cached
nodes show as `success` with a "Success (cached)" log line.

### Live progress over WebSocket (`workflow_run_updated`)
The engine emits `Event::WorkflowRunUpdated` on the shared event bus at **every
node transition** (start, finish/cached, skip) and at **run completion**:

```json
{ "type": "workflow_run_updated",
  "workspace_id": "<Id>", "run_id": "<Id>",
  "status": "running|success|error|canceled",
  "node_id": "<node_id | null>" }
```
- `node_id` is the node whose state changed; `null` when the event reflects the
  overall run status (run started / run reached a terminal state).
- The UI (`events.svelte.ts` → `workflowRunBus.apply()` → `WorkflowsPage`)
  re-fetches `GET /workflow-runs/{id}` whenever a matching `run_id` event fires.
- A **capped fallback poll** (700ms, backing off to 3s, max 300 ticks ≈ 3.5 min)
  keeps the UI live if the WS connection is unavailable.

### Inspect
`RunSteps.svelte` renders each step's status, duration, `attempts` (when a retry
policy ran), logs (including `⚠` typed-output warnings and `edge → … not taken`
branch lines), error, and the **"work product"** (an agent `reply` string is
rendered as text; everything else as pretty JSON, copyable). Steps that spawned
**openable sessions** (`NodeRunState.sessions` — agent / product / canvas / loop
turns) link to them so you can watch/inspect the agent **while the step runs**; the
timeline strip at the top jumps between steps.

### Run → Proof Pack
On completion the run links a **Proof Pack** (`WorkflowRun.proof_pack_id`)
assembling each node's output as evidence (a `log` artifact carrying the node's
pass/fail; each `human_approval` becomes an `approval` artifact recording the
approver). The run also records the workflow `version` it executed
(`WorkflowRun.workflow_version`).

### Human-approval pause
When a run hits a `human_approval` node it pauses; the page shows a banner —
*"Run paused — waiting for approval at &lt;node&gt;"* — with **Approve** / **Reject**
(→ `POST /workflow-runs/{id}/approve` with `{node_id, approved}`). Approve resumes
the run (records `approved_by`); reject errors the node ("rejected — &lt;note&gt;").
If no decision arrives within the node timeout, the node errors ("timed out").

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
| `POST /workspaces/{wid}/workflows` | ws editor | create (`CreateWorkflowReq`) |
| `POST /workspaces/{wid}/workflows/from-template` | ws editor | instantiate a template |
| `POST /workspaces/{wid}/workflows/generate` | ws editor | AI-generate from `{description, name?}` |
| `GET /workflows/{id}` | ws viewer | one workflow |
| `PATCH /workflows/{id}` | ws editor | update (e.g. `{graph}`) |
| `DELETE /workflows/{id}` | ws editor | 204 |
| `POST /workflows/{id}/run` | ws editor | `RunWorkflowReq?` `{start_node?, only_node?}` → `WorkflowRun` |
| `GET /workflows/{id}/runs` | ws viewer | `WorkflowRun[]` |
| `GET /workflow-runs/{id}` | ws viewer | one run (poll/refresh target) |
| `POST /workflow-runs/{id}/cancel` | ws editor | cancel a run |
| `GET /workflows/{id}/versions` | ws viewer | `WorkflowVersion[]` (snapshot history, newest first) |
| `GET /workflows/{id}/versions/{v}` | ws viewer | one snapshot (404 if unknown) |
| `POST /workflows/{id}/versions/{v}/restore` | ws editor | `{note?}` → copies `v`'s graph in as a **new** version |

Plus, on the Scheduled-Tasks surface: `POST /scheduled-tasks/{id}/convert-to-workflow`
(`{disable_task?}` → `{workflow_id, trigger_id?}`) materializes a task as a Workflow
+ schedule trigger — see `./scheduled-tasks.md`.

Trigger / webhook / approval routes (api.md Wave-3 additions, lines 1295–1300):

| Method & path | Auth | Notes |
|---|---|---|
| `POST /workflows/{id}/webhook/{token}` | **public-by-token** | run input = body; token matched against `workflow_triggers`; returns `{run_id}` |
| `GET /workflows/{id}/triggers` | ws viewer (Workflows:View) | `WorkflowTrigger[]` |
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

Cross-module search (api.md Wave-4, line 1323): `GET /workspaces/{id}/search?q=`
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
- **0051** `workflow_node_cache` (workflow_id, node_id, `params_hash`,
  `input_hash`, `output_json`; unique on the 4-tuple).
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
- **0014/0015** `api_client` base + `api_automations` (name, `steps_json`).
- `retry` and edge `condition` need **no migration** — they live inside
  `graph_json`.

All ids are ULID strings; timestamps are UTC RFC-3339; rows cascade-delete with
their workspace (and triggers/runs/cache cascade with the workflow). Migrations
are **append-only** — never edit or renumber an existing one.

---

## 10. Capabilities & limitations (be explicit)

**What works today**
- Build graphs by description (AI), template (orchestrator flows + game pipelines),
  or hand; pan/zoom canvas editor.
- Topological execution with failure propagation, partial runs (from-here / only),
  per-node output caching, run cancellation, and a global wall-clock timeout.
- **Branching & loops:** edge `condition`s + a `condition` node (if/else, with clean
  branch-skip vs failure-poison semantics), and a bounded `loop` (iterate-until)
  that reuses inner-node execution — all driven by the safe `otto_core::expr`
  language (also available as `{{ }}` templating).
- **Retry/backoff** per node (`retry`), and **typed outputs** (warn-only validation
  against each kind's `output_schema`).
- **All node kinds are real:** `manual_trigger`, `agent_prompt`, `http_request`,
  `transform`, `delay`, `log`, `db_query` (read-only), `broker_peek`,
  `channel_notify`, `budget_gate`, `human_approval`, `condition`, `loop`,
  `swarm_task`, `api_run`, and the **now-wired** `product_analyze`,
  `product_rewrite`, `product_plan`, `product_publish`, `review_run` (0–100 scored),
  `canvas`, `git_pr` (PR draft).
- **Visible sessions per step** (openable while running) and a **Proof Pack** linked
  to each completed run.
- **Versioning:** graph snapshot history with view + restore (append-only).
- Live WS run progress + per-step logs/output/"work product".
- **All three trigger kinds fire in the running daemon** — webhook, event, **and
  schedule** (spawned at boot; cron + IANA timezone via the shared cadence engine).
- A **chat trigger** (`Action: Workflow` Slack/Telegram/webhook message) starts a
  run by name. **Convert** a scheduled task into a workflow + schedule trigger.
- Human-approval pause/resume.
- API-client automations: ordered request runner with assertions + extracts.

**Caveats (still honest about the edges)**
- **`game_engine` / `verifier` are scaffolds** — real, runnable, useful for the
  game-pipeline templates, but they emit canned specs / scaffold reports awaiting
  a real external game engine + certifier (see footnote ¹ in §4).
- **`product_publish` defaults to a dry run** — a real RFC/Jira publish requires
  `dry_run:false` + an Atlassian `account_id` (and `project_key`/`space_key`).
- **`git_pr` only drafts** (`opened:false`) — opening the PR is left to the Git tab
  (engine auto-open is deliberately gated).
- **Wired nodes have prerequisites** — `db_query`/`broker_peek`/`swarm_task`/
  `review_run`/`git_pr`/`product_*` need their backing connection/cluster/swarm/repo/
  story set up, or the node errors (and downstream active-path nodes skip).
- **Agent-output caching is intentional but can surprise** — re-running a graph
  with unchanged params+input serves the **prior agent reply from cache**
  (duration `0ms`, `attempts:0`), not a fresh LLM call.
- **Typed-output validation is warn-only** — schema mismatches log `⚠` but never
  fail a run; `params_schema` is currently unpopulated (UI hint only).
- API-client automations have **no scheduler** — run-on-demand only.

---

## 11. Security & permissions

- **RBAC (per-workspace `Workflows` feature grant):** node-types and templates
  are **member**-readable; listing/reading workflows + runs + triggers is **ws
  viewer** (`Workflows:View`); creating/editing/deleting/running workflows, and
  all trigger mutations + run approval, are **ws editor** (`Workflows:Edit`).
  Runs resolve the workspace from the workflow/run row. (See `./daemon-http-api.md`
  and the multi-user RBAC doc for how feature grants and roles compose.)
- **Webhook triggers are public-by-token.** `POST /workflows/{id}/webhook/{token}`
  requires **no bearer auth** — the 32-byte URL-safe token in the path *is* the
  credential, matched against an enabled webhook trigger. Treat the URL as a
  secret; delete the trigger to revoke. Anyone with the URL can start runs (with
  attacker-controlled JSON input).
- **Chat `Action: Workflow` runs on the channel-trust model.** A structured
  message in a configured Slack/Telegram/webhook channel can start a run by name
  (the run acts as the workflow's `created_by`, falling back to a synthetic
  "Workflow" user for system-initiated runs). Anyone who can post to a wired
  channel can start any workflow in that workspace by name — treat channel access
  as run-start capability.
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
| A `review_run` node "passes" too easily / never passes | `score = 100 − 20×blocking − 5×advisory` (optionally blended with a goals score), `passed` needs `score ≥ threshold` **and** the review reaching `done`. Tune `threshold` (default 80) or check the finding counts in the node output. |
| A scheduled run never starts | The schedule scheduler **is** spawned at boot now (§5). Check the trigger is **enabled**, the cadence/`timezone` is right, and `last_run` shows it isn't mid-window; for cron, validate the `expr`. |
| A node re-runs instantly with "Success (cached)" and stale output | Per-node cache hit (§3) — params + assembled input unchanged. Change a param to bust it, or accept the cached value. |
| A branch I expected to run was `skipped (branch not taken)` | An incoming edge's `condition` evaluated false (or its upstream was branch-skipped). Inspect the `edge → … not taken` log line and the source node's output the condition tested. |
| Run fails immediately, no node ran | The graph has a **cycle** (topo-sort failed) — remove the back-edge. |
| A downstream node is `skipped` ("upstream did not succeed") | A predecessor errored or was skipped; fix/inspect the upstream node first. |
| `db_query` errors "missing connection_id" / connection not found | The `connection_id` must be a saved Database-Explorer connection id; create it there first (`./connections-ssh-sftp.md`). |
| `broker_peek` / `swarm_task` errors | The referenced cluster / swarm + project must exist and be set up. |
| `channel_notify` does nothing | No enabled Slack/Telegram integration, or the selected `channel` isn't configured (`./channels-slack-telegram.md`). |
| Run UI stops updating but isn't finished | WS dropped → the capped fallback poll (max ~3.5 min) took over; the run still runs server-side. Re-open the run or reload. |
| Webhook returns 401 | Token doesn't match an **enabled** webhook trigger on that workflow id. Re-check the token / re-enable the trigger. |
| Run "exceeded the N-minute time limit" | The global wall-clock fired; the graph (often an `agent_prompt` chain) is too long. Split it or reduce work. |
| Approve/Reject button does nothing | The run must actually be paused at a `human_approval` node (`waiting_approval`), and you need `Workflows:Edit`. |
| API-client automation step fails on a 2xx | It has a failing assertion. With **no** assertions a step passes on any 2xx; check the assertion `desc` in the report. |

---

## 13. Related docs

- `./api-client.md` — the HTTP/gRPC API client, environments, saved requests, and
  the request engine that **API-client automations** (§8) and the `api_run` node
  build on.
- `./daemon-http-api.md` — how the daemon's HTTP+WS surface, auth tokens, and
  RBAC feature grants work (the `Workflows:View/Edit` grants used here).
- `./agent-sessions.md` — agent sessions, which the `agent_prompt` / product /
  `canvas` nodes run as **openable** sessions (and which AI workflow generation
  uses).
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
