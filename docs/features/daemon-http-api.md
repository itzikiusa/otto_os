# Otto ottod HTTP API — Driving Otto Programmatically

Everything Otto's desktop UI does goes through one process: **`ottod`**, an Axum
HTTP+WebSocket server bound to `127.0.0.1:7700` (loopback only by default). The
Svelte UI is a thin client over it. Anything the UI can do, a script, an agent,
or a CI job can do too — authenticate once with a bearer token, then call
`/api/v1/*`. This guide is the operator's/developer's guided tour of that
surface: how to authenticate, a navigable map of every domain the daemon exposes,
the two WebSockets, the async-action pattern, and — the part that matters most —
**what you CAN and CANNOT do over the API**.

> **The contract is authoritative and FROZEN.** This doc summarizes
> [`docs/contracts/api.md`](../contracts/api.md) (the REST surface) and
> [`docs/contracts/ws.md`](../contracts/ws.md) (the WebSockets). Those files are
> the source of truth; the TypeScript types in `ui/src/lib/api/types.ts` mirror
> them. When this doc and a contract disagree, **the contract wins** — file an
> issue against this doc. Every path here is real and registered; none is
> invented. For the permission model behind the role columns, see
> [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) and
> [`docs/MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).

---

## 1. Overview & base URL

- **One surface.** `ottod` owns sessions, PTYs, git/PRs, code review, the product
  workflow, Jira/Confluence, Slack/Telegram channels, SSH/DB/broker connections,
  the Database Explorer, swarms, usage/insights, self-improvement, skills, the
  HTTP API client, workflows, memory, and admin tooling. All of it is HTTP.
- **Base URL.** `http://127.0.0.1:7700/api/v1`. The root level (no `/api/v1`
  prefix) carries the health/meta endpoints and the WebSocket + proxy routers
  (see §5 and the root-token table in §4).
- **Loopback-only by default.** The daemon listens on `127.0.0.1` and is *not*
  reachable from the network unless a root user explicitly enables a network
  listener (`PUT /api/v1/settings`, key `network_listener {enabled, port}`). Do
  not change that casually — see §8.
- **Wire conventions.** JSON bodies, **snake_case** fields, **RFC3339**
  timestamps, **ULID** ids. Errors are an HTTP status mapped from an
  `otto_core::Error` variant plus a body `Problem { code, message }`:

  | HTTP | `code` | Meaning |
  |------|--------|---------|
  | 400 | `invalid` | Malformed/invalid request |
  | 401 | `unauthorized` | Missing/expired/revoked token |
  | 403 | `forbidden` | Authenticated but not permitted (RBAC / scope / guard) |
  | 404 | `not_found` | No such row, or not visible to you |
  | 409 | `conflict` | Duplicate / write-guard block (`write_blocked: …`) |
  | 502 | `upstream` | A proxied tool/service failed (git, sftp, SMTP, …) |

- **Health check.** `GET /api/v1/health → {"ok":true}` is public (no token) —
  use it to confirm the daemon is up before authenticating.

---

## 2. Authentication

`ottod` accepts **bearer tokens** on every `/api/v1` route except a tiny public
set. All token kinds flow through the same `Authorization: Bearer <token>` path
(`crates/otto-rbac/src/tokens.rs`). There are four kinds, distinguished by
lifetime and scope:

| Kind | Lifetime | How obtained | Scope |
|------|----------|--------------|-------|
| **Login (`session`)** | 30-day **sliding** (refreshed on use, throttled hourly) | `POST /auth/login` | The user's full role set |
| **PAT (`api`)** | ~10-year **fixed** (never slid) | `POST /auth/tokens` | The user's full role set (inherits creator's roles) |
| **Share (`share`)** | short **fixed** TTL (60s–24h; OTP window ≤12h) | `POST /sessions/{id}/share` | ONE session, capped Viewer/Editor, never root |
| **Impersonation** | 30-min **fixed** (never slid) | `POST /admin/impersonate/{user_id}` | Effective = the target user; audited as the admin |

