# Coverage and Traceability

How to ensure every acceptance criterion is tested and no case tests something the
story does not promise.

---

## The two coverage rules

**Rule 1 — Every AC must have at least one case.**
If an acceptance criterion has no case, it will not be verified. Period. Before you
publish your case set, check each AC off against the cases you have written.

**Rule 2 — No case tests behavior the story does not promise.**
A case without an AC reference is a red flag. It either reveals a gap in the ACs (in
which case raise it as an open question) or it is out-of-scope speculation (in which
case cut it).

---

## Building the AC-to-case map

Before drafting cases, list every AC explicitly. Number them.

```
AC-1: A Finance Manager can approve a withdrawal in "Pending Approval" status.
AC-2: Approving a withdrawal changes its status to "Approved" and removes it from
      the queue.
AC-3: A user without the Finance Manager role cannot approve a withdrawal.
AC-4: If the payment provider rejects the payout, the withdrawal returns to
      "Pending Approval" and the Finance Manager is notified by email.
```

Then map each case to the AC(s) it exercises:

| Case title | Categories | ACs covered |
|------------|-----------|-------------|
| Finance Manager can approve a pending withdrawal | happy | AC-1, AC-2 |
| Standard user cannot approve a withdrawal | validation | AC-3 |
| Withdrawal returns to queue when payout provider rejects | error | AC-4 |
| Finance Manager receives email when payout is rejected | error | AC-4 |

After mapping, check:

- Every AC appears in the "ACs covered" column at least once. ✓
- No row lacks an AC reference. ✓

---

## When an AC maps to multiple cases

Some ACs require more than one case to be fully exercised. Common reasons:

- The AC describes a success path **and** an implicit failure path
  ("Users can reset their password" implies both success and the expired-link error).
- The AC involves multiple roles or states that each need a case.
- A boundary value creates two distinct outcomes at either side.

Split into separate cases. Do not bundle multiple behaviors into one case to hit an AC
with a single row.

---

## When a case has no AC

If you find yourself writing a case you cannot trace to an AC, one of these is true:

1. **The AC is missing from the story.** Raise it as an open question:
   > "Draft case 'X' has no matching AC. Should AC-N be added to cover [behavior]?"

2. **The case is out of scope.** The story does not promise this behavior. Cut the case.
   Do not test what was not agreed.

3. **The case is testing infrastructure, not the story.** (See "not over-defensive" in
   `test-design-heuristics.md`.) Cut it.

Never assign a case to the closest-sounding AC as a workaround. Mismatched traceability
is worse than honest coverage gaps.

---

## Coverage tiers

Not all coverage gaps are equal. Use priority to signal what matters most.

| AC type | Minimum coverage | Typical priority |
|---------|-----------------|-----------------|
| Primary success flow | One case per described flow | high |
| Explicit business constraint (min/max, required field) | One case showing enforcement | high |
| Role / permission rule | One case per role boundary | high |
| Stated error behavior | One case per named error | medium |
| Implicit / inferred behavior | Raise as open question first | low or skip |

---

## The traceability review checklist

Before finalising your case set, run through this list:

- [ ] All ACs numbered and listed.
- [ ] Every case has an `ac-refs` field with at least one entry.
- [ ] Every AC number appears in at least one `ac-refs` field.
- [ ] Cases with multiple AC refs each exercise a behavior from each referenced AC.
- [ ] No case tests a behavior that would require adding a new AC to justify it.
- [ ] The total case count is proportional to the story's complexity — neither
      suspicious in its brevity nor inflated with low-value cases.

---

## Handling missing or ambiguous ACs

If ACs are missing or ambiguous, do not guess. Produce a draft case set with clearly
marked gaps:

```
OPEN QUESTION (no AC): The story describes sending a confirmation email after deposit,
but does not state what happens if the email service is unavailable. Should AC-5 cover
this? Draft case: "Deposit succeeds even when confirmation email cannot be sent."
```

This surfaces the gap to the PO without inventing behavior. The PO can then decide
whether to add the AC, accept the implicit behavior, or defer it to another story.
