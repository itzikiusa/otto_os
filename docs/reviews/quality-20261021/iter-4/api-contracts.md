# Quality iteration 4 — lens: api-contracts

Reviewer scope: parity between `docs/contracts/{api,ws,rooms,room-recaps,product}.md`, the axum
routes and WS handlers in `crates/`, the TS mirrors (`ui/src/lib/api/types/*.ts`, `room-types.ts`,
`room-recap-types.ts`, `usage.svelte.ts`, module-local `types.ts`) and their UI callers, plus
`docs/features/` drift. Read-only: no cargo, npm, builds or tests were run. The excluded game,
school and classroom UI paths and `game-rooms.md` (being rewritten) were not read.

## Score

Start 10.0. Open findings: 0 blockers, 1 major, 3 minors, 2 nits.

`10.0 − 1×0.3 − 3×0.1 − 2×0.03 = 10.0 − 0.3 − 0.3 − 0.06 =` **9.34**

## Previous-finding status (iteration 3)

All 11 iteration-3 findings were re-verified against the current code and docs (commit
dc8ba8c0e). All 11 are fixed.

| id | status | evidence now |
|---|---|---|
| 3-01 | fixed | `api.md:1675` → `ProductTestcase`; `:1489` → `PrCommit[]` (no `parents`/`refs`). |
| 3-02 | fixed | `api.md:4731, 4733` owner/ws-admin/root (ownerless preset adopted); `:4629` and `personal-agents.md:302-309` name the `PUT …/memory` and `PUT …/context` routes. |
| 3-03 | fixed | `api.md:1514` lists the 400 index, 409 concurrent retry, 409 no stored diff (pre-0100), 500 diff write and branch-review 409s. |
| 3-04 | fixed | `api.md:6215` describes the bounded `4*limit` scan, short/empty pages and `/history/page` as the reliable pager. |
| 3-05 | fixed | No `?token=` fallback remains in `docs/features/` — every passage now says `?token=` → 401 (`daemon-http-api.md:140-148, 364`, `agent-sessions.md:250`, `api-client.md:635`, `rbac-multiuser-sharing.md:332, 434`, `remote-mobile-access.md:475`). |
| 3-06 | fixed | `api.md:762, 763, 1479, 1492, 1512, 1666, 3042` carry the DTOs and required keys. |
| 3-07 | fixed | `api.md:3602-3603`: `MergeResp {memory}` / `SplitResp {memories}`, `parts` ≥ 2. |
| 3-08 | fixed | `api.md:3567-3568` root (Usage:View passes the guard, non-root 403), `?days=` documented. |
| 3-09 | fixed | `api.md:3219-3220`, `daemon-http-api.md:142-143`: any-role member, registered-repo roots, 400/503; stream row needs `?workspace_id=` + API Client Edit. |
| 3-10 | fixed | `insights.md:442`, `workflows.md:318, 1505`, `skills-evaluator.md:480` quote the real strings. |
| 3-11 | fixed | `api.md:2174-2176` `member (Skills View)`; `:3723` `Agents:Edit + ws viewer + the view's owner or root`. |

## Areas covered (this iteration)

- **Re-verification of all 11 iteration-3 findings** (table above), including the iteration-3
  docs commit's new rows: `/workflow-runs/{id}/context/{browse,read}` (handler
  `routes/workflow_run_files.rs` vs `api.md:2345-2346` and `workflows.md:1261-1262`: 400/403/404,
  relative paths, ~400 KiB, key-like 403 — clean), the MCP token rotate (`api.md:4379`), the
  Bitbucket 202 merge poll (`api.md:132`), the memory undo state restore (migration 0190).
- **`product.md` P1–P36 request/response shapes and auth (iteration-3 gap)** — every
  `otto-product/src/http.rs` handler and the `otto-server` product routes (`modules.rs`,
  `otto-product/src/analysis.rs`) read against `product.md` and the `api.md` product rows,
  including the account-owner (S4) gates and the workspace-equality guards. → 4-01, 4-02, 4-03.
- **`rooms.md` WS command/event unions (iteration-3 gap).** `RoomAction` (30 variants),
  `RoomEvent` (10), `RoomSnapshot` (pending projection omits every admitted field and `recap`),
  `RoomMember`, `RoomPresentation`, `RoomAnnotation*`, `RoomIceServer`, `RoomSettings{,Req}`
  vs `ui/src/lib/api/room-types.ts` field by field; registry TTLs (8 h / 10 min / 2 min / 30 s);
  router (`rooms/mod.rs:21-57`) vs the HTTP table. All clean.
