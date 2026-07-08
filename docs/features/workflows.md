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
> `product_publish` nodes. A workflow can also carry **standing `instructions`**
> (free-text guidance every step follows, versioned alongside the graph), and a
> `prepare_context` node does app-side Jira-ticket fetching into the run's
> context dir. **All trigger kinds fire unattended now** — webhook, event, and
> schedule (the cadence scheduler is spawned at daemon boot, with cron +
> IANA-timezone parity), plus a **`chat`-kind trigger** (channel bindings,
> evaluated live by the channels Bridge, not polled). A structured
> `Action: Workflow` chat message, the simplified **`run <name>: <prompt>`**
> command, or a channel binding can all start a run by name — and such a run now
> **streams live per-step progress back into the same chat thread** (start line,
> per-step ▶/✅/❌, and a `🔍` review block for the fix→review loop), flushed
> before the final result (the run's `final-output.md` deliverable when it
> produced one, else `summary.md`). Agent steps can inline named **skills**;
> `review_run` fans out the full multi-provider × multi-lens **PR-review engine**
> and scores it, and `git_pr` can **auto-open the PR** once the review passes
> (the review is the approval — no human gate). The separate **API-client
> "Automations"** surface (a multi-step saved-request runner) is also fully
> functional. A couple of honest caveats remain (the game `game_engine`/`verifier`
> are scaffolds; agent output is cached) — stated inline; do not assume full
> parity with mature n8n/Zapier engines.

The doc is grounded in the code in `crates/otto-server/src/workflow_engine.rs`,
`crates/otto-server/src/workflow_context.rs` (run context files),
`crates/otto-server/src/workflow_prepare.rs` (`prepare_context`),
`crates/otto-server/src/routes/workflows.rs`,
`crates/otto-server/src/workflow_trigger_scheduler.rs`,
`crates/otto-server/src/workflow_chat.rs`, `crates/otto-core/src/expr.rs`,
`ui/src/modules/workflows/`, `ui/src/modules/api/AutomationsView.svelte`, the
migrations under `crates/otto-state/migrations/` (incl. **0089** — versioning +
run→proof link, **0096** — standing `instructions`), and the authoritative contracts `docs/contracts/api.md`
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
- The graph is executed in **topological order** (`topo_order`). A cycle makes
  the whole run fail immediately with an error (no nodes execute).

