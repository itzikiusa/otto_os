# Iteration 01 — L17–L20 access

Static trace of the current tree. No Cargo, no daemon, no engine-driver edits. Line numbers are from this read. Scores are this pass only; 9.8 is the target, not the result.

| Lens | Score | Confirmed defects |
|---|---|---|
| L17 privileged/root cross-session messaging | 9.6 | 0 blocking. Stale comments only. |
| L18 workflow cwd outside the workspace | 7.2 | 1 major: root is refused the same way as an editor. |
| L19 DB capability / read-only vs ordinary privileged execution | 6.8 | 1 major: a non-root owner on an enforced connection is sent through native-scope verification that a normal account cannot pass. |
| L20 authorization compatibility | 9.4 | 0 new cross-user hole. Do not add a root-only theatre check. |

## Do not reintroduce

These are the prior over-security bugs. The current tree does not have the first one. A fix for L18 must not recreate the second in the other direction (blocking registered repos, or blocking root again).

- Do not make `otto.send_message` / `POST /sessions/{id}/message` fail for root. A root person and a root-owned agent credential can already deliver a message to a non-shell agent session. `WorkspacesRepo::role_of` treats `is_root` as Admin in every workspace (`crates/otto-state/src/workspaces.rs` 416–422). `input_user` then allows root (`crates/otto-server/src/modules.rs` 53–62). `agent_message_rule` returns before the delegated-worker check when the caller is an agent credential and the target is `kind=agent` and `provider != "shell"` (`crates/otto-sessions/src/http.rs` 594–601). The feature guard lets root past the Agents Edit requirement (`crates/otto-server/src/feature_guard.rs` 278–282).
- Do not refuse a registered repo, a linked worktree of one, or a path inside the workflow workspace root. That already works for every caller, including a non-root editor (`ensure_path_in_workspace`, `crates/otto-server/src/workflow_engine.rs` 7726–7787).
- Do not drop `is_root` on a normal or agent token. Share links force `is_root=false` (`crates/otto-rbac/src/tokens.rs` 415–424). That is a real guest boundary.
- Do not let an agent credential pass `require_human` / Admin / Secret routes by being owned by root (`crates/otto-server/src/auth.rs` 142–158, `credential_class_gate` in `feature_guard.rs` 340–354). That is a real credential-class boundary, not theatre.
- Do not remove `__read_only_execution` from enforced reads. Owner and root reads on an enforced connection are supposed to run in a native read-only transaction so a function or view cannot hide a write (`docs/features/resource-access-and-database-changes.md`). Writes already strip the flag (`service.rs` 2226–2238). Production mutations stay on the reviewed-change path for root too (`service.rs` 885–892).

## L17 — messaging (9.6)

### What works

`POST /sessions/{id}/message` (`modules.rs` 6098–6136) reloads the caller, requires workspace Editor or Admin, then owner-or-admin, then `resource_sessions::check`, then `agent_message_rule`.

- A person's token has `agent_session_of == None`, so `agent_message_rule` falls through to `agent_input_rule`, which allows any target (`http.rs` 559–566). Root is Admin everywhere, so root can message another user's live agent session, including `provider=shell`.
- An agent credential (session API token: `kind=api` plus `session_scope`, `tokens.rs` 622–639 and 507) may message any non-shell agent the same user could message. A shell or SSH sibling stays 403 unless `meta.delegated_by` is that agent. The unit rule is `crates/otto-sessions/tests/agent_messaging.rs`. The live test `agent_token_broadcast_and_relay_never_reach_a_sibling` (`crates/otto-server/tests/personal_agent_policy.rs` 363–450) uses `provider=shell` and still expects 403. That shell wall should stay: a shell is an unsandboxed PTY, and `/message` is paste plus Enter.
- Direct session tools (`crates/ottod/src/mcp_tools.rs` `otto_send_message`, ~4210) POST the same route with the session token. They do not go through the outward approval gate. The catalog tool `otto.send_message` is in `DANGEROUS` (`crates/otto-mcp/src/outward/catalog.rs` 164–167) and the default `mcp_require_approval_dangerous` is true (`mcp_outward.rs` 79–86). That gate is the same one as `broadcast_message` and `open_session`. Leave it. Exempting root there would be theatre; the in-session tool already reaches the route.

### Not a bug

