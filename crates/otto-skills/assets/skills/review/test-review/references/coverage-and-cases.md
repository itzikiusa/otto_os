# Coverage & cases — the per-pass hunt list

Run one pass per lens; keep only that lens active and walk every test that touches the change.
This is a prompt, not a cage — a real weakness outside the list still counts. The recurring
question behind all of it: *would this test fail if the code broke?* (see `falsification.md`).

## 1. Does it test the change?
- Map each **new branch / new behavior** in the diff to the test that exercises it. Name any
  branch (`file:line`) with no test that fails when the branch breaks.
- **Bugfix:** is there a regression test that **fails on the old code**? Without it the bug can
  silently return. This is the single most common missing test in a fix PR.
- New public function / endpoint / event handler with **no test at all** for a non-trivial
  change → loud finding, not a buried nit.
- The change widened behavior (new param, new case) but the tests only cover the old shape.
- A guard / early-return / validation was added and **nothing drives it**.

## 2. Assertion strength (falsification)
- Truthiness oracles: `assert result`, `toBeTruthy`, `toBeDefined`, `assertNotNil` as the
  *whole* check — any non-failing return satisfies them.
- Asserting the **call**, not the **effect**: `expect(mock).toHaveBeenCalled()` /
  `verify(dep).save(any)` with no assertion on the resulting state or return value.
- Asserting "**no error thrown**" (`not.toThrow`, no `expect_err`) as the entire test — runs
  the code, checks nothing about *what it did*.
- **Echo oracle:** expected value computed by the same function/formula the code uses, or read
  back from the same store the code wrote to without transformation.
- **Over-broad snapshot:** a giant snapshot accepted wholesale — nothing specific is pinned, so
  a real regression hides inside an "updated snapshot."
- **Under-specified equality:** asserting only one trivial field of a rich result, leaving the
  field the change actually affects unchecked.
- Loose matchers where exact matters (`toContain`/`>= 0`/`any()` where the value is knowable).

## 3. Cases — negative, edge, error
- **Happy path only** — the default failure. Is the *failure* path the change introduced
  tested at all?
- New error path (`if err != nil`, `throw`, `Result::Err`, rejected promise) with no test that
  drives the error and asserts it surfaces correctly.
- Validation that **rejects** bad input — tested with bad input, or only good?
- Boundary values: empty / single-element / null / zero / negative / max / exactly-at-limit.
  The change touched a `<` or `<=`? Test both sides of it.
- The "second call" / re-entrancy / idempotency case the change made relevant.
- Error **message / type / code** the caller depends on — asserted, or just "it threw"?

## 4. Mocking — is it testing the mock?
- Assertions on `*-mock` / `data-testid="...-mock"` elements — verifies the stub, not the unit.
- The **unit under test is itself mocked** (or its core collaborator stubbed to return the
  exact value being asserted) → the test exercises nothing real.
- Mock so loose it would satisfy a **broken** implementation (returns the answer regardless of
  the input the code passes).
- **Partial mock** missing fields the real code consumes downstream → passes in test, breaks
  in integration; or worse, hides a real bug.
- Mock setup is >50% of the test, or mocks added "to be safe" without a named slow/external
  dependency they stand in for.
- After all the mocks, ask plainly: **what real production code path is left running?** If the
  answer is "almost none," the test proves almost nothing.

## 5. Coupling & flakiness
*Coupling (usually nit/minor — brittle, not broken):*
- Asserts on private methods, internal call order, or exact internal log strings.
- Asserts on DOM structure / CSS classes instead of user-visible behavior or roles.
- Breaks on a pure refactor that didn't change behavior (the test's real tell).

*Flakiness (usually major — trains the team to ignore red):*
- Real wall-clock `sleep`/`setTimeout` to "wait for" async work instead of awaiting/polling.
- Depends on **test execution order** or shared mutable state between tests (no isolation).
- **Unseeded randomness** (`Math.random`, `rand`, `uuid`) feeding an assertion.
- **Real time:** `Date.now()` / `now()` / `time.Now()` without a fixed/injected clock.
- **Real network / filesystem / external service** in a unit test; ports, real DB without a
  container, timezone/locale-dependent assertions.

## Completeness pass
- Which risky line of the change has no test that would fail if it broke? Name it.
- Which assertion did I accept without running the mutation? Run it now.
- Is the one solid-looking test actually pinned to a real oracle, or an echo?
- Did I confirm a "missing" case isn't covered in a sibling test file before flagging it?