### Standing instructions (`workflow.instructions`)
A `Workflow` carries a second free-text field alongside `description`:
`instructions` — standing guidance every run/step should follow "by the letter"
(e.g. spec-framework conventions, house style, things to never do), distinct
from `description` (a one-line summary shown in lists). It is:
- **Edited in the UI** via the canvas top-bar's collapsible **Instructions**
  panel (an "unsaved" badge + **Save** button — same pattern as the graph).
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
| `agent_prompt` | Agent / AI | Runs an agent turn as a **real, openable session**; output `{ "reply", "session_id" }`. `skill`/`skills` inline a skill body ahead of the prompt (see *Per-step skills*). | `prompt`, `provider?` (empty ⇒ the run's resolved default agent), `model?`, `skill?`, `skills?` | **Real** |
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
| `human_approval` | Human Approval / Flow | **Pauses the run**; sets `waiting_approval=1` and polls until an operator approves/rejects via the approve endpoint (or times out at `NODE_AGENT_TIMEOUT`). | `prompt` | **Real** |
| `condition` | Condition / Flow | Evaluates an `expr` on its input; outputs `{ result, value }` merged onto the input. Pair with **edge conditions** to branch. | `expr` (default `true`) | **Real** |
| `loop` | Loop (Until) / Flow | Bounded **iterate-until**: re-runs inner `steps[]` until `until` holds or `max_iterations`. Reuses inner-node execution; threads run-level keys + prev-step output to each step. Output `{ iterations, satisfied, last, history }`. No nested loops. | `max_iterations` (1–10, default 3), `until` (expr), `steps[]` ({kind,name,params,retry}), `continue_on_error` | **Real** |
| `swarm_task` | Swarm Task / AI | Enqueues a task in a running Agent-Swarm project (`todo` status; coordinator picks it up). | `swarm_id`, `project_id`, `title`, `description` | **Real** |
| `api_run` | API Run / Network | Executes an HTTP request **through the API-client engine** so env-var substitution + auth apply. | `method`, `url`, `headers`, `body` | **Real** |
| `product_analyze` | Product Analyze / Product | Runs a real single-agent turn (the **`grill`** lens) over the story's built context; outputs `{ story_id, analysis, session_id }`. | `story_id`, `instruction?` | **Real** ² |
| `product_rewrite` | Product Rewrite / Product | Rewrites the story (**`jira-story-writer`**); outputs `{ story_id, body_md, session_id }`; `persist:true` saves a `suggested` product version. | `story_id`, `persist?`, `instruction?` | **Real** ² |
| `product_plan` | Product Plan / Product | Breaks the story into a plan (**`story-task-breakdown`**); outputs `{ story_id, plan_md, session_id }`; `persist:true` saves a `plan` version. | `story_id`, `persist?`, `instruction?` | **Real** ² |
| `product_publish` | Product Publish / Product | Publishes a story as a Confluence **RFC** or a **Jira** issue. **`dry_run` defaults true** (no-op note); a real publish needs `account_id` (+ `project_key`/`space_key`). | `kind` (`rfc`/`jira`), `dry_run`, `account_id`, … | **Real** |
| `review_run` | Review Run / AI | Runs the **PR-review engine** (multi-provider × multi-lens reviewer agents + a scoring summarizer), polls to completion, emits a **0–100 `score`** (`100−20×blocking−5×advisory`), optional `goals` assessment blended in, `passed = score≥threshold && status==done`. `require_pass:true` **errors** the step when below threshold. Output also carries `blocking`/`advisory`/`findings`/`providers`/`lenses`. See *`review_run`, gating & auto-PR* below. | `repo_id`, `base` (default `main`), `providers[]`, `lenses[]` (alias `skills[]`), `threshold` (default 80), `require_pass`, `await`, `timeout_s`, `goals[]` | **Real** |
| `canvas` | Canvas Diagram / Product | Asks an agent for a **mermaid/excalidraw** diagram and writes it under the data dir (`workflow-canvas/{run}/{node}.{ext}`); output `{ scene_id, path, diagram, … }`. | `prompt`, `mode` (`mermaid`/`excalidraw`), `provider?`, `model?` | **Real** |
| `git_pr` | Git PR / Network | **Drafts** a pull request for a repo branch — the title/description are crafted by an agent (the node's `provider`/`model`, empty ⇒ run's resolved default; not the claude-only orchestrator). Default is draft-only (`opened:false`); **`open:true`** actually OPENS the PR on the remote (outward-facing, per-step opt-in) — gate it on the incoming edge (e.g. the review passing). | `repo_id`, `base` (default `main`), `open` (default `false`), `worktree_path?`, `provider?`, `model?` | **Real** |

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

### Prepare relevant data (`prepare_context`)
A dedicated node (`workflow_prepare.rs`) for the common "go read the ticket
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
   default `claude`) over the gathered context as a real, openable session; its
   `reply`/`session_id`/`working_directory` merge into the output.

`prepare_context` is the **only** kind excluded from the per-node output cache
(§3 *Run-time graph behavior*) — a re-run always re-fetches, since the ticket
can have changed since the last run. The `ui-test-authoring` and
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

### `review_run`, gating & auto-PR
`review_run` drives the **same multi-agent engine as a PR review**: it fans out one
reviewer agent per `providers[]` × `lenses[]` pair (e.g.
`providers:["claude","codex"]` × `lenses:["correctness-review","security-review",
"test-review"]`; `skills[]` is accepted as an alias for `lenses[]`), and a
summarizer consolidates + scores the findings into a **0–100 `score`** with
`passed`, `threshold` (0–100, default 80), `blocking`, `advisory`, `findings`
(brief strings), `providers`, and `lenses` in the output. When **both** `providers`
and `lenses` are empty it falls back to the stored/default PR-review config.
`require_pass:true` makes the step **ERROR** when the score is below `threshold`, so
any downstream step is error-skipped.

