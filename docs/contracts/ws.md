# Otto WebSocket Contract (FROZEN)

Two WS endpoints. Auth for both: a bearer token validated BEFORE the upgrade
completes; invalid token → HTTP 401, no upgrade.

- The event stream (`/ws/events`) accepts the token via the
  `Sec-WebSocket-Protocol` request header — the client offers
  `["otto-bearer", "<token>"]` and the server echoes back `otto-bearer` on
  success. This keeps the token out of the URL/query string (which is logged by
  proxies and servers). A `?token=<bearer token>` query parameter is still
  accepted as a backward-compatible fallback.
- The terminal stream (`/ws/term/{session_id}`) accepts the token the same way:
  prefer `Sec-WebSocket-Protocol: otto-bearer, <token>` (server echoes back
  `otto-bearer` on success, keeping the share token out of the URL). A
  `?token=<bearer token>` query parameter is still accepted as a
  backward-compatible fallback for existing clients.

## 1. Terminal stream — `WS /ws/term/{session_id}`

Auth: prefer `Sec-WebSocket-Protocol: otto-bearer, <token>` (server echoes
`otto-bearer` subprotocol on success, keeping the bearer token out of the URL).
`?token=<bearer token>` query parameter is accepted as a backward-compatible
fallback. An IP that fails token validation too many times is locked out (429).

Role: workspace **viewer** may attach (read-only); **editor**+ may send input/resize.
Input frames from viewers are silently dropped server-side (and a single JSON
`{"type":"error","code":"forbidden"}` is sent once).

### Client → server (JSON text frames)

```json
{"type":"input","data":"<base64 bytes>"}
{"type":"resize","cols":120,"rows":32}
{"type":"scrollback","lines":2000}
{"type":"search","query":"foo"}                     // server-side ring-buffer search (see below)
```

### Server → client

- **Binary frames**: raw PTY output bytes — write straight into xterm.
- **JSON text frames**:

```json
{"type":"scrollback","data":"<base64 bytes>"}      // response to scrollback request; send BEFORE live bytes resume
{"type":"status","status":"working"}                // running|working|idle|exited|reconnectable
{"type":"exit","code":0}                            // child exited; socket stays open
{"type":"terminated"}                               // session force-terminated (admin terminate / share-link revoke); socket closes immediately after
{"type":"error","code":"forbidden","message":"..."}
{"type":"search_result","query":"foo","matches":[{"line":42,"text":"foo bar baz"},...]}  // up to 200 matches
```

#### Server-side search (`{"type":"search"}`)

Grep the persistent ring-buffer scrollback (10 000 lines, survives WS reconnects) for `query`
(plain substring, case-insensitive, ANSI-stripped). The server replies with a single
`{"type":"search_result","query":"…","matches":[{"line":<ring-index>,"text":"<plain>"},…]}`
frame containing up to 200 matches in buffer order (oldest → newest). Empty `query` is
a no-op (no reply). This complements the xterm `SearchAddon` (which searches only the
current emulator viewport, lost on reconnect) — use server search when the session has
been reopened or when looking for output that scrolled off the visible viewport.

Unlike `exit` (the child process ended but the socket stays open so the user can
read the final output), `terminated` is the server forcibly dropping this viewer:
it is sent once and the socket is closed right after. Clients should treat it as
"this session is gone" (admin terminated it, or a mobile share-link was revoked).

Multiple clients may attach to one session simultaneously; all receive the same
output broadcast. Input is interleaved in arrival order. On attach the server
sends current `status` immediately.

## 2. Event stream — `WS /ws/events`

Server → client only. Each message is one JSON-serialized `otto_core::event::Event`
(see crate; tag field `type`, snake_case). The server filters events: a client
receives a **session-scoped** event only if it is a member (`viewer`+) of the
event's workspace **and** is the session's owner (`created_by`), a workspace
`admin`, or root. Other **workspace-scoped** events (improvement, swarm) reach
every member (`viewer`+) of the workspace. Root receives all. `Notice` events
are delivered to all authenticated clients.

