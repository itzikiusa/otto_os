# Value & INVEST — PO-Level Evaluation Guide

Use this reference when judging whether a story delivers real user value and is
well-formed enough to be committed to a sprint. This is a product lens, not a
technical one.

---

## Evaluating User Value

Value is not self-evident just because a feature was requested. Push every story
through these three questions before accepting it.

### Question 1: Value for whom?

Identify the beneficiary. A story that helps one power user once a quarter has
different priority than one that removes a daily friction point for every user.
Ask:
- How many users are affected?
- How frequently do they encounter this?
- How painful is the current state?

A useful mental model: **Reach × Frequency × Severity**. A story that scores
high on all three is high value. A story that scores high on only one needs
justification.

### Question 2: Value of what kind?

| Value type | Example | How to verify |
|------------|---------|---------------|
| Remove pain | "Users currently have to export to Excel to sort" | Show the current workaround |
| Enable a goal | "Compliance team needs a weekly audit trail export" | Link to the compliance requirement |
| Increase confidence | "Dashboard shows live data so managers trust it" | Research current trust/accuracy issues |
| Drive a metric | "Faster onboarding → higher 30-day retention" | State the hypothesis and baseline |
| Reduce cost | "Automate a step that takes the ops team 2h/week" | Quantify the current effort |

If the value type is unclear, the story is likely not ready.

### Question 3: How will we know we delivered it?

Every story should have an implied or explicit success signal. Examples:
- A metric moves (even if we won't track it perfectly from day one)
- A user workflow is shorter or eliminated
- A support ticket category disappears
- An audit requirement is satisfied

If there is no conceivable way to know the story made things better, the outcome
is not defined and the story is not ready.

---

## INVEST — Applied at PO Altitude

INVEST is a checklist for story quality. Apply it from a product perspective,
not an engineering one — the question is whether the story is well-formed,
not how it should be built.

### I — Independent
Can this story be delivered without waiting on another story to be done first?
If not, name the dependency. Stories with hard sequencing dependencies are harder
to plan and risk derailing a sprint.

**Warning sign:** "This depends on the new profile API being ready." That is a
dependency, not independence. It may be acceptable, but it must be explicit.

### N — Negotiable
The story describes a *need*, not a *solution*. Engineering and design should have
room to propose how to meet it. If the story prescribes the exact UI, data model,
or algorithm, it has over-constrained the solution space.

**Good:** "Users need to filter results by date range."
**Over-constrained:** "Add a DateRangePicker component to the top of the table
that sends startDate and endDate query params to the /reports endpoint."

The second form belongs in a design spec or technical task, not a story.

### V — Valuable
Would a user or sponsor notice if this story was never built? If the honest
answer is "probably not," reassess the priority. Valuable stories solve a real
problem or enable a real goal for a real user.

### E — Estimable
Engineering should be able to size it. A story is not estimable if:
- The scope is unbounded ("improve the search experience")
- Key information is missing (unknown third-party API, unclear data model)
- It is too large to fit in a sprint without splitting

If engineering says "we can't estimate this," treat it as a signal that the
story needs clarification or splitting, not that engineering is being difficult.

### S — Small
A story should ideally be completable within a sprint. If it covers a full
user journey with many edge cases, consider whether it is actually an epic that
needs decomposition. Good split patterns:
- Happy path first, edge cases as follow-on
- One platform at a time (web now, mobile later)
- One persona at a time
- Read-only first, write operations as follow-on

**Warning sign:** A story that will take more than a week to estimate. If you
can't even size it quickly, it is too big.

### T — Testable
There must be a way to confirm the story is done. Testable means:
- Acceptance criteria exist and are unambiguous
- A person (or automation) could verify each criterion independently
- Subjective claims ("the UI should feel polished") are resolved into observable ones
  ("buttons respond within 200ms; no layout shift on load")

If you cannot write a test — even a manual one — for a criterion, the criterion
is not good enough.

---

## When to Recommend Splitting

Split a story when any of the following is true:
- It touches more than one user persona in a meaningfully different way
- It contains both a happy path and multiple significant edge cases that each
  require non-trivial work
- It spans more than two systems or services
- Engineering estimates it at more than a week of work
- It contains the word "and" in the title or value statement

Suggested split patterns to offer the PO:
1. **Slice by flow stage:** Input → Processing → Output as separate stories
2. **Slice by persona:** Primary user first, secondary user as follow-on
3. **Slice by confidence:** Read/view now, edit/create next sprint
4. **Slice by platform:** Web first, then mobile, then API consumers
