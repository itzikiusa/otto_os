# Quality iteration 5 — lens: api-contracts

Reviewer scope: parity between `docs/contracts/{api,ws,rooms,room-recaps,product,game-rooms}.md`
(+ `ui-commands.json`), the axum routes and WS handlers in `crates/`, the TS mirrors
(`ui/src/lib/api/types/*.ts`, `game-room-types.ts`, module-local `types.ts`) and their UI
callers, plus `docs/features/` drift. Read-only: no cargo, npm, builds or tests were run.
Base: `fix/quality-iter5-20261022` at 8177c4d85.

## Score

Start 10.0. Open findings: 0 blockers, 0 majors, 3 minors, 2 nits.

`10.0 − 3×0.1 − 2×0.03 = 10.0 − 0.30 − 0.06 =` **9.64**

## Previous-finding status (iteration 4)

All six iteration-4 findings were re-checked against the current code and docs. All six are fixed.

| id | status | evidence now |
|---|---|---|
| 4-01 (major) | fixed | `ui/src/lib/stores/product.svelte.ts:104-116` resolves the story's own `workspace_id` for analyze / rewrite / plan / testcases / inject; `AttachProductStory.svelte:31,41` filters to the session's workspace; `api.md:1624-1631` and `:764` document the global list and the creating-workspace 404. |
| 4-02 | fixed | `api.md:1657, 1662, 1672, 1683` read `ws editor + account owner/root` (body account / bound account). |
| 4-03 | fixed | `api.md:1657` → `ProductStoryDetail`; `:1662` → `ProductStoryDetail`; `:1672` → `{posted:[{id,ref,url}]}`; `:1683` → `PublishTestsReq {space_key?, parent_id?}` → `{url}`; `:1679` `?section=`; `:1490` `RequestChangesReq {body?}`; `:1631` analyze 200 / others 202; `product.svelte.ts:810` `rewrite(): Promise<void>`. |
| 4-04 | fixed | `api.md:764` `ws editor + session owner-or-admin` → `Session` with `meta.product_story`. |
| 4-05 | fixed | `features/product.md:672` `UploadReq {filename, mime, kind?, data_b64}` (JSON, base64). |
| 4-06 | fixed | `features/agent-sessions.md:214` names `<cwd>/.agents/mcp_config.json` for agy. |

## Areas covered (this iteration)

- **Re-verification of all 6 iteration-4 findings** (table above).
- **Scripted route inventory, both directions.** Every literal `.route("…")` in `crates/*/src`
  (1,061 paths) was matched against every path in the six contract files, and every documented
  `METHOD /path` row was matched back to the code. Both directions are clean. The only hits are
  test fixtures (`/botTOKEN/*`, `/rest/api/3/*`, `/__e2e/*`). A method-level pass (GET/POST/…
  per path) found no row with a wrong verb.
- **Scripted UI→route existence check.** Every `api.(get|post|put|patch|del)` template path in
  `ui/src` was resolved against the route table. The leftovers are dynamic action segments, and
  each was checked by hand: goal-loop lifecycle, swarm start/pause/abort/resume, handover
  retry/acknowledge, admin-session terminate/remove, PR approve/decline, personal-agent
  memory/context, stage/unstage, rebase-skip/merge-abort and plugin enable/disable. All exist.
- **Scripted handler-return ↔ UI type-parameter join** (1,132 route→handler pairs). No UI call
  types a body on a `StatusCode`-only (204) route. The 53 name mismatches were read by hand:
  renamed aliases (`DiscoverResp`, `RunResp`, `ListSharesResp`, `ModelsResp`, `SendResp`,
  `ExecuteResp`, `PrListResp`, `TestMessageResp`, `ResetSecretStoreResp`, …) or static routes
  shadowing a `{param}` pattern. No real drift.
