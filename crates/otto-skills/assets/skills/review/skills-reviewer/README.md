# Skills Reviewer Skill

A best-in-class Agent Skill for auditing other Agent Skills / `SKILL.md` packages.

It checks:

- Agent Skills structure and frontmatter validity
- Trigger clarity and scope boundaries
- Workflow quality and output shape
- Examples, references, and progressive disclosure
- `evals/evals.json` coverage
- Script safety and dependency hygiene
- Bloat, contradictions, and risky instruction patterns

## Package layout

```text
skills-reviewer/
├── SKILL.md
├── README.md
├── LICENSE
├── CHANGELOG.md
├── Makefile
├── agents/openai.yaml
├── scripts/
│   ├── skill_review.py
│   └── run_evals.py
├── schemas/
│   ├── evals.schema.json
│   └── review-report.schema.json
├── references/
│   ├── review-rubric.md
│   ├── smell-catalog.md
│   ├── evaluation-guide.md
│   ├── schema.md
│   └── reference-sources.md
├── examples/
│   ├── review-request.md
│   ├── expected-review-good.md
│   └── expected-review-bad.md
└── evals/
    ├── evals.json
    ├── eval_queries.json
    ├── files/
    └── cases/
```

## Install

Copy or unzip this folder into any Agent Skills-compatible location.

For Codex local authoring, common locations include repository-scoped `.agents/skills/` and user-scoped `$HOME/.agents/skills/`.

## Usage

Ask your agent:

```text
Use the skills-reviewer skill to review ./my-skill and tell me if it is ready to publish.
```

Or run the bundled static reviewer:

```bash
python3 scripts/skill_review.py ./path/to/skill --format markdown
python3 scripts/skill_review.py ./path/to/skill --format json
```

Run bundled evals:

```bash
python3 scripts/run_evals.py --evals evals/evals.json
```

## Review philosophy

This reviewer treats a skill as production agent infrastructure. Syntax matters, but reliability depends on trigger precision, focused instructions, examples, references, evals, and conflict control.

## Included quality assets

- `evals/evals.json` contains runnable regression fixtures.
- `evals/eval_queries.json` contains trigger/non-trigger prompt examples.
- `schemas/` contains JSON Schemas for eval metadata and machine-readable review output.
- `examples/` contains good and bad sample skill packages plus expected reports.
