# Agent Swarm Enhancements — Design (2026-06-26)

Status: **design** (branch `worktree-swarm-enhance`). Grounded in the current code;
every integration point cites a file. Four requirements, each mapped to a concrete,
mostly-additive change that reuses machinery already in the tree.

## 0. Requirements → solution map

| # | Requirement | Core change | Reuses |
|---|---|---|---|
| 1 | Several agents on one repo MUST use worktrees, coordinated by the leader, merged properly; notify the feed on worktree creation + on shared functionality | Make worktree the **default** for code projects; surface the `created` flag from `ensure_cwd` → post a feed event; per-branch changed-file overlap detector → feed warning; **leader merge-back** on goal pass | `worktree_add_if_absent`, `merge_preview`/`merge_branch` (otto-git), board feed |
| 2 | Specific skill set per agent **team** and per **project**, layered on the agent default | `skills_json` on swarm config (team) + `swarm_projects` (project); union at `provision_agent` | `otto_context::materialize::provision`, existing per-agent `skills_json` |
| 3 | Goals per task; leader plans + assigns; after dev done, leader verifies **each goal separately, one per iteration**; on fail → fix-request to dev with all issues, wait, **re-verify more thoroughly**; user-defined **max retries** → "could not be achieved"; leader decides **blocker vs warning**, focuses on important things not nits | New `swarm_goals` table + standing-goals; a **verification controller** (pure engine + injected ops) modeled on `goal_loop`; sequential per-goal loop with severity verdict, escalating scrutiny, bounded retries, merge on success | `goal_loop.rs` pattern, `run_swarm_agent`, `run_verify_cmd`, `merge_branch` |
| 4 | Launch the team via Slack message / webhook (channels are workspace-level today) | Webhook already exists (`/webhooks/swarm/{ws}/{sid}`) — extend with goals + origin; add `swarm_channel_triggers` + a `SwarmTrigger` hook in the channel `Bridge`; reply/escalate back to the originating chat | existing swarm webhook, `Bridge`, `improve_notify` send path |

Design principle: **extend, don't rebuild.** The swarm already has worktree mode,
per-agent skills, a retry counter, a board feed, an openable-agent-turn primitive,
and a webhook launcher. The work is wiring these into the four behaviours, plus one
genuinely new subsystem (the goal-verification controller).

---

## 1. Worktree isolation, feed notifications, leader merge

### 1.1 Worktree is the default for code projects

`swarm_workspace.rs::cwd_mode()` today returns `"repo"` (a **shared** checkout — all
agents in one working tree, racing the index) whenever the project has a `repo_path`
and no explicit per-agent mode. That is the clobbering bug.

Change `cwd_mode()`:

- When `has_repo` and no per-agent override: return `"worktree"` (was `"repo"`).
- An explicit **per-agent** `cwd_mode: "repo"` still opts a *single* agent out (a
  deliberately single-agent project), and `"scratch"` for non-code roles. There is **no
  global "disable isolation" switch** — the requirement says agents sharing a repo MUST
  use worktrees, so we don't offer a knob that re-introduces the clobbering (review Gap D).
- Bonus (confirmed by review): `"repo"` mode today makes `provision_agent` write
  `CLAUDE.md`/`AGENTS.md`/`.claude/skills` into the user's **real** repo
  (`swarm_workspace.rs:305-322`). Worktree-default removes that hazard too.

**Pinned base ref (review M3).** `ensure_cwd` currently bases each worktree on the live
`git.current_branch()` *at creation time* (`swarm_workspace.rs:112-115`), so two agents
can base on different HEADs if the operator switches branches. Instead, capture a single
pinned **integration branch** per project, once, and base every agent worktree on it:

- `integration_branch` = a dedicated swarm branch `swarm/<short_swarm>/<short_project>/int`,
  created once from the repo's HEAD at first worktree creation, persisted on
  `swarm_projects.integration_branch` (new column).
- Agent worktrees (`swarm/<s>/<a>`) are based on `integration_branch` — one pinned base,
  so merges compose. This is the "all is handled and merged properly" backbone.

### 1.2 Notify the feed when a worktree is created

`ensure_cwd` already learns `created: bool` from `worktree_add_if_absent` but discards
it. Refactor to surface it:

```rust
pub struct CwdInfo { pub path: String, pub mode: String,
                     pub created: bool, pub branch: Option<String>, pub base: Option<String> }
pub async fn ensure_cwd_info(ctx, swarm, agent, project) -> Result<CwdInfo>
```