A token is **32 random bytes, hex-encoded (64 chars)**; only its SHA-256 hash is
stored. The raw secret is shown **exactly once** at creation.

### 2.1 Creating a Personal Access Token (PAT)

The intended credential for scripts, CI, agents, and the Otto operating skills.

**In the UI:** Settings → **Personal Access Tokens** → enter an optional label →
**Create token**. The raw secret is shown once in a green banner — copy it
immediately; it is never shown again. The list afterward shows only the 12-char
`token_prefix`, last-used, and expiry.

**Over the API** (bootstrap with a one-time login):

```bash
# 1. Log in (or onboard the first root user on a fresh daemon via /onboarding/root)
LOGIN=$(curl -s -X POST http://127.0.0.1:7700/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"me","password":"…"}' | jq -r .token)

# 2. Mint a long-lived PAT
curl -s -X POST http://127.0.0.1:7700/api/v1/auth/tokens \
  -H "Authorization: Bearer $LOGIN" \
  -H 'content-type: application/json' \
  -d '{"label":"CI pipeline"}' | jq    # → { "token": "…(shown once)…", "info": {…} }

# 3. Save the secret and use it everywhere
export OTTO_API_TOKEN=…
curl -s http://127.0.0.1:7700/api/v1/auth/me \
  -H "Authorization: Bearer $OTTO_API_TOKEN" | jq
```

PAT lifecycle endpoints (api.md #87–#89, all `member`/self-scoped):

| Method & path | Purpose |
|---|---|
| `POST /auth/tokens` | Mint a PAT `{label?}` → `{token, info}` (secret once) |
| `GET /auth/tokens` | List your PATs (`ApiTokenInfo[]`; never the secret; newest first) |
| `DELETE /auth/tokens/{id}` | Revoke one of **your own** PATs (404 if not found/owned) |

`ApiTokenInfo = {id, label?, token_prefix, created_at, last_seen_at, expires_at}`.
`last_seen_at` updates on use (throttled to once/hour). **An impersonation
session cannot mint a PAT** (`POST /auth/tokens` → 403 when real ≠ effective) — an
admin acting-as a user can't forge a long-lived credential as that user.

### 2.2 The login token

`POST /auth/login {username, password} → LoginResp {token, …}` returns a 30-day
**sliding** session token (its expiry slides forward on use, refreshed at most
once an hour). It works as a bearer token identically to a PAT, but is meant for
the interactive UI, not for automation. `POST /auth/logout` revokes it.
`GET /auth/me` returns `{user, real_user, impersonating}` — `user` is the
effective (acted-as) identity, `real_user` is the token owner.

### 2.3 Public (no-token) routes

The *only* `/api/v1` routes that need no bearer token:

`GET /health`, `GET /meta`, `POST /onboarding/root` (only while 0 users exist,
else 409), `POST /auth/login`, the share-gate pair `POST /share/verify` and
`POST /share/extend` (the share token in the body is the auth), and the four
`/ingest/*` routes (session-token gated, §2.5).

### 2.4 Root-level `?token=` routers (WebSockets & proxies)

The WebSocket and proxy endpoints live at the **server root** (not under
`/api/v1`) and self-authenticate via a query parameter or the
`Sec-WebSocket-Protocol` header rather than the bearer middleware:

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /ws/term/{session_id}` | `Sec-WebSocket-Protocol: otto-bearer, <token>` (preferred) or `?token=` | Terminal stream (§5) |
| `GET /ws/events` | `Sec-WebSocket-Protocol: otto-bearer, <token>` (preferred) or `?token=` | Daemon event stream (§5) |
| `GET /ws/lsp?lang=&root=&token=` | `?token=`; ws editor | LSP WebSocket bridge |
| `GET /ws/api-client/stream?token=` | `?token=`; ws editor | API-client streaming-response bridge |
| `GET /browser/proxy?url=&token=` | `?token=` | In-app browser HTTP proxy |

> Prefer the `Sec-WebSocket-Protocol: otto-bearer, <token>` form for the two
> WebSockets — it keeps the token out of the URL (which proxies and servers log).
> `?token=` remains a backward-compatible fallback. Any token kind works.

### 2.5 The per-session ingest token

Agent **hooks** run inside a spawned CLI session and have no user bearer token. To
write telemetry they use a **per-session ingest token** that Otto sets on the
agent PTY's environment, presented as `X-Otto-Session` + `X-Otto-Token` headers
(verified inside each handler). These routes are reachable without a user bearer:

| Method & path | Purpose |
|---|---|
| `POST /ingest/claude` · `POST /ingest/codex` | Agent hook events → trail/tasks/status |
| `POST /ingest/usage` | Per-session token-usage ingest (feeds the usage store) |
| `POST /ingest/swarm/board` | A swarm agent posts to its shared board (`otto-post` helper) |

You do not call these by hand; they exist so the agent's own hooks can feed the
activity trail, task tracker, usage metering, and swarm board. The bearer-auth'd
`GET …/trail`, `PUT …/tasks`, etc. are how *humans* read/write the same data.

### 2.6 RBAC: roles gate endpoints

Authorization runs on three independent axes; **root bypasses all of them**:

1. **Feature capability** — per user, per feature: `None < View < Edit < Admin`
   (default-deny). One central guard maps each route to a `(feature, capability)`.
2. **Workspace role** — `viewer < editor < admin`, per workspace. The "Auth"
   column in the maps below names the minimum: `member` (any authenticated user),
   `ws viewer/editor/admin`, or `root` (global).
3. **Ownership** — sessions, query history, saved queries (and optionally
   connections) are private to their creator.

A PAT inherits its creator's roles: a token minted by a root user has root;
otherwise it has that user's workspace roles + feature grants. Full model and the
"Database-only user" recipe: [`docs/MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).