- **MCP profiles (merged after iteration 4).** CP44–CP48 (`otto-mcp/src/profiles/routes.rs`) vs
  `api.md:4393-4403`: auth (policy `Require(Mcp, View/Admin)` + `require_person`), 201/204,
  400 locked/validation, 404, 409 case-insensitive duplicate (`idx_mcp_profiles_name … COLLATE
  NOCASE`), the order (`locked DESC, builtin DESC, name`), `McpProfile` / `McpToolGroup` /
  `McpProfileList` / `McpProfileReq` / `McpToolFilter` vs `ui/src/lib/api/types/mcp.ts`,
  CP24a `?profiles=` and CP32 `?profile=` (`mcp_http.rs`, `mcp_outward.rs`), call refusal as
  `isError: true` (`jsonrpc.rs:191`), `PATCH /sessions/{id}` `meta.mcp_profiles` validation
  (`otto-sessions/src/http.rs:545-563, 678-684`), the `ottod mcp-server --profile` /
  `OTTO_MCP_PROFILE` path, and the picker's "default stores nothing" rule
  (`ui/src/lib/mcpProfiles.ts:69-72`) vs `features/mcp-control-plane.md:484-514`. Clean.
- **Jira issue creation + Confluence write verification (merged after iteration 4).**
  `POST /issue/{account_id}/issues` (`CreateIssueReq` / `CreateIssueResp` /
  `IssueVerification`, local + Linear-only problems, 400/502), `PageWriteResp` /
  `CommentWriteResp` (`#[serde(flatten)]`, so the old fields stay top-level), `body_html`
  well-formedness 400, and the `otto.create_issue` / `list_issue_projects` / `list_issue_types`
  catalog tiers vs `api.md:1876-1901, 4419` and `types/git.ts:188-295`. Clean apart from 5-03.
- **Iteration-4 fix commits** (`5af1dca36`, `185b542f8`, `6148b13ab`): session files
  (`routes/session_files.rs` vs `api.md:2356-2357`, `feature_guard.rs:369-371`), the LSP root rule
  and its new order (lookup after the access check), the personal-agent owner gates
  (`autonomy.rs` vs `api.md:4712, 4716`), the swarm identity gate (→ 5-05), the broker reset
  409, dry-run `warning` and replay status (→ 5-02, 5-04), the MCP server keychain 503 (→ 5-01),
  convert-to-workflow (`api.md:4608`, `scheduledTasks.ts:45`), the DB saved-query / widget
  delete gate, and the transcript-missing marks (0191, internal).
- **Game rooms (in scope this iteration).** `game-rooms.md` vs `game_rooms/{mod,http,types,
  registry,socket}.rs` and `game-room-types.ts`: the create/join bodies (`deny_unknown_fields`),
  `Credential` (invite fields skipped for guests), maps per game, capacity 4/8 counting the
  host, the 16-entry cap, 32 global / 2 per owner (409), the 10-minute invite (401), 404 /
  409 / 401 for join, the 2 KiB body limit, the `Command` / `Event` unions, `View` /
  `MemberView` (slot, left, rematch), the 20 s grace, the 15 s idle, the 2 s send timeout, the
  256-message queue and the 1-hour expiry. Clean.
- **Classroom / Otto School.** `ClassroomsBox.svelte` uses `GET /sessions?archived=…&limit=…`
  (`api.md:84`, the `ListQuery` in `otto-sessions/src/http.rs:195-206`) and
  `GET /sessions/{id}/screen` (`SessionScreen` in `otto-sessions/src/screen.rs:20-28` vs
  `api.md:820`). Clean.
- **Design Hall, workbench, vault docs-agents: status codes** (the gap iteration 4 left).
  Design project hard-delete 409 / 404 (`store.rs:692-721`), the epic-story 404 on create,
  workbench 201 / 204 / 409 (`routes/workbench.rs`) vs `api.md:6441-6452`, and vault
  docs-agents cancel / delete / resolve 404 / 409 (`vault_docs_agent.rs:839-1013`) vs
  `api.md:3488-3495`. Clean.
- **WS:** the `Event::type_name()` count (80) matches `ws.md:602`. `workbench_doc_changed`
  fields vs `ws.md:1817` and `types/workbench.ts:146`. Clean.

## Areas NOT covered yet (next iteration)

- **Prose response rows:** about 100 rows still have a prose response cell. This pass read only
  the self-improvement rows (`api.md:2028-2036`, which do not mention `RunNowResp {run_id}` or
  the "a run is already in progress" 409) and the notification rows (no status codes). They are
  vague but not wrong, so they are not scored. A docs pass could name the DTOs.
- **409 on `PATCH /mcp/profiles/{id}` rename to a duplicate name**: the CP46 row lists only 400 and
  404. The CP45 row implies the 409. Not scored.
