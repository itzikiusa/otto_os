# Goal Loops

> _Expanded guide — Goal Loops is a recent addition (engine `otto-server::goal_loop`,
> UI `ui/src/modules/loops`, migration `0065`)._

> Give Otto a **goal**, a **budget** (max iterations + max active time), and a team of
> agents iterates toward it — **Plan → Execute → Evaluate → Digest**, carrying context
> forward — stopping the moment the goal's **machine-checked** acceptance criteria are met
> or a hard limit is hit. All work happens on an **isolated git branch**; your working
> tree is never touched.

This document is the end-user and operator reference for Goal Loops: the concepts, a
step-by-step how-to, a full walkthrough of every view and control, the controller engine,
the real REST/WS contract, capabilities and limits, security, and troubleshooting. It
describes what the code actually does — the engine in `crates/otto-server/src/goal_loop.rs`,
the worktree/parse helpers, the routes in `crates/otto-server/src/routes/goal_loops.rs`,
the domain types in `crates/otto-core/src/domain.rs`, and the UI in
`ui/src/modules/loops/`.

---

## 1. What it is

Goal Loops is the **"iterate until done"** construct. You define a goal with concrete,
ideally machine-checkable **acceptance criteria** and a **budget**; a per-loop controller
then runs bounded iterations — each a full **Plan → Execute → Evaluate → Digest** cycle —
on a dedicated `goal-loop/<id>` git branch, carrying a context digest forward, until the
evaluator's verdict is `achieved` **and every criterion is met** (command criteria
ground-truthed by exit code), or a hard limit (iterations / active time) is reached.

It composes existing Otto infrastructure rather than reinventing it:

- **Executors** run as live, openable agent **sessions** (the review engine's
  spawn/inject/watch plumbing — `agent_run` + `review_session`).
- The **planner, evaluator, digester, and goal-definer** are headless agent turns
  (`Orchestrator::run_agent`), each wrapped in a per-phase timeout.
- The **controller's** budget/lifecycle model mirrors Agent Swarm: a single background
  task that is the **sole writer** of the loop's runtime fields.

### Where it lives

