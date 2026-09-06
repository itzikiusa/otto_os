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
{"type":"claim"}                                    // claim size authority (sent on terminal focus)
```

**Size authority.** Multiple viewers share one PTY; the connection that most
recently sent `input` or `claim` (editor+ only) owns the session's size, and
`resize` frames from other connections are ignored while the owner is
attached (a passive tile/preview/idle phone tab can no longer pin a wide
pane's TUI to its own small grid). Authority is STICKY: a detaching owner
does NOT release it — while the user is away from the pane the session keeps
the pane's grid (agent output printed meanwhile stays at the owner's width
instead of being hard-wrapped narrow forever); only a newer `claim`/`input`
transfers the right, and a daemon restart clears the map. The PRIMARY
session pane sends `claim` immediately on attach (before its first `resize`)
— without that, a session nobody has typed into has no owner and the LAST
viewer to attach/refit set the grid, letting a later-attaching preview
shrink an open pane. Preview / monitor embeds must never claim on attach; a
session only ever watched by claim-less embeds (vault-run / workflow panels)
stays first-come-sized.

**Resize semantics.** A `resize` matching the PTY's current grid is dropped
server-side before the ioctl — no SIGWINCH, no emulator rewrap, no meta
write — so clients may re-push their grid unconditionally on focus/reconnect.
Clients should send at most ONE `resize` per settled layout change (pure
trailing debounce), and only after the measured grid re-measures IDENTICAL
~150ms later: a macOS window/fullscreen animation can pause longer than a
settle window mid-flight, and a single mid-animation measurement SIGWINCHes
the TUI into re-rendering its transcript at a transient width that then
fossilizes in scrollback (measured: 0.8s at 69×44 on a 165×48 pane). Every
SIGWINCH makes agent TUIs reprint their live region, and codex re-emits
transcript lines that permanently accumulate in scrollback. Clients must
NOT rebuild their buffer from a `scrollback` snapshot mid-resize; the
client terminal's own native reflow is authoritative for the live view.
ONE deterministic rebuild is allowed (and recommended for agent panes)
~1s AFTER a stability-confirmed grid change lands: a bottom-anchored TUI's
post-widen repaint leaves a blank void between the rejoined transcript and
its live region, and the snapshot (contiguous by construction) closes it —
the automated version of the manual reconnect users performed. Skip it
while the viewport is scrolled up (a rebuild yanks it to the bottom).

### Server → client

- **Binary frames**: raw PTY output bytes — write straight into xterm.
- **JSON text frames**:

```json
{"type":"scrollback","data":"<base64 bytes>","epoch":3}  // response to scrollback request; send BEFORE live bytes resume.
                                                    // `data` is a FULL rebuild: formatted history rows + a coherent
                                                    // current-screen frame + input-mode restoration (bracketed paste,
                                                    // keypad). The client MUST reset its terminal and repaint from this
                                                    // snapshot — appending under locally-kept scrollback duplicates the
                                                    // transcript once per reconnect. `epoch` = PTY spawn counter
                                                    // (0/absent when no live handle), informational.
                                                    // Also pushed unsolicited to resync a viewer whose output stream
                                                    // lagged (dropped chunks) or whose dead session came back alive.
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
`session_meta_updated`, `session_renamed`, `session_removed`, `trail_appended`,
`tasks_updated`, `transcript_appended`, `transcript_live`, `artifact_added`) reach only the session's owner (`created_by`), a workspace
`admin`, or root — and only after the `viewer`+ membership gate on the event's
`workspace_id`; other **workspace-scoped events** (improvement, swarm) reach
every member with `viewer`+ on the event's `workspace_id` (root receives all);
**broadcast events** (`Notice`) reach every authenticated client. There are 48
variants (the sections below cover them; each `## …`/`### …` heading is one
feature family).

Session lifecycle (session-family — owner/admin/root, viewer-gated):

```json
{"type":"session_status","session_id":"…","workspace_id":"…","status":{…SessionStatus…}}
{"type":"session_created","session":{…Session…}}
{"type":"session_meta_updated","session_id":"…","workspace_id":"…","meta":{…}}
{"type":"session_renamed","session_id":"…","workspace_id":"…","title":"…"}
{"type":"session_removed","session_id":"…","workspace_id":"…"}
```

