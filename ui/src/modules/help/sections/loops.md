---
id: loops
title: Goal Loops
group: Automate
route: loops
summary: Give agents a goal, acceptance criteria and a budget; they iterate Plan → Execute → Evaluate → Digest until the criteria pass or a limit is hit.
---
## What it's for

A goal loop is bounded, unattended work toward a concrete finish line. You describe the outcome and how to check it. Otto then runs iterations — a planner plans, executors do the work, an evaluator checks every acceptance criterion, a digester carries context forward — until every criterion passes, a limit is reached, or the loop needs a decision from you.

Build loops work in an isolated `goal-loop/<id>` git worktree, so your own checkout is never the working directory. Research loops work in a dedicated folder and must produce a `findings.md` report.

## Getting started

1. Open **Goal Loops** and choose **New goal loop**.
2. Pick a **Mode**: **Build** (needs a repository — type the path or choose **Browse…**) or **Research** (no repository).
3. Choose the **Definer provider** and model, describe the goal in plain words, and choose **Define with AI**. Otto drafts a name, objectives and acceptance criteria.
4. Not quite right? Type what to change in the refine box and choose **Refine**. Edit any criterion by hand, or **+ Add criterion**.
5. For each criterion pick how it is verified: **Command** (a shell command — exit 0 means met), **Agent assessment**, or **Human verification**.
6. Set the budget (iterations, minutes, per-phase minutes, number of executors) and the executor provider and model.
7. Choose **Launch loop**. The detail view opens and follows progress live.

## Everything it can do

**Definition**
- Build or Research mode. The mode is fixed once the loop is created.
- AI-drafted definition with any number of refine rounds before launch.
- Acceptance criteria, each with a description plus one of three checks:
  - **Command:** Otto runs it in the loop's working folder; the exit status is the ground truth. "What this checks" documents the purpose separately from the command.
  - **Agent assessment:** the evaluator must cite evidence. (Older loops may show "Agent assessment (legacy)"; it means the same thing.)
  - **Human verification:** the model can never mark it met. You record what you verified.
- Optional source, spec and plan links (one per line) and named skills, passed to the agents as context.

**Roles and budget**
- 1–6 executors, run one after another in the same working folder, each with a provider and model.
- Separate provider and model for the planner, evaluator and digester.
- Limits: max iterations (default 5), max minutes of active time (default 30), per-phase minutes (default 10), and executor recovery attempts (default 3). A 4-hour controller backstop always applies.
- **Allow local commits** (off by default; not available for Research). Without it the executor leaves changes uncommitted for you to review.
- **Require independent completion review** adds a separate review turn; unresolved findings block success.

**Monitoring**
- A card per loop on the list: status, progress bar, iteration count and the current phase while running.
- Detail view: progress, the Plan → Execute → Evaluate → Digest stepper, iteration and time used, branch, retained work path and the next action.
- Expandable iteration history: the plan, each executor's summary, per-criterion met/unmet chips with evidence, and the digest carried forward.
- **Agents and roles:** every executor, planner, evaluator, digester and review session with its provider and state. **Open** any of them to watch or respond in a live terminal.
- Live updates over WebSocket, with polling as a fallback.

**Controls**
- **Pause** (banks active time), **Resume**, **Stop** and **Delete**.
- **Retry** one executor of the current iteration while the loop is blocked.
- Answer questions the loop raises (**Record answer**). Answers never restart the loop on their own — choose **Resume** when ready.
- **Record verification** for a Human criterion while the loop is paused, blocked or exhausted.

**States**
- Draft, Running, Paused, Blocked (needs a decision, repeated failure, a human check or review findings), Exhausted (hit a limit), Succeeded, Failed, Stopped.

**Evidence**
- Build loops attach the real working contents (committed, staged, unstaged and untracked changes), command output and evaluator evidence to a proof pack. Research reports are attached as evidence.

## Keyboard shortcuts

None specific to this page.

## Tips and limits

- Build mode needs a repository with at least one commit.
- Write Command criteria that run unattended in the isolated folder (for example `npm test -- import`), not commands that need your shell's state.
- Two iterations with identical unmet criteria and evidence block the loop and ask for a new approach. This threshold is fixed.
- Further executor work invalidates earlier human approvals. After a final human acceptance, **Resume** rechecks the iteration without spending another one.
- Pause, Stop, success, failure and exhaustion all keep the working folder and branch. Deleting a loop removes its history, not its files; the detail view shows the path before you delete.
- A daemon restart pauses running loops on purpose. Inspect the retained work, then **Resume**.
- To continue an exhausted loop, raise its limits first.
- A money limit per loop is not supported. Workspace usage budgets still gate defining, launching and resuming.
- Pushing, publishing and opening PRs are never part of automatic completion.
- Skills named in a loop are passed as context; they are not installed for you.
- Viewers can see loops. Defining, launching, answering, verifying and every control need the workspace Editor role.

## Related

- [Agents](#/walkthroughs/agents)
- [Swarm](#/walkthroughs/swarm)
- [Workflows](#/walkthroughs/workflows)
- [Proof](#/walkthroughs/proof)
- [Mission Control](#/walkthroughs/mission-control)