`root_authority` is withheld on agent writes that are not `RouteClass::Outward` (`feature_guard.rs` 93–110, `policy.rs` 1696–1698). `/sessions/{id}/message` is untagged, so a root agent's token has `root_withheld` for that POST. The message handler never calls `root_authority`. It uses `user.is_root` from the user row. Withholding root authority here does not block send.

### Residual (nit, do not "fix" by narrowing)

Comments still say an agent token may message only itself and workers it opened:

- `modules.rs` 6021–6023 (`open_agent_session`)
- test header at `personal_agent_policy.rs` 363–367

The rule and `docs/contracts/api.md` (POST `/sessions/{id}/message`) already say a sibling non-shell agent is allowed. Update the comments when root next touches the file. Do not tighten `agent_message_rule` back to delegated workers only.

## L18 — workflow cwd (7.2)

### Bug

`ensure_run_location` / `ensure_path_in_workspace` (`workflow_engine.rs` 7644–7787) accept a path only when it is inside the workflow workspace root, a repo registered in any workspace, or a linked worktree of one. There is no `is_root` branch. Relative paths and `..` are refused for everyone (keep that).

The manual route calls it for every editor, root included (`crates/otto-server/src/routes/workflows.rs` 597–603). Chat uses the same check with no caller (`crates/otto-workflows/src/chat.rs` 706–715).

`manual_run_location_is_limited_to_registered_repos` (`crates/otto-server/tests/run_repo_scope.rs` 365–367 and 424–442) creates the caller with `is_root: true` and expects 400 for an unregistered absolute folder, `/`, and a comma-separated mix that includes that folder. The test locks in the over-restriction.

Registered repos outside the workspace already return 200 in that test (lines 484–519). Do not touch that case.

### Minimal fix (root applies)

Add an exemption only for a person's manual run whose caller `is_root` is true:

- Still require an absolute path (`~` expanded) and still reject `..`.
- Allow an unregistered absolute folder.
- Do not key the exemption off the workflow creator. Chat, webhook, and event starts have no person at the keyboard. A Slack `Working Directory:` must stay on the registered-repo rule even when the workflow owner is root. Otherwise a channel message can point a root-owned workflow at an arbitrary folder.
- Pass the caller into `ensure_run_location` from `run_workflow` only. Leave `WorkflowCtx::check_run_location` strict for chat.
- Update `manual_run_location_is_limited_to_registered_repos`: the seeded user is root, so the foreign-folder case must expect 200 for that user, and a second non-root user must still get 400. Keep the relative, `..`, and unknown `org/name` refusals for root.

Do not also special-case workspace Admin. Admin is a delegated role. The authorized exception is root.

### Do not "fix" these as if they were the same bug

- `check_session_cwd` (`crates/otto-sessions/src/manager.rs` 2804–2823) still refuses `/`, `$HOME` or a parent of it, and Otto's data dir, including for root, because that cwd becomes a Seatbelt write grant. A root workflow pointed at `$HOME` itself will pass a location exemption and then fail when the agent node spawns, unless the process sandbox is off and the exception at 2818 applies. Leave that guard. The L18 bug is an unregistered project folder, not the filesystem root.
- `check_agent_cwd` (2826–2883) applies only when the opener's credential is an agent session, and only to repos registered in that workspace. A person's workflow run does not enter it. Do not extend it to root-started workflow cwds.
- `node_cwd` (`workflow_engine.rs` 8567–8580) can take `params.cwd` from the graph. That path is not passed through `ensure_run_location`. An editor can already store an absolute folder on a node, and session create then only hits `check_session_cwd`. Do not close that by refusing root. If it is tightened later, use the same root exemption as the manual run.

## L19 — database execution (6.8)

### Bug

New connections start enforced and get an owner allow rule whose `credential_connection_id` is `None` (`crates/otto-connections/src/service.rs` 502–547, `initialize_owner_policy` in `crates/otto-state/src/resource_access.rs` 626–661). `credential_profile_with` then always returns `Some(scope)` in enforced mode (`crates/otto-dbviewer/src/access.rs` 204–240), including for the connection's own account.

`verify_native_snap` (`crates/otto-dbviewer/src/service.rs` 905–935) skips native inspection only when `user.is_root`. Every other caller on an enforced connection calls `driver.native_grants`.