- `session_status` — a session's live status changed (`SessionStatus` enum).
- `session_created` — a session was created (by any client or the orchestrator); carries
  the full `Session`.
- `session_meta_updated` — a session's `meta` changed; carries the full merged `meta`
  object so clients update their cached session in place (e.g. live handover-progress flags).
- `session_renamed` — a session's `title` changed; carries the new `title` so clients
  update their cached session in place. Emitted on a user rename
  (`PATCH /sessions/{id}`, `meta.title_source = "user"`) and by the background
  provider-title auto-namer adopting the CLI's own session title
  (`meta.title_source = "provider"`).
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
  A notice whose `source_key` ends in `:waiting` with an `open_session` action marks the
  session as "needs you" in the SPA (sticky flag, cleared on open/input). Two producers:
  claude's native Notification hook, and — for every AGENT provider — the daemon's
  Working→Idle turn-finish transition (`session:{id}:waiting`). Shell sessions keep the
  plain `session:{id}:idle` key and never raise the flag.

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
{"type":"swarm_project_cleared","workspace_id":"…","swarm_id":"…","project_id":"…"}
{"type":"swarm_message_posted","workspace_id":"…","swarm_id":"…","message":{…SwarmMessage…}}
{"type":"swarm_goal_updated","workspace_id":"…","swarm_id":"…","task_id":"…|null","goal":{…SwarmGoal…}}
```

- `swarm_status` — a swarm's lifecycle status changed (`active|paused|aborted`).
- `swarm_run_updated` — a swarm run was created or changed.
- `swarm_task_updated` — a swarm task was created or changed.
- `swarm_project_cleared` — a project's board was cleared (all tasks + project-scoped
  feed deleted, in-flight runs stopped). Clients drop local task/board state for the
  project instead of waiting for per-row updates that won't come.
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
review row transitions state (queued → running → done / error / cancelled —
`cancelled` comes from `POST /reviews/{review_id}/cancel`). A per-agent stop
(`POST /reviews/{review_id}/agents/{index}/stop`) re-emits the event with
`status: "running"` (no new event type) so open panels refresh the agent rows.
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
  "section": "analysis|rewrite|testcases|plan|source|tree",
  "status": "done|error|draft|changed"
}
```

- `section` — which product tab completed: `analysis`, `rewrite`, `testcases`, or `plan`;
  `source` when a swarm agent publishes a top-level draft (`status: "draft"`); **`tree`**
  (`status: "changed"`) when the epic tree changed — a swarm agent minted a project epic,
  filed a child under it or updated an existing child (`routes/swarm_ingest.rs`). The list
  pane refreshes on `tree`.
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
every node transition (start, finish/cached, error-skip, branch-skip — a not-taken
edge), whenever a node spawns an openable session (so the run view can open it
live), when a `human_approval` node pauses the run, and when the overall run
reaches a terminal status. `routes/workflows.rs` additionally emits on the
approve/reject decision and on cancel, so open views react without waiting for
the engine's own polls.

The payload carries enough for clients to apply the change **in place** instead
of refetching the whole run per event: `rev` is the run's monotonic revision
after this change, and `node` is the changed node's full `NodeRunState` —
omitted when its serialized size exceeds 32 KiB, in which case (or on a rev
gap / a run-level event) clients converge with one rev-guarded
`GET /workflow-runs/{id}`. A 2.5s fallback poll remains while a viewed run is
non-terminal so the view converges even with no WS connection.

```json
{
  "type": "workflow_run_updated",
  "workspace_id": "<Id>",
  "run_id": "<Id>",
  "status": "running|success|error|canceled",
  "node_id": "<node_id | null>",
  "rev": 7,
  "node": { "node_id": "…", "status": "…", "logs": ["…"], "…": "…" },
  "nodes_done": 2,
  "nodes_total": 5,
  "waiting_approval": false
}
```

- `node_id` — the node whose state changed; `null` when the event reflects the
  overall run status (run started / paused for approval / decision / terminal).
