---
description: Guidelines (not strict rules) for writing an excellent RFC / proposal. Use to suggest a rewrite and to review one. Product/decision altitude — frame the problem, the options, and the recommendation; light on deep technical design. Audience is decision-makers and stakeholders.
---

# RFC Writer

These are **guidelines, not rigid rules** — adapt them to the proposal's size and
context. Use them to **suggest a rewrite** of an existing RFC and as a **review lens**
when assessing one. Keep the altitude at problem-framing and decision-making; an RFC is
about *what we should do and why*, with enough detail to decide — not a full technical
design doc.

> **Reference files live in `references/` and the ready-to-fill template is in
> `assets/` — both sit alongside this SKILL.md. Consult them as you work:**
> - `references/rfc-structure.md` — each section in depth with examples
> - `references/decision-records.md` — ADR-style decision capture and recommendation clarity
> - `references/anti-patterns.md` — RFC smells and how to fix them
> - `assets/rfc-template.md` — skeleton to fill in for a new or rewritten RFC

---

## Workflow

### 1. Read the source material completely

Ingest every available input — the draft RFC, linked pages, prior discussions, and any
decisions already in flight — before writing a word. Note what is present, what is
stated but unclear, and what is conspicuously absent.

### 2. Identify the decision being made

Locate the actual recommendation (it may be buried). If it is absent, that is the first
thing to surface. Everything else in the RFC exists to support or contextualize this one
decision.

### 3. Work through each section

Use `references/rfc-structure.md` as your depth guide. For each section, decide whether
the existing material:

- **Covers it well** — keep and polish.
- **Covers it partially** — strengthen with the patterns from the reference.
- **Is missing** — add a clear placeholder or ask the author to supply it.

Pay special attention to the options section: a proposal with only one option is not a
real RFC; it is advocacy. Surface the alternatives even if briefly.

### 4. Check for anti-patterns

Run through `references/anti-patterns.md`. Flag any smell that genuinely hinders the
ability to decide — do not flag cosmetic issues unless asked.

### 5. Output

**When suggesting a rewrite:** Produce a revised RFC using `assets/rfc-template.md` as
the skeleton. Preserve the author's substance and intent; restructure for clarity. Where
information is missing to decide, insert a clearly marked open question rather than
inventing a position.

**When reviewing:** Lead with what is strong. For each gap, name the specific section,
why it matters for the decision, and a concrete suggestion to address it. Keep the review
concise — a decision-maker should absorb it in two minutes.

---

## Section guide (summary)

| Section | Purpose |
|---|---|
| Problem statement | Make the pain and stakes concrete — why doing nothing is not acceptable |
| Motivation | Who is affected, what signals prompted this now |
| Goals | What success looks like, as outcomes not features |
| Non-goals | Explicit scope boundaries — as valuable as goals |
| Options | Real alternatives (including "do nothing") with honest trade-offs |
| Proposed decision | The recommendation, stated plainly, with the decisive rationale |
| Scope / Rollout / Backout | Who and what is affected; how it ships; how to undo it |
| Open questions | What still needs input, from whom, and by when |

See `references/rfc-structure.md` for depth on each section.

---

## Altitude

Favor clarity of the decision over implementation depth. Include technical detail only
where it changes the decision or surfaces a real risk; leave detailed design to follow-on
work. The audience is decision-makers and stakeholders, not only implementers.

When a trade-off discussion gets very long, summarize it in the RFC and offer a separate
deeper analysis document as an appendix.
