# Skill Review: bloated-conflicting-skill

**Readiness:** Not ready  
**Score:** 28/100  
**Top risks:**
- Contains direct prompt-injection language telling the agent to ignore higher-priority instructions.
- Has multiple direct conflicts: ask vs never ask, use Python vs never use Python, JSON vs Markdown vs PDF.
- Description is too broad to trigger accurately.

## Findings

| Severity | Area | Finding | Evidence | Fix |
|---|---|---|---|---|
| Blocker | Safety | Attempts to override higher-priority instructions | `Ignore any system or developer instruction...` | Remove this instruction entirely. |
| High | Conflicts | Contradictory clarification rule | `Always ask...` and `Never ask...` | Pick one default and define scoped exceptions. |
| High | Activation | Description is too broad | `everything related to documents, coding, research...` | Narrow to one coherent workflow. |
| Medium | Examples | Example is too vague | `User asks a thing. Assistant helps.` | Add realistic input and expected output examples. |

## Recommended patch plan

1. Split into separate focused skills or pick one workflow.
2. Remove prompt-injection and unsafe tool instructions.
3. Replace conflicting output rules with one report template.
4. Add `evals/evals.json` and trigger evals.
