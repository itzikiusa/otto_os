# INVEST and Story Structure

Reference for the Jira Story Writer skill. Use this when evaluating whether a story
meets the quality bar or when coaching an author on how to improve one.

---

## INVEST — the six qualities

A well-formed user story satisfies all six INVEST criteria. Each is a lens, not a
checkbox — a story that is mostly strong on five dimensions but completely absent on
one is still a problem.

### Independent

The story can be developed and delivered without depending on another story being
done first. Dependencies are the primary cause of sprint disruption.

- **Check:** Can this be picked up by a developer without waiting for another card?
- **Common failure:** Two stories that must ship together (they are one story, or one
  is a prerequisite that should be a separate dependency card).
- **Fix:** Split or reorder so each story stands alone, or make the dependency explicit
  in the story text.

### Negotiable

The story describes an outcome and a boundary, not a specification. The details — the
*how* — are worked out collaboratively during refinement and development.

- **Check:** Are there implementation decisions locked in that engineering has not yet
  agreed to?
- **Common failure:** A story that reads like a tech spec ("Use a REST endpoint
  returning JSON with fields X, Y, Z").
- **Fix:** Express the requirement as a user-observable outcome. Let engineering propose
  the implementation.

### Valuable

The story delivers something a user or the business cares about on its own — not just
scaffolding or infrastructure. A story that only has value when combined with three
others is probably part of an epic, not a standalone story.

- **Check:** If this shipped today and nothing else did, would someone notice a
  positive change?
- **Common failure:** A purely technical card ("Migrate table X to schema Y") with no
  user-visible outcome stated.
- **Fix:** Frame the technical work around the user or business outcome it enables. If
  no user outcome exists, reconsider whether this belongs as a task on another story.

### Estimable

The team has enough information to make a rough size estimate. A story that cannot be
estimated usually means it is under-specified, too large, or both.

- **Check:** Could the team give a relative size in under five minutes of discussion?
- **Common failure:** "Improve performance" with no baseline, no target, and no
  constrained scope.
- **Fix:** Add a measurable target (e.g., "p95 response time ≤ 200 ms on the current
  load profile") and a scope boundary.

### Small

The story can be completed within a single sprint. Larger items are epics or features
and should be split.

- **Check:** Could one developer finish this in one to three days?
- **Common failure:** A story covering an entire user journey across multiple surfaces
  ("Users can register, verify email, set preferences, and complete onboarding").
- **Fix:** Split along user-observable milestones. Each split should itself be
  independently valuable.

### Testable

There is a clear way to verify the story is done. Untestable stories lead to
subjective "done" calls and undetected defects.

- **Check:** Could you write the test before any code is written?
- **Common failure:** "The experience should feel smooth and intuitive."
- **Fix:** Replace subjective language with observable outcomes (see
  `acceptance-criteria.md`).

---

## Anatomy of a great story

### Title

A good title is verb-led and outcome-focused. It describes what changes for the user,
not what the system does internally.

| Weak | Strong |
|------|--------|
| Player balance display | Show real-time balance to logged-in players |
| Notification refactor | Notify players by email when a withdrawal completes |
| Admin search fix | Let admins search players by partial email |

Target: ≤ 10 words. Someone skimming a backlog should understand the value without
opening the card.

---

### Value statement (user story)

The canonical form: **"As a `<persona>`, I want `<capability>`, so that `<outcome>`."**

All three parts carry weight:

- **Persona** — be specific. "Player" is weaker than "player who has requested a
  withdrawal." Precision helps AC authors know whose perspective matters.
- **Capability** — what the user can do or experience, not what the system will build.
- **So that** — this is the value anchor. It should describe a real-world benefit,
  not a restatement of the capability. "So that I know my money is on the way" is
  stronger than "so that I have visibility into my transactions."

If the persona is internal (an operator, a support agent, a risk analyst), say so.
Internal users have real needs too; the story should reflect whose workflow improves.

---

### Context / background

One to three sentences answering: **Why now? What is the current situation? What
problem does this solve?**

Context prevents the story from aging badly. When someone picks up a card six months
later, the context block is what tells them whether the original intent is still
relevant.

Keep it factual. Avoid design decisions ("we will store this in Redis") — those belong
in engineering notes, not story context.

---

### Acceptance criteria

The testable core of the story. See `acceptance-criteria.md` for full guidance.

Key principles at a glance:
- Each criterion tests one observable outcome.
- A developer and a tester reading independently would reach the same verdict.
- Aim for 3–6 criteria. More usually means the story should be split.
- Avoid "the system should" language — describe what the user sees or experiences.

---

### Scope

**In scope:** What this story covers. Be explicit even if it seems obvious.

**Out of scope:** What this story deliberately does not cover. Name at least one item.
An empty out-of-scope section is a warning sign — it often means the boundary was
not considered.

Good out-of-scope items:
- Deferred edge cases ("Bulk withdrawal is out of scope; handled in PROJ-456")
- Adjacent features ("Email notification for deposits is not part of this story")
- Future iterations ("Multi-currency support is planned for Q3; this story covers EUR only")

---

### Open questions

Questions that must be answered before or during development. Naming them explicitly
prevents silent assumptions from becoming defects.

Format: one question per line. Assign an owner if known. Close each one before
the story moves to "In Progress."
