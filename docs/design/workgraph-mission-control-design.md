# Mission Control / Work Graph — Design

**Status:** design → review → implement
**Branch:** `feat/workgraph-mission-control`
**Requirements source of truth:** [workgraph-requirements.md](./workgraph-requirements.md)

## 1. Problem & thesis

Otto today has many strong panels — sessions, swarm, goal loops, workflows, PR
reviews, product stories, channels — but no single place that shows *every*
agentic activity as one traceable unit with a common spine: who/what started it,
what it belongs to, what context it used, what tools it called, what it cost,
what changed, what evidence exists, and what needs human approval.

**Mission Control** is that unifier, backed by a new internal concept,
**`otto-workgraph`**: a normalized graph of `work_items` linked by `work_edges`,
each carrying an append-only stream of `work_events`, plus `work_artifacts`
(evidence) and `work_approvals` (human gates).

## 2. Key architectural decision — projection, not rewiring

Every activity type **already broadcasts an `Event::*`** on the daemon event bus
(`broadcast::Sender<Event>`, created in `ottod/main.rs`, streamed to
`/ws/events`):

| Kind | Existing event(s) |
|------|-------------------|
| session / external_trigger | `SessionCreated`, `SessionStatus`, `SessionRemoved`, `TrailAppended`, `TasksUpdated` |
| swarm | `SwarmRunUpdated`, `SwarmTaskUpdated`, `SwarmStatus`, `SwarmMessagePosted` |
| goal_loop | `GoalLoopUpdated` |
| workflow | `WorkflowRunUpdated` |
| review | `ReviewChanged` |
| product_story | `ProductChanged`, `PlanRun` |

So "every module emits events into this graph" is satisfied **without touching
the 8 modules**: a single **`WorkGraphProjector`** background task subscribes to
the bus and materializes/updates work items, edges, events, and artifacts. This
is the lowest-coupling, most robust choice:

- **Re-derivable.** Items are upserted by a natural key `(workspace, kind,
  source_id)`. The bus is lossy (a lagging receiver gets `Lagged`); a periodic
  reconciliation sweep + an on-demand **backfill** re-derive from the
  authoritative repos, so drift self-heals.
- **No new failure surface in hot paths.** Modules keep emitting exactly as they
  do; the projector is the only consumer that writes the graph.
- **Single source of truth per fact.** Cost/status/title live in their owning
  repos; the graph denormalizes a snapshot for the cross-cutting view and links
  back for detail.

A thin **ingest seam** (`WorkGraphService::record`) is still public, so a future
module *can* push directly if an event ever lacks the needed signal.

### Dependency direction (no cycles)

```
otto-core      ── domain types (WorkItem/WorkEdge/WorkEvent/WorkArtifact/
                   WorkApproval + enums) + Event::WorkGraphUpdated
otto-state     ── WorkGraphRepo (SQL, migration 0077) — depends on otto-core
otto-workgraph ── WorkGraphService (projection/derivation/normalization/risk +
                   emits WorkGraphUpdated) — depends on otto-core + otto-state
otto-server    ── WorkGraphProjector (bus→service, enrichment via usage + source
                   repos), HTTP routes, policy classification, WS scope arm
```

Domain types live in **otto-core** (the dependency-free crate) to avoid a cycle
(`otto-state` needs the types; `otto-workgraph` needs the repo).

## 3. Data model (migration `0077_workgraph.sql`)

All tables are workspace-scoped and `ON DELETE CASCADE` from `workspaces`.

### `work_items` — the spec entity, verbatim fields + projection key