Client→server messages on this socket are ignored. Ping/pong handled by the
transport layer (axum auto-responds to pings; server sends a ping every 30s).

### Full event catalog

Every variant of `otto_core::event::Event` (`crates/otto-core/src/event.rs`). The tag is
the `type` field (snake_case of the variant name); the remaining keys are the payload.
Delivery scope: **session-family events** (`session_status`, `session_created`,
`session_meta_updated`, `session_removed`, `trail_appended`, `tasks_updated`)
reach only the session's owner (`created_by`), a workspace `admin`, or root —
and only after the `viewer`+ membership gate on the event's `workspace_id`;
other **workspace-scoped events** (improvement, swarm) reach every member with
`viewer`+ on the event's `workspace_id` (root receives all); **broadcast
events** (`Notice`) reach every authenticated client. There are 16 variants.

Session lifecycle (session-family — owner/admin/root, viewer-gated):

```json
{"type":"session_status","session_id":"…","workspace_id":"…","status":{…SessionStatus…}}
{"type":"session_created","session":{…Session…}}
{"type":"session_meta_updated","session_id":"…","workspace_id":"…","meta":{…}}
{"type":"session_removed","session_id":"…","workspace_id":"…"}
```

- `session_status` — a session's live status changed (`SessionStatus` enum).
- `session_created` — a session was created (by any client or the orchestrator); carries
  the full `Session`.
- `session_meta_updated` — a session's `meta` changed; carries the full merged `meta`
  object so clients update their cached session in place (e.g. live handover-progress flags).
- `session_removed` — a session row was removed (PTY killed).

Notices & notifications:

```json
{"type":"notice","level":"info|warn|error","title":"…","body":"…"}
{"type":"notification","notice":{…Notice…},"user_id":"…"}
```

- `notice` — free-form transient notice surfaced as a toast (broadcast to all authenticated
  clients).
- `notification` — a **persisted** notification row was created (credential expiry, session
  event, …). The SPA appends it to the notification center and may raise a native OS
  notification for `warn`/`error` severities. Carries the full `Notice` domain object.
  `user_id` is optional (`None` is omitted from the JSON via `skip_serializing_if`): when
  present the notification targets a single user and is delivered only to that user;
  when absent it is delivered per the standard workspace/broadcast scoping.

Activity trail & tasks (session-family — owner/admin/root, viewer-gated):

```json
{"type":"trail_appended","workspace_id":"…","session_id":"…","event":{…TrailEvent…}}
{"type":"tasks_updated","workspace_id":"…","session_id":"…","tasks":[{…AgentTask…}]}
```

- `trail_appended` — a new entry was appended to a session's activity trail.
- `tasks_updated` — a session's task tracker changed; carries the full current task list.

Self-improvement (workspace-scoped):

```json
{"type":"improvement_run_started","workspace_id":"…","run_id":"…"}
{"type":"improvement_run_finished","workspace_id":"…","run_id":"…","status":"done|skipped|failed","applied":0,"pending":0}
{"type":"improvement_edit_applied","workspace_id":"…","run_id":"…","edit_id":"…","target_ref":"…"}
{"type":"improvement_approval_pending","workspace_id":"…","run_id":"…","edit_id":"…","target_ref":"…"}
```

- `improvement_run_started` — a self-reflection run started.
- `improvement_run_finished` — a run finished; `status` ∈ `done|skipped|failed`, with
  `applied`/`pending` edit counts.
- `improvement_edit_applied` — an edit was auto-applied to a skill/memory file
  (`target_ref` names the file).
- `improvement_approval_pending` — an edit is awaiting human approval.

### Agent Swarm events

