# Quality iteration 2 — lens: api-contracts

Reviewer scope: parity between `docs/contracts/{api,ws,rooms,room-recaps,game-rooms,product}.md`,
the axum routes and WS handlers in `crates/`, the TS mirrors (`ui/src/lib/api/*.ts`,
module-local `types.ts`) and their UI callers, plus `docs/features/` drift.
Read-only. No builds or tests were run. The excluded game, school and classroom
UI paths were not read.

## Score

Start 10.0. Open findings: 0 blockers, 0 majors, 8 minors, 5 nits.

`10.0 − 8×0.1 − 5×0.03 = 10.0 − 0.8 − 0.15 =` **9.05**

## Previous-finding status (iteration 1)

All 13 iteration-1 findings were re-verified against the current code. All 13 are fixed.

| id | status | evidence now |
|---|---|---|
| 1-01 | fixed | `ProofPage.svelte:405-417`: `packRepoPath` reads `GET /workspaces/{id}/repos` (route at `otto-git/src/http.rs:102`). A lookup failure is returned as `error` and shown in the prompt, not swallowed. |
| 1-02 | fixed | TS `ReviewFinding` (`types.ts:3954-3964`) no longer has `fingerprint` or `state`. `ReviewAgents.svelte:200` keys on `path:line:i`. |
| 1-03 | fixed | `ReviewAgentCfg.skill?` exists (`types.ts:4055`). `saveAsPreset` copies it (`ReviewPanel.svelte:959-960`). |
| 1-04 | fixed | `api.md:3715` documents `204` with `require_human`. |
| 1-05 | fixed | `api.md:3074-3075`: GET is `member (Git View)`; PUT is root (person credential). |
| 1-06 | fixed | `api.md:80` #16a lists Workbench under `scratch`. |
| 1-07 | fixed | `on_restart` is on `Workflow`, `WorkflowVersion` and `UpdateWorkflowReq` (`types.ts:5899,6124,6146`). |
| 1-08 | fixed | `proof-packs.md:624` and troubleshooting `:808` document the cwd 403. |
| 1-09 | fixed | `git.md:319-321,638-639` cover cleanup-base and the landed marks. |
| 1-10 | fixed | None of the 23 names remain in `api.md`. Other non-existent names remain; see 2-10. |
| 1-11 | fixed | `ImprovementTrigger` includes `'live'` (`types.ts:4574`). |
| 1-12 | fixed | `ws.md:587` points at `Event::type_name()`. |
| 1-13 | fixed | `K8sCluster.params` and `created_by: Id \| null`, plus `ImportK8sClusterReq.color` (`types.ts:9416,9419,9445`). |

## Areas covered (this iteration)

Every check below started as a scratch script over the source. Each reported item was then read
by hand at the cited lines.

- **UI callers vs routes (re-run).** Every `api.get/post/put/patch/del` in `ui/src`
  (exclusions applied) was matched against the 1000+ route registrations, including those
  added since iteration 1 (rooms games epic, iteration-1 fixes). The result is clean. Every
  hit was a dynamic `${path}` false positive that I checked by hand.
- **MCP outward catalog vs REST.** All 134 `route_for` arms
  (`otto-mcp/src/outward/exec.rs`): each `/api/v1/...` self-call path and method exists.
  Tool `inputSchema` properties were compared with the args each arm reads. The `number` and
  `pr_number` aliases resolve in `refs.rs:269`. The closures (`opt("cql")`, `opt_query`) read
  the declared keys. No dead or missing args.
- **`ui-commands.json` vs `ui_commands.rs`.** Both sides embed the same file (`include_str!`
  and a TS import) and a unit test pins them. No drift is possible.
- **`game-rooms.md`** against `game_rooms/{types,http,socket,registry}.rs` and
  `game-room-types.ts`: maps, capacities (4/8), 16 member entries, 401/404/409 codes, the
  frame limit, subprotocol, rate and heartbeat rules, and the full command/event unions. All
  match.
