# Goal Loops

> Give Otto a **goal**, a **budget** (max iterations + max active time), and a team of
> agents iterates toward it — **Plan → Execute → Evaluate → Digest**, carrying context
> forward — stopping the moment the goal's **machine-checked** acceptance criteria are met
> or a hard limit is hit. All work happens on an **isolated git branch**; your working tree
> is never touched.

This is the "iterate until done" construct. It composes the existing infra: executors run
as live, openable sessions (the review engine's spawn/watch plumbing); the planner,
evaluator, digester, and goal-definer are headless agent turns (the orchestrator); the
controller's budget/lifecycle mirrors the Agent Swarm.

## Concepts

- **Goal definition** — title, objectives, and **acceptance criteria**, each with a
  `verify` (ideally a `command` whose exit-0 means "met"). The criteria are the loop's
  stop contract; the evaluator checks them every iteration.
- **The loop** — one `GoalLoop` row + config + a live progress pointer (status, phase,
  iteration, progress %). The controller is the **sole writer** of these runtime fields.
- **Iteration** — one Plan → Execute → Evaluate → Digest cycle (`GoalLoopIteration`),
  persisting the plan, per-executor live state, evaluation, and the carried-forward digest.
- **Roles** — `planner`, `executor(s)`, `evaluator`, `digester` (and a pre-launch
  `definer`). Executors **write code and commit**; the other roles are headless.
- **Shared context** — two layers: the **git worktree** (primary, lossless: real files +
  commit history) and a bounded **context digest** (secondary narrative memory). The planner
  always also receives the full criteria + the last evaluation's unmet items uncompressed,
  so a lossy digest can't drop the goal or the open work.

## Walkthrough

1. **Define** (`POST /workspaces/{id}/goal-loops/define`) — type a rough goal + repo path;
   the AI definer returns a structured, editable draft (objectives, acceptance criteria with
   `verify`, suggested limits/lineup). Re-run with `feedback` to refine.
2. **Launch** (`POST /workspaces/{id}/goal-loops` with `autostart`) — creates the loop and
   spawns the controller. A fresh worktree + branch `goal-loop/<id>` is provisioned from the
   repo's HEAD at launch.
3. **Monitor** — the Loops page shows status, a Plan→Execute→Evaluate→Digest **stepper**,
   iteration N/max, active time vs cap, an evaluator progress bar, and an expandable
   iteration timeline. A `waiting` executor (blocked on input) surfaces fast; **Open** its
   live session and type to unblock, or **Retry** it.
4. **Control** — Pause / Resume / Stop at any time.
5. **Finish** — the loop ends `Succeeded` (all criteria met, command-criteria ground-truthed
   by exit code), `Exhausted` (budget hit), `Blocked` (needs you — explicit Resume to
   continue), `Failed`, or `Stopped`. The result lives on the branch; **you** decide next
   steps (open the diff, hand off to a session). Otto never auto-commits to your branch,
   merges, pushes, or opens a PR.

## Stop condition (why it's trustworthy)

A loop stops `Succeeded` only when the evaluator's `verdict == "achieved"` **and every**
acceptance criterion is met. For `verify_kind: "command"` criteria, "met" is decided by
actually **running `verify_cmd` in the worktree** (exit 0), overriding the model's opinion.
An over-optimistic `achieved` with any unmet criterion is coerced to `continue`.

## Hard limits & bounding

- `max_iterations` — gated on an immutable `iterations_started` counter, **before** each
  iteration is created (so it runs ≤ N).
- `max_runtime_secs` — **active** time (`elapsed_secs` + the current window). Pausing banks
  the window and **does not refund** it. Checked at each phase boundary.
- `per_phase_timeout_secs` — wraps every agent turn (executor watch loop and each headless
  role), so no single turn can wedge the loop.
- `max_attempts_per_executor` — bounded auto-recovery per executor (default 3).
- A 4-hour controller-lifetime backstop applies regardless of config.
- `max_cost_usd` — **advisory** only (cost lands late; one iteration may overshoot).

## Safety

- Executors run on the **isolated** `goal-loop/<id>` worktree (fresh-created; the launch
  fails loudly rather than reuse a foreign/stale path) — never your checkout.
- No outward/irreversible actions: never push, force-push, merge, open PRs, or post to
  Jira/Slack/Telegram. `DELETE` removes the worktree and the row but **keeps the branch**.
- **Stop** is cooperative at phase boundaries and kills executor sessions immediately.
  **Blocked** does not auto-continue (no livelock) — it waits for your Resume.
- **Crash recovery**: on daemon boot, loops left running/paused/blocked are failed, their
  worktrees removed, and any `goal_loop` executor sessions killed — nothing dangles.
- RBAC: launch/control = ws **Editor**; view = ws **Viewer** (enforced per handler).

## API & WS surface

- REST: see `docs/contracts/api.md` → **Goal Loops** (endpoints #91–#101).
- WS: `goal_loop_updated` — see `docs/contracts/ws.md` → **Goal-loop progress**.
- Persistence: `goal_loops` + `goal_loop_iterations` (migration
  `crates/otto-state/migrations/0065_goal_loops.sql`); repo `otto_state::GoalLoopsRepo`.
- Engine: `crates/otto-server/src/goal_loop.rs` (controller), `goal_loop_workspace.rs`
  (worktree), `goal_loop_parse.rs` (tolerant JSON-object extraction). Routes:
  `crates/otto-server/src/routes/goal_loops.rs`. UI: `ui/src/modules/loops/`.

## Limits / not in v1 (deferred)

- **Concurrent** executors (own worktree per executor + integrator merge). v1 runs executors
  **sequentially** on one worktree (each commits; the next sees it) — safe, no git-index races.
- Resume-after-daemon-restart (v1 fails interrupted loops and cleans up).
- Scheduled/recurring loops; auto-PR/auto-commit (always explicit, by design); a `repo`
  cwd-mode escape hatch (always worktree); a hard cost stop (advisory only).
- A dedicated `GoalLoops` RBAC capability (v1 gates by ws-role; the feature-capability axis
  is `Exempt`).

## Troubleshooting

- **Define returns an error / no draft** — the agent didn't emit a parseable JSON object;
  re-run (optionally with clearer `feedback`).
- **Loop ends `Exhausted` with progress < 100** — raise `max_iterations` / `max_runtime_secs`
  via `PATCH` (while Paused/Blocked/Exhausted) and **Resume**.
- **Executor stuck `waiting`** — Open its session and respond, or **Retry** the executor.
- **`Blocked`** — the evaluator decided progress needs you; address the note, then **Resume**.
- **Nothing happens after launch** — check the daemon log for worktree provisioning errors
  (e.g. bad `repo_path`); the loop fails fast and surfaces the error on the detail view.