- **`room-recaps.md`** routes, `RecapMetadata`/`RecapRevision`/`RecapDraft`/`RecapEventData`
  (12 variants)/request DTOs vs `room-recap-types.ts`. Clean.
- **Module-local / differently named TS types (iteration-3 gap):** usage
  (`AttributionRow`, `ForecastReq/Resp`, `UsageStatus.tailer`/`TailerStatus` — the iteration-3
  "not done" TS mirror now exists at `usage.svelte.ts:228`), insights (`InsightsConfig`,
  `ReportView`, `RunReq/Resp`, `ActiveRun`, `ReportStatus`), browser (10 renamed pairs, e.g.
  `BrowserLiveSession`↔`LiveSessionInfo`), `NetworkProfile` (flattened input) / `SessionNetwork`.
  Clean.
- **Scripted same-name struct ↔ interface field diff over every `ui/src/lib/api/types/*.ts`
  file against all `crates/*/src`.** Every reported delta was read by hand; all were name
  collisions, `#[serde(flatten)]` wrappers (`ConsumeOpts`, `GrpcOptions`, `OffsetResetMode`),
  `skip_serializing` fields (`Memory.undo_token`) or TS reading a subset. No drift.
- **`/ws/browser/{tab_id}/live`** `ClientMsg`/`ServerMsg` (`otto-browser/src/live/protocol.rs`)
  vs `ws.md` §1b. Clean. **`/ws/term` server frames** (`status`, `scrollback` text/binary,
  `credit`, `probe_ack`, `search_result`, `grid`, `exit`, `terminated`, `error`) vs `ws.md` and
  `types/ws-term.ts`. Clean.
- **Scripted route→handler join** (1,297 route registrations): handlers that extract `Json<T>`
  where the `api.md` request cell is "—" (→ 4-03), handlers with a `Query<T>` where the row lists
  no query (all optional keys; not reported), and `StatusCode::ACCEPTED` vs documented `202`
  (all consistent once helper functions were followed).
- **`/admin/sessions*`** (`routes/admin_sessions.rs`) vs `api.md:2884-2886`. Clean.
- **Recent commits:** PR #113 `RefBranch.merged_into` (Rust/TS/`api.md:1461`/`git.md:647`
  consistent), PR #114 agy MCP (→ 4-06), iteration-3 MCP launcher env files
  (`connections-ssh-sftp.md:451-454` matches), `PUT /settings` atomic write.

## Areas NOT covered yet (next iteration)

- **404/409 semantics for every row.** Only product, sessions extras, admin sessions, git
  push/merge and the run-context routes were read this time; vault, workbench, design and
  swarm rows remain.
- **Prose response rows** (≈165 rows whose response cell is prose) were sampled, not all read
  against the handler's return type — the product ones are in 4-03.
- **Optional query parameters missing from rows** (`?q=` on `/git/accounts/{id}/remote-repos`,
  `?provider=` on context materialize, `?backup=` on bundled install, `?scope=` on
  connection-sections, `?active=` on learnings, `?collection_id=`, `?status=` on improvement
  edits) — all optional, not scored; a docs pass could add them.
- **`game-rooms.md`** (skipped: the games are being rewritten).
- **The PTY-holder protocol** (`otto-pty/src/holder.rs`, internal).
- **Access-control consequence of 4-01:** `GET /workspaces/{ws}/product/stories` returns stories
  created in workspaces the caller is not a member of (only Viewer on the path `ws` is checked),
  while their item routes 403. Flag to the access-control lens; not scored here.

## Findings

### api-contracts-4-01 — major — Product stories are a "global library" in the list route and docs, but seven agent routes 404 any story created in another workspace — and the UI always sends the *current* workspace

