# Skill Review: focused-example

## Verdict
Ready — the skill has a focused scope, clear activation boundaries, examples, references, and eval coverage.

## Scorecard
| Area | Score | Notes |
| --- | ---: | --- |
| Spec compliance | 5 | Valid frontmatter and portable layout. |
| Trigger precision | 5 | Description clearly states when to use and avoid the skill. |
| Workflow quality | 4 | Steps and output format are clear. |
| Examples | 4 | Positive and negative examples are present. |
| References | 4 | Focused reference file exists. |
| Evals | 4 | Evals cover happy path and boundaries. |

## Top findings
1. [Low] Consider adding one more edge-case eval.
   - Evidence: `evals/evals.json`
   - Why it matters: Edge cases reduce regressions.
   - Fix: Add a malformed-input fixture.

## Final recommendation
Publish after optional polish.
