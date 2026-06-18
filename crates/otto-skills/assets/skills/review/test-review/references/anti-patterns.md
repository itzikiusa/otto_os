# Test anti-patterns — the smell, why it fails, the fix

These are the named failure modes to hunt. Each row: the smell, the bug it lets through, and
the concrete fix to put in the finding. The unifying test is always falsification — *would
this fail if the code broke?* (`falsification.md`).

| Anti-pattern | Why it fails | The fix |
|---|---|---|
| **Happy-path-only** | The change's error/edge branch is never driven; that branch can break and stay green. | Add a test that exercises the failure path and asserts the error/rejection/empty result. |
| **Truthiness assertion** | `assert result` / `toBeTruthy` passes for almost any non-failing return — including a wrong one. | Assert the specific expected value (the oracle), not mere existence. |
| **Asserting the call, not the effect** | `expect(mock).toHaveBeenCalled()` passes even if the call did the wrong thing or its result was ignored. | Assert the resulting state / return value the call was supposed to produce. |
| **"No error thrown" as the whole test** | Runs the code, checks nothing about what it did; any silent-wrong result passes. | Assert the actual output or side effect, not just absence of an exception. |
| **Echo oracle** | Expected value is computed the same way the code computes it (or read back unchanged) — both wrong together, test still green. | Hand-write the expected value independently, or assert a property the code can't trivially satisfy. |
| **Testing the mock** | Asserts on a stub (`*-mock`) or on a collaborator that was stubbed to return the asserted value — no real code runs. | Unmock the unit under test; assert real behavior. Mock only the slow/external edge. |
| **Mocking the unit under test** | The thing being tested is replaced by a stub; the test verifies the stub, not the code. | Never mock the subject. Inject and stub its *dependencies* only. |
| **Over-mocking ("to be safe")** | So much is stubbed that no real path runs; the test passes by construction. | Mock the minimum (the slow/external boundary). Prefer a real collaborator where cheap. |
| **Partial / incomplete mock** | Stub omits fields the real code reads; passes in test, breaks in prod, or masks a bug. | Mirror the real response shape completely, including fields consumed downstream. |
| **No regression test for a bugfix** | Nothing fails on the pre-fix code, so the bug can return undetected. | Add a test that fails on the old code and passes on the fix; verify it reds before the fix. |
| **Over-broad snapshot** | A huge snapshot pins everything and nothing; regressions hide in "snapshot updated." | Assert the specific values that matter; reserve snapshots for stable, reviewed output. |
| **Boundary untested** | Code touches `<`/`<=`/limit but tests use a value far from it; off-by-one ships. | Test exactly at and around the boundary (`min-1`, `min`, `min+1`). |
| **Coupled to implementation** | Asserts private methods / call order / internal log strings; breaks on safe refactor, erodes trust. | Test through the public interface and observable behavior; drop internal assertions. (nit/minor) |
| **Flaky: real sleep/clock/random/network** | Passes or fails by timing or luck; teams learn to re-run red, masking real failures. | Inject a clock, seed randomness, await/poll instead of sleep, stub the network boundary. |
| **Order-dependent tests** | Pass only in a given order or via shared state; reorder/parallelize and they break. | Make each test set up and tear down its own state; no cross-test dependencies. |
| **Test-only code in production** | A `destroy()`/reset method exists solely for tests; dead/dangerous in prod, and the test "tests" plumbing. | Move test setup/teardown into test utilities; keep production surface clean. |
| **Tautological / restating the code** | Test asserts the literal output the code was just written to produce, structurally — proves nothing about correctness. | Assert behavior against an independent expectation, not the code's own shape. |
| **Disabled / skipped test shipped** | `it.skip`, `#[ignore]`, commented-out asserts → zero protection, looks covered. | Re-enable and fix, or delete it and note the gap honestly; never ship a skipped guard silently. |

## How to use this table

- It's a **lookup**, not a script. When a test smells off, find the matching row, confirm the
  failure mode applies by running the mutation, then write the finding with that row's fix.
- A test can hit several rows at once (over-mocked *and* truthiness-asserting). Report the most
  consequential as the finding; mention the others briefly.
- **Rank by consequence, not by smell.** Coupling and brittle snapshots are usually nit/minor —
  the test still catches the bug. Happy-path-only on a new error path, a missing regression
  test, or testing-the-mock are major/blocker — they let a real bug ship green.
