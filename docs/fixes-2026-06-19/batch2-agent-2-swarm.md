# Batch-2 Agent 2 — Swarm fixes (D1 data loss + D2 unschedulable tasks)

Date: 2026-06-19

Two verified-open swarm bugs. Both fixed, both covered by new unit tests.
`cargo check -p otto-server -p otto-swarm -p otto-state -p otto-git` passes;
all new and pre-existing tests in the touched modules pass.

Files changed (only the four I own):
- `crates/otto-git/src/local.rs`
- `crates/otto-server/src/swarm_workspace.rs`
- `crates/otto-swarm/src/service.rs`
- `crates/otto-state/src/swarm.rs`

---

## D1 — Worktree reset every turn discarded the agent's committed work

### Root cause (traced end-to-end)
`run_turn_inner` (`swarm_run.rs:249`) calls `ensure_cwd` on EVERY turn.
`ensure_cwd` (`swarm_workspace.rs`) unconditionally called
`git.worktree_add(path, branch, base)`, which runs:

```
git worktree add --force -B <branch> <base>
```

`-B` resets `<branch>` to `<base>` (the repo's current HEAD) and `--force`
re-points the existing path. So on turn 2+, the agent's branch — with the
commits it made on turn 1 — was reset back to base HEAD, throwing the work
away. A resumed session landed in the same directory but with an empty branch,
so the swarm could never make multi-turn progress.

### Fix
Make the first turn create the worktree (unchanged behavior) and every later
turn REUSE it (no `-B`, no `--force`, no reset).

`crates/otto-git/src/local.rs` — two new methods (and a doc note on the
existing `worktree_add` flagging it as destructive):

- `worktree_exists(path) -> bool`: reads `git worktree list --porcelain`,
  compares the requested path against each registered `worktree <abs>` line,
  canonicalizing both so symlink/`..` differences don't yield a false negative.
  Returns `false` (never errors) if the listing fails.
- `worktree_add_if_absent(path, branch, base) -> Result<bool>`: the
  non-destructive entry point. If `worktree_exists` is true it's a no-op and
  returns `Ok(false)`; otherwise it creates via the existing `worktree_add`
  (`-B` + `--force`, correct for first creation — `--force` still tolerates a
  stale on-disk dir that git no longer tracks) and returns `Ok(true)`.

`crates/otto-server/src/swarm_workspace.rs` — `ensure_cwd` now calls
`worktree_add_if_absent` instead of `worktree_add`. `base` (current HEAD) is
only consulted on first creation; on reuse it's ignored and the branch is left
exactly as the agent left it. The scratch fallback on error is preserved.

### Guard logic (exact)
- exists ⇒ reuse, branch untouched, prior commits intact (returns `false`).
- absent ⇒ create with `-B`/`--force` from base HEAD (returns `true`).

### Why this is the minimal correct fix
The only structural problem was the unconditional reset. First-creation
semantics (branch from current HEAD, `--force` to tolerate stale paths) are
exactly right and are preserved verbatim; we only added an exists-guard in
front of them.

### Test
`local.rs` → `worktree_add_if_absent_reuses_and_preserves_commits`: creates a
worktree (asserts created=true), commits a file on its branch, calls
`worktree_add_if_absent` again (asserts created=false), then asserts the branch
is unchanged, the commit SHA + subject survive, and the committed file still
exists. Plus `worktree_exists_tracks_registration` for the path-aware guard.

---

## D2 — Hand-added tasks ("Add task") defaulted to "backlog" and never ran

### Root cause (traced)
`SwarmService::create_task` (`otto-swarm/src/service.rs:243`) defaulted
`status` to `"backlog"`. `SwarmRepo::ready_tasks` (`otto-state/src/swarm.rs`)
only selects `status == "todo"`, and nothing promotes `backlog → todo`. So a
task added via the UI ("Add task" → HTTP `create_task` route at
`otto-swarm/src/http.rs:302`) sat in backlog forever and was never scheduled.

### Which fix I chose, and why
I changed the DEFAULT in `SwarmService::create_task` from `"backlog"` to
`"todo"` (one line), keeping `req.status.unwrap_or_else(...)` so an explicit
status is still honored.

I verified every other caller first:
- All planner/runtime paths construct `NewTask` directly in
  `swarm_runtime.rs` (lines 281, 347, 371, 704) and ALWAYS set an explicit
  `status` ("todo", etc.) — they never go through `SwarmService::create_task`,
  so they're unaffected.
- The ONLY path through `SwarmService::create_task` is the HTTP route used by
  the UI "Add task" button (`http.rs:302` → `service.rs:229`).

So flipping just the default makes manually-added tasks schedulable without
touching the planner, and without inventing a backlog→todo promotion step
(which would have been a larger, less local change with no other caller needing
it). A caller that explicitly wants to park a task in "backlog" still can.

### Tests
`otto-state/src/swarm.rs` (real in-memory SQLite + migrations, exercising the
actual `create_task` insert and `ready_tasks` SQL/logic):
- `ready_tasks_picks_up_todo_excludes_backlog`: a "todo" task IS returned by
  `ready_tasks`; an explicit "backlog" task is NOT — the exact D2 regression.
- `ready_tasks_respects_dependencies`: a "todo" task whose dependency isn't
  done is held back, then becomes ready once the dependency is "done" — guards
  that the default doesn't bypass the dependency gate.

---

## Verification

- `cargo check -p otto-server -p otto-swarm -p otto-state -p otto-git` — clean
  (only pre-existing dead-code warnings in `otto-usage`, which I don't own).
- `cargo test -p otto-git --lib local::` — 7 passed (2 new).
- `cargo test -p otto-state --lib swarm::tests` — 2 passed (both new).

No files outside my four were edited; no `cargo fmt`, no commit.