- `rev` — the run revision this event reflects (0 = unknown → refetch path).
  Clients drop events/snapshots whose rev is behind what they already show, and
  apply a node payload in place only when `rev` is exactly contiguous.
- `nodes_done`/`nodes_total` — step progress for the "Running" sidebar, updated
  in place without a second GET (`nodes_total` 0 = unknown, keep last counts).
- `waiting_approval` — true on the pause event; the approve/reject decision
  re-emits with false.
- UI routing: `events.svelte.ts` dispatches to `workflowRunBus.apply(event)`
  (run view) **and** `ws.applyWorkflowRunEvent(event)` (in-place "Running"
  sidebar update; a full active-list refetch only for unknown run ids).
- TypeScript type: the `OttoEvent` union in `ui/src/lib/api/types.ts` —
  `{ type: 'workflow_run_updated'; workspace_id: Id; run_id: Id; status:
  string; node_id?: Id | null; rev?: number; node?: NodeRunState | null;
  nodes_done?: number; nodes_total?: number; waiting_approval?: boolean }`.

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
- Eval lab: each matrix **cell** is a `skill_evals` row, so it emits this same
  event; the Matrix view also polls `GET /eval-matrices/{id}` (which lazily settles
  the matrix to `done` once all cells are terminal) for the live grid. The eval
  lab's per-iteration scoring additionally emits `proof_pack_updated` (the
  iteration's Proof Pack), documented above.

## Skills-review update (A12)

Workspace-scoped. Emitted by `crates/otto-server/src/skill_review.rs` as a skills
review advances (running/done/error/cancelled). Lets the Skills Lab Review panel
refresh on demand; the reviewer/summarizer agent shells stream separately over
each session's `/ws/term/{session_id}`.

```json
{
  "type": "skill_review_updated",
  "workspace_id": "<Id>",
  "review_id": "<skill_review_id>",
  "status": "running|done|error|cancelled"
}
```

- `review_id` — the `skill_reviews.id` that changed.
- UI routing: `events.svelte.ts` dispatches to `skillReviewBus.apply()`.
- TypeScript type: added to the `OttoEvent` union in `ui/src/lib/api/types.ts`
  as `{ type: 'skill_review_updated'; workspace_id: Id; review_id: Id; status:
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

---

### `otto_run_updated`

```json
{ "type": "otto_run_updated", "workspace_id": "<Id>", "run_id": "<Id>",
  "status": "queued|resolving_source|building_context|provisioning|executing|proving|reviewing|awaiting_approval|drafting_pr|completed|failed|rejected|cancelled" }
```

- Emitted by `otto_server::run_engine` on every Run with Otto stage transition,
  failure, or completion. `status` is the snake_case `RunStatus`.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`).
- The Run with Otto page re-fetches the run + its timeline on a matching tick.
- TypeScript type: `{ type: 'otto_run_updated'; workspace_id: Id; run_id: Id; status: string }`.

---

### `db_assist_session_started` / `db_assist_updated`

Workspace-scoped. Emitted by `crates/otto-server/src/db_assist.rs` as the DB
Explorer's file-backed "assistant" agent (see api.md → *DB Assistant*) runs a
turn. `db_assist_session_started` fires the moment the agent's session becomes
live (turn start) so the Database page attaches its embedded terminal
immediately, not only after the turn; `db_assist_updated` fires on every
`ANSWER.sql` change while the turn runs (per poll, mid-turn) and once more with
the committed answer.

```json
{ "type": "db_assist_session_started", "workspace_id": "<Id>", "connection_id": "<Id>",
  "assist_id": "<Id>", "session_id": "<Id>" }
{ "type": "db_assist_updated", "workspace_id": "<Id>", "connection_id": "<Id>",
  "assist_id": "<Id>", "sql": "<current proposed query>", "note": "<one-line status>" }
```

- `db_assist_session_started` — the assist's agent session (hidden from the
  Agents list via `meta.source = "db_assist"`) just started its turn; `session_id`
  is the live, attachable session for the matching `assist_id`.