Keep `ensure_cwd` as a thin wrapper returning `.path` (other callers unaffected).
In `swarm_run.rs` (which has `swarm`, `agent`, `task`, `project` in scope at the call
site, ~line 251), when `info.created`, post a board message:

- `kind: "worktree"` (new `SwarmMessage` kind), author = the agent,
- body: `🌿 {agent.name} created worktree \`{branch}\` (base \`{short_base}\`) for "{task.title}"`,
- `meta`: `{ event:"worktree_created", agent_id, task_id, branch, path, base }`.

The feed (`BoardFeed.svelte`) renders the new kind with a worktree chip; `RunInspector`
already shows the cwd.

### 1.3 Shared-functionality notification

After each agent turn persists, compute the files that agent's branch changed vs its
base: `git diff --name-only <base>...HEAD` in the worktree (otto-git helper
`changed_files(base) -> Vec<String>`; thin wrapper over the existing diff plumbing).

Maintain an in-memory map per swarm: `branch -> {agent_id, files, task_id}` (a
`Mutex<HashMap<...>>` keyed by swarm id, like the `swarm_agent_run::REGISTRY`). On each
turn-end:

1. Update this agent's entry.
2. For every **other active** agent entry, intersect file sets. If the intersection is
   non-empty and not already announced (dedupe by a hash of `(branchA, branchB, files)`),
   post `kind: "shared"` (new kind):
   - body: `⚠️ {agentA} and {agentB} both modified {N} shared file(s): \`a.ts\`, \`b.ts\` — coordinate before merge.`
   - `meta`: `{ event:"shared_files", agents:[a,b], files, tasks:[..] }`.

This is the "if some shared functionality is there, also notify" requirement — concrete
and best-effort. The leader sees it on the board and can sequence merges or reassign.
Agents are *also* told (in identity md) to `otto-post --kind concern` when they
knowingly touch shared code; the detector is the automatic backstop.

### 1.4 Leader merge-back — in a dedicated integration worktree (NEVER the user's repo)

**Critical safety constraint (review M3 / AGENTS.md "Do NOT damage user work").**
`merge_branch` checks out `target` and (with `auto_stash`) stashes uncommitted changes in
the `LocalGit`'s directory (`local.rs:987-1001`). It must therefore **never** run against
the user's primary checkout — that would `git checkout`/stash the user's live working
tree in the background. Instead:

- Otto keeps a dedicated **integration worktree** at
  `<data_dir>/swarm/<swarm_id>/<project_id>/_integration/wt`, checked out on
  `integration_branch`. **All** swarm merges happen here. Otto never touches the user's
  primary working tree or their real branches.
- Because `integration_branch` is a *dedicated swarm branch* (not the user's `main`),
  there is no "branch already checked out elsewhere" conflict, and the integration
  worktree stays clean (so `auto_stash:false` — nothing to stash).

Merge happens **only after a task's goals are resolved with no blocking unmet** (§3),
run by the verification controller as the leader:

1. Verification diff for a task is `git diff integration_branch...agent_branch` — exactly
   what *this* task added beyond what's already integrated (review B1; lets a per-agent
   branch be merged incrementally, one task at a time).
2. Under a **per-`integration_branch` async mutex** (serializes concurrent merges —
   review M4): `merge_preview(agent_branch → integration_branch)` in the integration
   worktree.
3. Clean → `merge_branch(MergeCommit, auto_stash:false)` → post `kind:"merge"`
   "✅ merged \`{agent_branch}\` → \`{integration_branch}\`"; mark task `done`.
4. Conflict → post `kind:"concern"` "❌ merge conflict on {files}"; create a follow-up
   `fix` task (assignee = the dev) to resolve; leave task `blocked`. **Never auto-resolve.**
5. **Post-merge integration gate (review M4):** re-run the task's *deterministic* goals
   (those with a `verify_cmd`) against `integration_branch` in the integration worktree;
   on a regression (a goal that passed per-branch now fails combined) post a concern +
   create a fix task. (Full LLM re-verify of quality goals is skipped for cost; per-branch
   verification ≠ integration verification, and this is the cheap, deterministic gate.)

**Hand-off, not hijack.** When every task in a project is done+merged, Otto posts the
`integration_branch` name to the feed (and to the originating channel, §4.3). The user
then reviews that branch and merges it into their real branch via the normal Git UI —
Otto does **not** auto-merge into the user's working branch.

---

## 2. Skill sets per team and per project

Per-agent skills already exist (`swarm_agents.skills_json`, resolved by `skill_names`
→ `provision_agent` → `materialize::provision`). Add two more layers:

