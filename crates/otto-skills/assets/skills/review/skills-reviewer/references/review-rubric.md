# Skill Review Rubric

Use this rubric when a deeper review is needed than the summary in `SKILL.md`.

## 1. Spec compliance

**5** — Fully valid package; `SKILL.md` has valid YAML frontmatter; `name` is lowercase kebab-case, <=64 chars, no leading/trailing/consecutive hyphens; description is <=1024 chars and includes use case and trigger terms; file references are relative and portable.

**3** — Mostly valid, but with minor portability or metadata gaps.

**1** — Missing or invalid required fields, malformed frontmatter, or package cannot be loaded reliably.

**0** — Missing `SKILL.md`.

## 2. Trigger precision

Look for:

- Clear trigger keywords.
- Clear non-triggers / boundaries.
- Description front-loads the main use case.
- Scope is neither too broad nor too narrow.
- No overlap with unrelated skills such as generic code review, writing, research, or deployment.

Red flags:

- “Use for everything.”
- “Always use this skill.”
- “Helps with documents/code/data/etc.” without boundaries.
- Long marketing description that will be truncated badly.

## 3. Workflow quality

A strong workflow has:

- Ordered steps.
- Explicit inputs.
- Explicit outputs.
- Decision points.
- Edge cases and fallbacks.
- A stable output contract.

A weak workflow has:

- Aspirational guidance only.
- Conflicting steps.
- No output format.
- Hidden assumptions about environment, tools, or user context.

## 4. Examples

Minimum bar:

- One positive invocation example.
- One negative/non-trigger example.
- One expected output shape or partial expected answer.

Best-in-class examples:

- Show realistic inputs.
- Show what files are read and what files are ignored.
- Include failure mode examples.
- Include before/after improvements when the skill modifies content.

## 5. References

References should be:

- Focused by topic.
- Loaded on demand, not pasted into `SKILL.md`.
- Source-aware: include origin, date, version, or rationale.
- Kept small enough to avoid context waste.

Avoid:

- Large unstructured dumps.
- Outdated external claims with no source note.
- Reference chains more than one or two levels deep.

## 6. Scripts

Scripts are valuable when they provide deterministic checks or transformations. They are harmful when they hide behavior that should be explicit.

Check:

- Script has clear CLI usage.
- Script fails safely and prints actionable errors.
- Script has minimal dependencies.
- Destructive actions require explicit confirmation.
- Network access, file writes, and shell execution are documented.
- Script output is machine-readable when useful.

## 7. Evals

Minimum eval suite:

- Positive trigger case.
- Negative trigger case.
- Happy path output case.
- Missing examples case.
- Missing references case.
- Conflict case.
- Bloat case.
- Script risk case.

Best-in-class evals:

- Are runnable locally.
- Use fixture skill folders.
- Define expected findings, severities, and minimum scores.
- Test that the reviewer does **not** over-flag good skills.
- Include regression cases for previously missed smells.

## 8. Bloat control

Recommended limits:

- `SKILL.md` ideally under 250 lines for most skills.
- Hard warning above 500 lines unless the skill is exceptional.
- Description should be concise and front-loaded.
- Detailed reference material belongs in `references/`.
- Examples can be in `examples/` if numerous.

Bloat symptoms:

- Repeated rules.
- Long background narrative.
- Embedded full API docs.
- Multiple unrelated workflows in one skill.
- Too many optional modes with no activation boundaries.

## 9. Conflict control

Check for:

- Internal contradictions: “always ask” and “never ask.”
- Tool conflicts: “never use scripts” and “run script first.”
- Scope conflicts: same skill claims multiple unrelated jobs.
- Hierarchy conflicts: attempts to override system, safety, or user instructions.
- Dependency conflicts: says no dependencies, but scripts import non-stdlib packages.

## 10. Maintainability

Best practices:

- Include version metadata.
- Include changelog for shared or production skills.
- Keep scripts tested and simple.
- Keep evals close to fixtures.
- Prefer clear file names and shallow directories.
- Make review output repeatable.
