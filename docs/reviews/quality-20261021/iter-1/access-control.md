# Access control review: iteration 1

Lens: who can access what. That covers route auth and RBAC (`otto-rbac`, the feature guard, `policy.rs`), the MCP outward tools and self-calls, share, remote and mobile access, room guests, API tokens and workspace scoping.

Calibration: these findings report over-restriction and inconsistent checks with the same weight as real reachable leaks. Theoretical hardening and loopback-only concerns are excluded.

## Score

Start at 10.0.

| ID | Severity | Deduction |
|----|----------|-----------|
| access-control-1-01 | major | -0.3 |
| access-control-1-02 | major | -0.3 |
| access-control-1-03 | minor | -0.1 |

**Score: 10.0 - 0.3 - 0.3 - 0.1 = 9.3**

## Previous findings

This is iteration 1, so there are no previous findings to track.

## Areas covered

- `crates/otto-server/src/feature_guard.rs`, all of it:
  - the scope branch for share links
  - the MCP-only branch
  - the credential-class gate
  - the read-only agent gate
  - the host-files rule
  - the plugin branch
  - root-route gates
  - the bound-session and resource-route tier lowering
- `crates/otto-server/src/policy.rs`: `route_class` for the Admin, Secret and Outward classes, `agent_root_write_allowed`, and the rooms, game-rooms, brokers, MCP and workspaces arms of `policy_for`.
- `crates/otto-core/src/auth.rs`: `root_authority` and request-credential withholding, `session_owner_or_admin` and `authorize_owner`.
- `crates/otto-rbac`: the `lib.rs` role checker, and `tokens.rs` (`AuthContext` construction, session API tokens, share list and revoke).
- `crates/otto-server/src/auth.rs`, `ui_bridge::is_human` and `self_call.rs` (session-bound self-call credentials).
- MCP outward:
  - the `route_for` write map in `otto-mcp/src/outward/exec.rs`
  - `catalog::DANGEROUS` and `tool_is_mutating`
  - `mcp_outward::governed_invoke` and `SelfCallAs`
  - `gateway_invoke`
  - the POST call sites of the native stdio tools (`ottod/src/mcp_tools.rs`)
- `personal_agent_policy.rs`: the read-only HTTP allow-list compared with the read-only tool policy.
- Sessions:
  - the `/sessions/{id}/input`, `/message` and `/wait` routes, and `input_user`
  - `agent_input_rule` and `agent_message_rule`
  - `open_agent_session` and its cwd confinement
  - `agent_attach_rule` on `/ws/term`
  - `resource_sessions.rs`