- **Team (swarm):** `skills` array in `Swarm.config` JSON (no migration).
- **Project:** `skills_json` column on `swarm_projects` (migration; same
  `[{name, must_use?}]` shape).

Resolution change in `swarm_workspace.rs`:

```rust
fn resolve_skills(swarm, project, agent) -> Vec<{name, must_use}>  // union, dedupe by name, must_use = OR
pub fn provision_agent(ctx, swarm, project, agent, identity_md, cwd)  // +swarm,+project
// cfg.skills = Some(resolve_skills(...).names)
```

`provision_agent`'s caller in `swarm_run.rs` has `swarm`/`project` in scope already, so
the signature change is local. `must_use` skills (from any layer) are rendered in the
identity md as before. The effective set is also written to the agent identity so the
agent knows which extra skills the project expects.

UI:

- **Team skills:** a "Skills" section in the swarm settings (new lightweight swarm
  settings modal opened from the `SwarmPage` header gear). Multi-select chips populated
  from `GET /library/skills`.
- **Project skills:** a skills multi-select in the project create/edit modal
  (`SwarmPage` project modal) and shown on the Kanban goal bar.

Standing example from the prompt: create a project "auto-test", attach the
`bo-autotest-guide` / `webapp-testing` skills → every agent working that project gets
them on top of their defaults, materialized into `.claude/skills`.

---

## 3. Goals + leader verification loop (the centerpiece)

### 3.1 Data model — `swarm_goals` (migration)

```
swarm_goals(
  id TEXT PK,
  swarm_id, workspace_id TEXT NOT NULL,
  project_id TEXT,            -- project-scoped goal (NULL ok)
  task_id TEXT,               -- task-scoped goal (NULL ok)
  kind TEXT NOT NULL,         -- 'explicit' | 'standing'
  title TEXT NOT NULL,        -- "Use Playwright instead of Selenium"
  description TEXT NOT NULL DEFAULT '',
  metric TEXT,                -- optional, e.g. 'runtime_seconds'
  comparator TEXT,            -- 'lte'|'gte'|'eq'|'contains'|'absent'
  target_value REAL,          -- pass threshold (e.g. 120 = under 2 min)
  block_value REAL,           -- > this (for lte) ⇒ blocker; between target..block ⇒ warning
  verify_cmd TEXT,            -- optional ground-truth command (run in the worktree)
  max_retries INTEGER NOT NULL DEFAULT 3,
  blocking INTEGER NOT NULL DEFAULT 1,   -- standing goals can be advisory (0)
  status TEXT NOT NULL DEFAULT 'pending',-- pending|verifying|passed|warned|unmet|skipped
  verdict_json TEXT,          -- last full verdict
  iterations INTEGER NOT NULL DEFAULT 0,
  order_idx INTEGER NOT NULL DEFAULT 0,
  created_by, created_at, updated_at
)
```

- **Explicit goals** attach to a task (the rewrite task) and/or project; user-authored
  in the UI, or carried by a channel/webhook trigger, or proposed by the planner.
- **Standing goals** are swarm-level templates (`task_id=NULL, project_id=NULL,
  kind='standing'`), seeded on swarm creation, editable. Defaults:
  - "Reuse existing flows/utilities instead of generating new ones."
  - "No code duplication."
  - "Follow the project's coding standards and conventions."
  - "Use the assigned skill set (the must-use skills) where applicable."

The **goal set for a task** = task explicit goals ∪ project goals ∪ swarm standing
goals, ordered: explicit by `order_idx`, then standing.

### 3.2 The verdict (leader's structured judgment)

Each verification turn forces the leader to emit:

```json
{ "target_met": true|false,      // did the goal's PRIMARY objective get achieved?
  "blocker": true|false,         // only meaningful when target_met=false: is the miss BAD (block) or close (warn)?
  "severity": "none"|"minor"|"major"|"blocker",
  "measured": "2m31s" | "120" | null,   // for metric goals
  "summary": "one line",
  "findings": [ {"severity":"...","title":"...","detail":"...","file":"path:line","fix":"..."} ] }
```

The **leader decides** (requirement). The prompt gives explicit, grounded guidance:

- **Focus on important things, never nits.** If the only issues are petty/stylistic
  (e.g. "I'd name `x` differently"), set `target_met:true` with the nits as notes — a
  **pass with notes**, never a fail. Nits must not consume a retry or block a merge.
