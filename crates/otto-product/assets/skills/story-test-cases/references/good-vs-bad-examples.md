# Good vs. Bad Examples

Side-by-side comparisons of sharp, PO-readable cases against vague or over-defensive ones.

---

## Principle: one case, one behavior, one unambiguous outcome

A case is ready when a PO can read the expected result aloud and immediately know
whether the system passed or failed. If the expected result requires interpretation,
it needs rewriting.

---

## Example 1 — Deposit amount validation

**Story context:** Users can deposit between £10 and £10,000. Below-minimum deposits
should be rejected with a specific error message.

---

### Bad case

```
Title: Test deposit validation
Category: validation
Priority: medium
Preconditions: User is logged in
Steps:
  1. Navigate to deposit page
  2. Enter various invalid amounts
  3. Submit the form
Expected: The system should show appropriate error messages for invalid inputs
```

**Why it is bad:**

- "Various invalid amounts" — which ones? The developer cannot implement against this.
- "Appropriate error messages" — what message? A PO cannot approve this without
  knowing what "appropriate" means.
- One case bundles multiple behaviors that should be separate.
- The precondition is incomplete (no mention of account state, payment method).

---

### Good case

```
Title: Reject deposit below £10 minimum
Category: validation
Priority: high
AC refs: AC-3 ("Deposits must be between £10 and £10,000")
Preconditions:
  - User is logged in with a verified account
  - User has at least one active payment method on file
Steps:
  1. Open the Deposit page
  2. Enter £5 in the Amount field
  3. Click "Deposit"
Expected:
  - The deposit is not processed
  - The Amount field shows the inline error: "Minimum deposit is £10"
  - The user remains on the Deposit page
```

**Why it is good:**

- Title states the outcome, not the mechanism.
- One specific amount (£5 — a clear representative of "below minimum").
- Expected result is exact: the specific error message, where it appears, and what does
  not change (user stays on the page).
- PO can read it and immediately verify it in a demo.

---

## Example 2 — Permission boundary

**Story context:** Only users with the "Finance Manager" role can approve withdrawals.

---

### Bad case

```
Title: Test withdrawal approval permissions
Category: validation
Priority: high
Steps:
  1. Log in as various user types
  2. Attempt to approve a withdrawal
  3. Verify only authorized users can approve
Expected: Unauthorized users cannot approve withdrawals
```

**Why it is bad:**

- "Various user types" — which roles? How many cases is this actually?
- "Verify only authorized users can approve" — this is the test objective restated as
  the expected result. It says nothing concrete.
- No preconditions about what withdrawal is being approved.

---

### Good cases (two, not one)

```
Title: Finance Manager can approve a pending withdrawal
Category: happy
Priority: high
AC refs: AC-1
Preconditions:
  - A withdrawal of £200 is in "Pending Approval" status
  - The acting user has the "Finance Manager" role
Steps:
  1. Navigate to the Withdrawals queue
  2. Open the £200 pending withdrawal
  3. Click "Approve"
Expected:
  - The withdrawal status changes to "Approved"
  - The user sees a confirmation banner: "Withdrawal approved"
  - The withdrawal disappears from the Pending Approval queue
```

```
Title: Standard user cannot approve a withdrawal
Category: validation
Priority: high
AC refs: AC-1
Preconditions:
  - A withdrawal of £200 is in "Pending Approval" status
  - The acting user has the "Standard" role (not Finance Manager)
Steps:
  1. Navigate to the Withdrawals queue
  2. Open the £200 pending withdrawal
Expected:
  - The "Approve" button is not visible
  - The withdrawal detail is read-only
```

**Why they are good:**

- Each case tests exactly one role boundary.
- Preconditions are specific (the exact withdrawal, the exact role).
- Expected results are exact — "button is not visible" is falsifiable; "read-only" tells
  a developer what to build.

---

## Example 3 — Over-defensive case to cut

**Story context:** A search field accepts free-text input. Results update as the user types.

---

### Case to cut

```
Title: Search rejects SQL injection attempt
Category: validation
Priority: high
Steps:
  1. Enter "'; DROP TABLE users; --" in the search field
  2. Press Enter
Expected: The system does not drop the database table
```

**Why to cut it:**

- SQL injection is a security concern handled by parameterized queries and a security
  review, not a story-level test case.
- The story says nothing about SQL injection — this case is not traceable to any AC.
- "Does not drop the database" is not a behavior a PO approves; it is an infrastructure
  guarantee.
- If injection prevention is genuinely in scope, it belongs in a dedicated security
  story with a security testing approach, not here.

---

## Writing unambiguous expected results

| Vague | Sharp |
|-------|-------|
| "The system responds appropriately" | "The user sees 'Invalid date range' below the end-date field" |
| "An error is shown" | "A toast notification appears: 'Payment failed — please try again'" |
| "The record is updated" | "The Status field shows 'Active' and the Updated At timestamp reflects the current time" |
| "The user is redirected" | "The user is taken to the Dashboard page at /dashboard" |
| "The action is blocked" | "The Submit button is disabled; hovering shows tooltip 'You must accept the terms first'" |
| "The data is saved correctly" | "The user's display name in the top nav updates to 'Jane Smith' without a page reload" |

The test is on the right: a developer knows exactly what to build, and a PO knows
exactly what to check in a demo.