- **`room-recaps.md`** against `rooms/mod.rs` routes, the `policy.rs:91-102` levels,
  `otto-core/src/api/recap.rs` and `room-recap-types.ts`: field-for-field parity. One nit
  (2-13).
- **`rooms.md`** against the routes, the registry limits (32 rooms, 4 admitted, 8 invites/pending,
  TTLs) and the `room-settings` auth.
- **`product.md`** (P1–P36): every DTO name exists in `otto-product/src/types.rs` and
  `ui/src/modules/product/types.ts`.
- **Same-named Rust↔TS DTOs across all of `ui/src`.** This includes module-local
  `types.ts` (product, swarm, canvas, design-hall), which iteration 1's `types.ts`-only
  pairing did not reach. Projection-only TS fields (`Session.live/viewers/held`,
  `NodeRunState.detail_version`, …) were confirmed to be server projections and dropped.
- **Iteration-1 fix commit 66b55a25c.** I reviewed its route-surface changes for contract
  drift: share revoke, swarm DELETE moved to `runtime::engine::routes`, fs `~user` 400,
  support-bundle redaction, MCP server source, project sessions page and the WS liveness
  rework. Only the WS liveness change is undocumented (2-11).
- **`api.md` status claims.** I checked every row documented `204` or `201` against the
  handler's return type (rows → handler → `-> ApiResult<Json<…>>`). See 2-03 and 2-09.
- **`api.md` `root` auth claims vs handlers.** I checked every row whose auth column is
  `root` against the handler body and `policy.rs`, and the reverse: non-root rows whose
  handler opens with `require_root`. See 2-02.
- **`docs/features/*` vs code and `api.md`.**
  - Every `METHOD /path` mentioned in all feature docs was checked against the route
    inventory. See 2-06 and 2-08.
  - Every feature-doc auth column was diffed against `api.md`'s, with each disagreement
    resolved by reading the handler. See 2-01, 2-04 and 2-05.
- **Type names in `api.md` table columns** were checked against all Rust and TS type
  definitions. See 2-10.

## Areas NOT covered yet (next iteration)

- **Remaining request/response shapes.** The `api.md` rows whose request or response is
  prose (e.g. "config", "trigger a run now") still need comparison with the handlers'
  extractors and return types. Iteration 2 only machine-checked the 201/204 rows and the
  named types.
- **Documented 4xx semantics (409/404/400) beyond status-code rows.** Sampled only (share
  revoke, game rooms, recaps).
- **Auth columns marked `ws viewer`/`ws editor` vs handler role** for families other than
  brokers, sessions activity, connections and channels. A mechanical role→handler map like
  the brokers one (`awk` over `WorkspaceRole::`) would scale to otto-git, otto-dbviewer,
  otto-k8s, otto-aws and otto-design.
- **Design Hall and API-client DTOs** whose Rust and TS names differ (they are not paired by
  name). Only `GrpcReflectReq` (flatten wrapper, OK) and `ExecuteApiReq` were checked.
- **`ws.md` per-connection frames** for `/ws/ui-bridge`, `/ws/lsp` and the PTY-holder
  protocol.
- **The `docs/features/*` troubleshooting sections** (error strings vs the `Error::…`
  texts in the handlers).

## Findings

### api-contracts-2-01 — minor — `api.md` session trail/tasks rows have the wrong auth, request bodies and statuses

- **Where:** `docs/contracts/api.md:724-727`. Code: `crates/otto-server/src/routes/activity.rs:49-150`
  (`list_trail`, `append_trail`, `list_tasks`, `put_tasks`). DTOs: `crates/otto-core/src/api.rs:3615`
  (`AppendTrailReq`) and `:3641` (`PutTasksReq`).