```sql
CREATE TABLE work_items (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,   -- session|swarm|goal_loop|workflow|review|product_story|pr|external_trigger
    source_id       TEXT NOT NULL,   -- natural key of the upstream row (session id, swarm id, loop id, run id, review id, story id, pr key, trigger id)
    title           TEXT NOT NULL,
    goal            TEXT,            -- the intent / objective
    status          TEXT NOT NULL,   -- pending|running|waiting|blocked|succeeded|failed|cancelled|done (normalized)
    owner           TEXT,            -- who/what started it (user id, or "agent"/"integration"/"system")
    owner_kind      TEXT NOT NULL DEFAULT 'system', -- user|agent|system|integration (the actor that started it)
    repo_id         TEXT,            -- repo this belongs to (path or registered repo id)
    branch          TEXT,
    cost_so_far     REAL NOT NULL DEFAULT 0,   -- USD, best-effort snapshot
    risk_level      TEXT NOT NULL DEFAULT 'low', -- low|medium|high|critical  (the "policy" axis)
    result_summary  TEXT,
    context_summary TEXT,            -- "what context was used" (prompt/soul/skills/PR/changed-files, best-effort)
    started_by_id   TEXT,            -- work_item id of the initiator, when known (also mirrored as a spawned edge)
    last_event_at   TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE(workspace_id, kind, source_id)
);
CREATE INDEX idx_work_items_ws        ON work_items(workspace_id, updated_at DESC);
CREATE INDEX idx_work_items_ws_kind   ON work_items(workspace_id, kind, updated_at DESC);
CREATE INDEX idx_work_items_ws_status ON work_items(workspace_id, status);
```

Spec fields R5.1 are all present (`id, workspace_id, kind, title, goal, status,
owner, repo_id, branch, cost_so_far, risk_level, result_summary, created_at,
updated_at`); the extra columns (`source_id`, `owner_kind`, `context_summary`,
`started_by_id`, `last_event_at`) are projection/traceability support.

### `work_edges` — spec entity, verbatim

```sql
CREATE TABLE work_edges (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    from_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    to_item_id   TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    relation     TEXT NOT NULL,  -- spawned|depends_on|fixes|reviews|verifies|blocks|belongs_to
    created_at   TEXT NOT NULL,
    UNIQUE(from_item_id, to_item_id, relation)
);
CREATE INDEX idx_work_edges_from ON work_edges(from_item_id);
CREATE INDEX idx_work_edges_to   ON work_edges(to_item_id);
```

### `work_events` — spec entity, verbatim

```sql
CREATE TABLE work_events (
    id           TEXT PRIMARY KEY,
    work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    ts           TEXT NOT NULL,   -- the spec's "timestamp"
    actor        TEXT NOT NULL,   -- user|agent|system|integration
    event_type   TEXT NOT NULL,   -- created|status_changed|tool_call|context|progress|result|approval_requested|approval_decided|artifact_added|edge_added|note
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_work_events_item ON work_events(work_item_id, ts DESC);
```

### `work_artifacts` — evidence / "what changed" / trace (R3.6, R3.7, R4.11)

```sql
CREATE TABLE work_artifacts (
    id           TEXT PRIMARY KEY,
    work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    kind         TEXT NOT NULL,   -- diff|commit|pr|test_run|report|file|link|finding|session
    title        TEXT NOT NULL,
    ref          TEXT,            -- url / path / sha / deep-link
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL,
    UNIQUE(work_item_id, kind, ref)
);
CREATE INDEX idx_work_artifacts_item ON work_artifacts(work_item_id);
```

### `work_approvals` — human gates (R3.8, R4.10)

```sql
CREATE TABLE work_approvals (
    id            TEXT PRIMARY KEY,
    work_item_id  TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    workspace_id  TEXT NOT NULL,
    status        TEXT NOT NULL,   -- pending|approved|rejected
    reason        TEXT,
    requested_by  TEXT NOT NULL,
    requested_at  TEXT NOT NULL,
    decided_by    TEXT,
    decided_at    TEXT,
    decision_note TEXT
);
CREATE INDEX idx_work_approvals_item ON work_approvals(work_item_id);
CREATE INDEX idx_work_approvals_ws   ON work_approvals(workspace_id, status);
```

A work item **needs human approval** iff it has ≥1 `pending` approval; surfaced
as `needs_approval` + `pending_approvals` in the DTO (status stays the normalized
lifecycle — approval is an overlay, not a lifecycle state).

## 4. Domain enums & normalization (otto-core + otto-workgraph)

- `WorkKind`: Session, Swarm, GoalLoop, Workflow, Review, ProductStory, Pr,
  ExternalTrigger.  (R2.1–R2.8)
