# Test Design Heuristics

How to decide which cases are worth writing and which to leave out.

---

## The core question

For every candidate case, ask: **"Would a real user or business be harmed if this
behavior were wrong?"** If yes, write the case. If the answer is "technically no but
let's be safe," cut it.

---

## Heuristics for choosing cases

### 1. Equivalence partitioning

Group inputs and states into classes that the system treats identically. Write one case
per class, not one case per value.

**Example — a deposit amount field with minimum £10 and maximum £10,000:**

| Class | Representative value | Case needed? |
|-------|---------------------|--------------|
| Valid range | £50 | Yes — happy path |
| Below minimum | £5 | Yes — validation |
| Above maximum | £15,000 | Yes — validation |
| Exactly at minimum | £10 | Yes — boundary (see below) |
| Exactly at maximum | £10,000 | Yes — boundary |
| Zero | £0 | Only if zero is handled differently from below-minimum |
| Negative | -£1 | Only if negative is possible in the UI; otherwise skip |
| Non-numeric string | "abc" | Only if the field allows free text input |

The last two rows are candidates to **skip** if the input is a numeric stepper — the
UI prevents them and a test would be testing the browser, not the story.

---

### 2. Boundary values

Test the exact boundary and one step either side only when the boundary is a business
rule stated in the acceptance criteria — not as a general habit.

**Write boundary cases for:**
- Numeric thresholds with a business consequence (minimum deposit, maximum withdrawal)
- Date ranges with eligibility rules (e.g., "promotion ends 23:59 on the last day")
- Counts with caps (e.g., "maximum 3 payment methods per account")

**Skip boundary cases for:**
- Implementation details not mentioned in the story (e.g., database column length)
- Constraints enforced entirely by the UI component with no server-side rule

---

### 3. State preconditions

Many bugs live in state transitions. For each business entity involved, ask:

- What states can it be in when this action is attempted?
- Which states are valid preconditions, and which should be rejected?
- Can the entity move into an invalid state partway through a multi-step flow?

Write a case for each state that **changes the outcome**. Skip states that are
functionally identical from the story's perspective.

**Example — a user attempting to verify their email:**

| Account state | Expected outcome | Case needed? |
|---------------|-----------------|--------------|
| Email unverified | Verification email sent, link valid for 24 hours | Yes — happy path |
| Email already verified | Informational message, no new email sent | Yes — edge |
| Account locked | Error: account locked, no email sent | Yes — error |
| Link expired | Error: link expired, user can request a new one | Yes — error |
| Link already used | Error: link already used | Yes — error |

---

### 4. Permission and role checks

If the story involves different user roles, write one case per role boundary:

- The role that **should** succeed — this is part of the happy path.
- The role that **should be blocked** — this is a validation or error case.

Do not enumerate every role combination. Write the minimum that demonstrates the
boundary is enforced.

---

### 5. Failure modes that actually occur

Realistic errors worth testing:

- **Dependency unavailable** — the payment provider is down, the identity service
  times out. Test if the system surfaces a useful message, not a raw error.
- **Concurrent modification** — two users acting on the same entity simultaneously
  (e.g., an admin and the player both updating the same setting).
- **Stale or missing data** — the action targets a record that no longer exists.
- **Authorization boundary** — the user's session is valid but they lack the specific
  permission this action requires.

Unrealistic errors to skip:

- Database unavailable (infrastructure failure, not a story behavior)
- Network partition (unless the story explicitly covers offline handling)
- Malformed JWT (covered by authentication infrastructure, not this story)

---

## The "not over-defensive" rule

### What to skip

| Pattern | Why to skip |
|---------|-------------|
| Every invalid value in a field | One representative value per equivalence class is enough |
| SQL injection / XSS inputs | Security scanning handles this; it is not a story test case |
| Browser-enforced constraints | Testing `type="number"` rejecting "abc" tests the browser, not the story |
| Impossible state combinations | If the system cannot reach the state, the case adds no value |
| Duplicate coverage | If a case exercises no behavior not already covered by another, cut it |
| "Just in case" cases | Cases without a clear "would be harmed if wrong" answer |

### The ratio check

If your validation/error cases outnumber your happy-path cases by more than 3:1,
review each low-priority validation case. Most should be cut or merged.

---

## A useful mental test

Before writing a case, complete this sentence:

> "If this behavior were wrong, [user or business consequence]."

If you cannot finish it specifically and believably, cut the case.