Workspace-scoped (delivered to members with `viewer`+ on the event's `workspace_id`). The
`run`/`task`/`message` payloads travel as serialized JSON (otto-core can't depend on
otto-state, so the row is embedded as `serde_json::Value`):

```json
{"type":"swarm_status","workspace_id":"…","swarm_id":"…","status":"active|paused|aborted"}
{"type":"swarm_run_updated","workspace_id":"…","swarm_id":"…","run":{…SwarmRun…}}
{"type":"swarm_task_updated","workspace_id":"…","swarm_id":"…","project_id":"…","task":{…SwarmTask…}}
{"type":"swarm_message_posted","workspace_id":"…","swarm_id":"…","message":{…SwarmMessage…}}
{"type":"swarm_goal_updated","workspace_id":"…","swarm_id":"…","task_id":"…|null","goal":{…SwarmGoal…}}
```

- `swarm_status` — a swarm's lifecycle status changed (`active|paused|aborted`).
- `swarm_run_updated` — a swarm run was created or changed.
- `swarm_task_updated` — a swarm task was created or changed.
- `swarm_message_posted` — a new message was posted to a swarm's shared board. New
  Coordinator-lifecycle message kinds — `worktree`, `shared`, `merge`, `verify`,
  `escalation` — arrive on this same event.
- `swarm_goal_updated` — a swarm goal was created or its verification status/verdict
  changed (drives the per-task Goals panel + Kanban goal badges live). `task_id` is the
  goal's task (null for project/standing goals).

The UI mirrors all of these in `OttoEvent` (`ui/src/lib/events.svelte.ts`) and routes the
`swarm_*` set into the `swarm` store, which updates the org tree, run graph, Kanban, runs
list and board live.

## Usage metrics tick (A9)

Emitted by the metrics sampler after each system-metrics sample is stored:

```json
{"type":"usage_metrics_tick","ts":"2026-06-20T12:00:00Z"}
```

- `ts` — UTC ISO-8601 sample timestamp.
- The UI subscribes and calls `usage.applyMetricsTick()` which triggers a throttled
  `/usage/metrics` refresh so the sparklines update in near-real-time.
- Source: `crates/otto-server/src/monitor.rs` → `spawn_metrics_sampler`.
- Throttle: the UI ignores ticks that arrive within 10 s of the last fetch.

## PR-review status change (A2)

Workspace-scoped. Emitted by `crates/otto-server/src/modules.rs` whenever a
review row transitions state (queued → running → done / error / cancelled).
Clients that have the Review Panel open use this to trigger an immediate poll
instead of waiting for the next timed tick.

```json
{
  "type": "review_changed",
  "workspace_id": "<Id>",
  "session_id": "<session_id | null>",
  "review_id": "<review_uuid>",
  "status": "queued|running|done|error|cancelled"
}
```

- `session_id` — the orchestrating session that owns the review; may be `null`
  for externally-triggered reviews.
- `review_id` — UUID of the `reviews` row that changed.
- `status` — the new status string (mirrors the `status` column in `reviews`).
- UI routing: `ReviewPanel.svelte` subscribes to `review_changed` events and
  calls `schedulePoll()` immediately when the event's `review_id` matches the
  currently viewed review, short-cutting the exponential back-off timer.
- TypeScript type: added to the `OttoEvent` discriminated union in
  `ui/src/lib/api/types.ts` as `{ type: 'review_changed'; workspace_id: string;
  session_id?: string | null; review_id: string; status: string }`.

## Review findings workflow (finding_updated / finding_action_started / proof_pack_exported)

Workspace-scoped. Emitted by `crates/otto-server/src/routes/{findings,proof_pack}.rs`
as the Review Findings workflow advances. The `FindingsBoard.svelte` board
subscribes to all three (routed through the `findingBus` in
`ui/src/lib/events.svelte.ts`) and refetches the matching review's findings — the
same pattern `review_changed` uses to drive the Review panel.

**`finding_updated`** — a finding's workflow `status` (or a tracked field) changed,
emitted after every triage action / transition:

```json
{
  "type": "finding_updated",
  "workspace_id": "<Id>",
  "review_id": "<review_id>",
  "finding_id": "<finding_id>",
  "status": "open|accepted|false_positive|fixed|verified|waived"
}
```

**`finding_action_started`** — an agent-backed action (fix / verify /
regression-test) spawned a live, openable session:

```json
{
  "type": "finding_action_started",
  "workspace_id": "<Id>",
  "review_id": "<review_id>",
  "finding_id": "<finding_id>",
  "action": "fix|verify|regression_test",
  "session_id": "<session_id | null>"
}
```

**`proof_pack_exported`** — a review's Proof Pack was exported (a snapshot
persisted + verified findings ingested into memory):