- `db_assist_updated` — the agent's working answer changed. `sql` is the current
  proposed query (its `ANSWER.sql`, else a fenced `sql` block in the reply); `note`
  is a one-line status (its `NOTE.txt`, else the reply's first line). Emitted
  per-poll mid-turn and once with the final committed answer.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`).
- UI routing: the Database page's DB Assistant panel attaches the session on
  `db_assist_session_started` and renders each `db_assist_updated` (`sql` in a
  read-only block with Insert/Run) as the agent works.
- TypeScript types: in the `OttoEvent` union in `ui/src/lib/api/types.ts` as
  `{ type: 'db_assist_session_started'; workspace_id: Id; connection_id: Id; assist_id: Id; session_id: Id }`
  and `{ type: 'db_assist_updated'; workspace_id: Id; connection_id: Id; assist_id: Id; sql: string; note: string }`.

---

### `canvas_updated` / `canvas_session_started`

Workspace-scoped. Emitted by `crates/otto-server/src/canvas_assist.rs` while an
Ask-AI agent turn edits a scene's backing source file (live, per-poll) and once
more with the committed result; `canvas_session_started` fires at the START of
the turn so the Canvas Assistant panel can attach the agent's shell immediately
instead of waiting for it to finish.

```json
{ "type": "canvas_updated", "workspace_id": "<Id>", "scene_id": "<Id>", "doc": {"type":"otto-canvas","format":"mermaid","source":"..."} }
{ "type": "canvas_session_started", "workspace_id": "<Id>", "scene_id": "<Id>", "session_id": "<Id>" }
```

- `doc` — the opaque canvas document (`{type,format,source,…}`); the open editor
  re-renders it for the matching `scene_id` without a refetch.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`).
- UI routing: `events.svelte.ts` → `canvasDocBus.apply()` (`canvas_updated`); sets
  `canvas.sessionId` directly for the open scene (`canvas_session_started`).
- TypeScript types: `{ type: 'canvas_updated'; workspace_id: Id; scene_id: Id; doc: unknown }`
  and `{ type: 'canvas_session_started'; workspace_id: Id; scene_id: Id; session_id: Id }`.

### `mockup_updated` / `mockup_session_started`

Workspace-scoped. Emitted by `crates/otto-server/src/mockup_assist.rs` (live
per-poll while the design agent edits the file, and once with the committed
result), by `product_media.rs` on every `PUT /product/attachments/{aid}/content`
save from the UI, by `routes/swarm_ingest.rs` when a swarm agent publishes an
artifact, and by `design_blender.rs` for each output a Blender render job
attaches — same shape and timing as the canvas pair above, but for a product
story's design artifact.

```json
{ "type": "mockup_updated", "workspace_id": "<Id>", "story_id": "<Id>", "attachment_id": "<Id>", "format": "html|mermaid|excalidraw|scene3d|<mime>", "content": "..." | null }
{ "type": "mockup_session_started", "workspace_id": "<Id>", "story_id": "<Id>", "attachment_id": "<Id>", "session_id": "<Id>" }
```

- `format` — a `DesignFormat` name for the four text formats; for uploaded /
  rendered binaries (`image/png`, `model/gltf-binary`) it is the attachment's mime.
- `content` — `Option<String>`: the new source for text formats; **an explicit
  `null`** (never omitted — no `skip_serializing_if`) for binaries and payloads over
  4 MB, in which case clients re-fetch `GET /product/attachments/{aid}`.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`).
- UI routing: `events.svelte.ts` → `mockupAssist.ingestLive()` (`mockup_updated`;
  the `null`-content branch re-fetches) / `mockupAssist.setSession()`
  (`mockup_session_started`); the Product → Design arena re-renders the viewer
  for the matching `attachment_id`. If the user has unsaved local edits the
  panel asks before replacing (no silent clobber).
- TypeScript types: `{ type: 'mockup_updated'; workspace_id: Id; story_id: Id; attachment_id: Id; format: string; content: string | null }`
  and `{ type: 'mockup_session_started'; workspace_id: Id; story_id: Id; attachment_id: Id; session_id: Id }`.

### `canvas_refs_changed`

Workspace-scoped. Emitted by `crates/otto-server/src/canvas_refs.rs` whenever a
Canvas scene is attached to or detached from an agent session.

```json
{ "type": "canvas_refs_changed", "workspace_id": "<Id>", "session_id": "<Id>" }
```

- Emitted after `POST /sessions/{sid}/canvas-refs` and
  `DELETE /sessions/{sid}/canvas-refs/{scene_id}`.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`), like
  the other canvas events — Canvas is a workspace-shared tool, not owner-gated.