The review's comparison base resolves in order: node `base` param → input
`base` → the run's ambient base → the repo's **detected default branch** — and
a named ref that doesn't exist locally falls back through `origin/<base>` to
the default instead of failing. The node publishes the **resolved** branch in
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
type TriggerKind = 'schedule' | 'webhook' | 'event' | 'chat';
interface WorkflowTrigger { id; workflow_id; kind; spec: object; enabled; created_at }
```

| Kind | Spec | Fires when… | Run input | Wired & firing in the daemon? |
|---|---|---|---|---|
| **webhook** | `{ token }` (32-byte URL-safe token auto-generated server-side on create) | An external system calls `POST /workflows/{id}/webhook/{token}`. The token **is** the credential — no bearer auth required. | The request body (JSON), or `null`. | **Yes** — handler `webhook_trigger` spawns the run. |
| **event** | `{ event_kind, filter_json? }` | A daemon event whose mapped name equals `event_kind` fires on the event bus. | `{ "trigger": "event", "event_kind": "..." }` | **Yes** — `spawn_workflow_event_trigger_listener` is started at daemon boot (`ottod` main, "workflow event-trigger listener started"). |
| **schedule** | `{ cadence, every_min, at, weekday, expr, timezone, last_run, enabled, prompt? }` (the **shared cadence** format — same as Scheduled Tasks; `prompt` is new — see *Prompts & chat bindings*) | Cadence comes due: `interval` (every N min, default 60), `daily` (at `HH:MM`), `weekly` (weekday 0=Mon at `HH:MM`), or **`cron`** (`expr`, 5-field). All interpreted in the spec's IANA **`timezone`** (default UTC). | `{ "trigger": "schedule" }`, plus `"prompt"` when `spec.prompt` is set. | **Yes** — `workflow_trigger_scheduler::start` is started at daemon boot (`ottod` main, "workflow schedule-trigger scheduler started"). |
| **chat** | `{ channel: "slack"\|"telegram", chat: "<id>", thread?: "<ts>", mention_only?: bool }` | Any inbound message that matches the binding (channel/chat exact, thread pinned or open, `@mention` if `mention_only`) — see *Prompts & chat bindings* below. | `{ trigger: "chat", origin_workspace_id, channel, chat, thread, user, prompt, msg, raw }` | **Yes** — evaluated **live** by the channels `Bridge` on every inbound message (not polled, unlike the other three kinds). |

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
rest of the graph — and replies in-thread (*"🚀 Started workflow … (run `…`)"*).

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
which the chat message doesn't carry — either run those templates from the UI
**Run…** editor (it pre-fills `repo_id`/`base`/`goals`) or have the first
"prepare relevant info" node resolve the repo from `working_directory`.

**Live progress (streamed while the run executes).** A run started from a chat
**streams brief, well-formatted progress lines back into the same thread** as it
goes (Slack/Telegram). A single pump task posts them in order, so the engine never
blocks on chat latency:

- a **run-start** line — `🚀 <name> started — N step(s) queued.` (plus `Goals: …`
  when the trigger carried goals);
- per **meaningful** step, `▶ <step> started` then `✅ <step> done (<dur>)` with a
  short summary of the work product (or `❌ <step> failed — <err>` on failure);
- for the **fix→review loop**: a `🔁 Iteration N/M` header, the inner sub-steps as
  `› ▶ …` / `› ✅ …` / `› ❌ …`, and a review block
  `🔍 Review #N done — score X/100 (pass ≥ T) — ✅ passed` / `⚠️ below threshold`
  followed by a bulleted findings list (or *"Findings: none 🎉"*).

Structural/plumbing nodes (`log`, `transform`, `delay`, `condition`,
`manual_trigger`) are intentionally **not** announced, to keep the thread readable;
`review_run` is also excluded from the generic ▶/✅ lines because it streams its own
richer `🔍` block. All streamed lines are **flushed before the final result is
delivered** (below), and every line is redacted. Manual UI runs and webhook-only
triggers **do not stream** (there is no chat target). The target is the trigger
origin (`channel`/`chat`/`thread`), or an explicit `result_chat` (+
`result_channel` / `result_thread`) override; the integration token is resolved
**per-workspace** (from the origin/trigger workspace — workflows themselves are
global).

**Result delivery (done/failed + deliverable).** When a run that was started from
a chat **finishes**, the engine replies in the **same channel + thread** it was
triggered from with a brief status (`✅/❌` + `N/M steps ok · failed · skipped`,
the review score if any, the proof-pack id) and attaches **one file**: on a
successful run that produced one, that's `final-output.md` — the run's actual
deliverable (see *Final output* in §7); otherwise it's the always-generated
`summary.md` (every step, its status/duration/attempts, and a peek at each
output). Both are redacted the same way before they leave the machine. This is
origin-driven (`deliver_run_result` reads `channel`/`chat`/`thread` from the run
input), so manual UI runs don't post anything. To also POST the result to an
external system, include a `result_webhook` (or `callback_url`) in the run input —
it's delivered through the SSRF-guarded webhook path, with the same
final-output/summary choice. Cancellations and time-outs report too (always with
`summary.md` — only a `success` run can have a `final-output.md`).

