# Test Case Template

Use this structure for every case. Fill in every field. Do not leave fields blank —
write "None" if there are no preconditions, or "N/A" if a field genuinely does not apply.

---

## Template

```
### TC-[N]: [Title — short, specific, outcome-oriented]

| Field | Value |
|-------|-------|
| Category | happy / validation / error / edge |
| Priority | high / medium / low |
| AC refs | AC-[N], AC-[N] |

**Preconditions**
- [State that must hold before the steps begin — account state, flags, data present]
- [Add one bullet per condition. Write "None" if there are no preconditions.]

**Steps**
1. [Action in plain language a PO can read and follow in a demo]
2. [Next action]
3. [Continue until the triggering action is complete]

**Expected**
- [What the user sees or experiences]
- [What the system records or changes — be specific about field values, messages,
   page destinations, emails sent, statuses updated]
- [Add one bullet per observable outcome. "The system responds correctly" is not acceptable.]
```

---

## Guidance on each field

**Title.** State the outcome, not the mechanism. Prefer:
- "Reject deposit below £10 minimum" over "Test minimum deposit validation"
- "Finance Manager can approve a pending withdrawal" over "Happy path approval"
- "Show error when payout provider is unavailable" over "Error handling test"

**Category.**
- `happy` — the primary success flow; the user gets what they came for
- `validation` — an input or business rule is violated; the user receives a clear rejection
- `error` — a realistic system or dependency failure; the user receives a useful message
- `edge` — a boundary or combination that changes behavior in a meaningful way

**Priority.**
- `high` — if this fails in production, a user or the business is materially harmed
- `medium` — important to cover; a failure would be noticed but not immediately critical
- `low` — worth testing once; low user/business impact

**AC refs.** Every case must reference at least one acceptance criterion. If you cannot
name one, raise the case as an open question rather than publishing it.

**Preconditions.** The state the world must be in before step 1. Be specific:
account status, existing data, active flags, user role. A developer reading this should
know exactly how to set up the test environment.

**Steps.** Actions only. Do not include assertions in the steps — those belong in
Expected. Write at the level of "what a PO would click through in a demo," not at the
level of API calls or database queries.

**Expected.** Concrete and falsifiable. Every item must be something that can be
observed and agreed upon by a PO, a developer, and a tester without further
interpretation. Name the exact message text, the exact status value, the exact page
the user lands on.

---

## Filled example — Deposit feature

**Story context:** Users can deposit between £10 and £10,000 using a saved payment
method. Deposits below the minimum are rejected with an inline error.

**Acceptance criteria:**
- AC-1: A verified user with a saved payment method can submit a deposit between £10 and £10,000.
- AC-2: Deposits below £10 are rejected with an inline error on the Amount field.
- AC-3: Deposits above £10,000 are rejected with an inline error on the Amount field.
- AC-4: A confirmed deposit triggers a confirmation email to the user.

---

### TC-1: Verified user deposits £50 successfully

| Field | Value |
|-------|-------|
| Category | happy |
| Priority | high |
| AC refs | AC-1, AC-4 |

**Preconditions**
- User is logged in with a verified email address
- User has one active saved payment method (Visa ending 4242)
- User's account is in "Active" status

**Steps**
1. Open the Deposit page
2. Confirm the saved payment method shown is Visa ending 4242
3. Enter £50 in the Amount field
4. Click "Deposit Now"

**Expected**
- A success banner appears: "Your deposit of £50 is being processed"
- The user is taken to the Transaction History page
- The deposit appears in the transaction list with status "Processing"
- A confirmation email is sent to the user's registered address within 1 minute

---

### TC-2: Reject deposit below £10 minimum

| Field | Value |
|-------|-------|
| Category | validation |
| Priority | high |
| AC refs | AC-2 |

**Preconditions**
- User is logged in with a verified email address
- User has one active saved payment method

**Steps**
1. Open the Deposit page
2. Enter £5 in the Amount field
3. Click "Deposit Now"

**Expected**
- The deposit is not submitted
- The Amount field shows the inline error: "Minimum deposit is £10"
- The user remains on the Deposit page
- No transaction is created

---

### TC-3: Reject deposit above £10,000 maximum

| Field | Value |
|-------|-------|
| Category | validation |
| Priority | high |
| AC refs | AC-3 |

**Preconditions**
- User is logged in with a verified email address
- User has one active saved payment method

**Steps**
1. Open the Deposit page
2. Enter £15,000 in the Amount field
3. Click "Deposit Now"

**Expected**
- The deposit is not submitted
- The Amount field shows the inline error: "Maximum deposit is £10,000"
- The user remains on the Deposit page
- No transaction is created

---

### TC-4: Deposit succeeds at the exact £10 minimum boundary

| Field | Value |
|-------|-------|
| Category | edge |
| Priority | medium |
| AC refs | AC-1 |

**Preconditions**
- User is logged in with a verified email address
- User has one active saved payment method

**Steps**
1. Open the Deposit page
2. Enter £10 in the Amount field
3. Click "Deposit Now"

**Expected**
- A success banner appears: "Your deposit of £10 is being processed"
- The user is taken to the Transaction History page
- The deposit appears with status "Processing"