- **Evidence:** the contract and the code disagree on every one of these rows.

  | Row | `api.md` says | Code does |
  |---|---|---|
  | All four: auth | ws viewer / ws editor only | Each handler also calls `require_session_owner_or_admin` (`activity.rs:56`, `:80`, `:123`, `:147`). A plain workspace member gets 403 on another user's session. |
  | `POST …/trail` | request `TrailEvent`, response `204` | Takes `Json<AppendTrailReq>` (`{source?, kind?, level?, summary, detail?}`). Returns `ApiResult<Json<TrailEvent>>`, which is 200 with the row. |
  | `PUT …/tasks` | request `AgentTask[]`, response `204` | Takes `Json<PutTasksReq>`, the object `{tasks: TaskInput[]}`. Returns `Json<Vec<AgentTask>>`. A client that follows the contract and sends a bare array gets a 422. |

  The feature doc (`agent-sessions.md:1033-1036`) already states the owner/admin rule. The UI
  uses the real shapes (`activity.svelte.ts:266`, `api.post<TrailEvent>`).
- **Fix:** Rewrite the four rows:
  - Auth: `ws viewer + session owner/admin (root)` for the GETs, `ws editor + session owner/admin` for the writes.
  - `POST …/trail`: `AppendTrailReq {source?, kind?, level?, summary, detail?}` → `TrailEvent`.
  - `PUT …/tasks`: `PutTasksReq {tasks: TaskInput[]}` → `AgentTask[]`.

### api-contracts-2-02 — minor — Library and skill-eval reads are documented as `root` but any member with the feature's View can call them

- **Where:**
  - `docs/contracts/api.md:2153, 2156, 2159-2160, 2166-2167, 2170-2171, 2174` (Library GETs);
    `:2289-2290` (`GET /library/bundled`, `GET /library/bundled/{name}`); `:2049`
    (`GET /settings/skill-eval`).
  - Code: `crates/otto-context/src/http.rs:161-177` (`list_skills` and `get_skill` take
    `Extension(_user)` and run no root check), `:398-414` (souls, same) and the context and
    default-soul handlers.
  - Policy: `crates/otto-server/src/policy.rs:341-348` (`/library/*` → `Require(Skills, if get { View } else { Admin })`).
  - Skill-eval: `crates/otto-server/src/skill_eval.rs:2391-2396` (`get_config` with `CurrentUser(_user)`).
  - Policy snapshot: `tests/snapshots/policy_decisions.txt:3498` (`GET /api/v1/settings/skill-eval => Require(SkillEval, View)`).
- **Evidence:** Only the writes (`put_skill`, `put_soul`, `put_context`, `delete_*`)
  call `require_root`. Every read is open to any member holding Skills:View (or SkillEval:View).
  This is the same class of drift as iteration 1's `/settings/pr-review` finding (1-05), in
  sections that pass did not reach.
- **Impact:** The contract understates who can read the shared skill, soul and context library
  and the skill-eval config. An admin auditing exposure from `api.md` would conclude that
  only root sees them.
- **Fix:**
  - Change the auth of the GET rows to `member (Skills View)`, and of `/settings/skill-eval` GET to `member (SkillEval View)`.
  - Keep `root` on the writes, and note that the library writes are also tagged Admin (`policy.rs:1631`).

### api-contracts-2-03 — minor — Library `PUT` rows document a raw body and `204`; the handlers take `{body}` and return the entry

- **Where:** `docs/contracts/api.md:2157` (`PUT /library/skills/{name} | root | skill body | 204`),
  `:2168` (souls) and `:2172` (context). The list rows `:2153`, `:2166` and `:2170` name
  `SkillEntry[]`, `SoulEntry[]` and `ContextEntry[]`. The GET-one rows say "skill body", "soul body"
  and "context body".
- **Evidence:**
  - `put_skill` (`otto-context/src/http.rs:179-193`) takes `Json<UpsertLibraryEntryReq>`
    (`otto-core/src/api.rs:2571`: `{ body: String }`) and returns `Json<LibrarySkill>`.
    `put_soul` (`:416-430`) and `put_context` (`:466-480`) do the same with
    `LibrarySoul` and `LibraryContext`.
  - `list_skills` returns `Vec<LibrarySkill>` (`:164`), and `get_skill` returns the
    `LibrarySkill` JSON object, not a bare body.
  - `SkillEntry`, `SoulEntry` and `ContextEntry` exist nowhere in `crates/` or `ui/src`. The TS
    mirrors are `LibrarySkill`, `LibrarySoul` and `LibraryContext` (`types.ts:4650,4836,4841`).