- **Metric goals** (`metric`/`target_value`/`block_value`, ground-truthed by running
  `verify_cmd` in the worktree): report `measured`, then
  - measured ≤ target ⇒ `target_met:true`;
  - target < measured ≤ block ⇒ `target_met:false, blocker:false` (close — push, warn);
  - measured > block ⇒ `target_met:false, blocker:true` (far — push, then block).
  Example baked into the prompt mirrors the user's: target "under 2 min", a final 2.5 min
  ⇒ warn-not-block, 7 min ⇒ block. `block_value` defaults to `2× target` if unset.
- **Quality / standing goals** (no dups, coding standards, reuse existing flows, use the
  skill set): `target_met:true` if genuinely satisfied; `target_met:false, blocker:true`
  for a *material* violation worth fixing; the nit rule above keeps trivia out.

### 3.3 The engine (pure, testable) + ops (injected)

A pure driver separated from I/O so the loop logic is unit-testable with scripted
verdicts:

```rust
#[async_trait] trait VerifyOps {
    async fn verify_goal(&self, goal:&Goal, scrutiny:u32) -> Option<Verdict>;  // leader turn; None = turn failed
    async fn request_fix(&self, goal:&Goal, v:&Verdict) -> FixOutcome;         // dev turn: Completed | Failed
    async fn merge_back(&self) -> MergeOutcome;                                // leader merge (integration worktree)
    async fn record(&self, goal:&Goal, status:&str, iterations:u32, v:Option<&Verdict>);  // PERSIST (survives restart)
    async fn post(&self, kind:&str, body:String);                             // board feed
    async fn escalate(&self, goal:&Goal, reason:&str);                        // unmet → feed + channel
    fn cancelled(&self) -> bool;                                               // abort / stop
    async fn over_budget(&self) -> bool;                                      // swarm budget / paused
}

// classify() — the single decision table (review Gap A + M5 + M6):
fn classify(goal:&Goal, v:&Verdict, iterations:u32) -> Decision {
  if v.target_met            { return Pass; }                 // incl. pass-with-nits — terminal
  if !goal.blocking          { return Warned; }               // advisory goal: never fix-loop, never block
  if iterations < goal.max_retries { return RequestFix; }     // blocking + unmet: PUSH the dev
  // retries exhausted: warn-band warns (non-blocking), block-band blocks + escalates
  if v.blocker { Unmet } else { Warned }
}

async fn run_verification(goals: Vec<Goal>, ops:&dyn VerifyOps) -> VerificationSummary {
  let mut blocked = false;                          // a blocking goal went unmet ⇒ merge impossible
  for goal in goals (ordered) {                     // SEQUENTIAL: goal N only after N-1 resolves
    let mut iterations = goal.iterations;           // resumed from persisted state (restart-safe)
    let mut scrutiny = iterations + 1;
    loop {
      if ops.cancelled() { return summary.cancelled(); }
      let Some(v) = ops.verify_goal(&goal, scrutiny).await else {  // leader turn failed → retry bounded, else skip
        ops.record(&goal,"error",iterations,None).await; break;
      };
      ops.post("verify", render(&goal,&v)).await;
      // Once merge is already impossible, verify remaining BLOCKING goals once for the
      // report but don't spend fix budget on them (review m5).
      match (classify(&goal,&v,iterations), blocked && goal.blocking) {
        (Pass, _)               => { ops.record(&goal,"passed",iterations,Some(&v)).await; break; }
        (Warned, _)             => { ops.record(&goal,"warned",iterations,Some(&v)).await; break; }
        (Unmet, _)              => { ops.record(&goal,"unmet",iterations,Some(&v)).await;
                                     ops.escalate(&goal,"goal could not be achieved").await;
                                     blocked = true; break; }
        (RequestFix, true)      => { ops.record(&goal,"unmet",iterations,Some(&v)).await; blocked = true; break; }
        (RequestFix, false)     => {
          if ops.over_budget().await { ops.record(&goal,"unmet",iterations,Some(&v)).await;
                                       ops.escalate(&goal,"budget exhausted").await; blocked = true; break; }
          match ops.request_fix(&goal,&v).await {              // sends ALL findings; blocks on the dev turn
            FixOutcome::Completed => { iterations += 1; scrutiny += 1; }     // re-verify MORE thoroughly
            FixOutcome::Failed    => { ops.record(&goal,"unmet",iterations,Some(&v)).await;  // dev turn errored:
                                       ops.escalate(&goal,"could not run fix turn").await;   // distinct reason,
                                       blocked = true; break; }              // don't burn the goal's retries (M2)
          }
        }
      }
    }
  }
  if !blocked { summary.merge = Some(ops.merge_back().await); }  // merge iff no blocking goal is unmet
  summary
}
```