```json
{
  "type": "proof_pack_exported",
  "workspace_id": "<Id>",
  "review_id": "<review_id>",
  "proof_pack_id": "<proof_pack_id>"
}
```

- `status` (finding_updated) — the new `FindingStatus` (snake_case).
- `session_id` (finding_action_started) — the spawned agent session; omitted/`null`
  when the action ran without one.
- UI routing: `finding_updated`/`finding_action_started`/`proof_pack_exported` are
  routed into `findingBus.apply(...)`; `FindingsBoard` re-fetches
  `GET /reviews/{id}/findings` when the event's `review_id` matches the open board.
- TypeScript types: added to the `OttoEvent` discriminated union in
  `ui/src/lib/api/types.ts`.

## Goal-loop progress (Goal Loops)

Workspace-scoped. Emitted by `crates/otto-server/src/goal_loop.rs` on every loop
transition: status change, phase change (Plan → Execute → Evaluate → Digest), a new
iteration, after each evaluation, and when an executor's live state flips (e.g.
→ `waiting`). The Loops UI updates the list row directly from these fields and
re-fetches `GET /goal-loops/{id}` (it also runs a low-frequency fallback poll while
a loop is active, covering any missed event).

```json
{
  "type": "goal_loop_updated",
  "workspace_id": "<Id>",
  "loop_id": "<loop_id>",
  "status": "draft|running|paused|blocked|succeeded|exhausted|failed|stopped",
  "phase": "planning|executing|evaluating|digesting|waiting|done",
  "current_iteration": 0,
  "progress_pct": 0
}
```

- `status` / `phase` — mirror the `goal_loops` row (snake_case enums).
- `current_iteration` — the iteration index in flight or last completed.
- `progress_pct` — the latest evaluator score (0–100).
- Executor sessions are real agent sessions and also emit the normal
  `session_status` events, so their live status dots work without extra plumbing.
- TypeScript type: added to the `OttoEvent` union in `ui/src/lib/api/types.ts` as
  `{ type: 'goal_loop_updated'; workspace_id: Id; loop_id: Id; status: GoalLoopStatus;
  phase: GoalLoopPhase; current_iteration: number; progress_pct: number }`.

## Product AI-run completion (A3)

Workspace-scoped. Emitted by `crates/otto-server/src/product_run.rs` at the end of every
AI-run task (analysis, rewrite, test-case generation, plan generation).

```json
{
  "type": "product_changed",
  "workspace_id": "<Id>",
  "story_id": "<Id>",
  "section": "analysis|rewrite|testcases|plan",
  "status": "done|error"
}
```

- `section` — which product tab completed: `analysis`, `rewrite`, `testcases`, or `plan`.
- `status` — final state of the run.
- UI routing: `events.svelte.ts` dispatches to `product.applyEvent()` in the product store,
  which fires per-section subscriber callbacks registered by each product tab (`AnalysisTab`,
  `PlanTab`, `RewriteTab`, `TestCasesTab`). Each tab's callback triggers a single poll to
  refresh its data immediately, supplementing the existing timed polling as a fallback.

## Multi-agent plan kickoff (A3)

