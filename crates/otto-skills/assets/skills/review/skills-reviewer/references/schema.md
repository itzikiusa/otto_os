# Static Reviewer JSON Schema

The `scripts/skill_review.py --format json` command emits this approximate shape:

```json
{
  "target": "./path/to/skill",
  "skill_name": "example-skill",
  "verdict": "Ready with fixes",
  "average_score": 3.7,
  "scorecard": {
    "spec_compliance": {"score": 5, "notes": "Valid frontmatter"}
  },
  "findings": [
    {
      "severity": "Medium",
      "code": "MISSING_NEGATIVE_EXAMPLE",
      "title": "No negative/non-trigger example found",
      "evidence": "SKILL.md",
      "why": "Negative examples help prevent over-activation.",
      "fix": "Add an example of an adjacent task that should not trigger the skill."
    }
  ],
  "assets": {
    "examples": "present",
    "references": "present",
    "evals": "weak",
    "scripts": "present"
  }
}
```

Fields are intentionally simple so CI jobs can gate on verdict, score, or finding codes.
