# Mission Control (the unified work graph)

> _New guide — Mission Control / the work graph is a recent addition (crate
> `otto-workgraph`, projector `otto-server::workgraph_projector`, UI
> `ui/src/modules/mission-control`, migration `0083`)._

> Otto runs many kinds of agentic work — agent sessions, swarm projects, goal loops,
> workflow runs, PR reviews, product stories, pull requests, channel triggers. **Mission
> Control** is the one place that shows *every* one of them as a single, traceable unit on a
> common spine: what it is, who/what started it, its normalized status, what it cost, how
> risky it is, what it's linked to, what evidence exists, and whether it needs human
> approval.

This document is the end-user and operator reference for Mission Control: the concept, the
node/edge model, how the projector builds the graph without rewiring any module, a full
walkthrough of every view and control, the real REST/WS contract, capabilities and limits,
security, and troubleshooting. It describes what the code actually does — the
`otto-workgraph` crate, the projector in `crates/otto-server/src/workgraph_projector.rs`,
the storage in `crates/otto-state/src/workgraph.rs`, the routes in
`crates/otto-server/src/routes/workgraph.rs`, and the UI in
`ui/src/modules/mission-control/`.

---

## 1. What it is

Mission Control is a **unified, read-mostly work graph** over every agentic activity in a
workspace. It does **not** replace the per-feature pages — it is a cross-cutting projection:

- A **`work_item`** is a projection of an authoritative module row (a session, a swarm
  project, a goal loop, a workflow run, a review, a product story, a PR, or a channel
  trigger), keyed by the natural key `(workspace_id, kind, source_id)`.
- **`work_edges`** link items (e.g. a review *reviews* a PR), **`work_events`** are an
  append-only audit trail, **`work_artifacts`** are evidence/trace, and **`work_approvals`**
  are human gates.
- Items are **materialized by a projector** that subscribes to the daemon's existing event
  bus plus a periodic reconcile/backfill — **no module is rewired** to feed it.

The design thesis: every activity
type already broadcasts an `Event::*`, so a single subscriber can project those events into
work items. This is the lowest-coupling option — re-derivable, no new failure surface in hot
paths, single source of truth per fact (cost/status/title live in the owning repos; the
graph denormalizes a snapshot and links back for detail).

### Where it lives

