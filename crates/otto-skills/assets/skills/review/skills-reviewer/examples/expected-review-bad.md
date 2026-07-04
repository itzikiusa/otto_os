# Skill Review: catch-all-helper

## Verdict
Do not publish — the skill is too broad, has conflicting instructions, and lacks evals.

## Top findings
1. [Critical] Potential higher-priority instruction override.
   - Evidence: `SKILL.md`
   - Why it matters: Skills must not override system, developer, safety, or user instructions.
   - Fix: Remove the override language.
2. [High] Description is too generic or broad.
   - Evidence: `description: Helps with everything.`
   - Why it matters: Generic activation language causes over-selection.
   - Fix: Narrow the skill to one task and add non-triggers.
3. [High] No evals/evals.json found.
   - Evidence: `evals/evals.json`
   - Why it matters: Reusable skills need regression checks.
   - Fix: Add evals for positive, negative, conflict, bloat, and safety cases.
