---
description: Guidelines (not strict rules) for writing an excellent Jira story. Use to suggest a rewrite and to review an existing story. PO-level — frame value and acceptance, stay out of technical implementation.
---

# Jira Story Writer

These are **guidelines, not rigid rules** — adapt them to the story's context and
the team's maturity. Use this skill two ways:

- **Rewrite mode** — given a rough story or bullet-point notes, produce a clean,
  paste-ready story the PO can drop straight into Jira with minimal edits.
- **Review mode** — given an existing story, flag what is already strong, call out
  specific gaps, and suggest targeted improvements.

Stay at the **Product Owner altitude** throughout: describe the *what* and the *why*.
Leave the *how* entirely to engineering.

> **Supporting files live alongside this SKILL.md. Consult them as you work:**
> - `references/invest-and-structure.md` — INVEST in depth + story anatomy
> - `references/acceptance-criteria.md` — patterns for crisp, testable AC
> - `references/anti-patterns.md` — story smells and fixes
> - `assets/jira-story-template.md` — ready-to-paste output template

---

## What a strong story has

| Element | What makes it good |
|---------|-------------------|
| **Title** | Verb-led, outcome-focused, ≤ 10 words. Who benefits is implicit or stated. |
| **Value statement** | "As a `<persona>`, I want `<capability>`, so that `<outcome>`." The *so that* is the point — keep it concrete and benefit-oriented. |
| **Context / background** | Enough of the *why* and the current situation that someone outside the conversation understands the motivation. One to three sentences. |
| **Acceptance criteria** | The heart of the story. Each criterion is observable and unambiguous — a developer and a tester would independently agree whether it is met. |
| **Scope** | What is **in** scope and, just as importantly, **out** of scope. Naming boundaries is the single most effective way to prevent rework. |
| **Open questions** | Anything that must be resolved before engineering starts, named explicitly rather than left implicit. |

See `references/invest-and-structure.md` for full anatomy guidance and INVEST
quality checks.

---

## Acceptance criteria

Good AC is the most common failure point. Each criterion must be:

- **Observable** — someone can actually check it
- **Unambiguous** — two readers reach the same verdict
- **Bounded** — it describes one thing, not three

Given/When/Then is a strong default. Bullet outcomes ("The system shows…", "The
user receives…") work well for simpler cases. See `references/acceptance-criteria.md`
for patterns, good/bad examples, and when to use each style.

---

## Keep out of the technical weeds

Do not prescribe database schemas, class designs, API contracts, or implementation
steps. If a technical constraint genuinely shapes the story — a required third-party
integration, a hard performance limit, a compliance rule — state it as a constraint
or acceptance criterion, not as a design decision. Engineering owns the *how*.

---

## Rewrite mode — workflow

1. **Read the input completely.** Understand the author's real intent before touching a word.
2. **Identify the core value.** Who benefits and what outcome do they get?
3. **Draft the value statement.** If persona/outcome are missing, infer what you can; mark what you cannot as open questions.
4. **Write tight acceptance criteria.** Each criterion tests exactly one observable outcome. Aim for 3–6 criteria; more often signals the story should be split.
5. **Draw the scope boundary.** Name at least one explicit out-of-scope item — it signals the boundary was considered, not just forgotten.
6. **Check against INVEST** (see `references/invest-and-structure.md`). If the story is not independently estimable, suggest a split.
7. **Fill `assets/jira-story-template.md`.** That template is the deliverable — preserve the author's intent, sharpen the language, do not invent requirements.

---

## Review mode — workflow

1. **Lead with what is strong.** Name at least one thing the story does well.
2. **Assess each structural element** against the table above. For each weakness, quote or reference the specific text rather than giving generic advice.
3. **Check the anti-pattern catalogue** in `references/anti-patterns.md`. Name the pattern when you spot it.
4. **Give actionable suggestions.** "Change 'the system should be fast' to a specific latency threshold in the acceptance criteria" is useful; "be more specific" is not.
5. **Stay constructive.** You are improving a teammate's story, not grading it. Flag deviations only when they actually weaken the story.

---

## Quality bar

- **Product altitude only.** If you find yourself writing about architecture, databases, or deployment, stop and refocus.
- **No invented requirements.** Where you cannot infer something, mark it as an open question.
- **Specific over generic.** Every gap you name must cite the story text or point to something that is absent.
- **One coherent pass.** Your output should be ready to paste into Jira or a Confluence comment.