---

## 3. API map by domain

Below is a navigable reference — one to three representative routes per domain,
with the minimum role and a one-line purpose. **It is not the full dump**: the
authoritative, exhaustive list is [`docs/contracts/api.md`](../contracts/api.md)
(every route the daemon registers via `crates/otto-server/src/modules.rs`). All
paths are under `/api/v1`. Item routes keyed by a row id (e.g. `/sessions/{id}`)
resolve their owning workspace from the row and role-check against it.

### Identity, users, workspaces, settings

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /meta` · `GET /auth/me` | public · member | Daemon/build metadata · current identity |
| `GET /users` · `POST /users` · `PATCH/DELETE /users/{id}` | root | User CRUD (root cannot be disabled) |
| `GET /workspaces` · `POST /workspaces` | member | List workspaces (root sees all) · create one |
| `GET/PUT /workspaces/{id}/members` | ws admin | Read / set workspace membership + roles |
| `GET/PUT /settings` | root | Read / write the settings map (`network_listener`, `providers`, `cli_auto_update`, …) |

### Sessions (agent / shell / connection PTYs)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET/POST /workspaces/{id}/sessions` | viewer / editor | List · create a session (`CreateSessionReq`) |
| `GET/PATCH/DELETE /sessions/{id}` | viewer / editor | Inspect · update · kill+remove |
| `POST /sessions/{id}/restart` | editor | Respawn (resume-aware when `provider_session_id` set) |
| `POST /sessions/{id}/input` | editor | Write a keystroke/paste into the PTY (`{text, submit?}`) |
| `POST /sessions/{id}/archive`·`/unarchive`·`/handover` | editor | Archive · restore · start an agent handover |
| `GET .../sessions/{sid}/trail` · `GET/PUT .../tasks` | viewer / editor | Read the activity trail · read/replace the task list |

> A `kind=connection` session requires a `connection_id`; provider is set
> server-side to the connection kind. See [`./agent-sessions.md`](./agent-sessions.md)
> for the session lifecycle and terminal attach.