Workspace-scoped. Emitted by `crates/otto-server/src/product_run.rs::run_generate_plan` each
time a planning (or the summarizer) session is created during a `plan/generate` run, carrying
the live session ids known so far. Lets the Plan tab tile the planning sessions side-by-side so
the user can watch them (and answer questions when `interactive`).

```json
{
  "type": "plan_run",
  "workspace_id": "<Id>",
  "story_id": "<Id>",
  "session_ids": ["<Id>", "..."],
  "interactive": false
}
```

- `session_ids` — live, openable sessions in spawn order (planners first; the summarizer is
  appended when it starts). The event is re-emitted as each new session appears, so later
  frames are supersets of earlier ones.
- `interactive` — mirrors the request: `false` (the default) means agents run unattended and
  are instructed NOT to ask questions; `true` means the user will answer questions in the tiles.
- UI routing: `events.svelte.ts` → `product.applyPlanRun()` → `PlanTab` subscribers, which call
  `ws.tileSessions(session_ids)` (switch to tiled view + open the sessions) and route to
  `#/agents` on the first frame, then keep a "Watching N planning agents" affordance fresh.

## Workflow run progress (A11)

Workspace-scoped. Emitted by `crates/otto-server/src/workflow_engine.rs` at
every node transition (start, finish/cached) and when the overall run reaches a
terminal status. Lets the Workflows page switch from a 700ms poll loop to
event-driven refresh; a capped fallback poll (backing off to 3s, max 300 ticks)
is kept for cases where the WS connection is unavailable.

```json
{
  "type": "workflow_run_updated",
  "workspace_id": "<Id>",
  "run_id": "<Id>",
  "status": "running|success|error|canceled",
  "node_id": "<node_id | null>"
}
```

- `node_id` — the node whose state changed; `null` when the event reflects the
  overall run status (run started / run terminal).
- UI routing: `events.svelte.ts` dispatches to `workflowRunBus.apply()`.
  `WorkflowsPage.svelte` subscribes to `workflowRunBus.tick` and re-fetches
  `GET /workflow-runs/{id}` whenever a matching `run_id` event fires.
- TypeScript type: added to the `OttoEvent` union in `ui/src/lib/api/types.ts`
  as `{ type: 'workflow_run_updated'; workspace_id: Id; run_id: Id; status:
  string; node_id?: Id | null }`.

## Skill-eval completion (A11)

Workspace-scoped. Emitted by `crates/otto-server/src/skill_eval.rs` at the end
of every skill evaluation (done/error/cancelled). Lets the Skill-Eval UI drop
its 2s×600 polling pattern and refresh on demand.

```json
{
  "type": "skill_eval_updated",
  "workspace_id": "<Id>",
  "run_id": "<eval_id>",
  "status": "done|error|cancelled"
}
```

- `run_id` — the `skill_evals.id` that reached a terminal state.
- UI routing: `events.svelte.ts` dispatches to `skillEvalBus.apply()`.
- TypeScript type: added to the `OttoEvent` union in `ui/src/lib/api/types.ts`
  as `{ type: 'skill_eval_updated'; workspace_id: Id; run_id: Id; status:
  string }`.

## Self-improvement update (A8)

Global (everyone-scoped). Emitted by `crates/otto-improve/src/engine.rs` when a
self-improvement run finishes or an approval becomes pending. Lets the
Self-Improvement settings pane refresh on the event instead of guessing with a
blind timer.

```json
{
  "type": "improvement_updated",
  "kind": "run_finished|approval_pending",
  "id": "<run_or_approval_id | null>"
}
```

- `kind` — `"run_finished"` after an `execute_run`/`evolve_session` completes,
  `"approval_pending"` when a new edit awaits approval.
- UI routing: `events.svelte.ts` dispatches to `improvementBus`; the
  Self-Improvement pane refreshes on it and keeps a capped poll fallback.
- TypeScript type: added to the `OttoEvent` union in `ui/src/lib/api/types.ts`
  as `{ type: 'improvement_updated'; kind: string; id?: string | null }`.