### Prompts & chat bindings
Two more ways to start a run by chat, both handled by `otto-server::workflow_chat`
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
**X**…"* without starting a run; the **bare** `run` keyword silently falls
through to normal chat (bare "run" reads too much like ordinary English to
hijack a message that wasn't meant for it).

**Channel bindings — pin a workflow to a chat.** A `chat`-kind `WorkflowTrigger`
(add one in the **Triggers** panel → **Chat binding**) starts its workflow on
**every** matching inbound message, no keyword required — good for a
workflow that IS the channel's purpose (e.g. a #test-requests channel where
every message is a test-writing ask). Spec: `{channel, chat, thread?,
mention_only?}`. `channel` + `chat` must match exactly; an absent `thread` in
the spec matches any thread, a present one requires an exact match (a
thread-pinned binding wins over an unpinned one when both match); `mention_only`
(default `false`) requires the message to contain a Slack `<@…>` mention. On a
match the run input is `{trigger:"chat", origin_workspace_id, channel, chat,
thread, user, prompt, msg, raw}` — `prompt`/`msg` are the mention-stripped text.

**Loop guard.** Slack drops any event carrying a `bot_id` (including the nested
`message` of a `message_changed` edit) before it reaches the bridge; Telegram's
`getUpdates` long-poll structurally never returns the bot's own sends — so
neither adapter needs bot-detection logic to avoid the bot triggering itself.
The channel-binding path (only, since it has no keyword to require) adds a
**second, defensive** guard on top: it never treats a message starting with the
bot's own ack prefix (`"🚀 Started workflow"`) as a binding trigger, so a
binding can't retrigger itself off its own start-line reply.

> **Ordering note.** `implement-feature`/`po-lifecycle`-style templates that need
> a `repo_id` still don't get one from a bare chat message or binding (chat
> carries `msg`/`prompt`/`jira_ticket` only) — run those from the UI **Run…**
> editor, or have the first node resolve the repo from `working_directory`, same
> as the `Action: Workflow` worked example above.

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
       a `fix → review_run` **loop** (`until last.passed == true`, lenses
       `correctness-review`/`test-review`) → **`git_pr` with `open:true`**, reached
       only by an edge gated on `output.satisfied == true` — so the PR **opens
       automatically once the loop passes** (the review is the approval; the old
       `human_approval` node was removed).
     - **`implement-feature`** — `product_analyze` → prepare → implement → the same
       review loop (lenses `correctness-review`/`security-review`) → **auto-opened
       `git_pr`** on pass (same `output.satisfied == true` edge gate; no approval
       node).
     - **`po-lifecycle`** — *PO discovery → RFC/Jira* (**unchanged**): discovery draft
       → `canvas` diagram → `human_approval` → `product_rewrite` → `human_approval`
       → `product_publish` (RFC, dry-run). They expect the run **input** to carry `repo_id` (and
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
  method/url/body; `review_run` → repo_id/base/threshold/goals). Kinds without a
  bespoke form yet — including `prepare_context` as of this writing — fall back
  to a **raw-JSON params editor**.
- **Save**: edits set an "unsaved" badge; **Save** (`PATCH /workflows/{id}` with
  the new `graph`) persists. Running while dirty auto-saves first.
- **Instructions**: a separate top-bar toggle opens the collapsible standing-
  instructions panel (its own "unsaved" badge + **Save**, independent of the
  graph editor) — see *Standing instructions* in §3.

### Triggers
Top-bar **Triggers** toggles the `TriggersPanel`: add/enable/disable/delete
schedule, webhook, event, or **chat binding** triggers — all four fire
unattended in the running daemon (see §5; the chat kind is evaluated live by
the channels Bridge on every message, not polled like the other three). The
**Chat binding** form (channel, chat id, optional thread, "@mention only")
maps directly onto the `{channel, chat, thread?, mention_only?}` spec.

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

