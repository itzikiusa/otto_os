# Jira Story Template

Copy this template and fill in each section. Guidance is in `[square brackets]` —
delete the guidance text before pasting into Jira. Sections marked *(optional)* can
be omitted if they are genuinely not applicable; do not leave them blank.

---

## [Story title — verb-led, outcome-focused, ≤ 10 words]

---

### Value statement

As a **[specific persona — e.g., logged-in player, support agent, back-office operator]**,
I want **[capability — what they can do or experience]**,
so that **[concrete benefit or outcome — not a restatement of the capability]**.

---

### Context / background

[1–3 sentences: why this work is needed now, what the current situation is, and what
problem or opportunity it addresses. Avoid implementation decisions here — those
belong in engineering notes. Example: "Currently, players receive no confirmation
when a withdrawal is approved, which drives a high volume of support contacts asking
for status updates."]

---

### Acceptance criteria

[Use Given/When/Then for scenario-dependent behaviour. Use bullet outcomes for
simpler, context-independent requirements. Aim for 3–6 criteria. Each criterion
must be independently verifiable — a developer and tester would agree on pass/fail
without asking the author. Delete any guidance lines before publishing.]

**Scenario: [happy path or primary scenario name]**
- Given [precondition]
- When [trigger or user action]
- Then [observable outcome]

**Scenario: [edge case or error path]**
- Given [precondition]
- When [trigger]
- Then [observable outcome]

[Or bullet style for simpler criteria:]
- [Observable outcome 1]
- [Observable outcome 2]
- [Observable outcome 3]

---

### In scope

[List what this story explicitly covers. Be specific. Example:]
- [Deliverable 1]
- [Deliverable 2]

---

### Out of scope

[List what this story deliberately does not cover. Naming the boundary is as
important as naming the scope. At least one item is expected. Example:]
- [Deferred item or adjacent feature — note the card or epic where it lives if known]
- [Future iteration]

---

### Open questions

[Questions that must be resolved before or during development. Leave none implicit.
Assign an owner where possible. Remove this section if there are genuinely no
open questions before the card moves to In Progress.]

- [ ] [Question] — Owner: [name], Resolve by: [date or milestone]
- [ ] [Question] — Owner: [name]

---

### Notes *(optional)*

[Any additional context that does not fit the sections above: links to designs,
related tickets, relevant data from research, or a known constraint that engineering
should be aware of. Keep this section factual; do not include implementation decisions.]
