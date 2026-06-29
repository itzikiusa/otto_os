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
migrations under `crates/otto-state/migrations/` (incl. **0088** — versioning +
run→proof link), and the authoritative contracts `docs/contracts/api.md`
(§ "Workflow engine", § "API client", Wave-3/Wave-4 routes) and
`docs/contracts/ws.md` (Workflow run progress).

---

## 1. Two different things called "automations"

Otto has **two unrelated automation surfaces**. Keep them straight:

| Surface | What it is | Where | Model | Backed by |
|---|---|---|---|---|
| **Workflow engine** | A visual node-graph you build and run; nodes call agents, HTTP, DB, brokers, channels, swarm, etc. | `#/workflows` page | `Workflow` / `WorkflowRun` / `WorkflowGraph` | `workflows`, `workflow_runs`, `workflow_node_cache`, `workflow_triggers` tables |
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
crates/otto-state/migrations/0088_workflow_orchestrator.sql — workflow versioning + run→proof-pack link
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

---

## 4. Node types

The catalog is returned by `GET /workflows/node-types` (`node_catalog()` in
`workflow_engine.rs`). Each descriptor (`NodeTypeSpec`) carries
`kind, label, category, description, inputs, outputs, color, icon`. The UI's
"+ Node" palette is built directly from this list, and per-kind parameter forms
live in `WorkflowsPage.svelte`.

**Status legend:** **Real** = executes against a live subsystem · **Stub** =
registered and runnable but only emits a `{"stub": true, ... "not wired"}` marker
that passes downstream (it does **not** error the run).

| Kind | Label / category | Purpose | Params (UI form) | Status |
|---|---|---|---|---|
| `manual_trigger` | Manual Trigger / Triggers | Entry node; emits the run input. `inputs:0`. | — | **Real** |
| `agent_prompt` | Agent / AI | Runs a **headless agent turn** with a prompt; output `{ "reply": "..." }`. | `prompt`, `model` (optional) | **Real** |
| `http_request` | HTTP Request / Network | Calls an HTTP endpoint, captures response. | `method`, `url`, `body` (JSON) | **Real** |
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
| `swarm_task` | Swarm Task / AI | Enqueues a task in a running Agent-Swarm project (`todo` status; coordinator picks it up). | `swarm_id`, `project_id`, `title`, `description` | **Real** |
| `api_run` | API Run / Network | Executes an HTTP request **through the API-client engine** so env-var substitution + auth apply. | `method`, `url`, `body` (JSON) | **Real** |
| `product_analyze` | Product Analyze / Product | *Intended:* run a product-analysis agent on a story. | (raw JSON; not used) | **Stub — not wired** |
| `product_rewrite` | Product Rewrite / Product | *Intended:* run a product-rewrite agent on a story. | (raw JSON; not used) | **Stub — not wired** |
| `product_plan` | Product Plan / Product | *Intended:* run a product-planning agent on a story. | (raw JSON; not used) | **Stub — not wired** |
| `review_run` | Review Run / AI | *Intended:* start a code-review run on a workspace repo. | (raw JSON; not used) | **Stub — not wired** |

> **¹ `game_engine` / `verifier` are real but "scaffold" nodes.** They execute
> and produce usable structured output — unlike the four "not wired" stubs they
> do real work and `verifier` genuinely errors on a missing/trivial game file in
> the game path. But `game_engine` returns a **canned spec template** with the
> note *"Scaffold build: wire a real game engine here."* and `verifier`'s
> non-game path emits a scaffold *"replace with the real certifier."* report.
> They are intended as a game-pipeline scaffold (the built-in templates use
> `agent_prompt → game_engine → verifier`), pending a real external game engine.
> Treat their output as a scaffold, not a certified production artifact.

### Why the four stubs are stubs (verbatim from the code)
The catalog comments and the executor arms are explicit:

```rust
// product_analyze / product_rewrite / product_plan: otto-product does
// not expose a standalone synchronous call; a full run needs an active
// ProductRunHandle and the product_run cancellation registry.  Stubbed.
// review_run: otto-orchestrator's start_review requires a full ReviewsRepo
// call chain + background session plumbing ... not reachable from the engine.
```

Each of the four executes like this (so the run does **not** crash — it succeeds
with a marker that downstream nodes can branch/log on):

```rust
"product_analyze" => Ok((
    json!({ "stub": true, "node_kind": "product_analyze",
            "note": "product_analyze is not yet wired ..." }),
    vec!["product_analyze: stub — not wired".into()]))
```

The UI mirrors this: selecting one of these four in the inspector shows a yellow
hint — *"This node kind is registered but not yet wired in the engine. It will
run as a stub and pass a 'not wired' marker to downstream nodes."*

> **Heads-up on the `swarm_task`/`db_query`/`broker_peek`/`api_run` group:**
> these are real but depend on other features being set up — a saved DB-Explorer
> connection, a broker cluster, a running swarm/project, etc. A missing
> dependency causes the **node** to error (and downstream nodes to skip), not a
> stub marker. The api.md Wave-3 note lists all eleven non-core kinds together;
> only the four above are stubs.

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
| **schedule** | `{ cadence, every_min, at, weekday, last_run, enabled }` (same format as the swarm scheduler) | Cadence comes due: `interval` (every N min, default 60), `daily` (at `HH:MM` UTC), or `weekly` (weekday 0=Mon at `HH:MM` UTC). | `{ "trigger": "schedule" }` | **Implemented + unit-tested, but NOT spawned at boot today — see warning below.** |

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

