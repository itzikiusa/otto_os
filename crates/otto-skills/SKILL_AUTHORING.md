# Otto bundled skills — authoring contract

This crate bundles Otto's curated skill library, organized by category and surfaced in the
app's **Settings → Skills** panel (manual install/update — never auto-injected). Skills are
seeded into the Otto **Library** on user request, materialized into agent sessions, and the
**review** / **product** categories also drive feature lenses.

## Directory layout

```
assets/skills/<category>/<skill-name>/
  SKILL.md            # required — frontmatter + body
  references/         # optional — loaded into context as needed
  assets/             # optional — output templates the skill fills in
  scripts/            # optional — executable helpers (.sh gets +x on seed)
```

- `<category>` ∈ `product | project | development | review | design`.
- `<skill-name>` is **globally unique** (the Library is flat by name); `kebab-case`.
- Exemplar to match: **`review/grill`** — read it before authoring.

## Completeness requirement (MANDATORY — a SKILL.md alone is not enough)

Every skill MUST be a real multi-file skill. A lone SKILL.md is rejected.

- **`references/` — required.** At least one substantive file carrying the skill's depth:
  the detailed checklist / catalogue / heuristics / good-vs-bad examples that don't belong
  in the scannable SKILL.md. Real content, not a stub.
- **`assets/` — required when the skill produces an artifact.** A fill-in output/report
  template the skill populates (findings report, overview doc, plan, etc.), tailored to
  THIS skill — model it on the exemplar but make it yours.
- **`scripts/` — required when a deterministic helper genuinely speeds the work.** A real,
  runnable script (bootstrap discovery, pattern scan, scaffold, validation). It must do
  something useful and be honest about its limits ("hints, not findings"). **No token
  scripts** — omit `scripts/` rather than ship a placeholder. `.sh` gets +x on seed.

SKILL.md must *reference* each bundled file so the agent knows when to load/run it. The
test of "properly set up": could someone do excellent work from this skill **without**
already knowing the domain? If the depth lives only in your head and not in the files, it
isn't done.

## Frontmatter (required)

```yaml
---
description: <one or two sentences — what it does + when to use it. This is the selector signal; make it specific.>
category: review            # one of the five
version: 1                  # integer; bump when content changes (drives Settings drift detection)
---
```

The dir name is the skill's name (no `name:` field — matches Otto's existing product skills).

## Quality bar (match the existing product skills + grill)

1. **A clear method, not a vibe.** Numbered workflow / passes. The reader knows exactly what
   to do in what order.
2. **Anti-patterns table.** Name the failure modes ("X | why it fails").
3. **Explicit quality bar.** One short section: what "great" looks like for this skill.
4. **Evidence / specificity.** Cite, locate, show — never "be more careful."
5. **References carry the long lists.** Keep SKILL.md scannable; push checklists/examples
   into `references/`, and a fill-in `assets/` template where the skill produces an artifact.
6. **Right-sized, not over-defensive.** High-leverage guidance only; no filler.
7. **Tone matches Otto's product skills:** direct, senior, constructive.

## The five categories (target catalog)

- **product** — story/PRD/discovery & domain (existing 7 in `otto-product` + `domain-modeling`, `to-prd`, `ubiquitous-language`).
- **project** — planning/coordination (`triage`, `plan-work`, `to-issues`, `estimate`, + existing `story-task-breakdown`).
- **development** — building (`tdd`, `implement`, `systematic-debugging`, `verification`, `resolve-merge-conflicts`, `refactor`).
- **review** — code-review lenses (`grill`, `correctness-review`, `security-review`, `performance-review`, `architecture-review`, `test-review`, `devex-review`).
- **design** — software/UX/architecture design (`codebase-design`, `design-it-twice`, `api-design`, `visual-design-review`, `architecture-diagram`).

## Source material to synthesize (take the best, write Otto-native)

- **mattpocock/skills** (`skills/engineering`): grill, codebase-design, domain-modeling,
  diagnosing-bugs, tdd, improve-codebase-architecture, triage, to-prd, to-issues, implement.
- **obra/superpowers**: systematic-debugging, test-driven-development,
  verification-before-completion, requesting/receiving-code-review, writing-plans.
- **garrytan/gstack**: design-review, devex-review, design-consultation, investigate, careful, guard, diagram.

Take the **methodology**, not the files. gstack skills are bound to their own runtime
(preambles, binaries, telemetry) — strip all of that; write clean, self-contained Otto skills.
