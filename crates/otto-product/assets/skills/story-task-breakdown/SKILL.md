---
description: Turn a refined product story into a superpowers-style implementation plan — an ordered list of small, independently-verifiable tasks. Each task states a goal, concrete steps as checkboxes, and a verification. PO-readable but actionable; right-sized, TDD where it fits, never over-engineered.
---

# Story Task Breakdown

You produce the **implementation plan** for a refined product story: a sequence of
small, ordered, independently-verifiable tasks that a developer (human or agent) can
execute one at a time, and that a Product Owner can read and track.

This is *not* a design doc and *not* a re-statement of the story. It is the concrete
plan of action: what to build, in what order, and how to know each step is done.

> **Reference files live in `references/` and the output template is in `assets/` — both
> sit alongside this SKILL.md. Consult them as you work:**
> - `references/plan-format.md` — the EXACT heading/checkbox structure the UI parses, the
>   marker convention (`[ ]` / `[~]` / `[x]`), and how to phrase goals, steps, and verifies
> - `assets/plan-template.md` — a filled example plan you can model your output on

---

## What "good" looks like (superpowers writing-plans style)

A good plan is:

- **Ordered.** Tasks run top to bottom. Earlier tasks unblock later ones.
- **Small.** Each task is one coherent, reviewable change — typically minutes to a couple
  of hours, not "build the whole feature." If a task has more than ~7 steps, split it.
- **Independently verifiable.** Every task ends with a concrete way to confirm it works
  (a test passing, a command's output, an observable behavior) — not "looks done."
- **TDD where it fits.** When a task adds behavior that can be tested, write the failing
  test *first* as an early step, then make it pass. State this explicitly in the steps.
- **Right-sized, not over-engineered.** Plan only what the story asks for (YAGNI). Do not
  invent abstractions, extra config, or speculative extensibility the story never requested.
- **Dependency-aware.** When a task depends on an earlier one, say so in its Goal
  ("Depends on Task 2"). Never reference a task that comes later.
- **PO-readable, dev-actionable.** A PO should understand each task's intent; a developer
  should be able to start it without guessing.

---

## Workflow

### 1. Read the refined context completely

You are given the story's refined picture: the latest story body, the answered
clarifying questions, the analysis summary, and any approved test cases. Read all of it
before planning. The answered questions and approved test cases are the source of truth
for scope and edge cases — your plan must cover them and must not contradict them.

If the context is too thin to plan against (no clear story body, no acceptance signal),
emit a single first task that says "Clarify scope" and lists exactly what is missing,
rather than inventing requirements.

### 2. Identify the work, then sequence it

- List the distinct pieces of work the story implies (data, backend, UI, wiring, tests,
  docs — only those that actually apply).
- Order them so each builds on the last. Foundational/data changes first, then the
  behavior that uses them, then the surface (UI/API), then end-to-end verification.
- Fold testing into the tasks it belongs to (TDD), and add a final verification task that
  proves the whole story works together.

### 3. Write each task in the required format

For EVERY task, emit exactly this shape (see `references/plan-format.md` for the rules
the UI parser depends on — follow it precisely):

```
### Task N: <short, outcome-oriented title>

**Goal:** <one sentence: what this task achieves, and any "Depends on Task X" note>

- [ ] <concrete step the developer performs>
- [ ] <next step — write the failing test here when doing TDD>
- [ ] <continue; keep to ~7 steps max>

**Verify:** <the exact command to run or observable result that proves this task is done>
```

- Use `### Task N:` headings numbered from 1, in execution order.
- Every step is a GitHub-style checkbox starting unchecked: `- [ ]`. The PO will tick
  these off as work completes, so each must be a single, checkable unit of work.
- Keep steps imperative and concrete ("Add column `status` to `plans` table", not
  "handle status").

### 4. Self-check before emitting

- Does the plan cover every acceptance criterion / answered question / approved test?
- Is anything over-engineered? Cut speculative work.
- Can each task be verified on its own? If a task has no meaningful Verify, merge or
  rework it.
- Are tasks ordered with no forward dependencies?

---

## Output contract (MANDATORY)

After planning, respond with **EXACTLY ONE** ```json code block (no prose before or
after) of this exact shape:

```json
{"plan_markdown": "### Task 1: ...\n\n**Goal:** ...\n\n- [ ] ...\n\n**Verify:** ...\n\n### Task 2: ..."}
```

- `plan_markdown` is the full plan as Markdown, using the `### Task N:` / `- [ ]` /
  `**Verify:**` structure above. It MUST be valid JSON (escape newlines as `\n`).
- Do not wrap the plan in extra prose. The plan markdown is the entire deliverable.