Every run owns `<data_dir>/workflow-context/<run_id>/` (`workflow_context.rs`)
— browsable in the run view under **Context files** (same tree + viewer as the
agent Files panel, scoped to this run). Every write here is best-effort — a
failure logs a warning and the run continues; context files never fail a node:

```
instructions.md                 # workflow.instructions, verbatim — only when non-empty
prompt.md                       # this run's ask, verbatim — only when a prompt exists
run-brief.md                    # mission brief: trigger, mission, repos table, planned steps
                                 #   (renamed from the old wf-<run_id>-instruction.md)
repos.json                      # live registry of repos/branches — only when repos declared
jira-<KEY>.md                   # a prepare_context node's fetched ticket (or fetch-failed notice)
step1-gather-info.md            # per-step handoff summary
step1-gather-info.output.json   # the step's raw output (5 MiB cap, truncation marker if hit)
step3-review-iter2.md           # loop inner steps, per iteration
final-output.md                 # on a successful run: the deliverable (see Final output below)
```

Steps hand context to each other **through files, not truncated prompt
text**: each agent-backed step is pointed at the directory and told to read,
**in order**, `instructions.md` → `prompt.md` → `run-brief.md` → `repos.json`
→ the prior `step*.md` files it needs (each named only when it exists — a run
with no standing instructions never tells a step to go read a file that isn't
there), then write its own `step{N}-{name}.md` summary before finishing. If it
doesn't, the engine writes a full-fidelity fallback — an agent's reply lands
**untruncated**, `review_run` files carry the score breakdown + findings, and
failed steps (including failed loop iterations) leave their error in the
trail. A retried step only trusts a summary written during the **winning**
attempt (a stale file left by an earlier failed attempt is replaced). The
inline `[input data]` excerpt in prompts remains as a quick glance; the files
are the complete channel.

Chat-triggered runs attach each meaningful step's `.md` handoff file to its
per-step progress message (success *and* failure, loop iterations included) —
the thread carries the brief, the attachment the full detail. Attachments are
redacted like `summary.md` and capped at 1 MiB (the full file always remains
in the run's context directory).

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

The run view's **Final output** panel (a sandboxed `<iframe>`, shown only when
`run.status === 'success'` and the run has a `context_dir`) fetches
`final-output.md` once per run and renders it — it just stays hidden when the
file doesn't exist. Chat/webhook delivery prefers it too: see *Result delivery*
in §5. This is why the `ui-test-authoring`/`api-acceptance-test-authoring`
templates' last step is an explicit "Final report" `agent_prompt` (§6) — it's
what ends up as the deliverable both in the UI panel and in the chat/webhook
attachment.

### Statuses
```ts
RunStatus  = 'pending' | 'running' | 'success' | 'error' | 'canceled';
NodeStatus = 'pending' | 'running' | 'success' | 'error' | 'skipped';
```
A run is `error` if **any** node errored ("one or more nodes failed"). Cached
nodes show as `success` with a "Success (cached)" log line.

### Live progress over WebSocket (`workflow_run_updated`)
The engine emits `Event::WorkflowRunUpdated` on the shared event bus at **every
node transition** (start, finish/cached, skip, session spawn), on the
**human-approval pause** and the **approve/reject decision**, on **cancel**,
and at **run completion**:

```json
{ "type": "workflow_run_updated",
  "workspace_id": "<Id>", "run_id": "<Id>",
  "status": "running|success|error|canceled",
  "node_id": "<node_id | null>",
  "rev": 7, "node": { "node_id": "…", "status": "…" },
  "nodes_done": 2, "nodes_total": 5, "waiting_approval": false }
```
- `rev` is the run's monotonic revision (also on `WorkflowRun.rev`); `node` is
  the changed node's full state (omitted when > 32 KiB serialized).
- The UI (`events.svelte.ts` → `workflowRunBus.apply()` → `WorkflowsPage`)
  **merges the change into the viewed run in place** when the event carries the
  node + the contiguous rev — no network at all. On a rev gap, a run-level
  event, or a missing payload it converges with **one single-flight,
  rev-guarded** `GET /workflow-runs/{id}`. The viewed run object is **never
  replaced wholesale**, so an expanded step, the timeline selection, and scroll
  positions all survive every update, and stale/out-of-order responses can
  never regress the view.
- A **2.5s fallback poll** (while the viewed run is non-terminal, uncapped)
  keeps the UI live if the WS connection is unavailable. The sidebar "Running"
  list updates in place from the event's `nodes_done`/`nodes_total`/
  `waiting_approval` (full refetch only for unknown run ids).