- **Fix:**
  - The PUT rows become `UpsertLibraryEntryReq {body}` → `LibrarySkill` / `LibrarySoul` / `LibraryContext`.
  - The list rows become `LibrarySkill[]` / `LibrarySoul[]` / `LibraryContext[]`.
  - The GET-one rows return the same entry types.

### api-contracts-2-04 — minor — `channels-slack-telegram.md` says `ws editor` for bridge config; the handler requires the root owner

- **Where:** `docs/features/channels-slack-telegram.md:586` (`PUT /workspaces/{id}/integrations/{channel} | ws editor`)
  and `:589` (`POST …/seed-from-loom | ws editor`). Code: `crates/otto-channels/src/http.rs:175-183`
  (`require_owner_for_bridge` → `root_authority(user)` or `403 "only the owner can configure a channel bridge"`),
  called at `:194` (upsert) and `:504` (seed).
- **Evidence:** `api.md:1915` and `:1919` correctly say `ws editor + root`. The feature
  doc's endpoint table, which it calls authoritative-adjacent ("Authoritative source:
  docs/contracts/api.md"), does not mention root anywhere (`grep -n -i 'root\|owner'` finds
  no auth mention). A workspace Editor following the feature guide gets a 403 they have no
  way to explain.
- **Fix:**
  - Set both rows to `ws editor + root (bridge sessions act as the owner; non-root → 403)`.
  - Add a troubleshooting line for the 403 text.

### api-contracts-2-05 — minor — Connections feature docs: SFTP download/upload are root-only, and global connections use feature capabilities, not root

- **Where:**
  - `docs/features/connections-ssh-sftp.md:419-420`: `POST …/sftp/download` and `…/sftp/upload`
    are documented as `ws editor`.
  - `docs/features/daemon-http-api.md:281`: `…/sftp/download·/upload·…` is documented as `editor`.
  - `connections-ssh-sftp.md:404-406` and `daemon-http-api.md:278`: `PATCH/DELETE /connections/{id}`
    and `PATCH …/pin` are documented as `ws editor (global: root)`.
- **Evidence:**
  - `sftp_download` and `sftp_upload` call `require_root_for_host_path` first
    (`crates/otto-connections/src/http.rs:793` and `:833`). That function returns 403
    `HOST_PATH_ROOT_ONLY` for any non-root or disabled user (`:770-779`). `api.md:956-957`
    correctly says `ws editor and root`.
  - Global edit and delete go through `check_conn_manage` → `check_conn_access(…, Capability::Admin)`
    (`:207-213`), a feature-capability check, not root. Pin uses `check_conn_role` (`:604`).
    `api.md:94-96` correctly says `Connections:Admin` / `Connections:Edit`.
- **Fix:**
  - In both feature docs, mark download and upload `ws editor + root (daemon-host path)`.
  - Replace `global: root` with `global: Connections:Admin` (pin: `Connections:Edit`).

### api-contracts-2-06 — minor — `message-brokers.md` endpoint table still lists the removed `/brokers/cluster-sections*` routes

- **Where:** `docs/features/message-brokers.md:547-550`.
- **Evidence:** The doc lists four endpoints:
  - `GET / POST /workspaces/{wid}/brokers/cluster-sections`
  - `PATCH /brokers/cluster-sections/{id}`
  - `DELETE /brokers/cluster-sections/{id}`
  - `POST /brokers/cluster-sections/{id}/move`

  No route with `cluster-sections` is registered anywhere in `crates/` (`grep -rn
  'cluster-sections' crates/*/src` finds nothing). `api.md:3524` says these endpoints are
  gone ("migration 0095 merged those rows in"). The same feature doc's §3 (`:156-157`) also
  says sections moved to `/workspaces/{id}/connection-sections`, so the doc contradicts itself.
  Following the table gives a 404.