- UI routing: `events.svelte.ts` → `canvasRefsBus.apply()`; the session's Canvas
  panel (`CanvasPanel.svelte`) re-fetches `GET /sessions/{id}/canvas-refs` when
  the event's `session_id` matches the open session.
- TypeScript type: `{ type: 'canvas_refs_changed'; workspace_id: Id; session_id: Id }`.

### `transcript_appended` / `transcript_live` / `artifact_added` / `history_index_progress`

Conversation view (`docs/design/conversation-view.md` §4.3). Emitted by
`crates/otto-server/src/transcript_tail.rs` (the per-session live tail, armed by
`GET /sessions/{id}/transcript` on a live session; 700 ms poll) and
`history_index.rs` (rescan progress).

```json
{"type":"transcript_appended","workspace_id":"…","session_id":"…","cursor":"<record_index>","turns":[{…Turn…}]}
{"type":"transcript_live","workspace_id":"…","session_id":"…","text":"⏺ Still exploring…","input":"option 2","status":"~ | Fable 5.1 | ▓▓░░ 11%","branch":"main"}
{"type":"artifact_added","workspace_id":"…","session_id":"…","artifact":{…Artifact…}}
{"type":"history_index_progress","workspace_id":"…","scanned":120,"total":2705,"done":false}
```

- `transcript_appended` — the session's transcript grew. `turns` are the turns
  touched by the new records, each sent WHOLE (a turn whose tool results just
  landed is re-sent) — clients replace by `Turn.id`. `cursor` is the index of the
  LAST folded record (`after_cursor`). A payload over 64 KB is sent with
  `turns: []`: re-fetch `GET …/transcript`. Session-family scoped
  (owner / workspace admin / root, viewer-gated) — transcript prose and tool
  output never reach other users.
- `transcript_live` — the agent's in-progress response as currently drawn on
  the session's terminal screen (plain text rows between the last prompt echo
  and the input box, spinner rows dropped, ≤ 16 KB tail). The provider writes
  a transcript record only when a block COMPLETES, so this is the sub-turn
  streaming signal; pushed at most once per tail poll (700 ms) and only when
  the text changed. `text: ""` = nothing streaming. Clients show it as a draft
  under the last turn while the session is `working` and drop it once the
  folded turn covers it. `input` = unsent text in the terminal's input box
  (the CLI appends a chat send to it and submits ONE message, so the chat shows
  it as the prefix); `status` = the CLI's own status rows under the box (model,
  context %, plan limits, mode …) joined by " · "; `branch` = the session cwd's
  git branch from `.git/HEAD` (re-read every 10 s). Session-family scoped.
- `artifact_added` — the fold found a new artifact (written file, PR link, image).
  Carries the full `Artifact`. Session-family scoped.
- `history_index_progress` — a `POST /workspaces/{wid}/history/rescan` walk
  advanced; `done: true` on the final tick. Workspace-scoped (viewer+). The boot
  scan emits nothing (it has no requesting workspace).
- TypeScript types live in `ui/src/lib/api/types.ts` (`// ── Transcript`).

### `browser_tab_updated` / `browser_annotation_added`

