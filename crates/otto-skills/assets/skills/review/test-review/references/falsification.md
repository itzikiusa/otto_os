# Falsification — would this test fail if the code broke?

This is the whole skill in one question. Coverage tools tell you a line **ran**.
Falsification asks whether anyone **checked the result**. A test that runs a line but asserts
nothing meaningful about its effect is a line of coverage and zero protection.

## The move

For each test that touches the change, **mentally mutate the production code** and predict
whether the test goes red. If it stays green under a mutation that genuinely breaks behavior,
the test does not test that behavior — that is your finding, and the mutation is your proof.

This is "mutation testing" done by hand, scoped to the change. You don't need a tool; you need
to imagine the bug and check the assertion.

## A catalogue of mutations to try

Apply the ones that fit the code under test. If the test survives the mutation, write the
finding citing the exact mutation.

| Mutation | What it catches |
|---|---|
| **Invert a condition** (`if x` → `if !x`, `>` → `>=`) | Tests that never drive both sides of the branch. |
| **Return the input unchanged** (skip the transform) | Tests that don't assert on the *transformed* value. |
| **Return a constant / empty** (`return []`, `return null`, `return 0`) | Assertions like `assert result` / `toBeTruthy` that any non-failing return satisfies. |
| **Drop the side effect** (delete the DB write, the emit, the log, the cache set) | Tests that assert the function returned, never that the effect happened. |
| **Drop an `await` / make it sync** | Async tests that don't actually await the result before asserting. |
| **Swallow the error** (delete `throw` / `return err`) | "Error path" tests that don't assert the error is actually raised/returned. |
| **Off-by-one the boundary** (`<= n` → `< n`) | Tests that only use a value far from the boundary. |
| **Skip the validation** (let the bad input through) | Negative-case tests that assert success but never that *bad* input is rejected. |
| **Return the bug's old behavior** (for a bugfix) | Regression tests that don't actually fail on the pre-fix code. |
| **Corrupt one field of the output** | Snapshot/equality tests so broad nothing specific is pinned — or so narrow the field isn't checked. |

## Worked examples

**Weak — survives mutation:**
```ts
test('applies discount', () => {
  const cart = applyDiscount(cart, '10OFF');
  expect(applyDiscount).toHaveBeenCalled();   // ← asserts the call, not the price
});
```
Mutation: make `applyDiscount` return the cart unchanged. Test **still passes**. It guards
nothing. → Finding: assert the resulting `cart.total`, not that the function ran.

**Strong — fails under mutation:**
```ts
test('applies 10% discount to a $100 cart', () => {
  const cart = applyDiscount({ total: 100 }, '10OFF');
  expect(cart.total).toBe(90);                // ← asserts the oracle
});
```
Mutation: return the cart unchanged → `total` is 100, `toBe(90)` fails. The test earns its
keep.

**Bugfix without a real regression test:**
```ts
// fix: empty email should be rejected
test('submits the form', async () => {
  const res = await submitForm({ email: 'a@b.com' });
  expect(res.ok).toBe(true);                  // ← only the happy path
});
```
Mutation: revert the fix (accept empty email). Test **still passes** — it never sends an empty
email. The bug can return undetected. → Finding (blocker for a bugfix): add a test asserting
`submitForm({ email: '' })` is rejected; it must fail on the pre-fix code.

## The oracle test

Every assertion needs a **real oracle** — an independent statement of the correct answer, not
one the test computed the same way the code does.

Red flags that there is no oracle:
- The expected value is produced by calling the same function/helper the code uses.
- The expected value is read back from the same source the code wrote to, with no transform.
- The assertion is `assert result`, `toBeTruthy`, `toBeDefined`, `not.toThrow()` as the
  *entire* check.
- A snapshot was accepted wholesale without anyone confirming it's correct.

If the test can't be wrong when the code is wrong, it isn't an oracle — it's an echo.

## Reporting a falsification finding

State the mutation explicitly so the author can verify it in seconds:

> `cart.test.ts:42` only asserts `applyDiscount` was called. **Mutation:** if `applyDiscount`
> returned the cart unchanged, this test still passes — it never checks `cart.total`. **Fix:**
> assert `cart.total === 90` for a `$100` cart with `10OFF`.

The mutation *is* the evidence. A falsification finding without a named mutation is just a
vibe — supply the mutation or downgrade it to a question.