- **`POST /workspaces/{id}/sessions` and `…/sessions/open` do not validate `meta.mcp_profiles`.**
  A malformed value silently runs under the default, and an agent opening a worker can set the
  worker's profile. The contract makes no create-time claim, so this is not scored here. It goes
  to the access-control lens: profiles "never grant", but they are documented as a person's
  choice.
- **Swarm, insights and api-client rows: 404/409 semantics**, still not read.
- **Shooter and kart payload encodings** (`games/*/net.ts`): their validation is described in
  `game-rooms.md` but was not traced, because the games are being split by other agents.
- **The PTY-holder protocol** (internal).

## Findings

### api-contracts-5-01 — minor — The CP4 `PATCH /mcp/servers/{id}` row documents a 503 "Keychain unavailable" that this route never returns

- **Where:** `docs/contracts/api.md:4342` (added by 6148b13ab): "McpServerDetail. **503**
  `"Keychain unavailable"` and nothing written when the stored secret can't be read or doesn't
  parse (PC-6); only a missing entry means "no secrets"".
- **Evidence:** CP4 is `otto-mcp/src/http.rs:455` `update_server`. When only one of
  `secret_env` / `secret_headers` is sent, it calls `McpService::resolve_secrets`
  (`otto-mcp/src/service.rs:373-410`), which answers:
  - `Error::Conflict` → **409** (`http.rs:47`) "MCP credentials are unavailable. Unlock the
    secret store and retry." for a failed read;
  - **409** "Saved MCP credentials are invalid…" for an unparsable blob;
  - **409** "MCP credentials are missing. Save the server credentials again." for a MISSING
    entry when `has_secret`. So a missing entry is not "no secrets".

  When both maps are sent, nothing is read at all (`:489-494`, the documented recovery path).
  The 503 `keychain_unavailable` (`UpdateError::KeychainUnavailable`) exists only on the other
  registry, `PATCH /mcp-servers/{id}` (`otto-server/src/routes/mcp_servers.rs`, documented
  correctly at `api.md:929`). The PC-6 plan named `/mcp/servers/{id}`; the code went to
  `/mcp-servers/{id}`, and the docs copied the plan to both rows.
- **Failure:** a client that branches on 503 / `keychain_unavailable` for a locked keychain on a
  control-plane server never sees it; it gets a 409 whose text it treats as a conflict. A client
  that trusts "only a missing entry means no secrets" for CP4 is wrong for a server whose
  Keychain entry vanished: CP4 refuses with 409.
- **Fix:** rewrite the CP4 cell: "McpServerDetail. Sending only one of `secret_env` /
  `secret_headers` re-reads the stored blob: **409** when the secret store can't be read
  ('MCP credentials are unavailable…'), the blob doesn't parse ('…invalid…') or the entry is
  missing while `has_secret` ('…missing…'), and nothing is written. Sending both replaces the
  blob without reading it (the recovery path)." The other option is to give CP4 the same 503
  as `/mcp-servers/{id}`, if that is the intended contract.

### api-contracts-5-02 — minor — The reset dry-run's `warning` (group has live consumers) is missing from the TS mirror and never shown, so the user previews, types the confirmation and only then gets the 409

