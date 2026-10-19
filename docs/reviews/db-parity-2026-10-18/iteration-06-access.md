# Iteration 06 — access (location strings)

Static pass. Cargo was not run. `python3 scripts/loc-ratchet.py` on this tree: `otto-server 130844/130939`. Baseline unchanged. The earlier failure was `131019 > 130939` (+80) before the helper move.

| Lens | Score | This pass |
|---|---:|---|
| L17 messaging | 9.7 | Not this pass. |
| L18 workflow cwd | 8.7 | Comma-separated directories and `~` traced. Assertion aligned with the relative-path sentence. Not re-executed. |
| L19 native grants | 8.3 | Not this pass. |
| L20 authorization compatibility | 9.4 | Not this pass. |

## What moved

`ensure_path_in_workspace`, `looks_like_path`, `match_repo_path`, `git_main_worktree`, `run_location_error`, and `existing_host_folder` are in `crates/otto-workflows/src/location.rs`. No `ServerCtx`. Tilde expansion in that file uses `dirs::home_dir`, the same helper the engine's other `expand_tilde` uses.

`ensure_run_location_as` in `crates/otto-server/src/workflow_engine.rs` still lists repos and calls `ensure_repo_registered`. `POST /workflows/{id}/run` still sets `allow_host_folder = user.is_root && ui_bridge::is_human`.

## Case: two directories in one field

`working_directory` is split on commas. Each non-empty segment is checked on its own. A value such as `<workspace-root>,/tmp/already-there` succeeds for a human root only when the second path is an existing directory, has no `..`, and is not `/`. The same value fails for an editor, a workspace Admin, and a root agent token on the second segment, with the registered-repo sentence and without the host-directory rule.

## Case: `~`

`~` and `~/somewhere` are expanded before the absolute-path check. After expansion, `~` is the home directory. For a human root, `existing_host_folder` accepts it: it exists, it is a directory, its parent is not absent, and the expanded string has no `..` component. A confined agent spawn still refuses `$HOME` in `check_session_cwd` (`crates/otto-sessions/src/manager.rs`). The workflow location check does not call that spawn guard. This pass does not add that refusal.

## Assertion

`manual_run_location_is_limited_to_registered_repos` no longer requires the substring `existing host directory` for `relative/dir`. That refusal must contain `absolute path`, `already exist`, `be a directory`, `no '..'`, and `filesystem root`. A missing `..` climb must contain `may not contain '..'` plus the same rule. An existing `..` escape, a missing directory, a file, and `/` must contain `existing host directory` plus the same rule.

Root's access run (Rust 1.99.1) had 7 passes and this one assertion failure. `root_agent_token_does_not_get_the_host_folder_exception` passed in that run. The assertion edit has not been compiled again.

## Not changed

`node_cwd` is still outside this check. No new path is refused.
