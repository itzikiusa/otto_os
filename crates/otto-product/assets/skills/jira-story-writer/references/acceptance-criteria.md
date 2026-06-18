# Acceptance Criteria Patterns

Reference for the Jira Story Writer skill. Use this when writing or reviewing
acceptance criteria (AC).

---

## The one rule

**A developer and a tester reading the same criterion independently must reach the
same verdict — pass or fail — without asking the author for clarification.**

If that is not true, the criterion is not good enough.

---

## Two main styles

### Given / When / Then (scenario-based)

Best when behaviour depends on a specific state or event. Forces the author to think
about preconditions, trigger, and expected outcome separately.

```
Given [a precondition that puts the system in a known state]
When  [the user takes a specific action or an event occurs]
Then  [the observable outcome]
```

**When to use it:**
- When the outcome depends on who is logged in, what data exists, or what happened before.
- When testing the criterion requires a specific setup step.
- When there are meaningful variations worth capturing as separate scenarios.

**Tips:**
- One Then per scenario. Two outcomes in a single Then means two scenarios.
- "Given the user is logged in as a VIP player" is a precondition; "Given the player
  exists in the database" is an implementation detail — keep preconditions at the
  user level.
- Avoid "And then" chains longer than two items; split into separate criteria.

---

### Checklist / bullet outcomes

Best when the outcome is simple and the precondition is obvious from context.
Easier to skim, less ceremony for straightforward requirements.

```
- The withdrawal confirmation email is sent within 60 seconds of approval.
- The email subject line reads "Your withdrawal of [amount] has been approved."
- Clicking "View details" in the email opens the transaction history page.
```

**When to use it:**
- For UI/content requirements where the trigger is implicit ("when the user does X").
- For non-functional requirements with a clear threshold (latency, uptime, character limits).
- When the scenario form would be repetitive and add no clarity.

---

## Good vs bad examples

### Example 1 — vague vs testable

**Bad:**
> The player should be informed when their withdrawal is processed.

Problems: "informed" is undefined (email? push? in-app?). "processed" is undefined
(approved? settled? bank-confirmed?). "Should be" is hedging language.

**Good:**
> When a withdrawal request moves to the "Approved" status, the player receives an
> email at their registered address within 5 minutes. The email contains the
> withdrawal amount, currency, and estimated settlement date.

---

### Example 2 — implementation in the AC

**Bad:**
> The system calls the `/v2/withdrawal/status` endpoint every 30 seconds and
> updates the Redis cache.

Problems: This is a design decision, not an acceptance criterion. Two equally valid
implementations could poll at 15 s or use webhooks. The AC has locked in an approach
that engineering has not agreed to.

**Good:**
> The player's withdrawal status on the dashboard reflects changes within 60 seconds
> of the status updating in the back office.

---

### Example 3 — missing edge cases

**Bad:**
> Players can search for other players by name.

Problems: What if the name is empty? What if there are 10,000 results? What if there
are no results?

**Good:**
> - Entering at least 3 characters in the name field returns a list of matching players.
> - Results are limited to 50 entries; a "Load more" link appears when additional
>   results exist.
> - Searching with fewer than 3 characters shows an inline message: "Enter at least
>   3 characters to search."
> - Searching a name with no matches shows: "No players found."

---

### Example 4 — compound criterion

**Bad:**
> The user can log in, see their balance, and withdraw funds.

Problem: This is three stories. One criterion cannot cover an entire flow.

**Good:** Split into separate stories, each with its own AC set.

---

## Non-functional acceptance criteria

Performance, security, and accessibility requirements are valid AC. They must be
equally concrete.

| Category | Weak | Strong |
|----------|------|--------|
| Performance | The page loads quickly | p95 load time ≤ 1.5 s on a 4G connection |
| Availability | The feature is reliable | The service handles up to 500 concurrent users without error rate exceeding 0.1% |
| Accessibility | The form is accessible | All form fields have associated labels; the form is keyboard-navigable; contrast ratio ≥ 4.5:1 |
| Security | Data is protected | Player PII is not logged; session tokens are not included in URLs |

---

## AC completeness checklist

Before finalising acceptance criteria, ask:

- [ ] Does each criterion have one observable outcome?
- [ ] Is every key edge case represented? (empty state, error state, boundary values)
- [ ] Are there non-functional requirements that belong here? (performance, security, a11y)
- [ ] Is the happy path covered?
- [ ] Is there at least one failure/error path covered?
- [ ] Do any criteria contain "should", "might", or "ideally"? (replace with definitive language)
- [ ] Would a developer know exactly when they are done? Would a tester know exactly what to test?