### Git, repos & pull requests

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /git/repos` · `GET /git/accounts` | Git:View · member | All repos you can view · your git accounts |
| `GET/POST /workspaces/{id}/repos` | viewer / editor | List · add a repo (clone runs async) |
| `GET /repos/{id}/status`·`/branches`·`/log`·`/diff` | viewer | Working-tree status, refs, history, diffs |
| `POST /repos/{id}/stage`·`/commit`·`/push`·`/pull`·`/checkout` | editor | Stage, commit, push, pull, checkout |
| `POST /repos/{id}/branch`·`/merge`·`/cherry-pick`·`/revert`·`/tag` | editor | Branch/merge/cherry-pick/revert/tag ops |
| `GET/POST /repos/{id}/prs` · `GET /repos/{id}/prs/{n}` | viewer / editor | List/create PRs · PR detail |
| `POST /repos/{id}/prs/{n}/merge`·`/approve`·`/decline`·`/comments` | editor | Merge/approve/decline/comment on a PR |
| `POST /repos/{id}/pr/draft`·`/draft-commit-message` | editor | AI-draft a PR title+body · a commit message |

### Code review (multi-agent)

| Method & path | Auth | Purpose |
|---|---|---|
| `POST /repos/{id}/prs/{n}/review` | editor | Start the agent fan-out review of a PR (async) |
| `GET /repos/{id}/prs/{n}/review`·`/reviews` | viewer | Latest review (live state) · history |
| `POST /repos/{id}/local-review` | editor | Review the working diff (no PR) |
| `POST /reviews/{rid}/handoff` · `.../agents/{i}/retry` | editor | Hand findings to a session · re-run a stuck agent |
| `GET /reviews/{rid}/findings`·`/merge-readiness` | viewer | Persistent fingerprinted findings · merge gate |

### Product (stories, analyses, plans, test cases)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET/POST /workspaces/{ws}/product/stories` | viewer / editor | List · import a story |
| `POST .../stories/{sid}/analyze`·`/rewrite`·`/testcases/generate`·`/plan/generate` | editor | AI actions (async, 202; §6) |
| `GET /product/stories/{sid}/analyses`·`/questions`·`/testcases` | viewer | Derived artifacts |
| `POST /product/stories/{sid}/to-swarm` | editor | Spin a swarm project from the story + seed tasks |
| `GET/POST /workspaces/{ws}/product/learnings` | viewer / editor | The global Stories+Learnings library |

> Full Product surface (versions, notes, transcripts, drafts, testcase
> runs/approve/publish) is in [`docs/contracts/product.md`](../contracts/product.md)
> and the Product section of `api.md`.

### Issue trackers (Jira / Confluence)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET/POST /issue/accounts` | member (owner) | Your Jira/Confluence accounts (token never returned) |
| `GET /issue/search`·`/projects`·`/confluence/search` | member | JQL/CQL search, projects, Confluence pages |
| `GET /issue/{account}/{key}`·`/full`·`/transitions` | member | Issue summary, detail, transitions |
| `POST /issue/{account}/{key}/comment`·`/transitions` | member | Comment · apply a transition |

### Channels (Telegram / Slack / Loom)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /workspaces/{id}/integrations` | viewer | Configured channel integrations |
| `PUT/DELETE /workspaces/{id}/integrations/{channel}` | editor | Upsert · remove an integration |
| `POST /workspaces/{id}/integrations/{channel}/test` | editor | Send a test message |

### Connections (SSH / MySQL / Redis / Mongo / ClickHouse) + SFTP

| Method & path | Auth | Purpose |
|---|---|---|
| `GET/POST /workspaces/{id}/connections` | viewer / editor | List (incl. global; secret never present) · upsert |
| `PATCH/DELETE /connections/{id}` | editor (global: root) | Edit (PATCH preserves guard) · delete (drops Keychain secret) |
| `POST /connections/{id}/open`·`/test` | editor | Open a connection PTY · connectivity probe |
| `GET /connections/{id}/sftp/list`·`/read` | viewer | Browse / read over the SSH connection's auth |
| `POST /connections/{id}/sftp/download`·`/upload`·`/mkdir`·`/remove`·`/rename` | editor | SFTP transfers/mutations (require `kind==ssh`) |