| Layer | Location |
|-------|----------|
| Controller engine (lifecycle, phases, recovery) | `crates/otto-server/src/goal_loop.rs` |
| Isolated worktree provisioning / removal | `crates/otto-server/src/goal_loop_workspace.rs` |
| Tolerant JSON-object extraction (definer/evaluator/executor replies) | `crates/otto-server/src/goal_loop_parse.rs` |
| HTTP routes (define / CRUD / lifecycle / retry) | `crates/otto-server/src/routes/goal_loops.rs` |
| Domain types + default role prompts | `crates/otto-core/src/domain.rs` (`GoalLoop*`) |
| Request DTOs | `crates/otto-core/src/api.rs` (`DefineGoalReq`, `GoalLoopDraft`, `CreateGoalLoopReq`, `UpdateGoalLoopReq`) |
| Persistence (`goal_loops`, `goal_loop_iterations`) | `crates/otto-state` (`GoalLoopsRepo`); migration `0065_goal_loops.sql` |
| Boot crash-recovery | `crates/ottod/src/main.rs` (`goal_loops_repo.fail_running`) |
| UI section + views | `ui/src/modules/loops/` (`LoopsPage`, `GoalDefineForm`, `LoopDetail`, `IterationRow`) |
| Contracts (authoritative) | `docs/contracts/api.md` (#91–#101), `docs/contracts/ws.md` (`goal_loop_updated`) |

Open it from the app's **Goal Loops** section ("Loops" module).

---

## 2. Concepts

- **Goal definition** (`GoalLoopDefinition`) — a `title`, `summary`, `objectives`,
  `constraints`, `out_of_scope`, a human-facing `success_signal`, and the heart of it:
  **`acceptance_criteria`**. Each criterion (`AcceptanceCriterion`) has an `id`, a `text`,
  a `verify` string, a `verify_kind` (`"command"` | `"manual"`), and — for command
  criteria — a `verify_cmd` shell command whose **exit 0 means "met"**. The criteria are
  the loop's stop contract; the evaluator checks them every iteration.
- **The loop** (`GoalLoop`) — one row + `limits` + `config` + a live progress pointer
  (`status`, `phase`, `current_iteration`, `progress_pct`, `context_digest`). The
  controller is the **sole writer** of these runtime fields.
- **Iteration** (`GoalLoopIteration`) — one Plan → Execute → Evaluate → Digest cycle. It
  persists the `plan`, per-executor live state (`agents`), the `evaluation`, and the
  carried-forward digest (`context_in` / `context_out`).
- **Roles** — `planner`, one or more `executor`s, `evaluator`, `digester`, and a
  pre-launch `definer`. **Executors write code and commit**; the other four are headless
  single-turn agents.
- **Shared context — two layers.** The **git worktree** is the primary, lossless context
  (real files + commit history that each executor sees). A bounded **context digest** is
  secondary narrative memory. The planner *always* additionally receives the full criteria
  plus the last evaluation's unmet items uncompressed, so a lossy digest can't drop the
  goal or the open work.

### Status & phase vocabulary

`GoalLoopStatus` (the row's lifecycle): `draft` → `running` → (`paused` / `blocked` /
`succeeded` / `exhausted` / `failed` / `stopped`). Terminal = `succeeded`, `failed`,
`stopped`. `blocked` and `exhausted` are **resumable** (explicit Resume).

`GoalLoopPhase` (where the controller is *within* an iteration): `planning`, `executing`,
`evaluating`, `digesting`, plus `waiting` (an executor looks blocked on input) and `done`
(between/after iterations).

---

## 3. Prerequisites

1. **At least one agent CLI installed and logged in.** The default lineup runs everything
   on **claude** (planner/evaluator/definer on `sonnet`, digester on `haiku`, one
   executor on the provider default). Executors may be set to `claude` / `codex` / `agy`;
   an empty executor `provider` falls back to `claude`.
2. **A local git repository.** You supply an absolute `repo_path`. The loop never touches
   your checkout — it provisions its own worktree from the repo's HEAD (§7).
3. **Workspace role.** Launching/controlling a loop requires workspace **Editor**; viewing
   requires **Viewer** (§10). There is no separate feature-capability gate (the
   feature-capability axis is `Exempt`; access is by workspace role).
4. **(Optional) Usage budget.** Define, create-with-autostart, start, and resume are gated
   by the workspace usage budget — if enforcement is on and the cap is exceeded, the
   request is rejected before any work begins.

---

## 4. Walkthrough (step-by-step)

### Step 0 — Open Goal Loops

Go to the **Goal Loops** section. The list shows one card per loop with its name, a status
pill, a progress bar, `iter N/max`, the progress %, and (while running) the current phase.
Empty state: *"No goal loops yet."* with a **Define your first goal** button. Click **New
goal loop**.

### Step 1 — Define the goal (AI-assisted)

`GoalDefineForm` opens. Enter a **Repository path** (absolute) and a rough **Goal**
(e.g. *"Make the export endpoint stream instead of buffering, and add a test."*), then
click **Define with AI**.

- `POST /workspaces/{id}/goal-loops/define` runs the **definer** agent (a headless
  `run_agent` turn, model `sonnet`, in the repo for codebase context, 300 s outer timeout).
- It returns a structured, **editable** `GoalLoopDraft`: a `definition` (title, summary,
  objectives, acceptance criteria with `verify`/`verify_kind`/`verify_cmd`, constraints,
  out-of-scope), plus `suggested_limits` and `suggested_config`.
- To refine, type into **Refine: what to change about the draft** and re-run — the prior
  draft is echoed back as `context` and your `feedback` steers the rewrite.

This step **persists nothing**; it only drafts. (You can also skip the AI and hand-author
the criteria.)

### Step 2 — Edit criteria & set the budget

The form lets you edit the **Name**, add/remove **Acceptance criteria** (each: description,
a `manual`/`command` kind selector, and either a shell `verify_cmd` — *"exit 0 = met"*, e.g.
`cargo test` — or a free-text *how to verify*; command criteria also keep a human-readable
"what this checks"), and the **Budget**:

| Field | Maps to | Form default |
|---|---|---|
| Max iterations | `limits.max_iterations` | 5 |
| Max minutes | `limits.max_runtime_secs` (×60) | 30 |
| Per-phase minutes | `limits.per_phase_timeout_secs` (×60) | 10 |
| Executors | duplicates the suggested executor cfg N× (1–6) | 1 |

> **Launch is blocked** until there is at least one criterion and every criterion has a
> non-empty description **and** verify. (The server re-validates: ≥1 criterion, every
> `verify` non-empty, every `command` criterion has a `verify_cmd`, and ≥1 executor.)

### Step 3 — Launch

Click **Launch loop**. The UI calls `POST /workspaces/{id}/goal-loops` with
`autostart: true`, which creates the loop **and** spawns the controller. A fresh worktree +
branch `goal-loop/<id>` is provisioned from the repo's current HEAD (captured as the diff
base). You're navigated to the loop's detail view.

