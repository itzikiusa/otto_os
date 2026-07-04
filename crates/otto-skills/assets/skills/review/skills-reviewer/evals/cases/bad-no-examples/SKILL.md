---
name: bad-no-examples
description: Audits SQL query optimization skills for trigger clarity, workflow structure, references, evals, and safe script usage. Use when reviewing a SQL-optimization Agent Skill package.
metadata:
  version: "0.1.0"
---

# SQL Optimization Skill Reviewer

## Scope

Use when reviewing a SQL optimization skill package. Do not use for optimizing a live SQL query directly.

## Workflow

1. Inspect the skill frontmatter.
2. Review workflow steps.
3. Check references and evals.
4. Return a verdict and output format.

## Output format

Return a verdict, scorecard, and fixes.

## References

See `references/sql-review.md`.