`max_retries` contract (review M5): `max_retries` = number of **fix attempts**;
`iterations` starts at 0 and is persisted on each `request_fix`, so #verifications =
`iterations + 1`. Continue to the next goal on exhaustion ("and so on"); a blocking unmet
sets `blocked` so the merge gate fails.

Notes tying back to the requirement:

- "each verification on a different iteration" → one goal per `verify_goal`; strictly sequential.
- "send the dev a fix request with all the issues and wait" → `request_fix` passes the
  full `findings` and blocks on the dev turn; a *failed* dev turn is distinguished from an
  *insufficient* fix (review M2) so a transient failure doesn't silently burn the budget.
- "when dev finishes, check again, even more thorough" → `scrutiny` increments and the
  prompt escalates ("a prior pass on THIS goal failed — re-examine the whole goal from
  scratch; run `verify_cmd` again; look for anything the first pass missed, not only the
  reported items").
- "warning but not block … after the number of iterations" (review Gap A) → a metric
  near-miss is **not** an early exit: it drives `RequestFix` until retries run out, and
  only *then* settles as `Warned` (non-blocking) — exactly "if it runs 2.5 after the
  iterations, warn not block; if 7, block".
- "max retries → indication it could not be achieved" → per-goal `max_retries`; on
  exhaustion `unmet` + `escalate` (feed `kind:"escalation"` + channel §4.3).
- "leader decides blocker vs warning, focus on important things" → §3.2 (`target_met` +
  the nit rule).

### 3.4 Real `VerifyOps` impl + trigger + lifecycle

`SwarmVerifyOps` wires the trait to the running daemon. The wiring incorporates the
review's blocker/major fixes:

- **`verify_goal`** → `run_swarm_agent(provider = leader.provider, kind="verify",
  task_id = the task)` as the **leader** agent, cwd = the dev's worktree, prompt = goal +
  `git diff integration_branch...agent_branch` + scrutiny + the verdict schema. Parse the
  verdict with `otto_swarm::recruiter::extract_json` (handles ```json fences); the
  `transcript_ok` closure is `|t| extract_json(t).is_some()` (same pattern as plan/recruit).
  For metric goals the leader runs `verify_cmd` in the cwd to ground-truth `measured`.
- **`request_fix`** → `run_swarm_agent(provider = dev.provider, kind="fix")`, cwd = the
  dev's worktree, prompt = the full findings + "fix these, then report done". This is a
  fresh turn (not a literal session resume) — the worktree files + findings are the
  context. It runs on the **dev's real provider** (review M1: `run_swarm_agent` is
  extended with a `provider` param + token/cost backfill — see below). Returns
  `Completed` on a finished turn, `Failed` if the turn errors/stalls/stops.
- **`merge_back`** → §1.4 in the integration worktree, under the per-`integration_branch`
  mutex.
- **`record`** persists `swarm_goals.status` + `iterations` + `verdict_json` (so a restart
  resumes mid-sequence); **`post`** → board; **`escalate`** → feed `kind:"escalation"` +
  channel origin (§4.3).

**`run_swarm_agent` extensions (review M1, M3-cost):** add a `provider: &str` param
(callers `plan`/`recruit` pass `"claude"` unchanged; verify/fix pass the agent's
provider) and an optional `task_id`, and backfill `tokens_input/output/cost_usd` on the
run via the same `session_usage` path `swarm_run::run_turn` uses (factored to a shared
helper) — so verify/fix spend counts against the swarm cost budget.

**Trigger (review m2/m3):** in `route_result`'s `"done"` arm
(`swarm_runtime.rs:441-446`), intercept **only when `res.reviews.is_empty()`** (a
human-requested review takes precedence and keeps its existing flow) **and** the task has
a non-empty *blocking* goal set: set task → **`verifying`** (a new task status; no CHECK
constraint exists, and `ready_tasks` only selects `todo` so the coordinator won't
re-drive it) and spawn the controller under a **test-and-set guard** (a `Mutex<HashSet<task_id>>`
cleared by an RAII drop-guard, incl. on panic). On finish: no blocking-unmet → task
`done` (already merged); else → task `blocked` (escalation already posted).

**Agent lock (review B1 — the key correctness fix):** while a task is under verification,
its **dev agent is locked** for the controller's whole lifetime. The coordinator's
`tick` skips an agent that is `under_verification(agent_id)` (a new check alongside
`agent_has_active_run`), so no *other* task of the dev runs on the same per-agent branch
mid-verification and `merge_back` only ever merges verified commits.

**Restart recovery (review B2):** `start_coordinator` runs a one-shot sweep — for each
task in status `verifying` whose `swarm_goals` aren't all terminal and that has no live
controller, re-spawn the controller (idempotent; it reads `iterations`/`status` from
`swarm_goals` and continues).

**Cancellation (review B3/B4):** the controller owns its **own** `CancelState` (NOT
`swarm_agent_run::begin(swarm_id)`, which is keyed by swarm and would clobber
plan/recruit). It's registered in a verify registry keyed by `task_id`; `abort()` and a
new `POST /swarm/tasks/{tid}/verify/stop` set the flag and call the controller's
`CancelState` (which kills tracked verify/fix sessions and short-circuits
`run_swarm_agent`'s 3-attempt retry). `cancelled()` is checked after every awaited turn.

**Budget (review M3):** the controller calls `over_budget()` (→ `budget_exceeded` /
paused flag) between goals and before each fix; if over, it records the goal `unmet` with
reason "budget exhausted", escalates, and stops.

**Leader resolution (review m3):** assignee's manager via the `reports_to` up-chain to
the first agent that has reports; fallback = swarm root (`list_agents().first()`). If the
assignee *is* the root, the leader is itself (self-verification) — acceptable and noted.

Bounds: per-goal `max_retries`, the `over_budget` gate, the swarm cancel flag, and the
existing 3-attempt × 1h backstop in `run_swarm_agent`.

### 3.5 Planner integration

`plan()` (multi-angle planner) optionally extracts **explicit goals** from `goal_md`
(numeric targets like "under 2 minutes", framework swaps like "playwright not selenium")
into `swarm_goals` rows alongside tasks — additive, behind the existing planner. Primary
path stays user/trigger-authored goals; the planner just bootstraps them.

---

## 4. Launch the team via Slack / webhook

### 4.1 Webhook (already exists — extend)

`routes/swarm_webhook.rs` already implements `POST /webhooks/swarm/{ws}/{sid}` (key
auth, creates a project from `goal`, enforces worktree mode, seeds tasks, starts the
coordinator). Extend `SwarmTriggerReq` with optional `goals: [{title, metric?,
target?, block?, verify_cmd?, max_retries?}]` → create `swarm_goals` rows on the seeded
task(s). Record channel origin = the webhook callback URL for reply (see §4.3).

### 4.2 Slack / Telegram → swarm (new)

Channels currently route every inbound message to a normal session via the `Bridge`.
Add a swarm-launch path:

- **Binding table** `swarm_channel_triggers(id, swarm_id, workspace_id, channel,
  match_chat, keyword, repo_path, auto_start, reply, enabled, created_*)` — "when a
  message arrives on this Slack channel (optionally starting with `keyword`), launch
  this swarm with the message as the goal."
- **`SwarmTrigger` trait** (defined in otto-channels, implemented in otto-server which
  has `swarm_repo` + runtime):
  ```rust
  #[async_trait] trait SwarmTrigger: Send+Sync {
      async fn try_launch(&self, ws:&str, channel:&str, chat:&str,
                          thread:Option<&str>, user:&str, text:&str) -> Option<LaunchAck>;
  }
  ```
  Injected into `Bridge` (`Option<Arc<dyn SwarmTrigger>>`, set via `with_swarm_trigger`).
- In `Bridge::handle`, after workspace resolution and **before** session creation, call
  `try_launch`. On `Some(ack)`: reply to the channel with `ack.message`
  ("🐝 Launched **{swarm}** on: _{goal}_ — I'll report back here.") and **return**
  (no normal session). On `None`: existing behaviour, unchanged.
- The impl looks up an enabled trigger matching `(ws, channel, chat)` (+ `keyword`
  prefix if set), strips the keyword, then runs the **same launch as the webhook**
  (create project, optional `repo_path` from the trigger, seed/plan, start), recording
  the channel origin `{integration channel, chat, thread}` on the project.

Wiring: the `Bridge` is built by `ChannelManager`; `ChannelManager` is constructed in
the otto-server daemon startup where `swarm_repo`/runtime are reachable — pass the impl
in there. (Confirmed reachable: `improve_notify` already bridges swarm↔channels at that
layer.)

### 4.3 Reply / escalate back to the originating channel

Store the origin on the project: new nullable columns
`origin_channel, origin_chat, origin_thread` on `swarm_projects` (set by the webhook and
the Slack trigger). A small helper `notify_channel_origin(ctx, project, text)` resolves
the workspace integration for `origin_channel`, builds the adapter (reusing
`improve_notify`'s send path, factored into a shared `channel_send`), and posts to the
origin chat/thread. Fired on:

- launch ack (in the trigger),
- a goal that **could not be achieved** (the §3 escalation),
- the swarm/project reaching a terminal state (final summary: per-goal verdicts + merged
  branches). This subsumes the existing opt-in `improve_notify` "swarm done" for
  channel-launched swarms (still gated, but now goes to the *originating* chat).

---

## 5. Migrations (append-only, next is 0077)

- `0077_swarm_goals.sql` — `swarm_goals` table + indexes (swarm_id, project_id, task_id).
- `0078_swarm_project_skills_integration.sql` — add `skills_json TEXT NOT NULL
  DEFAULT '[]'`, `integration_branch TEXT`, `origin_channel TEXT`, `origin_chat TEXT`,
  `origin_thread TEXT` to `swarm_projects`.
- `0079_swarm_channel_triggers.sql` — `swarm_channel_triggers` table + index
  (workspace_id, channel).
- Standing-goal seeding for **existing** swarms happens lazily (on first verification, if
  a swarm has no standing goals, seed defaults) — avoids a data migration over JSON.

Swarm config additions (`worktree_isolation`, team `skills`) are JSON — **no migration**.

---

## 6. API + WS surface (contracts updated in lockstep)

New REST (numbers continue from #86; `ws editor` unless noted):

- Goals: `GET /swarm/tasks/{tid}/goals`, `GET /swarm/projects/{pid}/goals`,
  `POST /swarm/{tasks|projects}/{id}/goals`, `PATCH /swarm/goals/{gid}`,
  `DELETE /swarm/goals/{gid}`; standing goals `GET/PUT /swarm/swarms/{sid}/standing-goals`.
- Verify: `POST /swarm/tasks/{tid}/verify` (manually kick the verification controller),
  `GET /swarm/tasks/{tid}/verification` (current per-goal state).
- Triggers: `GET /swarm/swarms/{sid}/triggers`, `POST /swarm/swarms/{sid}/triggers`,
  `PATCH /swarm/triggers/{id}`, `DELETE /swarm/triggers/{id}`.
- Team/project skills ride existing `PATCH /swarm/swarms/{sid}` (config.skills) and
  `PATCH /swarm/projects/{pid}` (skills_json) — no new endpoints.

New WS events (additive):

- `{type:"swarm_goal_updated", workspace_id, swarm_id, task_id?, goal: SwarmGoal}`.
- `swarm_message_posted` already covers the new feed kinds (`worktree`, `shared`,
  `merge`, `escalation`).

`docs/contracts/api.md`, `docs/contracts/ws.md`, and `ui/src/lib/api/types.ts` updated
together (repo rule).

---

## 7. UI surface (ui/src/modules/swarm)

- **Goals**: a "Goals" lane/panel on the Kanban (or a task drawer) listing each goal
  with status chip (pending/verifying/passed/warned/unmet), measured value, last verdict
  summary, iterations/max_retries; add/edit goal modal (title, metric+target+block,
  verify_cmd, max_retries, blocking). A swarm-settings "Standing goals" editor.
- **Verification feed**: the §1/§3 messages render in `BoardFeed` with kind chips
  (worktree 🌿, shared ⚠️, merge ✅, verify 🔎, escalation 🚫). `RunInspector` shows
  verify/fix runs.
- **Skills**: team-skills section in swarm settings; project-skills multi-select in the
  project modal (both fed by `GET /library/skills`).
- **Triggers**: a "Triggers" panel in swarm settings — bind a channel (workspace
  integration) + optional keyword + repo + auto-start/reply to this swarm.
- All new types added to `ui/src/modules/swarm/types.ts`; store methods in
  `swarm.svelte.ts`; live updates via the new `swarm_goal_updated` event.

---

## 8. Testing strategy

The verification loop and channel launch depend on live LLM turns — untestable in CI.
Split:

- **Rust unit tests (deterministic)** for the pure `run_verification` engine with a mock
  `VerifyOps`: scripted verdict sequences assert — sequential per-goal ordering;
  pass→next; warn(non-blocking)→next; fail→fix→re-verify with incremented scrutiny;
  `max_retries` exhaustion → `unmet` + escalate; merge fires iff no blocking unmet; a
  later goal still runs after an earlier goal warns; cancel short-circuits.
- **Rust integration test (real git)** for §1: build a temp repo, create two agent
  worktrees via `ensure_cwd_info`, make overlapping edits, assert the shared-files
  detector reports the overlap, then `merge_back` merges one branch and reports a
  conflict for the colliding one.
- **Rust tests** for goal-set resolution (union of task/project/standing) and skill
  resolution (union of agent/project/team, must_use OR).
- **UI E2E (Playwright, `ui/e2e/swarm-*.spec.ts`)** extends the existing seed: seed a
  swarm + project (temp git repo) + a task with goals + a channel trigger; assert the
  Goals panel renders + CRUD, verdict chips, standing-goals editor, team/project skill
  pickers, the trigger panel, and that worktree/shared/merge feed messages render. (Seed
  goal rows and feed messages via API so rendering is deterministic — same approach as
  the current swarm-mobile spec.)

Gate: `cargo build/test --workspace`, `cargo clippy --workspace --all-targets -D
warnings`, `npm run check`, `npm run build`, the swarm E2E specs.

---

## 9. Scope decisions & risks

- **Live-run E2E is out of scope** for CI (non-deterministic LLM). The loop logic is
  covered by the pure-engine tests; the wiring is exercised by build/clippy + a route
  smoke. This matches how the repo already tests the swarm.
- **Shared-files detection is best-effort** and post-turn (we can't know files before
  work). Documented as advisory; the leader + merge-preview are the real safety net.
- **Channel Bridge coupling**: the `SwarmTrigger` trait keeps otto-channels free of an
  otto-server dependency (otto-server injects the impl), mirroring the existing
  `PreSpawnHook`/`improve_notify` boundary.
- **Standing-goal seeding** is lazy to avoid a JSON data migration; new swarms seed on
  create, old swarms seed on first verify.
- **Merge target** is a dedicated swarm `integration_branch` (NOT the user's branch);
  conflicts never auto-resolve — they become a tracked fix task; the user merges the
  integration branch to their real branch themselves.

---

## 10. Delivery (the close-out workflow)

This work is being done on the isolated worktree `worktree-swarm-enhance` (branched from
local `main` @ 33087865). Close-out:

1. Full gate green in the worktree (§8): `cargo build/test --workspace`,
   `cargo clippy --workspace --all-targets -D warnings`, `npm run check`, `npm run build`,
   swarm E2E specs.
2. **Live manual smoke (non-CI):** with the rebuilt daemon, launch a swarm via the webhook
   on a throwaway temp git repo with one explicit metric goal, watch one verify→fix→verify
   iteration and the merge land on the integration branch. This is the "full E2E" the user
   asked for that CI can't do deterministically (review Gap B) — run by hand as part of
   verify, not in CI.
3. Merge the worktree branch → a feature branch; re-run the gate.
4. Merge the feature branch → **local `main` only** (no push — matches the user's
   standing "local-only" pattern and AGENTS.md "PRs over main"; this is an explicit
   user instruction for this task).
5. Rebuild + sign + install the Tauri app **and** the `ottod` daemon, force-quit the
   running app, relaunch on the new build, and leave it running (per the `otto-app-build-deploy`
   memory + "do it myself" rule — the user never restarts the app).

## 11. Review responses (what changed after the 3-agent review)

- **User-work safety (arch M3):** merges run in a dedicated *integration worktree* on a
  dedicated swarm `integration_branch`, never the user's checkout; `auto_stash:false`.
- **Warn timing (coverage Gap A):** `warn` is a terminal verdict reached only at
  retry-exhaustion; a near-miss drives fix retries first (`target_met` flag).
- **Worktree double-drive + unverified merge (corr B1):** dev agent locked during
  verification; diff + merge are relative to `integration_branch`.
- **Restart orphan (corr B2):** per-goal state persisted + coordinator-start recovery sweep.
- **Cancel (corr B3/B4):** controller owns its own `CancelState` keyed by task; abort wired.
- **Provider (corr/arch M1):** verify/fix run on the agent's real provider; `run_swarm_agent`
  gains a `provider` param + cost backfill; "resume the dev" claim dropped.
- **Failed fix turn (corr M2):** distinguished from an insufficient fix; doesn't burn a retry.
- **Budget (corr M3):** verify/fix cost backfilled + `over_budget` checked in the loop.
- **classify table (corr M5/M6):** one table reconciling leader `blocker` with goal
  `blocking`; `max_retries` = fix attempts, persisted.
- **Merge serialization + integration gate (corr M4):** per-branch mutex + deterministic
  post-merge re-check.
- **MUST worktree (coverage Gap D):** no global disable knob; only per-agent `repo` opt-out.
- **Delivery + live smoke (coverage Gaps B/C):** §10.