- Redis, MongoDB, and ClickHouse use the default (`crates/otto-dbviewer/src/driver.rs` 34–42): `native_scope_required: native privilege verification is unsupported for this engine`. A non-root owner cannot run a read or a write. Root can, because of the skip. The lexer classification in `access::operations` (656–677) already lets those engines parse; this call undoes that for everyone except root.
- MySQL `mysql_grants` (`native_access.rs` 17–59) refuses global privileges, roles, `WITH GRANT OPTION`, wildcards, and any privilege other than the mapped set. A normal admin login fails closed.
- PostgreSQL `native_grants` (`drivers/postgres.rs` 122–137) refuses `rolsuper`, createrole, createdb, replication, bypassrls, role membership, database owner, CREATE, or TEMP. A normal owner login fails closed.

That is the opposite of the documented split (`docs/features/resource-access-and-database-changes.md` 34): engines without a native ceiling reject **restricted** execution so Otto does not silently use a privileged shared account. The connection owner using the connection's own account is ordinary privileged execution. It is not a restricted profile.

`__read_only_execution` on enforced `db_query` / `db_export` (`service.rs` 1240–1246) is a separate, intended barrier. Root reads still get it. Do not strip it as part of this fix. `run_with` already removes it for enforced statements that are not pure `db_query` (2226–2238), and the MCP path sets it again after that (2239–2249).

`capabilities()` is static per driver and does not advertise this 403. The failure is the verifier, not the capability struct. Do not edit the engine drivers.

### Minimal fix (root applies, policy layer only)

In `verify_native_snap`, before `native_grants`:

- Keep the root skip.
- Also return `Ok` when `user.id == snap.conn.created_by` and the resolved profile id is the logical connection id (no alternate credential). That is the owner's own account.
- If the matched allow rule selects a different `credential_connection_id`, keep the current call. An unsupported engine must still return `native_scope_required` and must not fall back to the shared account.
- A non-owner whose allow rule has `credential_connection_id: None` is the forbidden fallback. Leave that on the native-scope error. Do not treat "profile id equals connection id" alone as ordinary privileged execution, or a delegated `db_query` rule would run as the admin login with no ceiling.

`unparsed_read_requires` still demands `db_data` for Redis, MongoDB, and ClickHouse reads (`access.rs` 740–744). An owner whose Database feature grant is only View does not receive `db_data` (`connections/service.rs` 514–527). That viewer ceiling is intentional (comment at `access.rs` 660–668). Do not remove it while fixing the native-scope bug. An owner with Database Edit already has `db_data` and `db_schema` on the owner rule; they are the ones blocked today.

## L20 — authorization compatibility (9.4)

Cross-user session boundaries still hold on the message path:

- `input_user` requires Editor or Admin, then root or creator or workspace Admin, then `resource_sessions::check` (`modules.rs` 43–64).
- `agent_message_rule`'s early `Ok` does not skip that check. `session_message`, `submit_session_text`, and `input_agents` all call `input_session` / `input_user` as well (`modules.rs` 95–122, 6109–6113).
- A non-admin editor's agent can message only sessions that editor owns. A workspace Admin's agent can message other people's non-shell agents in that workspace. The Admin's own login could already do that. Not a new cross-user hole.
- Resource rules: root is an allow in `ResourceAccess::evaluate_policy` (`crates/otto-rbac/src/resource_access.rs` 102–104). A disabled user is still denied (99–101). Explicit denies do not apply to root; that is the existing root bypass, and production changes still require review.
- Share tokens drop `is_root`. Agent tokens do not. Keep both.

No extra check should be added that only root fails. In particular, do not route root database reads through `native_grants`, and do not put the workflow folder exemption behind a second "confirmed external folder" prompt that chat cannot satisfy and that the manual root call does not need.

## Evidence and limits

- Read, not executed. No `cargo test`, no daemon request, no Docker. The workflow test cited above is source inspection of a test that encodes today's 400, not a run of that test.
- Engine drivers were read only at the native-grants and `__read_only_execution` seams (`driver.rs`, `mysql.rs` grants, `postgres.rs` grants, `clickhouse.rs` readonly flag). No driver edits.
- `resource_sessions::check` re-checks k8s, AWS, SSH shell, and DB Assistant bindings. A root user still passes `GrantsRepo::check_global` (`grants.rs` 292–300) and resource evaluation. A message to a normal agent session has no resource binding and returns at `resource_sessions.rs` 81–88.
- Iteration 01 does not claim 9.8. L18 and L19 need the patches above and a focused test each before a rescore.