- **Where:**
  - Server: `crates/otto-brokers/src/types.rs:788-797` (`DryRunResp.warning: Option<String>`,
    `skip_serializing_if`), set at `crates/otto-brokers/src/service.rs:1093`
    (`reset_blocked_reason`). Documented at `api.md:3545, 3597` and
    `features/message-brokers.md:443-446` ("The dry-run still returns the plan, plus `warning`
    with the same text").
  - TS mirror: `ui/src/modules/brokers/types.ts:44-49`. `DryRunResp {group, partitions,
    total_lag_before, total_lag_after}` has no `warning`.
  - Caller: `ui/src/modules/brokers/GroupsTab.svelte:236-239` stores the preview, and
    `:503-521` renders only the lag numbers and partitions. `rg '\.warning' ui/src/modules/brokers`
    finds nothing. `resetOffsets()` (`:262-292`) builds the typed-confirm text from the same
    preview without the warning.
- **Failure:** a user previews a reset of a group whose consumers are still running. The server
  says "consumer group g is Stable; stop its consumers first". The UI drops that, shows a normal
  lag preview, asks the user to type the group name and then fails with the 409 toast. The
  PC-5 preview warning, which exists to stop exactly this, never reaches the person.
- **Fix:** add `warning?: string` to `DryRunResp` in `modules/brokers/types.ts`. In
  `GroupsTab.svelte`, render `dryRunResult.warning` above the preview (`--warning` styling,
  `role="status"`) and disable or relabel "Reset" while it is set. Add a unit or e2e case that
  mocks the dry-run with `warning`.

### api-contracts-5-03 — minor — Four issue-tracker listing rows omit the required `?account_id=` (400 without it)

- **Where:** `docs/contracts/api.md:1876` (`GET /issue/projects`), `:1877`
  (`GET /issue/search`), `:1879` (`GET /issue/confluence/spaces`) and `:1880`
  (`GET /issue/confluence/search`). Each has request "—", and none mentions `account_id`. The
  neighbouring rows put it in the path: `GET /issue/my-work?account_id=` (`:1878`) and every
  `/issue/confluence/pages…?account_id=` row (`:1881-1885`).
- **Evidence:** each handler fails first with `Error::Invalid("account_id query param
  required")`: `list_projects` (`crates/otto-issues/src/http.rs:347-350`), `search_issues`
  (`:365-368`), `list_spaces_cf` (`:455-458`) and `search_pages_cf` (`:473-476`). The UI always
  sends it (`ui/src/lib/api/issues.ts:87, 95, 115`). `features/jira-confluence.md:144, 168`
  spells it out, and `api.md:4419` cites `GET /issue/projects?account_id=&meta=1`, so only the
  authoritative table is behind. This matters more now: `otto.list_issue_projects` /
  `otto.create_issue` point agents and external MCP clients at `/issue/projects` as the source
  of `project_key`.
- **Fix:** write the four rows as `GET /issue/projects?account_id=` (and the same for search,
  spaces and confluence/search), or add "`account_id` (required; 400 without it)" to each
  request cell, matching `my-work`.

### api-contracts-5-04 — nit — `BrokerReplayResp.status` is declared in TS but no route ever sends it

- **Where:** `ui/src/lib/api/types/brokers.ts:351-360` (`BrokerReplayStatus` and
  `status?: BrokerReplayStatus` with the doc comment "The stored record's status (PC-5)"), added
  in 185b542f8.
- **Evidence:** the only route that returns this type is `POST /brokers/clusters/{id}/replay`
  (`otto-brokers/src/http.rs:558-583`), and it serializes `ReplayResp`
  (`otto-brokers/src/types.rs:758-769`: `replay_id, source_topic, target_topic, count, evidence,
  error?, evidence_saved`), which has no `status`. The `running` / `done` / `failed` /
  `interrupted` value lives only in `broker_replays.status`
  (`otto-state/src/broker_ops.rs:236, 257`), and no HTTP route reads replay rows
  (`rg replays crates/otto-brokers/src/http.rs` is empty). `api.md:3585` and
  `features/message-brokers.md:564` correctly list no `status`. No UI code reads it.
- **Fix:** remove `status` and `BrokerReplayStatus` from the TS mirror. Alternatively, add
  `status` to `ReplayResp` (`"done"` / `"failed"` from `replay.rs:127`) and to `api.md:3585`
  together.

### api-contracts-5-05 — nit — The swarm agent PATCH row quotes a 403 text the route never sends and names fields that do not exist

- **Where:** `docs/contracts/api.md:355` (#67): "a change to `prompt`, `soul`, `provider`, `model`
  or `cwd` needs the swarm's creator, a ws admin or root (else 403 "only the swarm's creator or a
  workspace admin can change an agent's role definition")".
- **Evidence:** `crates/otto-swarm/src/http.rs:270-275` answers `"only the swarm's creator or a
  workspace admin can change an agent's {changed.join(", ")}"`, for example "…an agent's prompt,
  soul". There is no "role definition" text. `prompt`, `soul` and `cwd` are labels, not
  `UpdateAgentReq` fields (`otto-swarm/src/types.rs:123-153`); the request fields are
  `scope_md` / `specialization`, `soul_md` / `soul_name` and `cwd_mode`, which
  `features/agent-swarm.md:744-747` already names correctly. The same kind of quoted-string
  drift was scored in iteration 3 (3-10).
- **Fix:** in `api.md:355`, use the field names ("a change to `scope_md` / `specialization`,
  `soul_md` / `soul_name`, `provider`, `model` or `cwd_mode`") and quote the real text: "403
  'only the swarm's creator or a workspace admin can change an agent's <changed parts>' (e.g.
  'prompt, soul')".