---

### `insight_ready`

```json
{
  "type": "insight_ready",
  "period": "daily 2026-06-20",
  "session_id": "<session_id | null>"
}
```

- Emitted by `otto-server/src/insights.rs` after a scheduled insights run
  completes (conditioned on `period_done()` returning `true`).
- `period` — human-readable label combining the kind (`daily|weekly|monthly`)
  and the run's start date.
- `session_id` — the originating session, or `null` for a background scheduler run.
- Scope: `Everyone` (all connected clients receive this).
- `channels.notify_insight_ready` setting (default off) routes this to
  Slack / Telegram via `otto-channels/improve_notify.rs`.
- TypeScript type: `{ type: 'insight_ready'; period: string; session_id?: Id | null }`.

---

### `budget_exceeded`

```json
{
  "type": "budget_exceeded",
  "workspace_id": "<workspace_id>",
  "provider": "anthropic",
  "spend_usd": 42.50,
  "cap_usd": 40.00,
  "direction": "exceeded"
}
```

- Emitted when a spend cap is crossed (budget enforcement must be enabled).
- `direction` — currently `"exceeded"`; reserved for future `"recovered"` direction.
- Scope: `Everyone` (no per-workspace delivery filter — all admins should see it).
- `channels.notify_budget_exceeded` setting (default off) routes this to
  Slack / Telegram via `otto-channels/improve_notify.rs`.
- TypeScript type: `{ type: 'budget_exceeded'; workspace_id: Id; provider: string; spend_usd: number; cap_usd: number; direction: string }`.

### `work_graph_updated`

```json
{
  "type": "work_graph_updated",
  "workspace_id": "<workspace_id>",
  "item_id": "<work_item_id>",
  "kind": "session",
  "status": "running"
}
```

- Emitted by the `workgraph_projector` when a Mission Control work item is
  created or its normalized status changes (cost/title-only refreshes stay
  quiet). `kind` is the work kind (`session|swarm|goal_loop|workflow|review|
  product_story|pr|external_trigger`); `status` is the normalized lifecycle.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`).
- The Mission Control page re-fetches the workspace summary/list on a matching
  tick instead of polling.
- TypeScript type: `{ type: 'work_graph_updated'; workspace_id: Id; item_id: Id; kind: string; status: string }`.

---

### `proof_pack_updated`

```json
{ "type": "proof_pack_updated", "workspace_id": "<Id>", "proof_pack_id": "<Id>",
  "work_item_kind": "session|goal_loop|review|workflow_run|task|manual",
  "work_item_id": "<id>", "status": "missing|partial|passed|failed|waived",
  "risk_score": 0, "done_score": 0 }
```

- Emitted by `otto_server::proof::recompute_and_emit` whenever a proof pack is
  created, (re)assembled, gains/loses an artifact, or is waived.
- Scope: `Workspace` (gated on viewer access to that workspace).
- The UI re-fetches the affected pack and refreshes the workspace proof summary.
- `done_score` (0..100) is the done-contract readiness (added in Proof Packs v2).
- TypeScript type: `{ type: 'proof_pack_updated'; workspace_id: Id; proof_pack_id: Id; work_item_kind: string; work_item_id: string; status: string; risk_score: number; done_score: number }`.

---

### `scheduled_task_run_updated`

```json
{ "type": "scheduled_task_run_updated", "workspace_id": "<Id>", "task_id": "<Id>",
  "run_id": "<Id>", "status": "running|ok|error" }
```

- Emitted by `otto_server::scheduled_tasks_engine` when a scheduled-task run
  starts, finishes (`ok`), or errors.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`).
- The Scheduled Tasks page re-fetches the task's run history on a matching tick
  instead of polling.
- TypeScript type: `{ type: 'scheduled_task_run_updated'; workspace_id: Id; task_id: Id; run_id: Id; status: string }`.