- **Fix:** Replace the four rows with the shared `connection-sections` routes
  (`GET/POST /workspaces/{id}/connection-sections`, `PATCH/DELETE /connection-sections/{id}`,
  `POST /connection-sections/{id}/move`, `POST …/reorder`), or with a pointer to the
  Connections doc.

### api-contracts-2-07 — minor — `PATCH /product/stories/{sid}/draft` is documented with no request body

- **Where:** `docs/contracts/api.md:1674`
  (`| PATCH /product/stories/{sid}/draft | ws editor | — | create/update the working RFC draft |`).
  Code: `crates/otto-product/src/http.rs:1116-1128`; DTO `crates/otto-product/src/types.rs:290-293`.
- **Evidence:** The handler extracts `Json<UpdateDraftReq>`, which requires both `title` and
  `body_md` (no `#[serde(default)]`), so an empty or partial body is a 422. It returns
  `Json(ProductStoryDetail)`. The UI sends `UpdateDraftReq` and reads `ProductStoryDetail`
  (`ui/src/lib/stores/product.svelte.ts:453-460`).
- **Fix:** Request `UpdateDraftReq {title, body_md}` (both required); response
  `ProductStoryDetail`.

### api-contracts-2-08 — nit — Feature docs name non-existent routes or methods

- **Where and evidence:**
  - `docs/features/product.md:649`: `GET /workspaces/{ws}/product/drafts | viewer | list drafts`.
    Only `POST` is registered (`otto-product/src/http.rs:327`), so a GET is a 405.
  - `docs/features/canvas.md:431`: "`POST /api/v1/product/stories/{sid}/linked-canvases` …
    lists the Canvas scenes". The route is GET only (`otto-server/src/modules.rs:1094-1097`;
    `api.md:1650`).
  - `docs/features/discovery-chat.md:189-194`: says "api.md lists it as
    `POST /product/stories/{sid}/draft (PATCH)`". `api.md:1674` now lists `PATCH`, so the
    paragraph describes drift that no longer exists.
- **Fix:**
  - Drop the drafts-list row.
  - Change the canvas sentence to `GET`.
  - Reduce the discovery-chat paragraph to "`PATCH /product/stories/{sid}/draft`".

### api-contracts-2-09 — nit — `POST /connection-sections/{id}/move` is documented as `204`; it returns the section

- **Where:** `docs/contracts/api.md:850`. Code: `crates/otto-connections/src/http.rs:1094-1099`
  (`-> ApiResult<Json<ConnectionSection>>`).
- **Evidence:** Both UI callers read the body:
  `api.post<ConnectionSection>(\`/connection-sections/${id}/move\`…)` at
  `ui/src/lib/stores/brokers.svelte.ts:118` and `ui/src/modules/database/DatabasePage.svelte:542`.
- **Fix:** Response `ConnectionSection`.

### api-contracts-2-10 — nit — `api.md` still names types that don't exist

- **Where and evidence:**

  | `api.md` line | Says | Real type or detail |
  |---|---|---|
  | `:2009` | `PUT /workspaces/{id}/self-improvement` takes `ImproveConfig` | The handler extracts `UpdateSelfImprovementReq` and returns `SelfImprovementConfig` (`otto-improve/src/http.rs:91-98`). |
  | `:3564` | `CompatibilityResult {compatible, messages}`, auth `ws editor` | The Rust type is `CompatCheckResp` (`otto-brokers/src/types.rs:700`). The request is `CompatCheckReq {schema, schema_type?}`. The handler authorizes `WorkspaceRole::Viewer` (`otto-brokers/src/http.rs:550`); the feature gate is Database:Edit. |
  | `:1668-1681` | `Testcase[]`, `Transcript`, `Learning` / `Learning[]` | The types are `ProductTestcase`, `ProductTranscript` and `ProductLearning` (`otto-state/src/product.rs`, `ui/src/modules/product/types.ts:524`). `Transcript` is a different, unrelated type in `otto-transcript`. |

