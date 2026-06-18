---
description: Draft readable, right-sized test cases for a story — happy path plus meaningful validations and realistic errors. Not over-defensive. Plain language a PO can approve and a dev can implement against.
---

# Story Test Cases

You write the test cases a developer must satisfy and a Product Owner will approve.
Each case must be **readable by a PO** (plain given/when/then language, no code or
internal jargon) and **right-sized** — thorough where it matters, not bloated with
trivia.

> **Reference files live in `references/` and the output template is in `assets/` — both
> sit alongside this SKILL.md. Consult them as you work:**
> - `references/test-design-heuristics.md` — how to choose cases: equivalence classes,
>   boundaries, state, permissions, failure modes, and the "not over-defensive" rule
> - `references/good-vs-bad-examples.md` — side-by-side sharp vs. vague/bloated cases
> - `references/coverage-and-traceability.md` — mapping every AC to cases; no case
>   tests something the story doesn't promise
> - `assets/test-case-template.md` — the exact case structure with a filled example

---

## Workflow

### 1. Read the story and acceptance criteria completely

Ingest the full Jira ticket, linked Confluence or RFC pages, and attached specs before
writing a single case. List every acceptance criterion (AC) explicitly — number them
so you can reference them in traceability later.

If no ACs are present, stop and ask the requester to supply them. Drafting cases against
a vague story produces the wrong cases.

### 2. Identify the coverage areas

For each AC, determine which of these areas applies — you will need at least one case
per area that appears:

| Category | When to use it |
|----------|----------------|
| `happy` | The primary success flow the story exists to deliver |
| `validation` | Input rules and business constraints a user would realistically hit |
| `error` | Failure modes that genuinely occur: missing dependency, unauthorized, conflict |
| `edge` | A boundary or combination that changes behavior (not just a permutation) |

Use `references/test-design-heuristics.md` to judge which cases are worth writing and
which to skip. Consult `references/good-vs-bad-examples.md` when in doubt about a case's
quality.

### 3. Draft cases using the template

For every case, fill in every field in `assets/test-case-template.md`:

- **title** — short, specific, outcome-oriented (e.g., "Reject deposit below minimum")
- **category** — `happy` | `validation` | `error` | `edge`
- **priority** — `high` / `medium` / `low`, driven by user or business impact
- **ac-refs** — one or more AC numbers this case exercises
- **preconditions** — the state that must hold before the steps begin
- **steps** — the actions, in order, in plain language a PO understands
- **expected** — the observable result: what the user sees and what the system records

Write in the user's vocabulary. Avoid implementation terms (class names, SQL, HTTP
codes) unless they appear in the story's own language.

### 4. Check coverage and traceability

Using `references/coverage-and-traceability.md` as your guide, verify:

- Every AC maps to at least one case.
- No case tests a behavior the story does not promise.
- The set collectively tells a coherent story of "working correctly."

Write a one-line traceability note under each case's `ac-refs` if the link is not
obvious.

### 5. Produce the final case set

Group cases by category (`happy` first, then `validation`, `error`, `edge`). Within
each group, order by priority descending.

Prepend a short header: the story title, a one-sentence scope statement, and the total
case count by category. This header is what the PO scans first.

---

## Quality bar

- **PO-readable.** If a PO cannot read a step or expected result aloud and immediately
  understand it, rewrite it.
- **Not over-defensive.** Three sharp validation cases beat fifteen that no real user
  will ever trigger. If a case doesn't protect a genuine user or business outcome, cut it.
  See `references/test-design-heuristics.md` for the explicit "skip" list.
- **Unambiguous expected results.** Each expected result must state exactly what happens —
  not "the system responds appropriately" but "the user sees error message 'Amount must
  be at least £10'."
- **Independent cases.** No case should depend on the outcome of another unless you
  explicitly model a flow sequence.
- **No invented requirements.** If the story is silent on a behavior, leave a marked
  open question rather than asserting an outcome.
- **One coherent artifact.** The case set you produce is the contract the developer
  implements against. It should be ready to paste into Jira or a Confluence page.
