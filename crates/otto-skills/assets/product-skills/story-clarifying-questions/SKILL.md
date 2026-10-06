---
description: Surface the fewest, sharpest clarifying questions that make a story unambiguous. High-leverage only, categorized, each with why it matters. Not over-defensive.
---

# Story Clarifying Questions

Your goal is a story so clear that a developer could implement it and a tester could
verify it without guessing. You get there by asking the **smallest set of
high-leverage questions** — not an exhaustive checklist.

## Before you write a single question

1. **Read the story end-to-end.** Fully. Twice if needed.
2. **Mark what is already answered.** Never ask about something the story settles.
3. **List the open uncertainties.** Anything that could cause a developer to make a
   different choice than the PO intended.
4. **Apply the decision-unblocking test** (see `references/question-heuristics.md`):
   for each uncertainty, ask "what decision does this answer?" If you can't name one,
   discard it.
5. **Reduce to the minimum set.** Group overlapping questions; combine only where
   the answer is genuinely singular. Aim for 3–8.

## How to write each question

- **Plain language.** Phrase for a non-engineer PO, not a developer reading the ticket.
- **Singular.** One question, one answer. If you find yourself writing "and" or "or"
  in the question, split it.
- **Forward-looking.** Ask what should happen, not what the developer suspects.
- **Not leading.** Don't embed the answer you want.

Consult `references/categories.md` for the taxonomy and examples.
Consult `references/good-vs-nitpick.md` before finalising — each question should
pass the "would a senior PO roll their eyes at this?" test.

## For every question, capture three fields

- **text** — the question itself.
- **rationale** — *why it matters*: the decision, scope boundary, or risk it resolves.
  One or two sentences.
- **category** — one of `scope` | `data` | `ux` | `edge-case` | `dependency` | `other`.

Use `assets/questions-template.md` as the output format.

## Anti-patterns to avoid

| Anti-pattern | Why it fails |
|---|---|
| Re-asking what the story already states | Signals you didn't read it; erodes trust |
| Bundling multiple questions into one | Forces the PO to answer a compound; creates ambiguity |
| Leading questions ("Should it show X, as that seems best?") | Smuggles a design decision |
| Hypothetical edge cases nobody will hit | Noise; devalues the real questions |
| Implementation trivia ("Which table should we use?") | Engineering owns the how |
| Covering yourself ("Just to be safe…") | Over-defensive; not a PO concern |

## Quality bar

A great output feels like the minimum the PO must answer to make the story safe to
build. If the PO can answer all questions in 15 minutes and the story is then
unambiguous, you've succeeded. If the PO needs to schedule a meeting, you asked too
many or the wrong ones.