### Inspect
`RunSteps.svelte` renders each step's status, duration (a **live elapsed timer**
while the step runs, from `NodeRunState.started_at`), `attempts` (when a retry
policy ran), logs (including `⚠` typed-output warnings and `edge → … not taken`
branch lines), error, and the **"work product"** (an agent `reply` string is
rendered as text; everything else as pretty JSON, copyable). Step expansion is
**user-owned**: a step that errors auto-opens once for visibility, but a manual
open/collapse always wins afterward — live updates never fight it. Steps that
spawned **openable sessions** (`NodeRunState.sessions` — agent / product /
canvas / loop turns) link to them so you can watch/inspect the agent **while the
step runs**; the timeline strip at the top jumps between steps.

### Run → Proof Pack
On completion the run links a **Proof Pack** (`WorkflowRun.proof_pack_id`)
assembling each node's output as evidence (a `log` artifact carrying the node's
pass/fail; each `human_approval` becomes an `approval` artifact recording the
approver). The run also records the workflow `version` it executed
(`WorkflowRun.workflow_version`).

### Human-approval pause
When a run hits a `human_approval` node it pauses (`waiting_approval` +
`approval_node_id` ride both the run row and a `workflow_run_updated` pause
event, so the banner and the sidebar ⏸ badge appear immediately); the page
shows a banner — *"Run paused — waiting for approval at &lt;node&gt;"* — with
**Approve** / **Reject** (→ `POST /workflow-runs/{id}/approve` with
`{node_id, approved}`). Approve resumes the run (records `approved_by`; the
engine's resume poll runs every 2s); reject errors the node
("rejected — &lt;note&gt;"). Both decisions re-emit the event so open views
drop the banner at once. If no decision arrives within the node timeout, the
node errors ("timed out").

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

Trigger / webhook / approval routes (api.md Wave-3 additions):

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
- **0096** `workflows.instructions` + `workflow_versions.instructions` (both
  `TEXT NOT NULL DEFAULT ''`) — the standing-instructions field (§3).
- **0097** widens `workflow_triggers.kind` CHECK to `schedule|webhook|event|chat`
  (table rebuild — SQLite can't `ALTER` a CHECK) so `chat`-kind channel
  bindings can be persisted.
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
  or hand; pan/zoom canvas editor.
- Topological execution with failure propagation, partial runs (from-here / only),
  per-node output caching, run cancellation, and a global wall-clock timeout.
- **Branching & loops:** edge `condition`s + a `condition` node (if/else, with clean
  branch-skip vs failure-poison semantics), and a bounded `loop` (iterate-until)
  that reuses inner-node execution — all driven by the safe `otto_core::expr`
  language (also available as `{{ }}` templating).
- **Retry/backoff** per node (`retry`), and **typed outputs** (warn-only validation
  against each kind's `output_schema`).
- **All node kinds are real:** `manual_trigger`, `agent_prompt`, `prepare_context`
  (app-side Jira fetch + optional analysis agent), `http_request`,
  `transform`, `delay`, `log`, `db_query` (read-only), `broker_peek`,
  `channel_notify`, `budget_gate`, `human_approval`, `condition`, `loop`,
  `swarm_task`, `api_run`, and the **now-wired** `product_analyze`,
  `product_rewrite`, `product_plan`, `product_publish`, `review_run` (multi-provider
  × multi-lens PR-review engine, 0–100 scored, `require_pass`), `canvas`, `git_pr`
  (PR draft, or `open:true` to auto-open on a passing review).
- **Visible sessions per step** (openable while running) and a **Proof Pack** linked
  to each completed run.
- **Versioning:** graph snapshot history with view + restore (append-only), now
  covering **standing `instructions`** alongside the graph.
- Live WS run progress + per-step logs/output/"work product", plus a **Final
  output** panel that renders the run's `final-output.md` deliverable on success.
- **All trigger kinds fire in the running daemon** — webhook, event, and schedule
  (spawned at boot; cron + IANA timezone via the shared cadence engine), plus a
  **`chat`-kind** channel binding evaluated live on every inbound message.
- Three ways to start a run by chat: the structured `Action: Workflow` message,
  the simplified **`run <name>: <prompt>`** command, or a channel binding — all
  **stream live per-step progress** (▶/✅/❌ + a `🔍` review block) back into the
  thread, then deliver `final-output.md` (or `summary.md` if the run made none).
  **Convert** a scheduled task into a workflow + schedule trigger.
- **Per-step skills** (`skill`/`skills` on an `agent_prompt` step inline a skill body
  ahead of the prompt) and **threshold-gated auto-PR** (`review_run` →
  `git_pr open:true` via an `output.satisfied == true` edge — the review is the
  approval, no human gate).
- Human-approval pause/resume.
- API-client automations: ordered request runner with assertions + extracts.

**Caveats (still honest about the edges)**
- **`game_engine` / `verifier` are scaffolds** — real, runnable, useful for the
  game-pipeline templates, but they emit canned specs / scaffold reports awaiting
  a real external game engine + certifier (see footnote ¹ in §4).
- **`product_publish` defaults to a dry run** — a real RFC/Jira publish requires
  `dry_run:false` + an Atlassian `account_id` (and `project_key`/`space_key`).
- **`git_pr` defaults to draft-only** (`opened:false`) — set **`open:true`** (gate it
  on the incoming edge so it only fires when the review passed) to actually open the
  PR on the remote; the `write-tests`/`implement-feature` templates do exactly this
  on a passing fix→review loop.
- **Wired nodes have prerequisites** — `db_query`/`broker_peek`/`swarm_task`/
  `review_run`/`git_pr`/`product_*` need their backing connection/cluster/swarm/repo/
  story set up, or the node errors (and downstream active-path nodes skip).
- **Agent-output caching is intentional but can surprise** — re-running a graph
  with unchanged params+input serves the **prior agent reply from cache**
  (duration `0ms`, `attempts:0`), not a fresh LLM call.
- **Typed-output validation is warn-only** — schema mismatches log `⚠` but never
  fail a run; `params_schema` is currently unpopulated (UI hint only).
- API-client automations have **no scheduler** — run-on-demand only.
- **`prepare_context` has no dedicated inspector form yet** — configure it via
  the canvas's raw-JSON params editor (§6).

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
- **Chat-started runs (`Action: Workflow`, `run <name>: <prompt>`, or a channel
  binding) run on the channel-trust model.** A message in a configured
  Slack/Telegram/webhook channel can start a run by name (the run acts as the
  workflow's `created_by`, falling back to a synthetic "Workflow" user for
  system-initiated runs). Anyone who can post to a wired channel can start any
  workflow in that workspace by name — treat channel access as run-start
  capability. This is unaffected by the `workflow_triggers.kind` `CHECK` bug
  (§10) — the message-based paths never touch that table.
- **`prepare_context` fetches through the run's resolved Jira account** —
  `params.account_id`, else the run user's own account, else any Jira account
  configured on the daemon (§4). The fetched ticket (description, comments,
  attachments list) lands in the run's context dir and, on a chat-started run,
  can end up in the chat thread via a step's `.md` attachment or
  `final-output.md` — treat a bound channel's audience as the ticket's
  effective audience for that run.
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
| Run UI stops updating but isn't finished | WS dropped → the uncapped 2.5s fallback poll keeps the viewed run converging; the run still runs server-side. If the view is genuinely frozen, re-open the run or reload. |
| Webhook returns 401 | Token doesn't match an **enabled** webhook trigger on that workflow id. Re-check the token / re-enable the trigger. |
| Run "exceeded the N-minute time limit" | The global wall-clock fired; the graph (often an `agent_prompt` chain) is too long. Split it or reduce work. |
| Approve/Reject button does nothing | The run must actually be paused at a `human_approval` node (`waiting_approval`), and you need `Workflows:Edit`. |
| API-client automation step fails on a 2xx | It has a failing assertion. With **no** assertions a step passes on any 2xx; check the assertion `desc` in the report. |
| `prepare_context` node errors "required Jira fetch failed" | `params.require: true` and the fetch failed (no matching Jira key, no configured Jira account, or the API call errored) — check `jira-<KEY>.md` in the run's context dir for the reason, or drop `require`. |
| A run finished but the **Final output** panel is empty/hidden | The run isn't `success`, or every successful step was a utility kind (`log`/`delay`/`channel_notify`/`budget_gate`/`human_approval`/`manual_trigger` — §7 *Final output*) — add a content-bearing step (e.g. a closing `agent_prompt` "report" node) if you want a deliverable. |

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