### Database Explorer (engine access + saved artifacts)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /connections/{id}/db/schema`·`/capabilities` | viewer | Schema tree · engine capability flags |
| `POST /connections/{id}/db/query` | editor | Run a query (`RunQueryReq`; write-guard, §8) |
| `POST /connections/{id}/db/cancel` | editor | Engine-native cancel of an in-flight `query_id` |
| `POST /connections/{id}/db/export`·`/export-to-path` | editor | Stream results to browser CSV/JSON · to a local file |
| `GET/POST /workspaces/{wid}/db/saved-queries`·`/dashboards`·`/widgets` | viewer / editor | Saved queries, dashboards, widgets |

### Message Brokers (Kafka viewer)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET/POST /workspaces/{wid}/brokers/clusters` | viewer / editor | List (workspace+global) · add a cluster |
| `GET /brokers/clusters/{id}/overview`·`/metrics`·`/topics`·`/groups` | viewer | Cluster overview, metrics, topics, consumer groups+lag |
| `POST /brokers/clusters/{id}/topics/{topic}/consume`·`/produce` | viewer / editor | Peek messages · produce (guarded clusters need `confirm`) |
| `POST /brokers/clusters/{id}/groups/{group}/reset`·`/replay` | editor | Reset offsets (`?dry_run`) · replay topic→topic |

### Agent Swarm

| Method & path | Auth | Purpose |
|---|---|---|
| `GET/POST /workspaces/{id}/swarm/swarms` | viewer / editor | List · create a swarm (blank or from preset) |
| `GET/POST /swarm/swarms/{sid}/agents`·`/projects` | viewer / editor | Role agents · projects |
| `POST /workspaces/{id}/swarm/projects/{pid}/plan` | editor | Decompose a project into `SwarmTask[]` |
| `POST /swarm/tasks/{tid}/run` · `GET /swarm/runs/{rid}` | editor / viewer | Run a task · inspect a run (tokens/cost backfilled) |
| `POST /workspaces/{id}/swarm/swarms/{sid}/start`·`/pause`·`/abort`·`/resume` | editor | Lifecycle (budget guardrails auto-pause) |

### Workflows (visual node-graph automations)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /workflows/node-types`·`/templates` | member | Available node kinds · templates |
| `GET/POST /workspaces/{wid}/workflows` | viewer / editor | List · create (also `/from-template`, `/generate`) |
| `POST /workflows/{id}/run` · `POST /workflow-runs/{id}/cancel` | editor | Run a workflow · cancel a run |
| `POST /workflows/{id}/webhook/{token}` | public-by-token | Trigger a run via webhook (token in workflow_triggers) |

### API client ("Postman") + memory + context library

| Method & path | Auth | Purpose |
|---|---|---|
| `GET/POST /workspaces/{wid}/api-client/collections`·`/requests`·`/environments` | viewer / editor | Saved HTTP/gRPC requests & envs |
| `POST /workspaces/{wid}/api-client/execute`·`/grpc/invoke` | editor | Execute an HTTP request · a gRPC call |
| `GET/POST /workspaces/{ws}/memories` · `POST .../memory/search`·`/recall` | viewer / editor | Workspace knowledge store; hybrid recall |
| `GET/PUT /library/skills/{name}`·`/souls`·`/context` | root | The shared skill/soul/context library |
| `GET/PUT /workspaces/{id}/context` · `POST .../context/preview` | viewer/admin | Per-workspace context selection · dry-run spawn |

### Usage, insights, self-improvement, skill-eval

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /usage/summary?days=N`·`/by-kind`·`/metrics` | root | Token/cost rollups · per-feature · system CPU/RAM |
| `GET/PUT /usage/budgets` | root | Spend caps (enforcement opt-in, default off) |
| `GET /insights/reports`·`/report` · `POST /insights/run` | root | Generated reports · trigger an insights run |
| `GET/POST /workspaces/{id}/self-improvement`·`/run` | viewer/editor | Config · trigger a self-reflection run |
| `POST /workspaces/{id}/skill-evaluations` · `.../promote` | editor / root | Evaluate a skill · promote the winner (root) |

### Notifications, plugins, admin, trust & safety, operator tools

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /notifications` · `POST /notifications/read-all` | member | Your notices + global · mark all read |
| `GET /plugins` · `GET/POST /plugin-admin` | member / root | Enabled plugins for the sidebar · install/manage sidecars |
| `GET /admin/sessions` · `POST /admin/sessions/{id}/terminate` | Users:Admin or root | Daemon-wide session overview · force-terminate (audited) |
| `POST /admin/impersonate/{user_id}` · `/stop` | Users:Admin or root | Mint an act-as token · end it (audited) |
| `GET /audit-log` · `GET /security-posture` | root | Append-only security ledger · listener/loopback/token posture |
| `GET /fs/browse`·`/fs/read` · `GET /logs/daemon` | member / root | Path-picker FS reads · recent daemon log lines |