- `WorkStatus`: Pending, Running, Waiting, Blocked, Succeeded, Failed, Cancelled,
  Done.  Normalization table (source → unified):
  - session: Working/Running→Running, Idle→Waiting, Reconnectable→Waiting, Exited→Done.
  - swarm run: queued→Pending, running→Running, waiting→Waiting, done→Succeeded, error→Failed, stopped→Cancelled.
  - goal_loop: draft→Pending, running→Running, paused→Waiting, blocked→Blocked, succeeded→Succeeded, exhausted→Failed, failed→Failed, stopped→Cancelled.
  - workflow: pending→Pending, running→Running, completed→Succeeded, error→Failed, canceled→Cancelled.
  - review: running→Running, done→Succeeded, error→Failed, cancelled→Cancelled.
  - product_story: by stage (default Running; closed/done stages→Done).
- `EdgeRelation`: Spawned, DependsOn, Fixes, Reviews, Verifies, Blocks, BelongsTo. (R5.2)
- `WorkActor`: User, Agent, System, Integration. (R5.3)
- `RiskLevel`: Low, Medium, High, Critical (the "policy" axis, R4.8).
- `ArtifactKind`, `ApprovalStatus` as above.

**Risk heuristic** (default, overridable via PATCH): review whose title matches
`security|payment|auth|secret|prod` → High; goal_loop/swarm/pr → Medium; session
→ Medium; workflow/product_story → Low. Critical only when set manually.

## 5. Projection rules (WorkGraphProjector, in otto-server)

The projector owns one `events.subscribe()` receiver and a 60 s reconciliation
tick. For each event it builds a `WorkItemUpsert` and calls
`WorkGraphService::record`, which upserts the item, appends a `work_event`, and
broadcasts `Event::WorkGraphUpdated { workspace_id, item_id, kind, status }`.

| Event | Item kind & key | Action |
|-------|-----------------|--------|
| `SessionCreated` | session **or** external_trigger (if `provider=="channel"` or `meta.source` is a channel) | create item; owner=`created_by`; repo/branch from cwd; context_summary from title/provider/cwd; if channel-origin, also create the `external_trigger` item and a `spawned` edge → session |
| `SessionStatus` | session | normalize status; on terminal status refresh cost via `usage.session_totals_for` |
| `SessionRemoved` | session | status→Done |
| `TrailAppended` (kind ∈ tool_call, action) | session | append `work_event{actor:agent,event_type:tool_call,...}` (capped to notable kinds) — satisfies R3.4 |
| `SwarmStatus` / `SwarmRunUpdated` / `SwarmTaskUpdated` | swarm (key=swarm_id, title=swarm/project name) | upsert; cost from run rows; link agent sessions via `spawned` edge when session_id known |
| `GoalLoopUpdated` | goal_loop (key=loop_id) | read `GoalLoopsRepo.get` for title/goal/branch/worktree/cost/progress; upsert; artifact for branch/worktree |
| `WorkflowRunUpdated` | workflow (key=run_id) | read run; upsert; result artifact on completion |
| `ReviewChanged` | review (key=review_id) | read `ReviewsRepo`; upsert; create/upsert a `pr` item (key=`repo:pr_number`) + `reviews` edge; artifacts: review summary report + PR link |
| `ProductChanged` / `PlanRun` | product_story (key=story_id) | read `ProductRepo`; upsert; plan sessions `belongs_to` the story |

**Cost rollup** (R3.5/R4.4): swarm/goal_loop/workflow costs come from their
source rows (no ClickHouse). session/external_trigger/review costs come from
`usage.session_totals_for` refreshed on terminal status + once per reconciliation
tick for active items (bounded to ≤ 1 ClickHouse read/item/minute).

**Backfill** (`POST .../workgraph/backfill`, and once at daemon boot): enumerate
existing sessions, swarms (+projects), goal loops, workflow runs, reviews, and
product stories, and run them through the same mappers. Guarantees Mission
Control is populated for pre-existing work and that every kind is representable;
doubles as the deterministic E2E seeding path.

## 6. Server API (all under `/api/v1/workspaces/{wid}/workgraph/`)

