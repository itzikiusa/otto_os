---
name: good-focused
description: Reviews D2 diagram skill drafts for syntax workflow, diagram examples, references, and eval coverage. Use when asked to audit a D2-specific Agent Skill; do not use for generic code review.
license: MIT
metadata:
  version: "0.1.0"
---

# Good Focused Skill

## Scope

Use when the user asks to review a D2 diagram skill package. Do not use for generic code review or diagram rendering unrelated to skills.

## Workflow

1. Inspect `SKILL.md` for D2-specific trigger terms.
2. Check that examples include sequence, architecture, and failure diagrams.
3. Check references for D2 syntax and layout guidance.
4. Verify evals include positive, negative, conflict, bloat, and script cases.
5. Return a verdict and scorecard.

## Output format

Return a verdict, a scorecard, and concrete fixes.

## Positive example

```text
Review my D2 diagram skill before I publish it.
```

Expected: review the skill package.

## Negative example / non-trigger

```text
Please draw a D2 diagram of my API.
```

Expected: do not review a skill unless a skill package is involved.

## References

See `references/d2-review-notes.md`.
