# Good Questions vs. Nitpicks

Side-by-side examples of high-leverage questions and the low-value versions they
replace. Use this as a final filter before submitting your question set.

---

## The core test

A question is worth asking if **a senior PO would immediately understand why
it matters and have to think before answering it.**

A question is a nitpick if a senior PO would say "obviously X" in under three
seconds, or "why does the agent need to know that?"

---

## Side-by-side examples

### Scope

| High-leverage | Nitpick | Why the nitpick fails |
|---|---|---|
| "Does 'all users' include service accounts and API-only integrations, or just human login accounts?" | "Should the button be visible to admins?" | The story already says 'admin-only'; re-asking wastes the PO's time |
| "Is cancelling a subscription in scope for this story, or is that a separate story?" | "What happens if the user refreshes the page mid-flow?" | Teams handle transient browser states during build; not a PO decision |
| "Does this replace the existing bulk-edit flow or sit alongside it?" | "Should we support keyboard shortcuts?" | Nice-to-have UX polish the team decides; doesn't block build |

---

### Data

| High-leverage | Nitpick | Why the nitpick fails |
|---|---|---|
| "Is 'last login date' the browser-session timestamp or the SSO token issuance time? They can diverge." | "Which database table should we read this from?" | Engineering owns the implementation; not a PO question |
| "Should deleted records still appear in exports, or only live records?" | "Should timestamps be in UTC or local time?" | Reasonable default (UTC) exists; flag as assumption unless there's a stated user-facing requirement |
| "How long does audit log history need to be retained — 90 days, 1 year, indefinitely?" | "What format should the date column use in the CSV?" | Formatting convention; the team decides |

---

### UX

| High-leverage | Nitpick | Why the nitpick fails |
|---|---|---|
| "If the sync fails silently, should the user ever be notified, or is it fire-and-forget?" | "What color should the error message be?" | Visual design is the designer's call, not a PO decision |
| "When there are no results, should the page show an empty state with a CTA, or just a 'no data' message?" | "Should the modal have a close button in the top-right corner?" | Standard UI convention; no PO input needed |
| "Is there a confirmation step before the user permanently deletes their data, or is it immediate?" | "Should the button label say 'Delete' or 'Remove'?" | Copy is usually the team's call unless brand guidelines dictate it |

---

### Edge cases

| High-leverage | Nitpick | Why the nitpick fails |
|---|---|---|
| "If two team members approve the same request simultaneously, should one succeed and one get an error, or should both succeed?" | "What if the user has a very slow internet connection?" | Loading/timeout behavior is a UX standard the team applies; not a business rule |
| "If the payment provider is unreachable, should checkout be blocked entirely or fall back to manual processing?" | "What if the user types a comma in the amount field?" | Input validation is implementation-level; not a PO decision |
| "What should happen to in-progress orders when a campaign expires mid-checkout?" | "What if the user has 10,000 items in their cart?" | Performance optimization is an engineering concern unless the PO has a hard business limit |

---

### Dependencies

| High-leverage | Nitpick | Why the nitpick fails |
|---|---|---|
| "This story references the 'unified profile API' — is that already deployed, or does this ship depend on it?" | "Does QA need to sign off before we deploy?" | Internal process; the team knows its own workflow |
| "Does displaying billing history require legal review, given GDPR data visibility rules?" | "Should we write tests for this?" | Engineering standard; not a PO decision |

---

## Common structural anti-patterns

### Compound questions (split these)

**Bad:** "Should the export include deleted records, and if so, should it show them
differently than active ones, and does the format need to match the existing CSV?"

This bundles three separate decisions. Each answer is independent. Split into:
1. "Should the export include deleted records?"
2. "If yes, should deleted records be visually distinguished from active ones in the export?"
3. "Does the export format need to match the existing bulk CSV export?"

---

### Leading questions (rephrase these)

**Bad:** "Since users are likely to get confused if the error message is too long,
should we keep it under 20 words?"

This smuggles in the answer. The PO may not agree with the premise. Rephrase:
"Are there any guidelines for how error messages should be phrased or how long
they can be — brand guidelines, legal constraints, or UX patterns we should follow?"

---

### Over-defensive questions (drop these)

**Bad:** "Just to be safe — are there any accessibility requirements we should
know about?"

This signals you haven't checked the story or design system and are covering
yourself. Either check for accessibility requirements and state your finding,
or trust that the team's standard practices apply and omit the question.

---

## Quick gut-check

Before submitting, read each question and ask:

1. Would a **senior PO roll their eyes** at this? → Drop it.
2. Can a reasonable engineer **answer this themselves** during build? → Drop it.
3. Does the PO's answer **change a real decision** (scope, data model, user flow)? → Keep it.
4. Is the question **singular and clear** to a non-engineer? → Keep it.