- Session rooms (`rooms/auth.rs`, `http.rs`, `recap/http.rs`, the registry's owner checks) and game rooms (`game_rooms/http.rs`, `socket.rs`), checked against `docs/contracts/game-rooms.md`.
- Share routes (`routes/share.rs`) and the guest UI's API use (`SharePage.svelte`, `Terminal.svelte`).
- Workspaces: CRUD and members routes.
- Search: ranked search per-source feature gating and `otto_state::search`.
- `/ws/events` scoping, at the level of a summary.
- Workflows `run_workflow` (approval-skip and host-folder gates) and the AWS and Kubernetes `require_setup_authority`.
- Root-gated handlers in usage, insights, skills, context and channels, checked against the agent-credential withholding.
- Usage scoping (`own_ids`).
- Canvas: the global list and `get_file`.
- Agent rooms: the personal-agent rooms routes.

## Areas not covered yet (next iteration)

- `crates/otto-connections` and `crates/otto-dbviewer`: per-connection resource policy (Enforced compared with Legacy), DB Explorer write gates and the DB Assist `x-assist-key`.
- `crates/otto-k8s` and `crates/otto-aws`: the full `allowed()` operation matrix and child-namespace grants.
- `crates/otto-git`: repo registration by an Editor at any path, and account `authorize_owner` call-site coverage.
- `crates/otto-issues`: account ownership on every route (recent tests claim it is covered; not re-verified).
- `routes/impersonate.rs` and `routes/users.rs`, `grants.rs` and `access_groups.rs`: escalation paths such as granting oneself, disabling root, or editing the last admin.
- `otto-product`, `otto-design`, `otto-vault` and `otto-canvas` item routes: workspace resolution from the row on flat `/{id}` routes.
- `otto-swarm` webhook and ingest (`/ingest/*` per-session token), and `channel_webhook.rs` run scoping.
- `plugins.rs`: the sidecar host API token (`/plugin-host/*`) and the unauthenticated `/plugins/{slug}/ui/*` static serving.
- `routes/backup*.rs`, `state_archive`, and the `connection_export` Secret routes and their handler gates.
- `ws_events.rs` `allowed()`: the owner and admin caches across role changes during a long-lived socket.
- `lsp/mod.rs`, `browser_live.rs` and `api_stream.rs`: the handler side of the root-route gate.
- UI-side gating (`auth.isRoot` and feature checks) compared with the server for the remaining modules.

## Findings

### access-control-1-01: read-only agent sessions cannot use several read tools the policy allows (major)

**Files:**

- `crates/otto-server/src/personal_agent_policy.rs:504-540` (`READ_ONLY_POST_ALLOW` and `read_only_route_allowed`)
- `crates/otto-server/src/feature_guard.rs:182-202`

The governed policy lets a read-only session call every non-mutating catalog tool. `read_only_denies(bare)` is `tool_is_mutating(bare) || READ_ONLY_EXTRA_DENY.contains(bare)` (`personal_agent_policy.rs:53`). Each allowed tool then runs as a self-call bound to the calling session. `mcp_outward.rs:711` uses `SelfCallAs::Caller(self_call_binding(auth))`. `self_call::lease` then mints `issue_session_api_token_labeled`, so the replayed request carries `managed_session_id` (`tokens.rs:507`).

The feature guard's read-only branch handles that request. It refuses every non-GET request whose template is not in `READ_ONLY_POST_ALLOW`, and the list holds only ten templates.

These non-mutating tools self-call POST routes that are not on the list:

| Tool | Route it calls | Source |
|------|----------------|--------|
| `get_context_packet` | `POST /workspaces/{wid}/mcp/context-packet` | `exec.rs:301-307`; a Viewer-gated read in `mcp_capabilities.rs:282-288` |
| `consume_broker_messages` | `POST /brokers/clusters/{id}/topics/{topic}/consume` | `exec.rs:712-735`; `policy.rs:825-829` calls it a non-mutating peek |
| `aws_logs_insights` | `POST /aws/accounts/{id}/logs/insights` | `exec.rs:1755-1772`; not in `DANGEROUS` |
| `assistant_recall` | `POST /assistant/agent/recall` | Governed `exec.rs:1509-1527` and native `ottod/src/mcp_tools.rs:3334-3347`. `READ_ONLY_EXTRA_DENY` deliberately excludes recall and denies only remember and forget. |

**Scenario:**

1. A Proactive personal-agent run, or a run of a `read_only` scheduled task (`meta.read_only = true`), calls `otto_get_context_packet`, `otto_consume_broker_messages` or `aws_logs_insights`.
2. `governed_invoke` passes the call.
3. The self-call's guard returns 403 with "this agent session is read-only: it may read but not change anything".

The tool is advertised to the session and allowed by policy, yet it can never succeed. The read-only allow-list and the catalog's own read/write classification contradict each other, and no test ties them together. `read_only_http_allows_reads_and_listed_read_posts_only` checks only four hand-picked templates.

**Fix:**

1. Add these templates to `READ_ONLY_POST_ALLOW`:
   - `/workspaces/{}/mcp/context-packet`
   - `/brokers/clusters/{}/topics/{}/consume`
   - `/aws/accounts/{}/logs/insights`
2. For `assistant_recall`, the template `/assistant/agent/{tool}` also covers remember and forget, so it cannot go on the allow-list as a template. Allow the template for read-only sessions only when the concrete last segment is `recall`. Either do that in the guard by passing the concrete path to `read_only_route_allowed`, or reject `remember` and `forget` for read-only sessions inside `assistant::tools`.
3. Add a unit test that walks `otto_tool_specs()` and checks one condition for every tool where `!read_only_denies(tool)` and `route_for(tool, sample_args)` is not a GET: `read_only_route_allowed(POST, template)` must hold. The allow-list then cannot drift from the catalog again.

No mutating tool gains access, since all four routes are reads gated at Viewer or View.

### access-control-1-02: a share link on your own session cannot be revoked if someone else minted it, and the revoke reports success (major)

**Files:**

- `crates/otto-server/src/routes/share.rs:847-859` (`list_shares`) and `:904-937` (`revoke_share`)
- `crates/otto-rbac/src/tokens.rs:1671-1680` and `:1754-1765`

`GET /sessions/{id}/shares` is authorized by `require_session_owner_or_admin` and returns every live share on the session (`WHERE kind='share' AND session_scope = ?`, with no minter filter).

`POST /sessions/{id}/share` lets the session owner or a workspace Admin mint a link (`share.rs:307`). The row is stored under the minter's `user_id` (`issue_share_token(&user.id, …)`).

`DELETE /auth/shares/{share_id}` revokes with `UPDATE … WHERE id = ? AND user_id = ? AND kind='share'` bound to the caller. It ignores how many rows were affected and always returns 204.

**Scenario:**

1. Root, or a workspace Admin, mints an Editor share on Bob's agent session. Bob is a workspace Editor.
2. Bob opens Share on his session. `ShareModal.svelte:93` lists the link.
3. Bob clicks Revoke. `ShareModal.svelte:195` sends `DELETE /auth/shares/{id}`.
4. The update matches 0 rows, and the request returns 204.
5. The UI shows "Share revoked", but the link still lets its holder type into Bob's agent.

The mirror case also fails: an Admin who can see Bob's own share cannot revoke it either. The session owner cannot take away live remote control of their own session, and the UI falsely confirms that the revoke worked. This does not show in a single-user install, because there the minter is always the owner.

**Fix:**

1. In `revoke_share`, load the share row by id with `kind='share'`, and return 404 when it is missing.
2. Allow the revoke when either condition holds:
   - `row.user_id == caller` (the minter, as today, which keeps Settings → Active links working), or
   - `require_session_owner_or_admin(&ctx, &user, &session_of(row.session_scope))` passes. This is the same authority that `list_shares` and `mint_share` already use.
3. Revoke by id with `kind='share'` and no user filter.
4. Evict the session.
5. Audit the action with both the caller and the minter.
6. Return 404 instead of 204 when no row matches.

Every current legitimate flow keeps working, and the owner or admin gains the revoke that matches the list they can already see.

### access-control-1-03: Rooms reads and game hosting require Agents:Edit, though the module shows at Agents:View (minor)

**Files:**

- `crates/otto-server/src/policy.rs:94-106`
- `docs/contracts/game-rooms.md` ("HTTP")
- `ui/src/lib/sidebar.ts:96`

`policy_for` returns `Require(Agents, Edit)` for every method on these routes:

- `/rooms`
- `/rooms/{id}`
- `/rooms/{id}/invites`
- `/sessions/{id}/room`
- `/game-rooms`
- `/room-recaps*`
- `/room-recap-capabilities`

The Rooms module is shown at Agents:View: the sidebar's `feature: 'agents'` is checked at view. `RoomLobby.svelte:44` loads `GET /rooms` on open, so a non-root user with Agents:View gets 403 and the lobby shows an error. Yet that user can still join rooms through the public `POST /room-join` and play solo games.

`POST /game-rooms` is documented as needing only "a normal authenticated owner credential (no impersonation, managed-session, share or MCP-only credentials)". It is independent of sessions and workspaces, but it is refused without Agents:Edit.

The reads leak nothing:

- `rooms::http::list` returns only rooms where `room.owner == effective_user`.
- The recap `list`, `detail`, `image`, `export` and `revision` handlers are scoped to the owner through `get_archive(…, owner)`.
- `owner_auth` already refuses agent, MCP, share and impersonation credentials.

**Fix:**

1. In `policy_for`, require `Agents:View` for GET on `/rooms`, `/room-recaps*` and `/room-recap-capabilities`.
2. Require `Agents:View` for `POST /game-rooms`, or exempt it as the contract describes. Its handler already applies `owner_auth` and per-owner room limits.
3. Keep Edit on `POST /sessions/{id}/room`, `DELETE /rooms/{id}`, `/rooms/{id}/invites` and recap writes.
4. Regenerate `tests/snapshots/policy_decisions.txt`.

Hosting a session room still requires owning a running session, which already requires Agents:Edit.

## Checked and not reported (false-positive guard)

- Share-link scope (`scope_allows`): the guest page uses only `GET /sessions/{id}`, `/share/whoami`, `/ws/term` and input. All of them are allowed, and nothing else is.
- `/sessions/{id}/input` and `/message` use the same owner-or-admin, Editor and resource checks (`input_user` compared with `require_session_owner_or_admin`).
- Room, recap and game-room owner checks (`registry.rs:247`, `recap/http.rs:82`), and the WebSocket subprotocol tokens.
- Root withholding for agent credentials applies only to writes. The handlers checked that use `require_root` (insights, skills install, library, usage config, k8s and AWS setup) are person-administration routes, not flows agents use.
- `gateway_invoke` refuses every call from a read-only session. That is fail-closed for downstream tools whose mutability is unknown, so it was not reported.
- An external MCP token replaying as a person-classed self-call is a documented design choice, and its tools are approval-gated in `DANGEROUS`. It was not reported.
