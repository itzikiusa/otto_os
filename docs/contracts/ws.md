# Otto WebSocket Contract (FROZEN)

Two WS endpoints. Auth for both: a bearer token validated BEFORE the upgrade
completes; invalid token → HTTP 401, no upgrade.

- The event stream (`/ws/events`) accepts the token via the
  `Sec-WebSocket-Protocol` request header — the client offers
  `["otto-bearer", "<token>"]` and the server echoes back `otto-bearer` on
  success. This keeps the token out of the URL/query string (which is logged by
  proxies and servers). A `?token=<bearer token>` query parameter is still
  accepted as a backward-compatible fallback.
- The terminal stream (`/ws/term/{session_id}`) authenticates via the
  `?token=<bearer token>` query parameter.

## 1. Terminal stream — `WS /ws/term/{session_id}`

Role: workspace **viewer** may attach (read-only); **editor**+ may send input/resize.
Input frames from viewers are silently dropped server-side (and a single JSON
`{"type":"error","code":"forbidden"}` is sent once).

### Client → server (JSON text frames)

```json
{"type":"input","data":"<base64 bytes>"}
{"type":"resize","cols":120,"rows":32}
{"type":"scrollback","lines":2000}
```

### Server → client

- **Binary frames**: raw PTY output bytes — write straight into xterm.
- **JSON text frames**:

```json
{"type":"scrollback","data":"<base64 bytes>"}      // response to scrollback request; send BEFORE live bytes resume
{"type":"status","status":"working"}                // running|working|idle|exited|reconnectable
{"type":"exit","code":0}                            // child exited; socket stays open
{"type":"error","code":"forbidden","message":"..."}
```

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
```

- `swarm_status` — a swarm's lifecycle status changed (`active|paused|aborted`).
- `swarm_run_updated` — a swarm run was created or changed.
- `swarm_task_updated` — a swarm task was created or changed.
- `swarm_message_posted` — a new message was posted to a swarm's shared board.

The UI mirrors all of these in `OttoEvent` (`ui/src/lib/events.svelte.ts`) and routes the
`swarm_*` set into the `swarm` store, which updates the org tree, run graph, Kanban, runs
list and board live.
