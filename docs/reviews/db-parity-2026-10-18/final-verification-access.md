# Access final-gate evidence after iteration 10

Frozen. This is the access scorecard for the finished Rust nextest run. No score is 9.8. Doc tests and the UI gate were still running when this report was frozen, and they are not scored here. Root consolidates the 24-lens matrix. Source was not edited for this update.

Log: `/tmp/otto-parity-final-workspace-nextest.log`. `scripts/check.sh --all --check` ran rustfmt, clippy `--workspace --all-targets -- -D warnings`, then `cargo nextest run --workspace --profile ci --no-fail-fast`. rustfmt and clippy finished (`Finished dev profile` in 1m 22s). Nextest run ID `f42e75e0-536f-4663-92a6-1fa1399c3a0c`.

```
Summary [ 287.224s] 5806 tests run: 5806 passed, 124 skipped
```

The 124 skipped tests are the ones nextest reported at start (`124 tests skipped` across 104 binaries). After that summary the same log started `cargo test --workspace --doc` and was still compiling. That doc-test result is not in this snapshot.

The earlier `cargo test` log `/tmp/otto-parity-final-workspace.log` recorded six failures. All six are PASS in this nextest run, and the run summary is 5806 passed.

| Lens | Score | Why |
|---|---:|---|
| L17 messaging | 9.7 | The nextest run is green. It still does not name a cross-session send. Shell-sibling refusal and owner-or-admin checks are unchanged. |
| L18 workflow cwd | 9.7 | Every `run_repo_scope` test passed in this run, including root, non-root, chat, and the root agent token. A node `cwd` and a webhook trigger spec are still not passed through `ensure_run_location`. |
| L19 execution policy | 9.7 | Owner runtime, activation, and approved-preflight passed, including `owner_same_account_skips_activation_and_approved_preflight_native_grants` at 10.252s. The two read budgets and the three legacy MCP runs passed. View-only owners still do not receive `db_data`. Production writes still require an approved change. |
| L20 authorization compatibility | 9.7 | The agent token stays on the registered-repo rule, non-root manual runs stay refused, the three MCP runs pass the feature gate, and `share_and_mcp_tokens_reach_no_root_route` passed. |

## Passed in this run

`otto-server::it run_repo_scope`:

- `a_reply_in_a_bound_runs_thread_does_not_start_a_second_run` PASS 0.243s (4349/5806)
- `chat_working_directory_outside_the_workspace_is_refused_in_the_thread` PASS 0.247s (4351/5806)
- `launching_a_run_on_a_repo_the_launcher_cannot_edit_is_not_found` PASS 0.341s (4352/5806)
- `manual_run_location_is_limited_to_registered_repos` PASS 0.437s (4353/5806)
- `manual_run_outside_registered_repos_stays_refused_for_non_root` PASS 0.351s (4354/5806)
- `manual_workflow_run_takes_any_registered_repo_id` PASS 0.342s (4356/5806)
- `root_agent_token_does_not_get_the_host_folder_exception` PASS 0.335s (4358/5806)
- `webhook_body_cannot_set_reserved_run_input_keys` PASS 0.323s (4359/5806). That covers the untrusted webhook body. It does not check a directory saved on the trigger spec.

Previously failed in `/tmp/otto-parity-final-workspace.log`, PASS here:

- `otto-dbviewer service::perf_budget_tests::legacy_run_stays_within_state_read_budget` PASS 0.243s (1421/5806). The cargo-test failure was 5 state reads against budget 4.
- `otto-dbviewer service::perf_budget_tests::enforced_run_stays_within_state_read_budget` PASS 0.241s (1420/5806). The cargo-test failure was 12 state reads against budget 11.
- `otto-dbviewer::mcp_query read_passes_the_gate_and_reaches_execution` PASS 3.235s (1552/5806)
- `otto-dbviewer::mcp_query read_only_read_passes_the_gate_and_reaches_execution` PASS 3.242s (1550/5806)
- `otto-dbviewer::mcp_query the_same_write_without_read_only_is_not_refused` PASS 3.235s (1553/5806)
- `otto-apiclient grpc::tests::grpc_channel_is_dialled_once_and_redialled_after_forget` PASS 0.038s (17/5806). The cargo-test failure was `left: 6`, `right: 2`. This test is outside L17–L20.

Also PASS:

- `otto-dbviewer service::native_owner_tests::owner_same_account_skips_activation_and_approved_preflight_native_grants` PASS 10.252s (1689/5806)
- `otto-dbviewer service::native_owner_tests::owner_same_connection_skips_native_grants_alternate_and_non_owner_do_not` PASS 0.241s (1419/5806)
- `otto-dbviewer service::native_owner_tests::local_infile_grant_tracks_host_permission_for_incremental_scripts` PASS 0.240s (1417/5806)
- `otto-server feature_guard::root_route_tests::share_and_mcp_tokens_reach_no_root_route` PASS 0.013s (3300/5806)

## Still outside these scores

A workflow node `cwd` is not checked by `ensure_run_location`. A webhook or schedule spec can still carry a saved directory without that check. View-only owners still do not receive `db_data`. Production writes still require an approved change. ClickHouse JSON/Dynamic support is a separate limitation and is not scored here. Doc tests and the UI gate are outside this frozen snapshot.

LOC at freeze, before this report: `python3 scripts/loc-ratchet.py` reported `otto-server 130844/130939`. The baseline file was not changed.