- **Fix:** Use the real names. For the compatibility row, document `ws viewer` with
  Database:Edit.

### api-contracts-2-11 — minor — `ws.md` doesn't document the new liveness close on `/ws/events` and `/ws/term`

- **Where:** `docs/contracts/ws.md:441-442` ("Ping/pong handled by the transport layer…
  server sends a ping every 30s"). This is the only keepalive text. Code from commit 66b55a25c:
  - `crates/otto-server/src/ws_events.rs:39-49`: `PING_INTERVAL` 30 s, `INBOUND_DEADLINE`
    60 s, `SEND_TIMEOUT` 10 s, and `inbound_expired` at `:71-73`.
  - `crates/otto-sessions/src/ws.rs:44-69`: the same constants for the terminal socket.
- **Evidence:** Both sockets now close any connection that has sent no frame (Pong included)
  for more than 60 s. They also close a socket whose single send blocks for 10 s. This is a
  behavioural contract change for non-browser clients, such as the tray, scripts, the mobile
  share client or a custom WS client that disables auto-pong. Neither is mentioned in
  `ws.md`, so those clients see unexplained periodic drops.
- **Fix:** Add a "Liveness" paragraph to both the `/ws/events` and `/ws/term` sections. It
  should say:
  - The server pings every 30 s.
  - A client must send some frame (a Pong is enough) at least every 60 s, or the socket is
    closed.
  - A send stalled for 10 s closes the socket.
  - Reconnect on close.

### api-contracts-2-12 — nit — TS mirrors are missing fields the server serializes

- **Where and evidence** (each Rust field is serialized with no skip; the TS interface omits it):
  - `ui/src/modules/swarm/types.ts:298` `SwarmPreset` lacks `max_total_runs`,
    `max_runtime_secs`, `max_cost_usd` and `max_attempts` (`otto-swarm/src/types.rs:345-349`).
  - `ui/src/lib/api/types.ts:6973` `Memory` lacks `state`, `provenance_json` and
    `forgotten_at` (`otto-state/src/memory.rs:91-97`), although `api.md:3586` documents
    `POST …/memory/{mid}/state → Memory` by its `state`.
  - `ui/src/lib/api/types.ts:1071` `AppendTrailReq` lacks `level` (`otto-core/src/api.rs:3620-3622`,
    `info | warn | error`).
  - `ui/src/modules/canvas/types.ts:202` `UpdateSceneReq` lacks `story_id`
    (`otto-canvas/src/types.rs:40-42`, "Link/relink this scene").
  - `ui/src/modules/product/types.ts:73` `ProductAnalysisAgent` lacks `resume_count`
    (`otto-state/src/product.rs:107`). `:675` `DiscoveryRun` lacks `workspace_id`
    (`otto-state/src/product_discovery.rs:19`).
- **Fix:** Add the fields with matching optionality. For example, `max_total_runs: number | null`,
  `state: 'suggested' | 'accepted' | 'stale' | 'contradicted'`,
  `forgotten_at: number | null`, `level?: 'info' | 'warn' | 'error'`,
  `story_id?: string | null`, `resume_count: number` and `workspace_id: string`.

### api-contracts-2-13 — nit — `room-recaps.md` implies only settings *mutation* needs root; GET is root-only too

- **Where:** `docs/contracts/room-recaps.md:7` ("Settings mutation additionally requires
  root") and `:13-15` (`/room-recap-settings` is `Settings:Admin`). Code:
  `crates/otto-server/src/rooms/recap/http.rs:32-39`. `get_settings` calls `owner_auth` and
  then `require_root`, the same as `put_settings` (`:46`).
- **Evidence:** A non-root user with Settings:Admin passes the policy guard
  (`policy.rs:91-93`), but `GET /room-recap-settings` still answers 403.
- **Fix:** Say "Reading or changing `/room-recap-settings` requires root (on top of
  Settings:Admin)".