- **Where:**
  - List is global: `crates/otto-product/src/http.rs:378-386` (`list_stories` checks Viewer on
    the path `ws`, then `list_stories()` with no workspace filter — "Product is a global
    library — list every story regardless of workspace").
  - Workspace-bound routes, each `if story.workspace_id != ws_id { NotFound("story not found in
    workspace") }`:
    - `analyze` — `crates/otto-product/src/analysis.rs:46-51`
    - `rewrite` — `:307-312`
    - `testcases/generate` — `:372-377`
    - `plan/generate` — `:438-443`
    - `plan` (save) — `:536-541`
    - `inject-session` — `crates/otto-server/src/modules.rs:836-842`
    - `attach-product` — `crates/otto-server/src/modules.rs:957-965` (against the *session's*
      workspace)
  - UI callers pass the current workspace, not the story's: `ui/src/lib/stores/product.svelte.ts:210-214`
    (`wsId()` = `ws.currentId`), used at `:657-661` (analyze), `:788-792` (rewrite), `:802-804`
    (plan/generate), `:809-811` (plan), `:1198-1200` (testcases/generate), `:1262-1264`
    (inject-session). `ui/src/modules/agents/AttachProductStory.svelte:35` lists
    `/workspaces/{current}/product/stories` (all workspaces' stories) as the attach picker.
  - Docs: `docs/features/product.md:346-353` says "The `workspace_id` column is retained purely
    as the *creating* workspace for provenance … existing rows became visible everywhere";
    `docs/contracts/api.md:1622-1626` calls them "Workspace-scoped collections"; no product row
    (`api.md:1727-1733`, `product.md` P10/P23/P24/P30) or the attach row (`api.md:764`,
    `agent-sessions.md:1121`) documents the 404.
- **Failure:** a user imports story S in workspace A, switches to workspace B (where the Product
  page still lists S), opens S and clicks Analyze / Rewrite / Generate tests / Generate plan /
  Inject session → 404 "story not found in workspace". Attaching S to a session in B from the
  session's "Attach product story" picker fails the same way. The documented behaviour
  ("visible everywhere", provenance only) is not what the routes do.
- **Fix:** pick one contract and make code + docs agree:
  - Preferred (keeps the auth model): the UI builds these paths from the selected story's
    `workspace_id`, not `ws.currentId` (product store `analyze`/`rewrite`/`generatePlan`/
    `savePlan`/`generateTests`/`injectSession`), and the attach picker filters to stories whose
    `workspace_id` equals the session's workspace (or the route resolves the story's own
    workspace and role-checks it, like the flat item routes' `ws_from_story`).
  - Document it: `api.md:1622-1626` ("collections list every workspace's stories; the agent
    routes' `{id}` must be the story's creating workspace, else 404 'story not found in
    workspace'"), the rows at `api.md:1727-1733` and `:764`, `product.md` P10/P23/P24/P30, and
    replace "purely … for provenance" in `features/product.md:350-351`.

### api-contracts-4-02 — minor — Five product routes require the bound issue account's owner (or root); the contract auth columns say only `ws editor`

- **Where:** `docs/contracts/api.md:1651` (import), `:1656` (refresh), `:1659` (version publish),
  `:1666` (questions/post), `:1677` (testcase-run publish); `docs/contracts/product.md` P2, P6,
  P9, P17, P28 — all `ws editor`.
- **Evidence:** each handler adds an S4 ownership check after the role check:
  `crates/otto-product/src/http.rs:396-399` (`authorize_account_id(&req.account_id, …)`),
  `:555`, `:600-602`, `:735`, `:1024-1026` (`authorize_story_account`). Both delegate to
  `authorize_owner` (`crates/otto-product/src/service.rs:112-130`): a non-owner, non-root Editor
  gets 403. The sibling rows `api.md:1686-1687` (publish-as-rfc/story) already say
  `ws editor + account owner/root`, and `features/product.md:777-780` documents the rule, so
  only the contract tables are behind.
- **Fix:** set the auth cell of the five `api.md` rows and P2/P6/P9/P17/P28 to
  `ws editor + account owner/root` (refresh/publish/post: the story's bound account; import: the
  `account_id` in the body; a story with no bound account skips the check).

### api-contracts-4-03 — minor — Product and PR rows with wrong or missing request/response cells

- **Where and evidence:**

  | Row | Says | Handler |
  |---|---|---|
  | `api.md:1651` `POST /workspaces/{ws}/product/stories` | → `Story` | `ProductStoryDetail` (`otto-product/src/http.rs:400-401`, `service.rs:321-326`); the UI types it so (`product.svelte.ts:307`). `product.md` P2 is right. |
  | `api.md:1656` `POST /product/stories/{sid}/refresh` | "re-pull the source story" | `ProductStoryDetail` (`http.rs:557-558`) |
  | `api.md:1666` `POST …/questions/post` | "post questions back…" | `{posted:[{id,ref,url}]}` (`http.rs:741-751`) |
  | `api.md:1677` `POST /product/testcase-runs/{rid}/publish` | request `—`, "publish approved test cases" | `Json<PublishTestsReq {space_key?, parent_id?}>` — a JSON body is required (`http.rs:1015`, `types.rs:164-169`); response `{url}` (`http.rs:1036`) |
  | `api.md:1673` `GET /product/stories/{sid}/events` | no query | `?section=` filter (`http.rs:224-227`); `product.md` P22 has it |
  | `api.md:1490` `POST /repos/{id}/prs/{number}/request-changes` | request `—` | `Json<RequestChangesReq {body?}>` (`otto-git/src/http.rs:3129`, `otto-core/src/api/git.rs:773-776`) — a JSON body is required and `body` is the review comment |
  | `api.md:1624-1626` (section intro) | AI actions "return 202 Accepted" | `analyze` returns 200 `ProductAnalysis` (`analysis.rs:216`; the row at `:1727` says so) |
  | `ui/src/lib/stores/product.svelte.ts:785-795` | `rewrite(): Promise<ProductStoryVersion>` | the route answers `202` with no body (`analysis.rs:299, 354`), so the promise resolves `undefined` (`client.ts:497-501`); the only caller ignores it (`RewriteTab.svelte:207`) |

  A client built from the "—" cells sends no body and gets a JSON-extractor rejection (415/400),
  and cannot discover `space_key`/`parent_id` or the request-changes comment.
- **Fix:** fill the cells as listed (`ProductStoryDetail`; `{posted:[{id,ref,url}]}`;
  `PublishTestsReq {space_key?, parent_id?}` → `{url}`; `?section=`;
  `RequestChangesReq {body?}` → 204), say "analyze returns the `ProductAnalysis` row (200); the
  others 202" in the intro, and type `rewrite()` as `Promise<void>`.

### api-contracts-4-04 — minor — `attach-product` and `inject-session` rows omit the session-owner gate and their `Session` response

- **Where:** `docs/contracts/api.md:764` (`POST /sessions/{session_id}/attach-product | ws editor
  | {story_id} | attaches a product story to the session`), `docs/features/agent-sessions.md:1121`
  (same), `api.md:1733` (`inject-session … | inject story context into a session`).
- **Evidence:** `attach_product_story` (`crates/otto-server/src/modules.rs:942-996`) requires ws
  Editor **and** `require_session_owner_or_admin` (`:953`) — a non-owner Editor gets 403 — and
  returns `Json<Session>` with `meta.product_story {id, title}` set (`:985-995`; the UI reads it,
  `ui/src/lib/stores/workspace.svelte.ts:2068`). `inject_session` returns `Json<Session>`
  (`modules.rs:833`). Every other session-control row in that table (`api.md:753-763`) states the
  owner-or-admin gate.
- **Fix:** `api.md:764` and `agent-sessions.md:1121`: auth `ws editor + session owner-or-admin`,
  response `Session` (with `meta.product_story`; 404 when the story's workspace differs — see
  4-01). `api.md:1733`: response `Session`.

### api-contracts-4-05 — nit — `features/product.md` says attachments upload as multipart; the route takes base64 JSON

- **Where:** `docs/features/product.md:666` (`POST …/attachments` (multipart)).
- **Evidence:** `upload_attachment` (`crates/otto-product/src/media.rs:271-276`) extracts
  `Json<UploadReq {filename, mime, kind?, data_b64}>` (`:59-65`); `api.md:1775` correctly says
  "UploadReq (base64)". A multipart client gets 415.
- **Fix:** replace "(multipart)" with "`UploadReq {filename, mime, kind?, data_b64}` (JSON,
  base64)".

### api-contracts-4-06 — nit — The spawn walkthrough still says MCP servers go only into `.mcp.json`; agy now gets `.agents/mcp_config.json`

- **Where:** `docs/features/agent-sessions.md:205-213` (step 5: reconcile into the workspace
  `.mcp.json`; "Codex gets … `-c` overrides; grok as `.grok/config.toml`").
- **Evidence:** since 8a9897823, agy spawns/restarts also reconcile the same managed set (with the
  `ottoManagedServers` marker) into `<cwd>/.agents/mcp_config.json`
  (`crates/otto-sessions/src/manager/mcp_sync.rs:135-140`, `mcp.rs:107-113, 215-223`), because
  agy never reads `.mcp.json`. The connections guide already names that file
  (`connections-ssh-sftp.md:451`).
- **Fix:** add "agy gets the same managed set in `<cwd>/.agents/mcp_config.json` (it ignores
  `.mcp.json`)" to step 5.