| Method & path | Cap | Returns |
|---|---|---|
| `GET …/summary` | View | `MissionSummary` — counts by kind/status/risk, total cost, active, needs-approval |
| `GET …/items?kind=&status=&risk=&q=&limit=` | View | `WorkItemRow[]` |
| `GET …/items/{id}` | View | `WorkItemDetail` — item + edges(±peer titles) + events + artifacts + approvals + live cost + needs_approval |
| `GET …/graph?kind=&status=&limit=` | View | `GraphView` — `{nodes[], edges[]}` for the viz |
| `PATCH …/items/{id}` | Edit | annotate: risk_level / goal / result_summary |
| `POST …/items/{id}/edges` | Edit | add `{to_item_id, relation}` |
| `POST …/items/{id}/approvals` | Edit | request `{reason}` → `WorkApproval` |
| `POST …/approvals/{aid}/decide` | Edit | `{decision: approved|rejected, note}` |
| `POST …/backfill` | Edit | re-derive from source repos → `{created, updated}` |

- **Policy:** one rule in `policy.rs` — `p.starts_with("/workspaces/{id}/workgraph/")`
  → `Require(MissionControl, if get { View } else { Edit })`. Single prefix keeps
  the `policy_coverage` test green.
- **RBAC:** new `Feature::MissionControl` (domain.rs parse/as_str + doc count 19→20).
  Root → Admin (E2E root user passes); non-root needs a grant (consistent with all features).
- **Errors/success:** standard `ApiError`/`Problem`; handlers call
  `require_ws_role(.., Viewer|Editor)` for the workspace axis in addition to the
  feature gate.

## 7. WS event

Add `Event::WorkGraphUpdated { workspace_id, item_id, kind, status }` to
`otto-core/src/event.rs` and to the **Workspace** arm of `ws_events::scope_of`
(the exhaustive match forces this at compile time). UI subscribes for live
updates.

## 8. UI — Mission Control module

`ui/src/modules/mission-control/`:
- `MissionControlPage.svelte` — header with **summary stat tiles** (active, by
  status, total cost, needs-approval); a **filter bar** (kind / status / risk /
  search); a **view switch**: **List** (default) and **Graph**.
- `WorkItemList.svelte` — rows: kind icon, title, status chip, risk chip, owner,
  cost, last-event time, needs-approval badge. Click → detail drawer.
- `WorkItemDetail.svelte` — the full traceable unit: owner, goal, context,
  cost, status, risk(policy), result; tabs/sections for **Timeline** (events =
  audit), **Evidence** (artifacts), **Relations** (edges in/out with deep
  links), **Approvals** (request + approve/reject). Deep-links to the underlying
  session/swarm/loop/review/story/PR.
- `WorkGraphView.svelte` — node/edge graph (reuse the swarm `AgentGraph` radial
  approach; nodes coloured by status, sized by cost; edges labelled by relation).
- `missionControlBus` in `events.svelte.ts` + handler for `work_graph_updated`
  → page re-fetches.
- `lib/api/missionControl.ts` + types in `lib/api/types.ts` (mirror Rust).
- `sidebar.ts`: `{ id:'mission-control', icon:'chart-network'|'radar', label:'Mission Control', feature:'mission_control' }`; route in `App.svelte`; add `'mission-control'` to UI `Feature` union + E2E `PAGES`.

Design tokens only; active states use `#7ee787` + black per repo rule.

## 9. Requirement → design map (every requirement covered)

- **R1.1** graph of work_items/edges/events. **R1.2** `otto-workgraph` crate. **R1.3** Mission Control page.
- **R2.1–R2.8** eight `WorkKind`s, each with a projector mapper + backfill.
- **R3.1** owner/owner_kind + initiating event. **R3.2** repo_id/branch + `belongs_to`/`spawned` edges. **R3.3** context_summary + context events. **R3.4** tool_call work_events from trail. **R3.5** cost_so_far rollup. **R3.6** result_summary + diff/commit artifacts. **R3.7** work_artifacts. **R3.8** pending approvals → needs_approval.
- **R4.1–R4.11** owner, goal, context_summary, cost_so_far, result_summary, artifacts, status, risk_level(policy), work_events(audit), work_approvals, artifacts(trace).
- **R5.1–R5.3** the three tables verbatim (+ support columns).
- **R6.1–R6.8** projector mappers + backfill cover all 8 modules via the existing event bus.

