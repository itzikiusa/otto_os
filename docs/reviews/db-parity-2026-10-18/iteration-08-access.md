# Iteration 08 — access (execution policy vs setup)

Static pass. No Cargo. `native_owner_tests.rs` was not edited and was not run in this session.

| Lens | Score | This pass |
|---|---:|---|
| L17 messaging | 9.7 | Not this pass. |
| L18 workflow cwd | 8.7 | Not this pass. |
| L19 native grants | 8.3 | Owner bypass is only the runtime snap. Activation and reviewed changes stay stricter. |
| L20 authorization compatibility | 9.4 | View-only and production-write gates are unchanged. |

## Case: owner, same account, ordinary query

`verify_native_snap` (`crates/otto-dbviewer/src/service.rs`) returns before `native_grants` when the connection is not enforced, when `user.is_root`, or when `user.id == conn.created_by` and `Resolved.profile_id == conn.id`. `profile_id` is the id chosen by `credential_profile_with` and stored on `Resolved`. `__read_only_execution` is not cleared.

An alternate `credential_connection_id`, including one owned by the same user, still calls `native_grants`. A non-owner whose allow rule leaves `credential_connection_id` empty still calls `native_grants`.

## Case: activating Enforced for a non-root subject

`validate_access_policy` still requires the actor to be root. For a non-root, non-disabled subject it still rejects every engine other than MySQL and PostgreSQL with "restricted script execution is unsupported for this engine", and it still calls `native_grants` for MySQL and PostgreSQL. The runtime owner bypass does not apply at activation. A Redis, MongoDB, or ClickHouse owner can execute on their own account after the policy is already enforced, and still cannot be the reason a new enforced policy activates. This pass leaves that setup check in place.

## Case: approved change by that same owner

`approved_preflight` (`crates/otto-dbviewer/src/service/changes.rs`) skips the native-grant compare only when `user.is_root`. A non-root owner still needs one explicit database target and a grant set that matches that target and the script's operations. Reviewed execution is still MySQL and PostgreSQL only. Root still needs the persisted approval and claim. Production writes still require an approved change inside `authorize_snap`; root does not turn that into a direct write.

## Case: view-only owner

`authorize_snap` still runs before native verification. Redis, MongoDB, and ClickHouse reads are lexer-classified and `unparsed_read_requires` still adds `db_data`. A view-only owner does not receive `db_data`. The owner bypass does not grant that operation.

## Executed vs static

Root's access-suite log covered workflow location tests, not this policy test. This pass is static. 9.8 is not claimed.
