# Goals

> **Goals run inside Workflows.** A goal is a workflow of one fixed shape —
> work, check the criteria with evidence, repeat within a budget — that you
> start from a form and watch on a run page like any other run. It replaced
> the separate Goal Loops feature: there is no Goal Loops controller any more,
> and the loops made with it are a read-only history (*Older goal loops*
> below). Engine details — how an agent step runs on a thread, scheduling,
> holds, the chat message — are in [Workflows](./workflows.md); this page is
> the guide to goals.

Code: `crates/otto-workflows/src/goal.rs` (the goal's graph and prompt),
`criteria.rs` + `check_run.rs` (the check step's decisions), `hold.rs` +
`steering.rs`; `crates/otto-server/src/goals.rs`, `routes/goals.rs`,
`workflow_engine/nodes/{check,loop_node}.rs`, `workflow_engine/run/hold.rs`;
`ui/src/modules/workflows/goal/` (the form), `runs/{GoalCard,StateCards}.svelte`
(the run page), `ui/src/modules/loops/` (the history). Contract:
`docs/contracts/api.md` § "Goals", § "Older goal loops (read-only history)".

## What a goal is

```
Start ──▶ Work toward the goal  (a loop, with a budget)
            ├─ Work            an agent works toward the goal — one thread, kept across iterations
            └─ Check criteria  every criterion is decided, with evidence
          repeat until every criterion is met, or the budget ends
```

- **Work** gets the goal, the criteria and how each will be checked. From the
  second iteration on it also gets the last check's result and closes the
  unmet criteria first. It keeps the same conversation from one iteration to
  the next.
- **Check criteria** decides each criterion: a command's exit status, an
  independent checker agent that must cite evidence, or you. When every
  criterion is met with evidence the goal is done — whatever else the checker
  would still like.
- A goal's agents never stage, commit, push, publish, open a pull request or
  send a message. The work stays local, for you to review.

Each goal is its own workflow, named `Goal · <title>`. Goals fold under
**Goals** in the Workflows tab, their runs sit in **Runs** with every other
run, and you can open a goal's workflow in the editor to see — or change —
its steps.

## Start a goal

**Workflows → Run… → A goal…**, or ⌘K **New goal…**. The **New goal** form:

| Field | What it takes |
|---|---|
| **Goal** | What you want, in your words (up to 8,000 characters). **Start from…** offers seven kinds of goal — see [Goal templates](./goal-loop-templates.md); picking one fills the criteria and the budget and leaves your goal text alone. |
| **Title** | Follows the goal's first line until you edit it (up to 120 characters). |
| **Done when** | The criteria, 1 to 40. Each says **what must be true** and how it is checked: **A command passes** (give the command), **An agent finds evidence** (say where it should look) or **I check it** (say what you will look at). Reorder with the arrows; **Add criterion** adds one. |
| **Work in** | **A repository** of the workspace, **From the current commit** or from a branch — or **Research — no repository**. |
| **Budget** | "Up to **N** iterations and **1h** of active time": 1–50 iterations; 15m, 30m, 1h, 2h, 4h or 8h. "Paused and waiting time is not counted." |
| **Models** | **Work** and **Check**, each provider · model · effort with the Threads picker. Empty = the provider's default. |
| **Review when done** | "A separate agent reviews the finished work; findings block success." |

Under the fields the form says what it will do: "**This goal will** work in an
isolated copy of <repository> (branch otto-wf/…). Nothing is committed to your
branches, pushed or posted." **Start goal** creates the workflow, starts its
run and opens the run page. The form does not start while something is
missing, and says what ("Criterion 2 needs its command.", "Pick a repository,
or choose Research.").

Other ways a goal starts:
- **Insights → Hand off as a goal…** opens the form with the goal and title
  filled from the report's action.
- **Run agent to fix…** on a review in Git starts a goal that fixes the
  review's findings in an isolated worktree at the reviewed commit, re-reviews
  and repeats — up to 6 iterations or 2 hours of active time — and opens its
  run page.
- **Continue as a goal** on an older goal loop (below).
- An agent through MCP: `otto.run_goal_loop` (below).

## What a run does

A goal's run is queued in the **long lane** — its own two places
(`OTTO_WF_MAX_LONG_RUNS`), separate from ordinary runs, so an hours-long goal
never blocks a quick review. Then, each iteration:

1. **Work** runs as an agent step on a thread (the same thread every
   iteration), with a time limit of its own (20 minutes per iteration from
   the form).
2. **Check criteria** runs:
   - **Commands** first, one after another, in the goal's working directory.
     Exit 0 = met; the checker cannot overrule it. The evidence keeps the exit
     status and the last 8 KB of output; all of it is in a `.log` file under
     **Files**.
   - **Agent criteria** get one turn of the **Check** model on a *fresh*
     thread — never the worker's. It must give evidence for each: a "met"
     with no evidence counts as not met, and no usable answer leaves every
     agent criterion unmet.
   - **A criterion you check** is asked only when everything else is met, so
     you are never asked to verify unfinished work. The run then waits for
     you (below).
   - With **Review when done**, once every criterion is met a separate review
     turn must approve with no findings. Findings block the goal: the run is
     held **Blocked** with "The completion review has unresolved findings: …".
3. The loop ends **satisfied** when every criterion is met with evidence.
   Otherwise the next iteration starts — unless a stop rule holds the run
   (*Holds* below).

A research goal also gets one criterion by itself: `findings.md` exists and
is not empty.

## Watch it: the run page

The run page (see [Workflows § Inspect](./workflows.md#inspect-runs-and-the-run-page))
adds, for a goal:

- **The goal card** — "Criteria 2 of 3 met", one row per criterion with its
  (**Met** · **Not met** · **Verify** · **Not checked**), how it is checked (Command ·
  Agent · You · Rule) and the evidence (a command's output tail behind the
  expander), then the budget: "Iteration 2 of 5 · 18m of 1h active". When a
  more urgent card takes its place (a hold, a criterion to record, an error)
  the criteria are under **Result → Criteria**.
- **The iterations** as rows under the step "Work toward the goal"; a row
  opens that iteration's Work conversation or its check.
- **The Criteria tab** of the loop step and of each check, with the full
  evidence.
- When it is done: "Work kept on branch otto-wf/<run id>; nothing was
  pushed." or "Findings are kept in the run's research folder (Files)."

## When it waits for you

- **Verify** — a criterion you check. The card reads "Check that <criterion>
  — record what you verified". Write what you checked and what you saw, then
  **Record**; or **Not met…** with what is still wrong, which the next
  iteration reads as the reason. Your record is kept as the criterion's
  evidence ("Verified by <you>: …"). It counts for that iteration: more work
  means you are asked again.
- **Approval** or **Question** — the worker asked to run something that needs
  your approval, or asked a question; answer on the card or in the step's
  conversation.
- **Limited** — the provider's usage limit; with a known reset Otto continues
  the step at the reset by itself.

Otto raises a notification for each wait, and a goal started from a chat
thread posts there too.

## Holds: Paused, Blocked, Out of budget

A held run is stopped but not finished: its work, its worktree and its
conversations are kept, it is never pruned with old runs, and it waits for
you however long that takes.

| The run's word | Why | What you do |
|---|---|---|
| **Paused** | You chose **Pause** (the run's ⋯). The running step stops. | **Resume**. |
| **Blocked** | The check says it needs a person; or two checks in a row failed the same way ("Two checks in a row failed the same way — a new approach is needed"); or a step failed ("Work failed at iteration 3: …"). | **Add a note first** if you want to tell it something, then **Resume** — it goes on with the next iteration. |
| **Out of budget** | "Out of iterations: 5 of 5 used" or "Out of time: 1h of 1h active". | **Extend and resume…** — give new totals for **Iterations** and **Active time (minutes)**. The dialog proposes the current totals plus half, and both must be above what is already used ("Raise both limits above what is used: 5 iterations and 60 minutes."). |

Blocked and Out of budget are listed under **Needs you** and raise one
notification each ("“Goal · <title>” is blocked and needs you", "“Goal ·
<title>” ran out of budget"); a finished goal says "Goal “<title>” reached
its goal". **Resume**
keeps everything that finished — steps and whole iterations — and starts the
step that was in flight again. **Stop run…** on a held run ends it for good
and cleans up its worktree (what it changed stays on its branch — *Where the
work is* below).

Active time is the time steps actually work. Time waiting on you, time
paused and time the daemon was down are not counted.

## Tell it something: notes

**Add a note for the next step…** (a running run's ⋯; **Add a note first** on
a held one) queues a note of up to 4,000 characters. The next agent step that
starts — the next Work turn, usually — reads every waiting note before it
begins, and the note is recorded on that step. The run page shows "1 note
waiting for the next step" until then.

## Where the work is

- **A repository goal** works in an isolated worktree of the repository on
  the branch `otto-wf/<run id>`, cut from the branch you picked or the
  checkout's current commit. Your own checkout is not touched. A held run
  keeps its worktree. When the run ends, Otto removes the worktree folder
  and keeps what the goal changed on that branch — uncommitted changes are
  first saved there as one commit ("chore: preserve workflow leftovers (run
  …)"). Nothing is committed to your own branches, pushed or posted.
- **A research goal** works in a research folder inside the run's own
  folder, with no repository, and writes `findings.md` there (**Files** on
  the run page).
- Step files, command logs and the checks' tables are the run's context
  files, like any workflow's.

## Build your own

A goal is ordinary workflow steps, so you can use them on the canvas:

- **Check criteria** (`check`) — "Done when" with **Add**; each criterion
  checked by **A command passes**, **An agent finds evidence**, **I check
  it** or **A rule holds** (an expression over the iteration, e.g.
  `steps.review.passed == true`); the **Checker**'s model; optional
  instructions for it; and "When every criterion is met, have an independent
  review approve it". Outside a loop it is a gate with evidence; inside a
  loop a passing check ends the loop.
- **Repeat (Until)** (`loop`) — besides `max_iterations` and `until`, a loop
  takes `max_active_s` (a budget of active time, which also allows up to 50
  iterations), `stop_on_repeat_failure`, `hold_when_unmet` (hold the run
  instead of ending unsatisfied) and, per inner step, `same_thread` (keep the
  conversation across iterations). The loop's form edits iterations, until
  and the steps (`same_thread` through the steps' JSON); `max_active_s`,
  `stop_on_repeat_failure` and `hold_when_unmet` have no field in the form —
  the goal builder sets them, and so can an imported workflow or the API.
- The bundled templates' "Review → fix until passing" loop is built from the
  same pieces: review → check (the rule "A fresh independent review passes")
  → fix.

## From MCP

- `otto.run_goal_loop` keeps its name and input and **starts a goal** (it
  answers `{workflow_id, run_id}`). Roles a goal does not have — planner,
  digester, definer — are ignored and listed as ignored; an engineering
  graph and a cost cap are refused. It is a dangerous tool: off by default
  and approval-gated.
- `otto.list_goal_loops` lists goal runs, then the older loops.
- `otto.control_workflow_run` pauses or resumes any workflow run and can set
  a goal's new budget totals on resume. Off by default, approval-gated.

See [MCP control plane](./mcp-control-plane.md).

## Older goal loops

Loops made with Goal Loops before this release are kept as a read-only
history: **Workflows → Runs → Older goal loops (n)** at the foot of the page
(shown only when the workspace has any; old `#/loops` links and notices still
open it).

- The list shows each loop's status, "Iteration 3 of 5" and its progress; a
  loop opens to its progress, its iterations, its criteria with their
  evidence and the changes kept in its worktree.
- **Continue as a goal** (a loop that did not finish: a draft, or one that
  was paused, blocked or out of budget, and was not continued before) opens
  the goal form filled from the loop: its goal, criteria, models, and the
  budget it had left (at least 1 iteration and 15 minutes). **Work in** is
  fixed to "This loop's kept work", so the goal goes on where the loop
  stopped; where the loop got to and its next action are added to the goal
  text. "Earlier person verifications are not carried over: they were for
  earlier work." The loop is then marked stopped, "Continued as a goal".
- **Delete history…** removes the loop's record. "Its kept work on disk
  stays."
- **A loop that was running when Otto was updated** was paused once, with one
  notice ("Goal loop “…” was paused by the update …"). Its work is kept;
  continue it as a goal.

What Goal Loops had and a goal does not: separate planner and digester
agents (Work plans for itself and reads the last check), **Define with AI**
(the form's **Start from…** templates fill the criteria instead),
engineering graphs (put the tasks on a workflow canvas; continuing a graph
loop carries its tasks into the goal text, in order), per-phase time limits,
a cost cap and the "Allow local commits" option (a goal never commits).

## API

| Method & path | What |
|---|---|
| `POST /workspaces/{wid}/goals` | Start a goal from a `GoalSpec` (title, goal, criteria, work, budget, worker, checker, final_review) → `{workflow, run}`. 400 names the field that is wrong. |
| `GET /workspaces/{wid}/goals` | Goal runs (each goal's latest), then the older loops (`legacy: true`). |
| `POST /workspaces/{wid}/goals/from-loop` | The MCP tool's route: a Goal Loops request started as a goal. |
| `POST /goal-loops/{id}/continue` | Continue an older loop as a goal. 409 when it finished or was continued already. |
| `POST /reviews/{review_id}/fix-goal` | "Run agent to fix…" → `{workflow_id, run_id}`. |
| `POST /workflow-runs/{id}/pause` · `/resume` | Hold / re-enter a run; `{extend: {iterations?, active_s?}}` are new totals. 409 says which limit to raise. |
| `POST /workflow-runs/{id}/steer` | `{note}` — a note for the next agent step (a person's credential). |
| `POST /workflow-runs/{id}/nodes/{node_id}/criteria/{criterion_id}/verify` | `{met, evidence}` — record a criterion you check (a person's credential). |
| `GET /workspaces/{id}/goal-loops`, `GET /goal-loops/{id}`, `…/iterations/{idx}`, `…/diff`, `DELETE /goal-loops/{id}` | The older loops' history. Every other `/goal-loops` route answers 404. |

A goal's run is an ordinary workflow run: `GET /workflow-runs/{id}` and the
`workflow_run_updated` event carry its progress in `overview.goal`
(`iteration`, `max_iterations`, `criteria_met`, `criteria_total`,
`active_ms`, `max_active_ms`, `awaiting`) and its hold in `overview.hold`
(`kind: paused|blocked|exhausted`, `reason`, `since`). The `goal_loop_updated`
event is no longer sent.

## Limits and troubleshooting

| Symptom | Cause / what to do |
|---|---|
| The goal stopped as **Blocked** after two iterations | Two checks in a row failed the same way. Add a note saying what to try differently, then **Resume**. |
| **Resume** answers "Raise the iteration limit to resume: 5 of 5 iterations are used." | The run is out of budget: use **Extend and resume…** with totals above what is used. |
| A criterion stays **Not met** though the work looks done | Read its evidence. A command criterion is its exit status — run the command yourself in the worktree. An agent criterion needs evidence the checker can cite; say where to look in the criterion. |
| The checker says "met" but the row is Not met | A "met" without evidence counts as not met. |
| You are asked to **Verify** again | A record counts for one iteration; the goal did more work after it. |
| **Start goal** is refused for the repository | **Work in** must be a repository registered in this workspace. |
| A goal waits in **Queued** while nothing else runs | Two other goals hold the long lane; it starts when one ends or is held. |
| `otto.run_goal_loop` answers "Engineering-graph goals are not supported any more" | Build the tasks as steps on a workflow canvas, or start a plain goal. |
| Where are the goal's changes? | On the branch `otto-wf/<run id>` of the repository (a held run: still in its worktree). Nothing is on your own branches: review that branch and merge or cherry-pick what you want. |

Limits: 1–40 criteria; 1–50 iterations; 15 minutes to 8 hours of active
time; one Work turn up to its time limit, one checker turn up to 30 minutes;
two goals at once by default. A goal runs on the same engine as every
workflow, so its steps wait for the same machine places as other agents.

## Related

- [Workflows](./workflows.md) — the engine: how agent steps run, the run
  page, holds, the chat message, restart.
- [Goal templates](./goal-loop-templates.md) — the seven **Start from…**
  kinds.
- [AI code review](./code-review.md) — **Run agent to fix…** starts a goal.
- [Insights](./insights.md) — **Hand off as a goal…**.
- [MCP control plane](./mcp-control-plane.md) — the goal tools.
