# Skill Smell Catalog

Use this catalog to identify common skill quality problems.

## Activation smells

### Vague description

**Symptom:** Description says “helps with X” but lacks specific tasks or trigger terms.

**Impact:** The agent may not select the skill, or may select it for too many tasks.

**Fix:** State the exact task, inputs, outputs, and user phrases that should trigger it.

### Catch-all skill

**Symptom:** “Use this for all coding tasks,” “general assistant skill,” or several unrelated domains.

**Impact:** Crowds out other skills and creates unpredictable behavior.

**Fix:** Split into separate skills or narrow scope.

### Missing non-triggers

**Symptom:** No guidance for when not to use the skill.

**Impact:** Skill may activate on adjacent but inappropriate tasks.

**Fix:** Add boundaries in the description or body.

## Instruction smells

### Workflow fog

**Symptom:** Advice is descriptive rather than procedural.

**Impact:** Different runs produce inconsistent outputs.

**Fix:** Add imperative steps and output format.

### Contradictory rules

**Symptom:** Two rules cannot both be followed.

**Impact:** Agent has to guess which rule wins.

**Fix:** Remove one rule or add precedence.

### Hidden assumptions

**Symptom:** Assumes files, credentials, network, shell, or tools exist without saying so.

**Impact:** Skill fails in different runtimes.

**Fix:** State dependencies and fallback behavior.

## Packaging smells

### No evals

**Symptom:** No `evals/` directory or equivalent test cases.

**Impact:** Regressions are easy and quality is subjective.

**Fix:** Add eval cases for activation, workflow, edge cases, and conflicts.

### Example theater

**Symptom:** Examples are tiny, unrealistic, or only show perfect inputs.

**Impact:** Users and agents do not learn boundaries.

**Fix:** Add realistic good/bad examples and expected outputs.

### Reference dump

**Symptom:** Huge policy/API docs pasted into `SKILL.md`.

**Impact:** Wastes context and hides the workflow.

**Fix:** Move details into focused reference files.

## Script smells

### Script without purpose

**Symptom:** Script duplicates instructions or adds complexity without deterministic value.

**Impact:** Maintenance cost with little benefit.

**Fix:** Remove script or explain the deterministic check it performs.

### Unsafe default

**Symptom:** Script deletes files, writes outside the skill root, runs network calls, or shells out without guardrails.

**Impact:** Security and data-loss risk.

**Fix:** Make destructive actions opt-in, document permissions, and add dry-run mode.

### Dependency drift

**Symptom:** Script imports packages not declared anywhere.

**Impact:** Works for the author, fails elsewhere.

**Fix:** Use stdlib where possible; document dependencies and install commands.

## Governance smells

### Policy override attempt

**Symptom:** Skill says to ignore system/developer/user safety rules.

**Impact:** Unsafe and not acceptable for publication.

**Fix:** Remove immediately; mark Critical.

### Provenance gap

**Symptom:** External facts, legal/security claims, or standard claims have no source.

**Impact:** Reviewer cannot verify freshness or authority.

**Fix:** Add reference source, date checked, and scope.
