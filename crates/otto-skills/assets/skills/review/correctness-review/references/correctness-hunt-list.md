# Correctness hunt list — the bug taxonomy

One lens only: **does this code compute the right answer and take the right branch?** Run
one pass per section, keep only the correctness lens active, and walk the change line by
line against the *intended* behavior you fixed in Step 1. This list is a prompt, not a cage —
a real correctness bug outside it still counts. Anything off-lens (a leaked fd, a missing
index, a secret in a log) is **not yours** — note it in one line and move on.

For every suspect you flag here, you still owe a trace or a repro before it's a finding
(`trace-and-reproduce.md`). A long suspect list is worthless; a short *confirmed* one is the
product.

## 1. Logic & conditions

- Inverted boolean — `if (!ok)` where `if (ok)` was meant; a `return` on the wrong side.
- Comparison off by a step — `<` vs `<=`, `>` vs `>=`; `==` where `>=` was intended.
- Wrong connective — `&&` where `||` belongs (or vice versa); a guard that's too strict
  (rejects valid input) or too loose (admits invalid input).
- Operator precedence — `a && b || c`, `a + b << c`, `!a == b` parsing other than intended.
- De Morgan slip — negating a compound condition incorrectly (`!(a && b)` ≠ `!a && !b`).
- Short-circuit side effect — the right operand has a needed side effect that now never runs.
- A condition that's *always* true or *always* false (tautology / contradiction) — dead guard.

## 2. Off-by-one & boundaries

- Index / slice / substring bounds — `xs[len]`, `xs[i+1]` at the last iteration, `0..n` vs
  `0..=n`, inclusive/exclusive end confusion.
- Loop bounds — runs one too many or one too few times; `<= n` vs `< n`; counting from 0 vs 1.
- The first element and the last element — handled, or silently skipped/double-counted?
- The empty collection — does the loop body's assumption ("there's at least one") hold?
- Exactly at the limit — `== max`, `== min`, `== capacity`; the value on the fence.
- Pagination / chunking — last partial page, offset math, `+1`/`-1` in window sizing.

## 3. Null / None / nil / undefined / absent

- A value that can legitimately be absent reaching a deref / `unwrap()` / `!` / `.field` /
  index / call without a guard. **Trace the path where it *is* absent.**
- A lookup (`map[k]`, `find`, `get`) whose miss case isn't handled — the `nil`/`None`/`undefined`
  flows on and corrupts a later computation.
- Default-on-absent that silently changes the answer — missing field becomes `0`, `""`,
  `false`, or `now()` and the result is quietly wrong instead of erroring.
- `null` vs `undefined` vs absent-key vs empty-string treated as interchangeable when they
  aren't; `0`/`false`/`""` falsy-coerced where only true-absence was meant.
- Optional chaining that hides a real "this should never be null here" violation.

## 4. Branches & cases (the path never run)

- The `else` / fallthrough the author obviously didn't exercise.
- An enum/match/switch with a case left unhandled, or a `default`/`_` arm that silently
  swallows a case that needs distinct handling (and will swallow future cases too).
- Early `return` / `break` / `continue` that skips required follow-up logic (the update, the
  decrement, the append) that the rest of the function assumes ran.
- An exception/error path that leaves a half-updated result the happy path doesn't expect.
- A flag/branch combination that's reachable but was never considered (the 2×2 the author
  only tested 2 cells of).

## 5. Invariants & state transitions

- A field/relationship this change can now push out of sync (`count` vs `len(items)`, `total`
  vs the sum, `isOpen` vs the underlying handle, two caches that must agree).
- A state machine reaching a state it shouldn't (a transition newly enabled), or failing to
  reach one it must (a transition newly removed/guarded-out).
- Ordering assumption broken — code that relies on "A always runs before B", "this list is
  sorted", "this is called once", and the change makes that no longer true.
- Idempotency broken — a "run twice" path that now double-applies (double-charge,
  double-increment, double-insert).
- An assertion/precondition the function documents but the new path can violate.

## 6. Data & math

- Lossy conversion — `i64→i32`, `float→int`, `decimal→float`, narrowing that truncates a
  real value; silent wraparound on overflow.
- Rounding / truncation direction wrong (`floor` vs `round` vs `ceil`); banker's vs
  arithmetic rounding mismatch; money in floats.
- Float equality (`==`) where a tolerance is required; accumulation drift over a loop.
- Sign error — a subtraction reversed, an absolute value missing, a negative where positive
  was meant.
- Unit / scale mismatch — ms vs s, bytes vs KB, cents vs dollars, 0-based vs 1-based, radians
  vs degrees, percent vs fraction.
- Integer division truncation where a remainder mattered; modulo on negatives.

## 7. Copy-paste & stale references

- A pasted block still referencing the source's variable / index / field instead of the new
  one (`for j` body still using `i`; `b.x` where `a.x` was meant).
- A condition correct in the original location and wrong here (different invariant holds).
- A duplicated computation that drifted — two places compute "the same" value differently.
- A renamed concept updated in one place but not its twin (the producer changed, the consumer
  still assumes the old shape).

## How deep to go on each suspect

A suspect graduates to a finding only after you've traced it or reproduced it. If tracing
shows it's actually guarded upstream (the caller already null-checks, the value can't reach
that branch), **kill it** — don't report a guarded non-bug. Reachability is part of the
trace: a "bug" on an unreachable path is not a bug.
