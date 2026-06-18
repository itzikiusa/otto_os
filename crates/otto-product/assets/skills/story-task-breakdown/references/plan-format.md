# Plan Format — the structure the UI parses

The Otto UI parses your plan markdown into a **task tree** with live, toggleable
checkboxes. To make that work, your output must follow this format **exactly**. Small
deviations (wrong heading level, missing space in a checkbox) break parsing.

---

## Task headings

Each task is a Markdown heading of level 2–4 whose text begins with `Task N:`:

```
### Task 1: Add the plans table
```

- Prefer `###` (level 3), as in `### Task N: <title>`.
- Number tasks from 1, in the order they should be executed.
- The title is short and outcome-oriented ("Persist plan as a story version", not
  "Backend changes").

Everything between one task heading and the next belongs to that task.

## Goal line

Immediately under the heading, one line:

```
**Goal:** Persist the generated plan as a `kind="plan"` story version. Depends on Task 1.
```

State dependencies here ("Depends on Task X") — never point forward to a later task.

## Steps — the checkboxes

The steps are GitHub-style task-list items. The parser recognizes a checkbox line as:

- starts with optional indentation, then `-` or `*`, then a space,
- then `[` + a single marker character + `]`, then a space,
- then the step text.

The **marker convention** (this is what the UI reads and writes back):

| Marker | Meaning       | Emit as |
|--------|---------------|---------|
| `[ ]`  | not started   | `- [ ]` |
| `[x]` or `[X]` | done  | `- [x]` |
| `[~]`, `[>]`, or `[-]` | in progress | `- [~]` |

**Always emit new steps as `- [ ]` (todo).** You are creating a fresh plan; the PO and
the executing agent update markers later. The UI writes `[ ]`, `[~]`, `[x]` as the PO
cycles a checkbox, so your initial output should only ever use `- [ ]`.

Example:

```
- [ ] Write a failing test asserting an empty plan parses to zero tasks
- [ ] Add the `plans` table migration with `id`, `story_id`, `body_md`
- [ ] Run the migration and confirm the schema
```

Keep to roughly seven steps per task. If you need more, the task is too big — split it.

## Verify line

End each task with one line stating how to confirm the task is done:

```
**Verify:** `cargo test -p otto-state plans::` passes; the new table exists in the schema.
```

The Verify must be concrete: a command and its expected result, or a precisely described
observable behavior. "It works" is not a Verify.

---

## A minimal, correctly-formatted task

```
### Task 2: Parse plan markdown into a task tree

**Goal:** Add a pure parser that turns plan markdown into tasks + checkbox items. Depends on Task 1.

- [ ] Write a failing unit test for a two-task plan with mixed `[ ]`/`[x]` markers
- [ ] Implement `parsePlan(md)` matching `### Task` headings and `- [ ]` lines
- [ ] Make the test pass
- [ ] Add a test that a `[~]` item yields status `in_progress`

**Verify:** the parser unit tests pass and a task with all items `[x]` reports status `done`.
```

Follow this shape for every task and the UI will render a clean, trackable plan.
