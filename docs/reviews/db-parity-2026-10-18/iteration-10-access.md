# Iteration 10 — access (where a run is admitted)

Static pass. No Cargo. No new location refusal.

| Lens | Score | This pass |
|---|---:|---|
| L17 messaging | 9.7 | Held from pass 09. |
| L18 workflow cwd | 8.7 | Admission paths differ. The manual and chat gates are unchanged. |
| L19 native grants | 8.3 | Held from pass 08. |
| L20 authorization compatibility | 9.4 | External caller input still cannot widen a run. |

Scores stay below 9.8. The relative-path assertion has not been re-executed. ClickHouse JSON/Dynamic coverage is outside L17–L20; root consolidates all 24 lenses.

## Case: five start paths

| Start | Location gate |
|---|---|
| `POST /workflows/{id}/run` | `ensure_run_location_as` with the human-root flag |
| Chat `Working Directory:` | `WorkflowCtx::check_run_location` → strict `ensure_run_location` |
| Webhook `POST /workflows/{id}/webhook/{token}` | Body location and target keys are stripped. `copy_location_defaults` copies the trigger spec, then `spawn_run`. No `ensure_run_location`. |
| Schedule tick | Input is `trigger`, optional `prompt`, and result destinations. No working directory is copied. No `ensure_run_location`. |
| Event listener | Same shape as schedule, with `event_kind`. No `ensure_run_location`. |

Chat refuses a foreign directory even when `workflow.created_by` is root. The webhook body cannot put that directory back: `LOCATION_KEYS` and `TARGET_KEYS` are reserved for `InputSource::Webhook`. The directory that can still be used is the one an Editor saved on the trigger spec. Refusing that spec here would reject runs that start today. This pass does not add that check.

Schedule and event inputs do not carry `working_directory`. A node `cwd` / `working_directory` param is still applied later by `node_cwd` and is still not passed through `ensure_run_location`. Closing that would be a new refusal of graph data that already runs. It stays open.

## Case: `git_pr` assumes the start was vetted

The `git_pr` step comments that `ensure_run_location` vetted a `repo_id` from run input. That is true for the manual route and for chat. A webhook spec's `repo_id` is copied and not passed through `ensure_repo_registered` at admission. An unknown id fails later, when the step resolves the repo, rather than at the webhook response. Left as it is.

## Executed versus static

| Check | Result |
|---|---|
| Root access suite, Rust 1.99.1, before the assertion edit | 7 passed. `manual_run_location_is_limited_to_registered_repos` failed on the `relative/dir` substring. `root_agent_token_does_not_get_the_host_folder_exception` passed. |
| Assertion edit and helper move | Static. `rustfmt --edition 2021` clean on the touched Rust files. Not compiled. |
| `python3 scripts/loc-ratchet.py` | `otto-server 130844/130939`. Baseline file not modified. |
| Passes 06–10 | Static traces only. Passing tests were not repeated. |
| Mongo live, focused suite, distinct live | Previously reported green by root. Not re-run here. |

`crates/otto-workflows/src/location.rs` is new. The server build depends on it. The LOC ratchet counts tracked files; this check passed with that file still untracked because the limit is the server `src` total.
