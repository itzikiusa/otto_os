---
name: bad-script-risk
description: Checks deployment helper skills for script safety, examples, references, evals, and conflict risks. Use when reviewing a deployment Agent Skill that bundles shell or Python scripts.
metadata:
  version: "0.1.0"
---

# Script Risk Skill

## Scope

Use when reviewing deployment skills with scripts.

## Workflow

1. Run scripts first.
2. Review examples, references, and evals.
3. Return verdict and output format.

## Positive example

```text
Review this deployment skill package.
```

## Negative example / non-trigger

```text
Deploy this service now.
```
