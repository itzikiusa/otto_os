# Reference Sources

These notes explain the public sources used to design this reviewer.

## Agent Skills format

- Agent Skills specification: `SKILL.md` frontmatter requires `name` and `description`; optional directories include `scripts/`, `references/`, and `assets/`; progressive disclosure encourages keeping `SKILL.md` concise and moving detail into referenced files.
- OpenAI Codex Agent Skills docs: skills package instructions, resources, and optional scripts; Codex uses skill descriptions for activation and recommends focused skills, imperative steps, and testing prompts against descriptions.
- ChatGPT Skills help center: skills are reusable, shareable workflows that can include instructions, examples, and code.

## Review principles encoded here

- Validate the formal spec first.
- Treat description quality as activation-critical.
- Prefer progressive disclosure over large prompt dumps.
- Require examples and evals because skills are reusable operational artifacts.
- Treat unsafe scripts and policy override instructions as release blockers.

## Source freshness

Checked: 2026-07-04.