> **⚠️ Schedule triggers do not auto-fire in the running daemon (verified gap).**
> `workflow_trigger_scheduler::start` (a 60-second supervisor that scans enabled
> `schedule` triggers, checks `is_due`, advances the `last_run` cursor, and
> spawns runs) is **fully written and has passing unit tests**, but is **not
> called anywhere in `crates/ottod/src/main.rs`** — only the *event* listener is
> started at boot. So in practice today: you can **create** a schedule trigger
> and the UI will show "daily at 09:00 UTC" etc., but **no run will be started on
> that cadence** until the scheduler is wired into `main` (mirroring how
> `swarm_scheduler::start`, the insights scheduler, and the CLI-update scheduler
> are launched). **Use webhook or event triggers for unattended runs today.**
> *(If you are reading this after a later release, re-check `main.rs` for a
> `workflow_trigger_scheduler::start(ctx.clone())` call before relying on it.)*

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
3. **From a template** — the sidebar lists built-in **game-pipeline templates**
   (`POST /workspaces/{wid}/workflows/from-template`), each chaining an agent
   design step into the game engine + verifier.

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
  selected node (e.g. `agent_prompt` → prompt + model; `http_request` →
  method/url/body). Unrecognized or future kinds fall back to a **raw-JSON
  params editor**.
- **Save**: edits set an "unsaved" badge; **Save** (`PATCH /workflows/{id}` with
  the new `graph`) persists. Running while dirty auto-saves first.

### Triggers
Top-bar **Triggers** toggles the `TriggersPanel`: add/enable/disable/delete
schedule, webhook, or event triggers (subject to the §5 caveats).

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
`RunSteps.svelte` renders each step's status, duration, logs, error, and the
**"work product"** (an agent `reply` string is rendered as text; everything else
as pretty JSON, copyable). The timeline strip at the top jumps between steps.

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
- **0014/0015** `api_client` base + `api_automations` (name, `steps_json`).

All ids are ULID strings; timestamps are UTC RFC-3339; rows cascade-delete with
their workspace (and triggers/runs/cache cascade with the workflow). Migrations
are **append-only** — never edit or renumber an existing one.

---

## 10. Capabilities & limitations (be explicit)

**What works today**
- Build graphs by description (AI), template, or hand; pan/zoom canvas editor.
- Topological execution with failure propagation, partial runs (from-here / only),
  per-node output caching, run cancellation, and a global wall-clock timeout.
- **Real** node kinds: `manual_trigger`, `agent_prompt`, `http_request`,
  `transform`, `delay`, `log`, `db_query` (read-only), `broker_peek`,
  `channel_notify`, `budget_gate`, `human_approval`, `swarm_task`, `api_run`.
- **Real-but-scaffold** node kinds: `game_engine`, `verifier` — they run and
  produce real structured output (and `verifier` errors on real game-file checks),
  but emit canned/scaffold specs pending a real external game engine.
- Live WS run progress + per-step logs/output/"work product".
- **Webhook** and **event** triggers fire in the running daemon.
- Human-approval pause/resume.
- API-client automations: ordered request runner with assertions + extracts.

**Known gaps / not done (do not assume these work)**
- **Four stub nodes** — `product_analyze`, `product_rewrite`, `product_plan`,
  `review_run` — run but only emit a `{"stub": true, ...}` "not wired" marker.
  Wiring them needs deeper coupling to otto-product's run handles and
  otto-orchestrator's review plumbing (per the code comments).
- **`game_engine` / `verifier` are scaffolds** — real, runnable, useful for the
  game-pipeline templates, but they emit canned specs / scaffold reports awaiting
  a real external game engine + certifier (see footnote ¹ in §4).
- **Schedule triggers do not auto-fire** — the cadence supervisor
  (`workflow_trigger_scheduler::start`) is implemented + tested but **not
  spawned at daemon boot**. Only webhook/event triggers actually start runs.
- **No expression/templating engine** between nodes beyond `channel_notify`'s
  simple `{key}` substitution and the predecessor-output input assembly rules.
- **No branching/conditionals/loops** as first-class nodes — execution is a plain
  topological pass; control flow is expressed only via which edges exist and
  upstream-failure skipping.
- **Agent-output caching is intentional but can surprise** — re-running a graph
  with unchanged params+input serves the **prior agent reply from cache**
  (duration `0ms`), not a fresh LLM call.
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
| A `product_analyze` / `product_rewrite` / `product_plan` / `review_run` node "succeeds" but does nothing | It's a **stub** (§4). Its output is `{"stub": true, ... "not wired"}`. Remove it or replace with a real node. |
| A scheduled run never starts | **Schedule triggers don't auto-fire** (§5 warning). Switch to a webhook or event trigger, or trigger the run via `POST /workflows/{id}/run`. |
| A node re-runs instantly with "Success (cached)" and stale output | Per-node cache hit (§3) — params + assembled input unchanged. Change a param to bust it, or accept the cached value. |
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
- `./agent-sessions.md` — agent sessions, which the `agent_prompt` node runs a
  headless turn of (and which AI workflow generation uses). *(If this file does
  not exist yet, see the Agents/Sessions material in `README.md` and
  `docs/contracts/api.md`.)*
- `./agent-swarm.md` — Agent Swarm, the target of the `swarm_task` node.
- `./channels-slack-telegram.md` — Slack/Telegram integrations used by
  `channel_notify`.
- `./message-brokers.md` — Kafka clusters used by `broker_peek`.
- `./connections-ssh-sftp.md` / Database Explorer — connections used by `db_query`.
- Contracts: `docs/contracts/api.md` (§"Workflow engine", §"API client",
  Wave-3/Wave-4) and `docs/contracts/ws.md` (Workflow run progress).