> You can also create a loop **without** autostart (a `draft`) and start it later with
> `POST /goal-loops/{id}/start`. While a loop is a draft, its `config` (executor lineup) is
> still editable; once running, only `name` and `limits` (the latter only while paused/
> blocked/exhausted) can change.

### Step 4 — Monitor

`LoopDetail` shows, live (driven by `goal_loop_updated` WS events plus a low-frequency
fallback poll):

- A **progress bar** (the evaluator's latest `progress_pct`).
- A **Plan → Execute → Evaluate → Digest stepper** highlighting the active phase (a
  `waiting` executor renders under *Executing*).
- A **meta line**: `iteration N/max`, active-time `Xm / Ym`, `% complete`, and the branch
  name.
- The **Goal** with its acceptance criteria.
- An expandable **Iterations** timeline (`IterationRow`, newest open): the **Plan** text,
  the **Executors** (status dot + name + note/summary, an **Open** button for the live
  session, a **Retry** button when applicable), the **Evaluation** (per-criterion ✓/○
  chips with evidence on hover + feedback), and the **Context carried forward**.

A `waiting` executor (blocked on input) surfaces fast — **Open** its live session and type
to unblock it.

### Step 5 — Control

From the header: **Pause** / **Stop** while running; **Resume** / **Stop** / **Delete**
while paused/blocked/exhausted; **Delete** in terminal states. (See §6 for exact semantics.)

### Step 6 — Finish

The loop ends in one of:

| Status | Meaning |
|---|---|
| `succeeded` | All criteria met (command criteria proven by exit 0) and verdict `achieved`. |
| `exhausted` | A hard limit (iterations / active time / cost / 4 h backstop) was hit first. **Resumable.** |
| `blocked` | The evaluator decided progress needs you. Waits for explicit Resume (no auto-continue). **Resumable.** |
| `failed` | An internal error (e.g. worktree provisioning, iteration bookkeeping). |
| `stopped` | You stopped it. |

The result lives on the `goal-loop/<id>` branch; **you** decide next steps (open the diff,
hand off to a session). Otto never auto-commits to your branch, merges, pushes, or opens a
PR.

---

## 5. The controller engine (how iterations actually run)

`start_loop` provisions the worktree, marks the loop `running`, registers a control handle
(cancelling any prior controller for the same loop), and spawns the `controller` task. Each
iteration of the controller loop:

1. **Cancel / pause checks.** A cancel flag finalizes the loop as `stopped` (only if this
   handle is still the registered controller — a rapid restart that superseded it must not
   tear down the new run). A pause flag idles in 500 ms slices.
2. **Hard-limit gate (before creating an iteration).** Checks `iterations_started >=
   max_iterations`, active time `>= max_runtime_secs`, accumulated `cost_usd >=
   max_cost_usd`, and the **4-hour controller-lifetime backstop** — any hit → `exhausted`.
3. **New iteration.** Bumps the immutable `iterations_started` counter (the cap basis),
   creates a `GoalLoopIteration` seeded with the current context digest and executor lineup.
4. **PLAN.** Runs the **planner** (headless, in the worktree) with the goal context, the
   running digest, and the previous evaluation's unmet criteria + feedback. If the planner
   turn fails, a sentinel plan is used so executors still work directly toward the criteria.
5. **EXECUTE (sequential).** For each executor in order: writes its prompt to a temp file,
   spawns a real **agent session** (`SessionKind::Agent`, tagged `meta.source="goal_loop"`,
   cwd = the worktree), pastes the prompt, and watches for its result file with bounded
   auto-recovery (`max_attempts_per_executor`, default 3). The executor is told to do the
   work, **`git add -A && git commit`**, then write a `{summary, changed_files, notes,
   blockers}` JSON object to its out-file as the last thing it does.
6. **EVALUATE.** Runs the **evaluator** (headless), then **ground-truths** every
   `verify_kind:"command"` criterion by actually running its `verify_cmd` in the worktree
   (`proof::run_command`); exit 0 sets `met=true`, overriding the model. Any criterion the
   model omitted is added as unmet. An over-optimistic `achieved` with any unmet criterion
   is **coerced to `continue`**. `progress_pct` is stored.
7. **DIGEST.** Runs the **digester** (headless, model `haiku`) to compress prior context +
   this iteration into a concise running summary, saved as the loop's new `context_digest`
   and the iteration's `context_out`.
8. **DECISION.**
   - `verdict == "achieved"` **and all criteria met** → assemble a **proof pack** (§5.1),
     finalize `succeeded`.
   - `verdict == "blocked"` → bank the active window, set `blocked`, raise a notice; **no
     auto-continue**.
   - else (`continue` / anything) → next iteration.

### 5.1 Proof packs ("no done without evidence")

When an iteration would finalize `succeeded`, the controller assembles a **proof pack** for
the loop (`WorkItemKind::GoalLoop`): each verify command becomes a `command` test artifact
(with exit code + duration), the worktree diff vs the loop's base commit becomes a `diff`
artifact, the evaluator's feedback/rationale/per-criterion evidence becomes a `self_review`,
and the acceptance criteria become a `review` summary. The pack's recomputed status is
appended to the success summary (*"Proof: passed."* / *"partial (no machine-checked
test)."*).

**Opt-in teeth.** With `OTTO_PROOF_REQUIRE_GOAL_LOOP=1` (or `true`), the loop **refuses** to
finalize `achieved` without a passing machine-checked test — it injects feedback and
continues gathering proof — *unless* this is the last allowed iteration (then it accepts the
partial pack). This is bounded by the iteration cap, so it can never spin forever. Default
off; the pack is always assembled regardless.

### 5.2 Why the stop condition is trustworthy

A loop stops `succeeded` only when the evaluator's `verdict == "achieved"` **and every**
acceptance criterion is met. For `verify_kind:"command"` criteria, "met" is decided by
**running `verify_cmd` and checking exit 0**, overriding the model's opinion entirely. The
self-grading verdict can only ever *tighten* (an `achieved` with unmet criteria becomes
`continue`), never wave work through.

---

## 6. Lifecycle, limits & bounding

### Lifecycle actions

- **Start** (`/start`, draft only) — provision worktree, mark running, spawn controller.
  Use **Resume** to continue an existing loop.
- **Pause** (`/pause`, running only) — **banks** the current active window into
  `elapsed_secs` (so the time budget can't be refunded), clears the run anchor, sets
  `paused`, and flags the controller to idle. In-flight phases finish their slice.
- **Resume** (`/resume`, paused/blocked/exhausted) — re-spawns a controller (re-anchors the
  active-time clock; reuses the existing worktree and its commits). Usage-budget gated.
- **Stop** (`/stop`) — cancel the controller (it finalizes `stopped`) or finalize directly
  when none is live; **kills executor sessions** and **removes the worktree** (keeps the
  branch).
- **Delete** (`DELETE`) — stops, removes the worktree, deletes the row. **Keeps the
  branch** so the diff survives.
- **Retry executor** (`/iterations/{idx}/agents/{agent}/retry`) — re-run one executor from
  its persisted prompt. **Only allowed while the loop is `blocked`** (a running loop's
  controller may be actively running that slot; a second run would duplicate the session and
  race the agents-state write).

### Hard limits (enforced at phase boundaries)

| Limit (`GoalLoopLimits`) | Meaning | Default |
|---|---|---|
| `max_iterations` | Gated on the immutable `iterations_started` counter **before** each iteration is created (so it runs ≤ N) | 5 |
| `max_runtime_secs` | **Active** time (`elapsed_secs` + the current open window). Pausing **banks** the window and does not refund it | 1800 (30 m) |
| `per_phase_timeout_secs` | Wraps every agent turn (each headless role and each executor watch loop), so no single turn can wedge the loop | 600 (10 m) |
| `max_attempts_per_executor` | Bounded auto-recovery per executor before it's recorded as errored | 3 |
| `max_cost_usd` | **Advisory** only — cost lands late, so one iteration may overshoot | `null` (unset) |
| controller lifetime | Absolute backstop regardless of config | **4 h** |

Internal tuning constants (in `goal_loop.rs`): role-turn stuck window `240 s`, executor
`waiting` idle `45 s`, executor `stuck` idle `180 s`, executor retry backoff `3 s`,
cooperative-cancel slice `500 ms`.

---

## 7. Isolated worktree & branch (your checkout is never touched)

Every loop runs on its own branch **`goal-loop/<id>`** in a dedicated worktree under the
daemon data dir (`<data>/goal-loops/<id>/work`) — **never** the user's checkout. Because the
branch name is ULID-unique, provisioning is a **fresh create**, not the destructive
`-B --force` reuse the swarm uses for multi-turn agents:

- **Fresh launch** — captures the repo's HEAD as the diff `base_commit`, then
  `git worktree add` on the new branch.
- **Resume with a still-registered worktree** — reuses it as-is (the loop's prior commits
  are preserved).
- **Resume where the worktree was removed but the branch exists** — **re-attaches**
  non-destructively (never `-B`-resets, which would discard accumulated commits).
- **A pre-existing path with no record** — **fails loudly** rather than clobber a foreign or
  stale tree.

Executors run **sequentially** on this single worktree (each commits; the next sees it), so
there are no git-index races. `remove_worktree` (on finalize for truly-terminal states,
delete, and boot cleanup) removes the worktree but **keeps the branch**, so the diff
survives. (`exhausted` keeps its worktree — it is resumable, and recreating it would reset
the branch and destroy commits.)

---

## 8. API & WS surface

All routes under `/api/v1`; JSON snake_case; item routes resolve the workspace from the loop
row; every handler enforces ws Viewer/Editor. Authoritative: `docs/contracts/api.md`
(#91–#101) and `docs/contracts/ws.md` (Goal-loop progress).

| # | Method & path | Auth | Request → Response |
|---|---|---|---|
| 91 | `POST /workspaces/{id}/goal-loops/define` | ws editor | `DefineGoalReq` → `GoalLoopDraft` (runs the AI definer; persists nothing; `feedback` refines) |
| 92 | `GET /workspaces/{id}/goal-loops` | ws viewer | → `GoalLoop[]` |
| 93 | `POST /workspaces/{id}/goal-loops` | ws editor | `CreateGoalLoopReq` → `GoalLoop` (validates criteria/verify; starts when `autostart`) |
| 94 | `GET /goal-loops/{id}` | ws viewer | → `GoalLoopDetail` (`{loop, iterations}`) |
| 95 | `PATCH /goal-loops/{id}` | ws editor | `UpdateGoalLoopReq` → `GoalLoop` (`name` non-terminal; `limits` not while running; `config` draft-only) |
| 96 | `POST /goal-loops/{id}/start` | ws editor | — → `GoalLoop` (draft only) |
| 97 | `POST /goal-loops/{id}/pause` | ws editor | — → `GoalLoop` (running only) |
| 98 | `POST /goal-loops/{id}/resume` | ws editor | — → `GoalLoop` (paused/blocked/exhausted) |
| 99 | `POST /goal-loops/{id}/stop` | ws editor | — → `GoalLoop` |
| 100 | `POST /goal-loops/{id}/iterations/{idx}/agents/{agent}/retry` | ws editor | — → 202 (re-run a stuck executor; blocked only) |
| 101 | `DELETE /goal-loops/{id}` | ws editor | — → 204 (stops + removes worktree; **keeps the branch**) |

### Key DTOs

- **`DefineGoalReq`** — `{ seed, repo_path, context?, feedback? }`.
- **`GoalLoopDraft`** — `{ definition, suggested_limits, suggested_config }`.
- **`CreateGoalLoopReq`** — `{ name, repo_path, definition, limits, config, autostart? }`.
- **`UpdateGoalLoopReq`** — `{ name?, limits?, config? }`.
- **`GoalLoopDetail`** — `{ loop, iterations[] }` (the `loop` field is JSON-renamed from
  `loop_`).

### WebSocket event (`/ws/events`, workspace-scoped)

`goal_loop_updated` fires on every transition — status change, phase change, a new
iteration, after each evaluation, and when an executor's live state flips (e.g. →
`waiting`):

```json
{
  "type": "goal_loop_updated",
  "workspace_id": "<Id>",
  "loop_id": "<loop_id>",
  "status": "draft|running|paused|blocked|succeeded|exhausted|failed|stopped",
  "phase": "planning|executing|evaluating|digesting|waiting|done",
  "current_iteration": 0,
  "progress_pct": 0
}
```

The Loops UI updates the row directly from these fields and re-fetches
`GET /goal-loops/{id}`. Executor sessions are ordinary agent sessions and also emit the
normal `session_status` events, so their live status dots work without extra plumbing.

---

## 9. Capabilities & limitations

**You can:**

- AI-draft a structured, machine-checkable goal from a rough sentence, then edit and refine
  it before launching.
- Run a bounded loop of Plan → Execute → Evaluate → Digest iterations on an **isolated
  branch**, with a budget (iterations + active time + per-phase timeout + per-executor
  attempts).
- Define **command** acceptance criteria that are **ground-truthed by exit code**, so a
  "done" verdict is backed by a passing test rather than the model's opinion.
- Run **multiple executors** sequentially (1–6) on one worktree.
- **Open any executor as a live terminal session**, type to unblock a `waiting` one, and
  **Retry** a stuck/errored executor (while blocked).
- **Pause / resume / stop** the loop; **resume** an `exhausted` loop after raising limits.
- Get an automatic **proof pack** (diff + verify-command runs + self-review + criteria) on
  success, with optional hard enforcement (`OTTO_PROOF_REQUIRE_GOAL_LOOP`).

**You cannot (today) — deferred:**

- **Concurrent executors** with a worktree-per-executor + integrator merge. v1 runs
  executors **sequentially** on one worktree (safe, no git-index races).
- **Resume after a daemon restart.** On boot, loops left running/paused/blocked are **failed**
  (worktrees removed, executor sessions killed) — start them again to continue.
- **Scheduled/recurring loops** and **auto-commit / auto-merge / auto-PR** (always explicit,
  by design — Otto never publishes the result for you).
- A **`repo` cwd-mode** escape hatch (always an isolated worktree).
- A **hard cost stop** — `max_cost_usd` is advisory (cost lands late; one iteration may
  overshoot the cap).
- A dedicated **`GoalLoops` RBAC capability** — v1 gates by workspace role only (the
  feature-capability axis is `Exempt`).

---

## 10. Security & permissions

- **RBAC.** Launch and all control (define, create, start/pause/resume/stop, patch, retry,
  delete) require workspace **Editor**; list/detail require **Viewer** (checked per handler
  against the loop's `workspace_id`). See `docs/MULTI-USER-RBAC.md`.
- **Isolation.** Executors run only on the **fresh** `goal-loop/<id>` worktree (launch fails
  loudly rather than reuse a foreign/stale path) — never your checkout. Executor sessions are
  tagged `meta.source="goal_loop"` + `loop_id` so cleanup kills exactly the loop's sessions.
- **No outward/irreversible actions.** The engine never pushes, force-pushes, merges, opens
  PRs, or posts to Jira/Slack/Telegram. `DELETE` removes the worktree and row but **keeps the
  branch**.
- **Cooperative stop.** **Stop** is cooperative at phase boundaries and kills executor
  sessions immediately. **Blocked** does not auto-continue (no livelock) — it waits for your
  Resume.
- **Crash recovery.** On daemon boot, loops left running/paused/blocked are failed
  (*"Interrupted by a daemon restart — start it again to continue."*), their worktrees
  removed, and any `goal_loop` executor sessions killed — nothing dangles.
- **Budget gate.** Define, create-with-autostart, start, and resume are gated by the
  workspace usage budget.
- **Loopback by default.** The daemon listens on `127.0.0.1:7700` unless you explicitly
  enable a network listener; the loop engine doesn't change that.

---

## 11. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| **Define returns an error / no draft** | The definer didn't emit a parseable JSON object. Re-run, optionally with clearer `feedback`. |
| **Launch button stays disabled** | Need ≥1 acceptance criterion, each with a non-empty description **and** verify. Command criteria need a `verify_cmd`. |
| **Loop ends `exhausted` with progress < 100** | A hard limit was hit. Raise `max_iterations` / `max_runtime_secs` via `PATCH` (while paused/blocked/exhausted) and **Resume**. |
| **Executor stuck `waiting`** | It looks blocked on input. **Open** its session and respond, or — if the loop went `blocked` — **Retry** the executor. |
| **`blocked`** | The evaluator decided progress needs you (no auto-continue). Address the feedback note, then **Resume**. |
| **Nothing happens after launch** | Check the daemon log for worktree provisioning errors (e.g. bad `repo_path`, or a pre-existing unregistered worktree path). The loop fails fast and surfaces the error on the detail view. |
| **Loop `failed` after a daemon restart** | Expected — interrupted loops are failed on boot (resume-after-restart is deferred). Start it again. |
| **`succeeded` but a criterion looks wrong** | Manual criteria rely on the evaluator's judgement; prefer `command` criteria with a `verify_cmd` for ground truth. Enable `OTTO_PROOF_REQUIRE_GOAL_LOOP` to require a passing test before "done". |
| **`402` / budget-exceeded on start** | Workspace usage cap reached; raise/adjust the budget. |

---

## 12. Related docs

- [`./agent-sessions.md`](./agent-sessions.md) — how the live executor sessions work
  (opening, attaching, the terminal).
- [`./code-review.md`](./code-review.md) — the multi-agent review engine whose spawn/watch
  plumbing the executors reuse.
- [`./agent-swarm.md`](./agent-swarm.md) — the swarm coordinator whose budget/lifecycle model
  the loop controller mirrors.
- [`./mission-control.md`](./mission-control.md) — Goal Loops appear as `goal_loop` work
  items in the unified work graph.
- `docs/contracts/api.md` (#91–#101) and `docs/contracts/ws.md` (Goal-loop progress) — the
  authoritative API/event contract.