---

## 4. WebSockets

Two long-lived WebSockets carry live data. Both validate the token **before** the
upgrade completes (invalid → HTTP 401, no upgrade); an IP that fails too often is
locked out (429). Both live at the server root (not `/api/v1`) and accept the
token via `Sec-WebSocket-Protocol: otto-bearer, <token>` (preferred) or `?token=`.
Spec: [`docs/contracts/ws.md`](../contracts/ws.md).

### 4.1 Terminal stream — `WS /ws/term/{session_id}`

Attach to a session's PTY. **Viewer** may attach read-only; **editor**+ may send
input/resize (viewer input frames are silently dropped, with one
`{"type":"error","code":"forbidden"}`). Multiple clients may attach to one
session and all receive the same output broadcast.

- **Client → server** (JSON text frames): `{"type":"input","data":"<base64>"}`,
  `{"type":"resize","cols":…,"rows":…}`, `{"type":"scrollback","lines":…}`,
  `{"type":"search","query":"…"}` (server-side ring-buffer grep, ≤200 matches).
- **Server → client**: raw **binary** PTY bytes; plus JSON frames `status`
  (`running|working|idle|exited|reconnectable`), `exit` (child ended; socket
  stays open), `terminated` (force-dropped — admin terminate or share revoke;
  socket closes), `scrollback`, `search_result`, `error`.

### 4.2 Event stream — `WS /ws/events`

Server → client only; each message is one JSON `otto_core::event::Event` (tag
field `type`, snake_case). The server **filters by scope**: a client receives a
**session-family** event only if it is a `viewer`+ member of the event's
workspace AND the session's owner / a workspace admin / root;
**workspace-scoped** events (improvement, swarm, workflow, review, product, etc.)
reach every `viewer`+ member of the workspace; **broadcast** events reach all
authenticated clients. Root receives all. Client→server frames are ignored.

```bash
# wscat with the token kept out of the URL via the subprotocol:
wscat -s "otto-bearer, $OTTO_API_TOKEN" -c "ws://127.0.0.1:7700/ws/events"
# (or the fallback) wscat -c "ws://127.0.0.1:7700/ws/events?token=$OTTO_API_TOKEN"
```

### 4.3 Event catalog (what you can subscribe to)

| Event `type` | Scope | Meaning |
|---|---|---|
| `session_status` | session-family | A session's live status changed |
| `session_created` · `session_removed` | session-family | A session row was created · removed (PTY killed) |
| `session_meta_updated` | session-family | A session's `meta` changed (e.g. handover progress) |
| `trail_appended` · `tasks_updated` | session-family | Activity-trail entry · task-list change |
| `notice` | broadcast | Transient toast (info/warn/error) |
| `notification` | per-user / scoped | A persisted notification row (credential expiry, …) |
| `improvement_run_started`·`_finished`·`_edit_applied`·`_approval_pending` | workspace | Self-improvement run lifecycle |
| `improvement_updated` | everyone | Self-improve run finished / approval pending |
| `swarm_status`·`swarm_run_updated`·`swarm_task_updated`·`swarm_message_posted` | workspace | Swarm lifecycle, runs, tasks, board |
| `review_changed` | workspace | A PR/local review changed state |
| `product_changed` | workspace | A product AI-run completed (analysis/rewrite/testcases/plan) |
| `plan_run` | workspace | Multi-agent plan kickoff — live planning session ids |
| `workflow_run_updated` | workspace | A workflow run/node transitioned |
| `skill_eval_updated` | workspace | A skill evaluation reached a terminal state |
| `usage_metrics_tick` | everyone | A new system-metrics sample was stored |
| `insight_ready` | everyone | A scheduled insights run finished |
| `budget_exceeded` | everyone | A spend cap was crossed (enforcement on) |