Workspace-scoped. Emitted by `crates/otto-server/src/routes/browser.rs` when a
browser tab is created or navigated (including the reader-mode fetch pipeline
adopting the fetched page's title) and when a DOM annotation is created. `tab`
and `annotation` are the serialized `otto_state::browser::{BrowserTab,
BrowserAnnotation}` rows (opaque here — otto-core can't depend on otto-state,
like `canvas_updated`'s `doc`).

```json
{ "type": "browser_tab_updated", "workspace_id": "<Id>", "tab": {"id":"…","workspace_id":"…","url":"…","title":"…","mode":"reader","created_at":"…"} }
{ "type": "browser_annotation_added", "workspace_id": "<Id>", "annotation": {"id":"…","workspace_id":"…","tab_id":"…","url":"…","selector":"…","excerpt":"…","text":"…","comment":"…","color":"…","created_at":"…"} }
```

- `browser_tab_updated` — emitted after `POST /workspaces/{wid}/browser/tabs`
  and `PATCH /browser/tabs/{id}` (a mode change and/or navigation).
- `browser_annotation_added` — emitted after
  `POST /workspaces/{wid}/browser/annotations`.
- Scope: `Workspace` (delivered to members with viewer+ on `workspace_id`), like
  the other canvas-family live-edit events.
- TypeScript types: `{ type: 'browser_tab_updated'; workspace_id: Id; tab: unknown }`
  and `{ type: 'browser_annotation_added'; workspace_id: Id; annotation: unknown }`.

### `aws_account_updated` / `aws_install_updated`

Global scope (the AWS account registry is a global library, like connections)
— delivered to every authenticated client; the client re-lists
`GET /aws/accounts` and RBAC filtering happens on that call. Emitted by
`crates/otto-aws` (`accounts.rs` / `install.rs`).

```json
{ "type": "aws_account_updated", "account_id": "<Id>", "deleted": false }
{ "type": "aws_install_updated", "tool": "aws", "state": "running" }
```

- `aws_account_updated` — after `POST /aws/accounts`, `PATCH /aws/accounts/{id}`,
  `DELETE /aws/accounts/{id}` (`deleted: true`) and after a permission probe
  refreshed `permissions_json`.
- `aws_install_updated` — on every installer state change
  (`idle → running → done | failed`); the UI polls `GET /aws/status` for the
  `log_tail` while `running`.
- TypeScript types: `{ type: 'aws_account_updated'; account_id: Id; deleted: boolean }`
  and `{ type: 'aws_install_updated'; tool: 'aws'; state: 'idle'|'running'|'done'|'failed' }`.

### `k8s_cluster_updated` / `k8s_install_updated`

Global scope (every authenticated client) — the Kubernetes console's cluster
registry and its installers are global, like connections. Emitted by
`crates/otto-k8s` (`clusters.rs` on create / import / patch / delete;
`install.rs` on every installer state transition).

```json
{ "type": "k8s_cluster_updated", "cluster_id": "<Id>", "deleted": false }
{ "type": "k8s_install_updated", "tool": "kubectl", "state": "running" }
```

- `k8s_cluster_updated` — after `POST /k8s/clusters`, `POST /k8s/clusters/import`,
  `PATCH /k8s/clusters/{id}` (`deleted: false`) and `DELETE /k8s/clusters/{id}`
  (`deleted: true`). Clients re-fetch `GET /k8s/clusters` (or drop the row).
- `k8s_install_updated` — `tool ∈ kubectl | k9s`, `state ∈ running | done | failed`
  (`idle` is never broadcast). Clients polling `GET /k8s/status` every 1.5 s during
  an install can stop on `done`/`failed`; the full `InstallJob` (log tail, error)
  is only in `/k8s/status`.
- TypeScript types: `{ type: 'k8s_cluster_updated'; cluster_id: Id; deleted: boolean }`
  and `{ type: 'k8s_install_updated'; tool: 'kubectl' | 'k9s'; state: string }`.

### `k8s_monitor_cycle`

Global scope. Emitted by `crates/otto-k8s/src/monitor/collector.rs` after every
monitoring cycle of an enabled cluster (see `api.md` "Kubernetes monitoring").

```json
{ "type": "k8s_monitor_cycle", "cluster_id": "<Id>", "ok": true,
  "pods_scraped": 106, "pods_failed": 2, "cycle_ms": 28417 }
```

- `ok` — samples were written this cycle (`false` = the cluster was unreachable
  or the ClickHouse write failed; `GET /k8s/clusters/{id}/monitor` carries the
  error text in `status.last_error`).
- Clients on the Monitor dashboard re-fetch `GET /k8s/monitor/overview` and the
  open cluster's `…/monitor/workloads`; nothing else needs to react.
- TypeScript: `{ type: 'k8s_monitor_cycle'; cluster_id: Id; ok: boolean; pods_scraped: number; pods_failed: number; cycle_ms: number }`.
