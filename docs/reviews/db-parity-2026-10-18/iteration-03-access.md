# Iteration 03 — access follow-up (workflow error, credential class)

Frozen for the Cargo slot. No test has been executed in this session. 9.8 is not claimed. Mongo live results do not cover these tests.

| Lens | Iteration 02 | Iteration 03 | Why |
|---|---|---|---|
| L17 messaging | 9.7 | 9.7 | Unchanged. |
| L18 workflow cwd | 8.5 | 8.6 | A failed human-root path now explains the host-directory rule. Agent-token regression is written, not run. |
| L19 native grants | 8.3 | 8.3 | Unchanged. |
| L20 authorization compatibility | 9.4 | 9.4 | The agent exclusion is the existing credential class, not a new lockout. |

## Error text

`ensure_path_in_workspace` still tells a non-root caller, and an external trigger, that a run may only use a registered repo or the workspace root.

When the caller is allowed the host-directory exception and the path still fails, every refusal from `run_location_error` adds the same rule: the folder must already exist, be a directory, contain no `..`, and not be the filesystem root. That includes a relative path, a `..` that cannot be canonicalized, a missing directory, a file, `/`, and an existing directory reached through `..`. A non-root caller and an external trigger do not get that sentence.

`manual_run_location_is_limited_to_registered_repos` requires that shared rule on every human-root refusal. A relative path must also say "absolute path". A `..` that cannot be canonicalized must say "may not contain '..'". An existing `..` escape, a missing directory, a file, and `/` must say "existing host directory". The phrase "existing host directory" is not required for the relative-path refusal. The non-root test requires the registered-repo sentence and the absence of "existing host directory".

## Credential class

`root_agent_token_does_not_get_the_host_folder_exception` inserts an agent session owned by the root user, mints `issue_session_api_token`, and checks `managed_session_id` plus `!is_human`. The foreign existing directory expects 400 with the registered-repo wording. The workspace root still expects 200, so the token is not locked out of a normal run.

The chat test still refuses a foreign working directory when the workflow owner is root, and now also asserts that reply does not offer the host-directory rule.

`docs/contracts/api.md` for `POST /workflows/{id}/run` matches this wording. No other docs were consolidated.

## Limits

These tests were not executed. L19 residuals from iteration 02 (`validate_access_policy`, `approved_preflight`, `node_cwd`) are unchanged.
