# Sweep checklist — what to hunt in each pass

Run one pass per lens. Within a pass, keep only that lens active and walk the change line
by line. This list is a prompt, not a cage — a real defect outside the list still counts.

## 1. Correctness
- Inverted/lazy boolean, wrong comparison (`<` vs `<=`), wrong operator precedence.
- Off-by-one in indexing, slicing, ranges, loop bounds.
- Broken invariant: a field two functions away that this change now violates.
- State machine: a transition that's now reachable but shouldn't be (or unreachable but needed).
- Copy-paste left a wrong variable / stale condition.
- The case the author obviously never ran (the `else`, the empty collection, the retry).

## 2. Edge cases & inputs
- Empty, single-element, null/None/nil, zero, negative, max value, overflow.
- Duplicate input, out-of-order input, the second invocation, re-entrancy.
- Unicode / very long / whitespace-only strings; locale, timezone, DST, rounding, float `==`.
- Boundary exactly at the limit (`>= min` vs `> min`).

## 3. Error & failure handling
- Swallowed error (`catch {}`, `_ = err`, ignored Result/Promise rejection).
- `unwrap()`/`!`/`as!`/bare `except:`/`panic` on a value that can legitimately be absent.
- Partial failure leaves inconsistent state; no rollback/compensation.
- Error loses context (wrapped without the cause, or replaced with a generic message).
- Retry without idempotency; timeout missing; failure path leaks a resource.

## 4. Concurrency & ordering
- Check-then-act (TOCTOU), read-modify-write that isn't atomic.
- Shared mutable state without a lock; lock held across an await/IO; lock-ordering deadlock.
- Assumes ordering between async tasks / events / callbacks that isn't guaranteed.
- Cancellation/signal not handled; goroutine/task started and never joined.

## 5. Resources & lifecycle
- File/socket/connection/cursor opened and not closed on every path (incl. error paths).
- Unbounded cache/queue/slice growth; missing eviction or limit.
- Subscription/listener/timer added without removal; goroutine/task/thread leak.
- Temp files / locks not cleaned up.

## 6. Security
- Untrusted input → SQL/shell/HTML/log/path/redirect sink without escaping or validation.
- AuthZ: does this endpoint/action check the caller may do it? AuthN bypass?
- Secrets in source, logs, error messages, or URLs; PII logged.
- Deserialization of untrusted data; SSRF via user-controlled URL; path traversal via `..`.
- Crypto misuse (static IV, weak hash for passwords, `Math.random` for tokens).

## 7. Performance
- Query in a loop (N+1); fetch inside iteration that could be a join/batch.
- Re-computing or re-fetching a value already in hand; needless clone/copy/serialize.
- Missing index for a new query predicate; full scan on a large table.
- Accidental O(n²) (nested scan, `list.contains` in a loop); blocking call on a hot path.

## 8. API & contract
- Changed function signature / struct field / JSON shape / status code consumers rely on.
- Changed default value or implicit behavior; removed or renamed a public name.
- DB migration that isn't backward-compatible with the currently-deployed code.
- Event/topic schema change without versioning.

## 9. Types & data modeling
- A type that admits an illegal state (two booleans where an enum belongs).
- Stringly-typed where a type exists; nullable that should be required; magic numbers.
- Lossy conversion (i64→i32, float→int), silent truncation.
- An enum/match/switch with a case left unhandled (or a `default` that hides new cases).

## 10. Tests
- Does a test exercise the *new* behavior — and its failure modes, not just the happy path?
- Assertion is weak (`assert result` / `toBeTruthy`) or checks the mock, not the behavior.
- Test passes even if the implementation is wrong (no real oracle).
- No negative/edge/error case; flakiness (time, ordering, network, randomness).

## 11. Docs & comments
- A comment that now lies about the code; a stale doc-string; a misleading name.
- Public/behavioral change with no doc or changelog; a `TODO`/`FIXME` shipped as done.

## 12. Consistency with this codebase
- A new pattern where an established one already exists (logging, errors, config, DI).
- Naming/structure that breaks local convention.
- Logic duplicated that already lives in a shared helper.
