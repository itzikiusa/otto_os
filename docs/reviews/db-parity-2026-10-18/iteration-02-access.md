# Iteration 02 — L17–L20 access

Source changes for the two confirmed defects (L18, L19) and the L17 comment residual. No Cargo, no daemon, no engine-driver edits. The MySQL slot was left free. Scores are this pass only. 9.8 is not claimed: the new tests were not executed.

| Lens | Iteration 01 | Iteration 02 | Why it moved |
|---|---|---|---|
| L17 privileged/root cross-session messaging | 9.6 | 9.7 | Stale comments now match the live sibling rule. Behavior unchanged. Tests not run. |
| L18 workflow cwd outside the workspace | 7.2 | 8.5 | Manual root exception is in the route and the location check. Regression tests are written, not run. |
| L19 DB capability / read-only vs ordinary privileged execution | 6.8 | 8.3 | `verify_native_snap` skips native grants for the owner on the connection's own account. Alternate profiles still validate. Tests written, not run. |
| L20 authorization compatibility | 9.4 | 9.4 | No new cross-user hole in the patch. The host-folder exception is root on a person's manual run only. |

## Role model (L18)

`User` has one privilege flag: `is_root` (`crates/otto-core/src/domain.rs`). There is no separate "may use an arbitrary host directory" capability.

- `WorkspaceRole::Admin` is membership in one workspace. `role_of` grants it to every workspace only when `user.is_root` (`crates/otto-state/src/workspaces.rs`). A non-root Admin is a delegated workspace role.
- `Capability::Admin` is the top of a feature ladder (Workflows, Database, and the others). It does not mean the host filesystem.
- Host file import/export is already root-only (`require_local_path_access` in `crates/otto-dbviewer/src/service.rs`).

No non-root principal was given the folder exception. A workspace Admin with Workflows Edit is covered by the new refusal test.

## L18 — what changed

`POST /workflows/{id}/run` calls `ensure_run_location_as` with the exception only when `user.is_root` and the credential is a person's (`ui_bridge::is_human`). An agent token keeps `is_root` on the user row and does not get the exception.

The exception accepts an absolute directory that already exists, after `~` expansion, with no `..` component, and that is not the filesystem root (`existing_host_folder`, `workflow_engine.rs`). Relative paths, `..`, missing paths, and files stay refused for root. Registered repos, linked worktrees, and the workspace root are unchanged and do not need the flag.

`WorkflowCtx::check_run_location` still calls strict `ensure_run_location`. Chat, webhook, and event starts stay there. The exception is not keyed off `workflow.created_by`.

Tests in `crates/otto-server/tests/run_repo_scope.rs`:

- `manual_run_location_is_limited_to_registered_repos` — caller is root. Existing foreign directory and a `repos` path to it expect 200. Relative, `..`, `/`, a missing directory, and a file stay 400.
- `manual_run_outside_registered_repos_stays_refused_for_non_root` — Editor and workspace Admin expect 400 for that foreign directory and 200 for the workspace root.
- `chat_working_directory_outside_the_workspace_is_refused_in_the_thread` — asserts the workflow owner is root and the Slack working directory is still refused.

Contract sentence for `POST /workflows/{id}/run` in `docs/contracts/api.md` matches this.

Left as they were:

- `check_session_cwd` still refuses `/`, `$HOME` or a parent, and Otto's data dir at agent spawn, including for root.
- `node_cwd` / `params.cwd` on a graph node is still not passed through `ensure_run_location`. An editor can already store an absolute folder there. This patch does not close that, and does not refuse root there.

## L19 — what changed

`Resolved` now carries `profile_id` from `credential_profile_with`. `verify_native_snap` (`service.rs`):

- Still returns before `native_grants` when `user.is_root`.
- Also returns when `user.id == conn.created_by` and `profile_id == conn.id` (the owner's own account).
- Any other profile, including the owner's alternate `credential_connection_id`, still calls `native_grants`. Redis, MongoDB, and ClickHouse keep the default `native_scope_required` error. There is no fallback onto the shared privileged account.
- A non-owner whose allow rule has `credential_connection_id: None` resolves to the logical connection and still hits `native_grants`.
- `__read_only_execution` is not removed. The test asserts it is still set on the owner resolve.

`crates/otto-dbviewer/src/service/native_owner_tests.rs` (`owner_same_connection_skips_native_grants_alternate_and_non_owner_do_not`) builds an enforced Redis connection with a non-root owner, an editor, a root user who did not create it, and a second profile with the same endpoint. It does not open a socket: the real Redis driver has no `native_grants` override, so the default error is the ceiling. Drivers were not edited.

`docs/features/resource-access-and-database-changes.md` states the owner / alternate / non-owner split. Governed reads stay read-only. `unparsed_read_requires` still asks Redis, MongoDB, and ClickHouse reads for `db_data`. A View-only owner still does not receive `db_data`.

Not changed, on purpose:

- `service/validation.rs` `validate_access_policy` still runs `native_grants` for every non-root subject when a policy is activated, and still rejects non-MySQL/Postgres engines there.
- `service/changes.rs` `approved_preflight` still compares native grants for a non-root owner. Root still skips that compare. Production review is unchanged.

## L17 — comments only

- `crates/otto-server/src/modules.rs` (`open_agent_session`): an agent token may message a non-shell sibling. Shell and SSH stay refused unless `delegated_by` names that agent.
- `crates/otto-server/tests/personal_agent_policy.rs`: the broadcast/relay test's sibling is `provider=shell`. The 403 assertion is unchanged.

`agent_message_rule` was not tightened.

## Limits

- Cargo was not run. Nothing here is a passing test result.
- Engine drivers were not edited.
- `node_cwd` and the policy-activation / change-preflight native checks remain the residuals above.
- Iteration 02 does not claim 9.8.
