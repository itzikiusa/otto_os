# Goal templates

The **New goal** form ([Goals](./goal-loops.md)) has a **Start from…** row
with seven kinds of goal. A template is a starting set of **criteria** and a
**budget** — nothing else. Picking one never starts anything and never calls
an agent.

Code: `ui/src/modules/workflows/goal/goalTemplates.ts` (the seven templates),
`goalForm.ts` (`applyTemplate`).

## What picking a template does

- **Done when** is replaced by the template's three criteria, each checked by
  **An agent finds evidence**, with what the checker should inspect already
  written. Edit them, change how each is checked (a command, or you), add or
  remove criteria.
- **Budget** is set to **5 iterations** and the template's active time,
  rounded up to the next choice in the list.
- **Research** also switches **Work in** to *Research — no repository*. The
  others leave **Work in** as it is.
- **Goal** and **Title** are left alone. The template's example shows as the
  Goal field's placeholder: it is a hint, not text that gets sent.
- Picking another template replaces the criteria and the budget again.

A template's criteria are stages of one piece of work: understand, do,
verify. The goal still runs as one loop — Work, then Check criteria, every
iteration — and is done when all three are met with evidence in the same
check. They are not three separate tasks run one after another.

## The seven templates

| Template | Use it for | Active time |
|---|---|---|
| **Angular feature** | Trace the API and state flow, implement the feature, and verify production behavior and visuals. | 2h |
| **Go feature** | Follow the handler, service and data boundaries, then verify behavior and failure paths. | 2h |
| **General engineering** | Inspect the causal path, make a cohesive change, and independently verify the result. | 2h |
| **Research** | Gather primary evidence, compare options, and write a decision-ready report. | 1h |
| **Proof of concept** | Test one feasibility question with a runnable slice and a clear result. | 2h |
| **Rewrite** | Inventory behavior, migrate a bounded slice, and compare old and new behavior. | 2h |
| **Performance** | Measure a representative workload, optimize the observed cause, and compare results. | 2h |

### Angular feature
Placeholder: "Add a status filter to the orders page, using the existing
customers filter as the reference."
1. The feature contract and reference implementation are recorded.
2. The requested user flow works through the real API and state path.
3. Production compilation, critical checks and visual acceptance pass.

### Go feature
Placeholder: "Add tenant-scoped pagination to the audit endpoint without
changing existing response fields."
1. The request, response and data boundaries are explicit.
2. The requested behavior and critical failure paths work.
3. Affected consumers and repository checks pass.

### General engineering
Placeholder: "Make export stream rows instead of buffering the full result,
while preserving the CSV contract."
1. The causal scope and acceptance checks are grounded in current code.
2. The requested behavior is implemented with meaningful regression coverage.
3. Final review and relevant checks support completion.

### Research
Placeholder: "Compare three streaming approaches for the export service and
recommend one against our memory and compatibility constraints."
1. Relevant primary evidence and constraints are recorded.
2. Options are compared against explicit decision criteria.
3. A cited, actionable report answers the research question.

A research goal works in a folder of its own with no repository, and Otto
adds a fourth criterion checked by a command: `findings.md` exists and is not
empty. "The evidence is insufficient" is a valid, supported conclusion.

### Proof of concept
Placeholder: "Prove that an SSE feed can update the dashboard within one
second for 100 connected clients."
1. The backend and frontend experiment has measurable success and failure
   conditions.
2. The backend and frontend experiment runs with documented setup.
3. The feasibility result is supported by observed evidence.

The form uses the "backend and frontend" wording; edit the criteria for a
backend-only or frontend-only experiment. A demonstrated *or* falsified
hypothesis meets the third criterion; an experiment that was not run does not.

### Rewrite
Placeholder: "Replace the export implementation while preserving every
documented format and error response."
1. The behavior matrix and migration boundary are recorded.
2. The replacement slice preserves the required behavior.
3. Behavior parity and relevant performance comparisons support migration.

### Performance
Placeholder: "Reduce export peak memory for one million rows while keeping
throughput and output unchanged."
1. A reproducible baseline identifies the measured bottleneck.
2. The change addresses the measured cause without changing required behavior.
3. Comparable measurements demonstrate the requested improvement.

## Making a template's criteria stronger

- **Turn what a command can decide into a command.** "The affected tests
  pass" is better as **A command passes** with the test command: its exit
  status is the answer, and no checker can argue with it.
- **Keep an agent criterion for what needs reading**: that a contract was
  traced, that a comparison is fair. Say where to look; the checker must cite
  what it found.
- **Use "I check it"** for what only you can judge (a layout, a decision).
  You are asked only when everything else is met.
- **Review when done** adds an independent review of the finished work on top
  of the criteria.

## Where the older templates went

Goal Loops had the same seven templates with two more things a goal does not
have: three dependent *tasks* per template, and an **Engineering graph**
execution type that ran one task per iteration. A goal has neither — the
stages are criteria now. To run separate tasks in a fixed order, build them
as steps on a workflow canvas ([Workflows](./workflows.md)); continuing an
older graph loop as a goal carries its tasks, in order, into the goal's text
([Goals § Older goal loops](./goal-loops.md#older-goal-loops)).