## 10. Risks & mitigations

- **Bus loss / lag** → upsert-by-key + 60 s reconcile + on-demand backfill.
- **Trail volume** → ingest only `tool_call`/`action` kinds; detail links to the
  full session trail (already paginated) rather than copying it.
- **ClickHouse cost reads** → bounded to terminal-status + once/min for active
  session items; non-session kinds read cost from their own rows.
- **policy_coverage test** → single `/workgraph/` prefix rule; verified by the test.
- **Scope match exhaustiveness** → compiler enforces the new event is scoped.
- **No damage to user work** → projection is read-mostly of source repos; the
  graph is a separate, additive set of tables; no source row is mutated.

## 11. Review resolutions (gating — folded into implementation)

Adversarial design review (Block) found real issues; resolutions:

- **B1 — name collision with existing B4 "Mission Control."** There is already an
  in-Agents *work-queue* lens called "Mission Control" (`routes/mission.rs`,
  `ui/modules/agents/MissionControl.svelte`, `viewMode:'mission'`, a TabBar
  toggle). Resolution: the **new top-level module is the marquee "Mission
  Control"** (the request asks for exactly this), backed by the work graph; the
  **existing Agents toggle is renamed to "Work Queue"** (a label-only change in
  `TabBar.svelte`) to disambiguate. The B4 `/mission` routes stay (additive — no
  deletion of working code). Different feature gate, different route prefix.
- **B2 — policy placeholder must match the route template verbatim.** Routes use
  `/api/v1/workspaces/{wid}/workgraph/…`; the policy rule is
  `p.starts_with("/workspaces/{wid}/workgraph/")` — **identical `{wid}`**. One
  prefix rule covers every route; verified by `policy_coverage`.
- **M1 — external_trigger detection** = `session.meta["source"] == "channel"`
  (read `meta.channel` for sub-kind), NOT `provider`. Sessions whose
  `meta.source` is an internal-assistant marker (`db_assist`, `canvas`, `mockup`,
  …) are **excluded** from the graph (they're hidden helper PTYs).
- **M2 — pr coverage.** Derive `pr` items from reviews (`reviews` edge) **and**
  enumerate open PRs from the Git store in backfill, so an un-reviewed PR still
  appears. Standalone PRs without a session cost show cost 0.
- **M3 — swarm granularity** = key by **`project_id`** (kind=swarm, title=project
  name, goal=`goal_md`, repo=`repo_path`); `SwarmTaskUpdated`/`SwarmRunUpdated`
  carry `project_id`; backfill enumerates `list_projects`.
- **M4 — cost sources (honest):** session/external_trigger via `usage`
  (off-hot-path), swarm via `SwarmRun.cost_usd`, goal_loop via `GoalLoop.cost_usd`,
  review via its agent sessions' usage, pr via its review's cost. **workflow** and
  **product_story** have no cost field → MVP shows 0 (documented; child-session
  costs still visible on their own items). The field is always surfaced.
- **M5 — projector perf split:** the event-loop task does only cheap DB upserts;
  a separate **60 s reconcile task** (and a **`tokio::spawn`ed boot backfill**)
  do the ClickHouse cost reads, so a per-call `clickhouse local` process spawn
  never blocks the event loop or daemon startup.
- **m1 — trail kinds** are `Tool`/`Command`/`File`/`Web`/`Skill`; ingest
  `Tool`/`Command`/`Skill` → `work_event{event_type:"tool_call"}`.
- **m2 — grants UI:** add `'mission_control'` to `ALL_FEATURES` **and**
  `FEATURE_LABELS` in `ui/modules/settings/Users.svelte`.
- **m3 — UI feature key** is snake_case `'mission_control'`; the sidebar
  id/route is kebab `'mission-control'`.
- **m4 — self-feedback:** the projector match handles only source events and
  no-ops on `WorkGraphUpdated`/everything else (it emits on the same bus).
- **m5 — evidence:** MVP populates derivable artifacts (PR/branch links, review
  report, session deep-link); the tool-call timeline covers "what changed" detail.
</content>