| Layer | Location |
|-------|----------|
| Domain enums + row/DTO types + `from_source` normalization | `crates/otto-state/src/workgraph.rs` (`WorkGraphRepo`) |
| Service (persist mutation + audit event + live broadcast) | `crates/otto-workgraph/` (`lib.rs`, `service.rs`, `normalize.rs`) |
| Projector (event bus → service, backfill, cost enrichment) | `crates/otto-server/src/workgraph_projector.rs` |
| HTTP routes (summary / items / graph / detail / patch / edges / approvals / backfill) | `crates/otto-server/src/routes/workgraph.rs` |
| Policy class + WS scope | `crates/otto-server/src/policy.rs` (`Require(MissionControl, …)`) |
| Persistence (5 tables) | migration `crates/otto-state/migrations/0083_workgraph.sql` |
| Live signal | `Event::WorkGraphUpdated` → `work_graph_updated` (`docs/contracts/ws.md`) |
| Feature flag | `Feature::MissionControl` (snake `mission_control`) |
| UI section + views | `ui/src/modules/mission-control/` (`MissionControlPage`, `WorkItemList`, `WorkGraphView`, `WorkItemDetail`, `lib.ts`) |
| Contracts (authoritative) | `docs/contracts/api.md` (#126–#134), `docs/contracts/ws.md` (`work_graph_updated`) |

Open it from the app's **Mission Control** section (sidebar icon `radar`).

> **Two surfaces share the "Mission Control" name — don't confuse them.** This guide is the
> **work graph** (the sidebar page, the `mission-control` UI module, routes under
> `/workspaces/{wid}/workgraph/…`, gated by `Feature::MissionControl`). There is an
> **older, separate** 6-bucket "work-queue" endpoint (`GET /workspaces/{id}/mission`,
> `crates/otto-server/src/routes/mission.rs`, gated by `Feature::Agents`) that aggregates
> needs-you / working / review-ready / waiting / failed / budget-warn rows on the fly. It is
> a different API and is **not** what the Mission Control page renders. Everything below is
> the work graph.

---

## 2. Concepts & data model

Five workspace-scoped tables (migration `0083_workgraph.sql`), all `ON DELETE CASCADE` from
`workspaces`.

### The eight kinds (`WorkKind`)

The graph unifies exactly these kinds — one per Otto activity type (UI labels in
`ui/src/modules/mission-control/lib.ts`):

| `kind` | UI label | Projected from | `source_id` is |
|---|---|---|---|
| `session` | Session | a user-driven agent PTY session | session id |
| `swarm` | Swarm Project | a swarm **project** | project id |
| `goal_loop` | Goal Loop | a goal loop | loop id |
| `workflow` | Workflow Run | a workflow **run** | run id |
| `review` | PR Review | a code-review run | review id |
| `product_story` | Product Story | a product story | story id |
| `pr` | Pull Request | a PR a review reviews | `"<repo_id>:<pr_number>"` |
| `external_trigger` | External Trigger | a channel-spawned session (Slack/Telegram/webhook) | session id |

### The work item (`work_items`)

`{ id, workspace_id, kind, source_id, title, goal, status, owner, owner_kind, repo_id,
branch, cost_so_far, risk_level, result_summary, context_summary, started_by_id,
last_event_at, created_at, updated_at }`, unique on `(workspace_id, kind, source_id)`.

- **`status`** — the **normalized** lifecycle (`WorkStatus`): `pending`, `running`,
  `waiting`, `blocked`, `succeeded`, `failed`, `cancelled`, `done`. Every source status maps
  onto this via `WorkStatus::from_source(kind, raw)` (per-kind tables; an **unknown** value
  maps to `running`, so an item never silently drops out of the active view).
- **`owner_kind`** (`WorkActor`) — who/what started it: `user`, `agent`, `system`,
  `integration`.
- **`risk_level`** (`RiskLevel`, the "policy" axis) — `low`, `medium`, `high`, `critical`.
  Derived on **create** from kind + title (`otto_workgraph::risk`): a sensitive keyword in
  the title (`security`, `payment`, `auth`, `secret`, `credential`, `wallet`, `compliance`,
  `fraud`, `prod`/`production`, `deploy`, `migration`) → `high`; otherwise `medium` for
  goal-loop/swarm/pr/session/external-trigger/review, `low` for workflow/product-story. It is
  **human-governable** thereafter (PATCH, up to `critical`).
- **`cost_so_far`** — a USD snapshot (refreshed on-demand for sessions/triggers; see §3).
- **`context_summary`** — a best-effort "what context was used" line per kind.

### Edges, events, artifacts, approvals

- **`work_edges`** — directed links with a `relation` (`EdgeRelation`): `spawned`,
  `depends_on`, `fixes`, `reviews`, `verifies`, `blocks`, `belongs_to`. Idempotent on
  `(from, to, relation)`.
- **`work_events`** — the append-only audit trail. `event_type` ∈ `created`,
  `status_changed`, `tool_call`, `context`, `progress`, `result`, `approval_requested`,
  `approval_decided`, `artifact_added`, `edge_added`, `note`; each carries an `actor` and a
  JSON `payload`.
- **`work_artifacts`** — evidence/trace (`ArtifactKind`): `diff`, `commit`, `pr`, `test_run`,
  `report`, `file`, `link`, `finding`, `session`. Idempotent on `(work_item_id, kind, ref)`.
- **`work_approvals`** — human gates (`ApprovalStatus`): `pending` → `approved` | `rejected`.

All enums are string-backed, snake_case on the wire and in SQLite (one `str_enum!` macro
keeps `as_str`/`parse` in lockstep with the stored column).

---

## 3. How the graph is built (the projector)

`workgraph_projector::spawn` starts **three** detached tasks (design §5 + review resolution
M5 — the split is deliberate):

1. **Live event loop** — subscribes to the broadcast bus and projects each source event into
   a work item. **Cheap, SQLite-only** — it never calls the usage engine inline (a
   `clickhouse local` process spawn), so it can't back the broadcast buffer up into `Lagged`.
2. **Reconcile loop** — every **5 minutes** (`RECONCILE_SECS = 300`) re-derives every
   workspace's graph from the authoritative repos. Idempotent; **self-heals** missed/lagged
   events. SQLite-only.
3. **Boot backfill** — a one-shot `backfill_all` is `tokio::spawn`ed so it never delays
   daemon startup.

### Event → projection mapping

| Source event | Projector action |
|---|---|
| `SessionCreated` | upsert a `session` (or `external_trigger`) item + a "session" deep-link artifact |
| `SessionStatus` / `SessionRemoved` | normalize + set the session item's status (cheap; no ClickHouse) |
| `TrailAppended` (tool/command/skill) | append a `tool_call` event on the owning session item (quiet — no broadcast) |
| `SwarmRunUpdated` / `SwarmTaskUpdated` / `SwarmStatus` | upsert the affected swarm **project** item (cost = SUM of its runs) |
| `GoalLoopUpdated` | upsert the `goal_loop` item + a branch `link` artifact |
| `WorkflowRunUpdated` | upsert the `workflow` run item |
| `ReviewChanged` | upsert the `review` item **and** a derived `pr` item, link `review --reviews--> pr`, attach a verdict report + PR-link artifact |
| `ProductChanged` / `PlanRun` | upsert the `product_story` item |

### How sessions are classified (`classify`)

A live PTY only becomes a work item if it is a non-archived **agent** session. Then:

- **`source = "channel"`** → kind `external_trigger` (owner = the channel, owner_kind
  `integration`).
- **`source` empty or `"user"`** → kind `session` (owner = creator, owner_kind `user`).
- **anything else** (review / swarm / goal_loop / `*_assist` / product-analysis sub-agents)
  → **Skip** — those are represented by their parent work item, not surfaced on their own.

### Cost enrichment is on-demand, never on a loop

Cost via the usage engine (`clickhouse local`) is refreshed **only** when you open an item's
detail (`refresh_item_cost`, sessions/triggers only) or trigger a **backfill** for one
workspace (`refresh_session_costs`). The background reconcile is deliberately SQLite-only, so
a per-call ClickHouse spawn can never storm the usage engine. Swarm-project cost is summed
from `swarm_runs`; goal-loop cost is the loop's own snapshot; workflow/PR/story cost is
surfaced as `0` (documented — not modeled for MVP).

### What the projector auto-links (and what it doesn't)

The projector currently auto-creates only the **`review --reviews--> pr`** edge and always
passes `started_by_id = None` (so `spawned` provenance edges are not auto-wired in MVP). All
other relations (`spawned`, `depends_on`, `fixes`, `verifies`, `blocks`, `belongs_to`) are
**user-authored** via the edges API (§6). `record()` broadcasts `WorkGraphUpdated` **only**
on create or a real status change — cost/title-only refreshes stay quiet, so the ~2 s
session-status churn doesn't spam the UI or bloat the audit trail.

---

## 4. Walkthrough (the Mission Control page)

`MissionControlPage.svelte` loads three calls in parallel for the current workspace —
`summary`, `items`, `graph` — and reloads on workspace change, any filter change, and each
live `work_graph_updated` tick.

### Summary tiles

Four tiles across the top (from `GET …/workgraph/summary`): **Work items** (total),
**Active** (status ∈ pending/running/waiting/blocked), **Needs approval** (distinct items
with a pending approval — the tile turns amber when > 0), and **Total cost** (sum of
`cost_so_far`).

### Toolbar — filters & view toggle

- **Filters**: by **kind**, **status**, **risk**, and a debounced **title search** (`q`).
  A **Clear** button appears when any filter is set. Filters are passed to all three calls
  (so the graph and list stay in sync).
- **View toggle**: **List** ⇄ **Graph**.
- **Refresh** (header) → `POST …/workgraph/backfill`: re-derives this workspace's graph from
  every source and refreshes session costs, then reloads. This is the "materialize existing
  work" button (also offered in the empty state).

### List view (`WorkItemList`)

One row per item: a kind icon, the title, a sub-line (kind label · short repo · owner), and a
right-aligned meta cluster — a **Needs approval** badge (when pending), a **status** chip, a
**risk** chip, the **cost**, and a relative time (`last_event_at` ?? `updated_at`). Click a
row to open the detail panel.

### Graph view (`WorkGraphView`)

An SVG laid out in **columns by kind** (only kinds present are shown), one node per item.
Each node is a circle **colored by normalized status**, with its **radius scaling with cost**
(`sqrt`-scaled, capped). A dashed **amber ring** marks items that need approval. Edges are
drawn between nodes that are both in the current (filtered) set, **labelled by relation**.
Click (or Enter/Space) a node to open its detail.

### Detail panel (`WorkItemDetail`)

Opening an item refreshes its cost on-demand then loads
`GET …/workgraph/items/{id}` (`WorkItemDetail`):

- **Header**: kind icon + title + `kind · source_id`.
- **Banner**: status chip, risk chip, a **Needs approval (N)** badge, and — for `session` /
  `external_trigger` items — an **Open session** button that jumps to the live terminal.
- **Facts grid**: owner (+ actor kind), cost so far, repo, branch, created/updated relative
  times.
- **Goal & context** (editable): **Edit** reveals inline fields for **Goal**, **Result
  summary**, and **Risk (policy)** → `PATCH` (`{risk_level?, goal?, result_summary?}`).
- **Approvals**: list each gate with its status/reason/requester; **Approve** / **Reject** a
  pending one; **Request approval** with an optional reason.
- **Relations**: the item's edges, each showing direction (→/←), relation, the peer's icon,
  title, and status; click to navigate to the peer.
- **Evidence**: artifacts (kind + title; a URL `ref` renders as an "open" link).
- **Timeline**: the append-only events, newest first — an actor-colored dot, the event type,
  the actor, a relative time, and a payload preview.

Empty state (no items): a radar icon with *"Mission Control unifies every agentic
activity…"* and a **Refresh / backfill** button.

---

## 5. The work item lifecycle (normalization)

Every source status collapses onto the eight-state `WorkStatus` via `from_source`. The
per-kind tables (in `otto-state/src/workgraph.rs`) include, for example:

- **session / external_trigger** — `working`→`running`, `idle`/`reconnectable`/`reconnecting`
  →`waiting`, `exited`/`closed`/`archived`→`done`.
- **swarm** — `queued`/`draft`/`paused`→`pending`, `active`→`running`, `error`→`failed`,
  `aborted`/`stopped`→`cancelled`, `done`→`succeeded`.
- **goal_loop** — `draft`→`pending`, `paused`→`waiting`, `blocked`→`blocked`,
  `exhausted`/`failed`→`failed`, `succeeded`→`succeeded`.
- **workflow** — `completed`/`success`→`succeeded`, `error`→`failed`.
- **review** — `done`→`succeeded`, `error`→`failed`.
- **pr** — `open`→`running`, `merged`/`closed`→`done`.
- **product_story** — `done`/`closed`/`shipped`/`released`→`done`, `blocked`→`blocked`.

Any unmapped source value falls back to `running` so the item stays visible in the active
view rather than disappearing.

---

## 6. API & contract reference

All routes nest under `/api/v1/workspaces/{wid}/workgraph/…` (so a single `policy.rs` rule —
`Require(MissionControl, View|Edit)` — covers them) and additionally enforce the workspace
role (`Viewer` for reads, `Editor` for writes). Writes are limited to human annotation,
manual edges, approvals, and a re-derive backfill — the graph is otherwise a projection,
never user-authored. Authoritative: `docs/contracts/api.md` (#126–#134) and
`docs/contracts/ws.md` (`work_graph_updated`).

| # | Method & path | Auth | Request → Response |
|---|---|---|---|
| 126 | `GET …/workgraph/summary` | mission_control view + ws viewer | → `MissionSummary` `{total, active, needs_approval, total_cost, by_kind[], by_status[], by_risk[]}` |
| 127 | `GET …/workgraph/items` | view + viewer | query `kind?,status?,risk?,q?,limit?` → `WorkItem[]` (newest-updated first; limit clamped 1–1000, default 200) |
| 128 | `GET …/workgraph/graph` | view + viewer | query `kind?,status?,risk?,limit?` → `GraphView` `{nodes[], edges[]}` (edges only where both endpoints are in the node set) |
| 129 | `GET …/workgraph/items/{id}` | view + viewer | → `WorkItemDetail` `{…WorkItem, edges[], events[], artifacts[], approvals[], pending_approvals, needs_approval}` (refreshes session cost on-demand) |
| 130 | `PATCH …/workgraph/items/{id}` | edit + editor | `{risk_level?, goal?, result_summary?}` → `WorkItem` |
| 131 | `POST …/workgraph/items/{id}/edges` | edit + editor | `{to_item_id, relation}` → `WorkEdge` (both endpoints validated in-workspace) |
| 132 | `POST …/workgraph/items/{id}/approvals` | edit + editor | `{reason?}` → `WorkApproval` (pending) |
| 133 | `POST …/workgraph/approvals/{aid}/decide` | edit + editor | `{decision: approved\|rejected, note?}` → `WorkApproval` |
| 134 | `POST …/workgraph/backfill` | edit + editor | — → `{ok, summary: MissionSummary}` (re-derive **this** workspace + refresh its session costs) |

### Key DTOs

- **`WorkItem`** — see §2.
- **`MissionSummary`** — `{ total, active, needs_approval, total_cost, by_kind[],
  by_status[], by_risk[] }` (`by_*` are `{key, count}` buckets).
- **`GraphView`** — `{ nodes: GraphNode[], edges: GraphEdge[] }`; a `GraphNode` is `{id,
  kind, title, status, risk_level, cost_so_far, owner_kind, needs_approval}`; a `GraphEdge`
  is `{from_item_id, to_item_id, relation}`.
- **`WorkItemDetail`** — the `WorkItem` (flattened) + `edges` (`EdgeView` with
  direction/peer), `events`, `artifacts`, `approvals`, `pending_approvals`, `needs_approval`.

### WebSocket event (`/ws/events`, workspace-scoped)

`work_graph_updated` is emitted by the projector when a work item is **created** or its
**normalized status changes** (cost/title-only refreshes stay quiet):

```json
{
  "type": "work_graph_updated",
  "workspace_id": "<workspace_id>",
  "item_id": "<work_item_id>",
  "kind": "session|swarm|goal_loop|workflow|review|product_story|pr|external_trigger",
  "status": "pending|running|waiting|blocked|succeeded|failed|cancelled|done"
}
```

Delivered to members with Viewer+ on the workspace. The page re-fetches its summary/list/
graph on a matching tick instead of polling.

---

## 7. Capabilities & limitations

**You can:**

- See **every** agentic activity (8 kinds) as one filterable list and one node/edge graph,
  with normalized status, a risk (policy) axis, cost, and a needs-approval flag.
- Drill into any item: facts, goal/context, **edges** to related work, **evidence**
  artifacts, and the full append-only **timeline** (including tool calls).
- **Annotate** the human-governable fields (risk, goal, result summary).
- **Link** items manually with any of the seven relations, **request** human-approval gates,
  and **approve/reject** them.
- **Open the underlying session** for session/external-trigger items.
- **Refresh / backfill** to re-derive a workspace's graph (self-heals drift) and refresh its
  session costs.
- Everything updates **live** over `work_graph_updated` — no polling.

**Limitations / honest caveats:**

- **Projection, not authority.** The graph **mirrors** the owning repos; it is never the
  source of truth for a fact. The owning feature (sessions, swarm, …) is. You can't *create*
  or *delete* work through Mission Control — only annotate, link, and gate.
- **Auto-edges are minimal.** Only `review → pr` is derived automatically; `started_by_id`
  is not populated, so `spawned`/provenance edges are **manual** today. Other relations are
  user-authored.
- **Cost coverage is partial.** Sessions/external-triggers get live cost on-demand; swarm
  projects sum their run costs; **workflow runs, PRs, and product stories surface `0`** (not
  modeled for MVP).
- **Internal sub-agents are hidden by design.** Review/swarm/loop/assist PTYs are *skipped*
  (represented by their parent item), so the graph isn't cluttered with machinery.
- **The bus is lossy.** A lagging projector gets `Lagged` and skips events; the 5-minute
  reconcile + on-demand backfill heal the drift, so a status can briefly trail reality.
- **Reconcile is workspace-wide but bounded.** Backfill caps recent reviews (50) and runs per
  workflow (5); very old history may not be materialized until touched.
- **Separate from the `/mission` work-queue.** The older 6-bucket queue (gated by `Agents`)
  is a different surface (see §1) and is not affected by anything here.

---

## 8. Security & permissions

- **Feature gate + RBAC, both.** Every route is classed `Require(MissionControl, View|Edit)`
  in `policy.rs` **and** the handler enforces the workspace role (`Viewer` for reads,
  `Editor` for writes — annotate, edges, approvals, decide, backfill). See
  `docs/MULTI-USER-RBAC.md`.
- **Workspace isolation.** Items, edges, events, artifacts, and approvals all carry
  `workspace_id`; every query is workspace-scoped, item fetches are scoped (a cross-workspace
  id 404s), and manual-edge endpoints validate **both** endpoints live in the same workspace.
  `work_graph_updated` events are only delivered to members with Viewer+ on that workspace.
- **Approvals are decided once.** `decide_approval` only transitions a `pending` gate
  (deciding an already-decided one returns a conflict), and each decision is recorded as an
  `approval_decided` audit event with the deciding user.
- **No outward actions.** Mission Control reads and annotates; it never pushes, merges,
  posts, or runs anything — opening a session just navigates you to the existing terminal.
- **No secrets in the graph.** Cost comes from the usage engine; the graph stores only
  references (session ids, repo ids, branch names), never credentials. The daemon listens on
  loopback only.

---

## 9. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| **No work items yet** | The projector hasn't seen events for this workspace, or it's a fresh boot. Click **Refresh / backfill** to materialize existing sessions/swarms/loops/runs/reviews/stories. |
| **An item's status looks stale** | The bus may have lagged. Wait for the 5-minute reconcile or hit **Refresh** — the upsert is idempotent and self-heals. |
| **Cost shows `$0.00`** | Workflow runs, PRs, and product stories don't model cost (MVP). For sessions/triggers, open the detail (refreshes on-demand) or **Refresh** the workspace; `$0` also means usage tracking is off / not flushed. |
| **A review/swarm sub-agent session is missing** | Intended — internal sub-agent PTYs are *skipped* and represented by their parent item (the review, the swarm project). |
| **Two items appear for one PR review** | Expected — a `review` item *and* the `pr` item it reviews, joined by a `reviews` edge. |
| **My manual edge didn't appear in the graph view** | The graph only draws an edge when **both** endpoints are in the current (filtered) node set. Clear filters or widen the limit. |
| **Can't edit / approve** | Writes need workspace **Editor** plus `Feature::MissionControl` **Edit**; reads need **Viewer** / **View**. |
| **Looking for needs-you / failed buckets** | That's the separate `/mission` work-queue endpoint (gated by `Agents`), not this page. |

---

## 10. Related docs

- [`./agent-sessions.md`](./agent-sessions.md), [`./agent-swarm.md`](./agent-swarm.md),
  [`./goal-loops.md`](./goal-loops.md), [`./code-review.md`](./code-review.md),
  [`./workflows.md`](./workflows.md), [`./product.md`](./product.md) — the source features
  whose rows Mission Control projects.
- `docs/contracts/api.md` (#126–#134) and `docs/contracts/ws.md` (`work_graph_updated`) — the
  authoritative API/event contract.
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — workspace roles and the feature-
  capability axis Mission Control is gated on.