---

## 5. Async actions (202 Accepted + WS completion)

Several AI-producing routes don't block on the agent — they **kick off
background work and return immediately**, then stream progress over `/ws/events`.
The pattern:

1. `POST` the action (e.g. `…/product/stories/{sid}/analyze`,
   `…/rewrite`, `…/testcases/generate`, `…/plan/generate`,
   `…/prs/{n}/review`, swarm `…/tasks/{tid}/run`). It returns **202 Accepted**
   (or a record to poll, e.g. a `Review`/`Analysis`/`SwarmRun` with live state).
2. **Subscribe to `/ws/events`** and wait for the matching completion event —
   `product_changed`, `review_changed`, `swarm_run_updated`,
   `workflow_run_updated`, `skill_eval_updated`, etc.
3. **Or poll** the corresponding `GET` for the latest state (the events exist so
   you don't have to poll tightly, but a poll is always a valid fallback).

Repo clone (`POST /workspaces/{id}/repos`) is similarly async — progress arrives
as `Notice` events. Plan generation additionally emits `plan_run` frames listing
the live planning sessions as they spawn.

---

## 6. Capabilities & limitations — what you CAN and CANNOT do

### You CAN (everything the UI can — it's the same API)

- **Drive agent/shell/connection sessions end to end** — create, send input
  (`/sessions/{id}/input`), restart (resume-aware), archive, hand over, attach to
  the live terminal over `/ws/term`, and read the activity trail + task tracker.
- **Operate git fully** — status/diff/log, stage/commit/push/pull/checkout,
  branch/merge/cherry-pick/revert/tag, and the full PR lifecycle
  (create/comment/approve/merge/decline), plus AI-drafted PR/commit text.
- **Run multi-agent code review** and product workflows (analyze, rewrite,
  generate test cases, plan) — as async 202 actions completed over WS.
- **Query databases and brokers natively** — schema, query (with engine-native
  cancel), stream exports to file, peek/produce Kafka, consumer-group lag/reset.
- **Browse/transfer files over SSH** (`/connections/{id}/sftp/*`).
- **Read usage, cost, insights, and system metrics** (root); manage swarms,
  workflows, the API client, the memory store, and the context library.
- **Subscribe to everything live** over `/ws/events`.
- **Administer** (if root or `Users:Admin`) — user CRUD, grants, the daemon-wide
  session overview, force-terminate, and audited impersonation.

### You CANNOT (or must clear a gate first)

- **Reach the API off-box by default.** Loopback only unless a root user enables
  the `network_listener` setting. Don't change that casually (§8).
- **Exceed your RBAC.** A token is capped at its owner's feature grants + workspace
  roles + ownership. Non-root tokens get 403/404 on routes above their grant; a
  user only sees/touches their own sessions and per-user data.
- **Use a share/impersonation token as a general key.** A share token reaches
  exactly ONE session, capped Viewer/Editor, never root; while OTP-pending it
  reaches **nothing** but `/share/verify`. An impersonation token is effective-as
  the target, expires in 30 min (never slid), can't nest, can't target root or a
  fellow Users-admin, and **can't mint a PAT**.
- **Write to a guarded connection without confirming.** On a connection with
  `environment=prod` or `read_only=true`, a write/DDL is rejected `409
  write_blocked: …` unless the request sets `confirm_write:true` (DB) /
  `confirm:true` (brokers); a confirmed write is audited (`db.write_confirmed`).
- **Read another user's secrets.** Tokens, passwords, SASL/registry creds, git/Jira
  tokens never come back over the API — only opaque references and `has_*` flags;
  the raw secret lives in the macOS Keychain.
- **Add new shapes via the API.** The contract is **FROZEN**: changes are
  *additive only* (new optional fields, new routes) — no existing path/shape may
  change without a contract bump. There is no generic "run arbitrary command"
  escape hatch beyond opening a `shell`/`connection` session and writing to its PTY.
- **Call ingest routes as a user.** The four `/ingest/*` routes are gated by the
  per-session ingest token, not a bearer; they're for agent hooks, not callers.

---

## 7. Security

- **Loopback by default.** `127.0.0.1:7700` only. A network listener is opt-in,
  root-only, audited (`network_listener.toggle`), and surfaced in
  `GET /security-posture`. For LAN/remote access prefer the scoped, expiring,
  OTP-gated **share links** over opening the listener — see
  [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md).
- **Three-axis RBAC + root bypass** (§2.6). Default-deny on the feature axis.
- **Secrets in the Keychain.** The SQLite state DB stores only opaque key
  references; tokens/passwords/app-passwords live in `otto-keychain`. The raw
  token secret is shown once and stored only as a SHA-256 hash.
- **Append-only audit log.** Security-relevant actions (login success/failure/
  lockout, token mint/revoke, settings change, listener toggle, confirmed guarded
  writes, grant changes, session terminate, impersonate start/stop) are written
  best-effort to a ledger that is never updated or deleted (`GET /audit-log`,
  root). Secret *values* are never captured.
- **Revocation is immediate.** Revoking a PAT/share/login token evicts it from the
  auth cache before returning, and revoking a share evicts attached `/ws/term`
  viewers (they get `{"type":"terminated"}`). Share/impersonation tokens are never
  cached at all.
- **Frozen contract.** Additive-only; the loopback default and RBAC gates are not
  to be weakened casually.

---

## 8. Operating skills & `OTTO_API_TOKEN`

Otto ships a set of operating skills that wrap this API for terminals and agents,
under `~/.claude/skills/otto-*`:

- **`otto-api`** — the foundation: one-time token setup
  (`scripts/otto-setup-token.sh` logs in / onboards root, mints a PAT, prints
  `export OTTO_API_TOKEN=…`), a thin `otto METHOD PATH [BODY]` client, and the
  complete endpoint catalog (`references/api-endpoints.md`). All routes are
  relative to `/api/v1`.
- **`otto-sessions`** — open/close/list/drive sessions + terminal attach.
- **`otto-usage`** — token-cost analytics (provider/day/session/by-kind).
- **`otto-insights`** — the scheduled-reports workflow.
- **`otto-swarm`** — agent-team orchestration.

Convention: export the PAT once and reuse it everywhere.

```bash
export OTTO_API_TOKEN=…                 # from otto-setup-token.sh, or your shell profile
export OTTO_BASE_URL=http://127.0.0.1:7700
otto whoami                              # GET /auth/me
WID=$(otto ws-id)                        # first workspace id
otto POST /workspaces/$WID/sessions '{"kind":"agent","provider":"claude"}'
```

The client adds `Authorization: Bearer $OTTO_API_TOKEN` automatically and
pretty-prints JSON (non-2xx → `HTTP <code>` on stderr, non-zero exit). If the
daemon predates `/auth/tokens`, setup falls back to a 30-day sliding login token —
rebuild `ottod` to upgrade.

---

## 9. Related docs

- [`./api-client.md`](./api-client.md) — the **in-app** HTTP/gRPC API client
  ("Postman"); this doc is its operator-facing counterpart (driving `ottod`
  itself, not requests built inside Otto).
- [`./agent-sessions.md`](./agent-sessions.md) — the session lifecycle, PTYs,
  trust/prompt-guard, and terminal attach behind the `/sessions` + `/ws/term` API.
- [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) — the full RBAC
  model, per-session isolation, impersonation, and share-link/OTP remote access.
- [`docs/contracts/api.md`](../contracts/api.md) · [`docs/contracts/ws.md`](../contracts/ws.md)
  · [`docs/contracts/product.md`](../contracts/product.md) — the **authoritative,
  frozen** contracts. Always defer to these.
- [`docs/MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — the deployed multi-user
  operator guide (features, capabilities, the "Database-only user" recipe).
