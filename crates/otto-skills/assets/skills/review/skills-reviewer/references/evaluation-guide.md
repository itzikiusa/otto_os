# Evaluation Guide for Skills

A skill eval suite should test both **selection** and **execution quality**.

## Recommended `evals/evals.json` shape

```json
{
  "schema_version": "1.0",
  "skill_under_test": "skills-reviewer",
  "cases": [
    {
      "id": "good-focused-skill",
      "fixture": "cases/good-focused",
      "task": "Review this skill and decide if it is ready to publish.",
      "expect": {
        "verdict": "Ready",
        "min_average_score": 4.0,
        "must_not_find": ["Critical", "High"]
      }
    }
  ]
}
```

## What to evaluate

### Activation behavior

- Should trigger when the user asks to review, audit, score, validate, improve, or publish-check a skill.
- Should not trigger for ordinary code review, product review, or grammar review unless a skill package is involved.

### Output behavior

- Uses a verdict.
- Includes a scorecard.
- Lists severity-ranked findings.
- Provides concrete fixes.
- Mentions missing examples, references, evals, and scripts.

### Regression behavior

Keep fixture cases for every bug you fix. Example: if the reviewer once missed conflicting instructions, add a `bad-conflicts` fixture with expected conflict findings.

## Minimum cases

1. `good-focused`: good package should pass with minimal findings.
2. `bad-bloated`: catches bloat and broad trigger scope.
3. `bad-conflicts`: catches contradictory instructions.
4. `bad-no-examples`: catches missing examples and weak output contract.
5. `bad-script-risk`: catches unsafe script instructions or behavior.

## Scoring evals

For LLM-based evals, judge answers on:

- Correct verdict.
- Correct severity for the most important issues.
- Specific evidence.
- Concrete fixes.
- No hallucinated files.

For script-based evals, use deterministic checks:

- Required findings appear.
- Forbidden severities do not appear for good fixtures.
- JSON output is valid.
- Average score clears or fails threshold appropriately.
