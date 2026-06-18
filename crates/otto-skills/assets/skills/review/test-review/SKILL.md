---
description: A focused, single-lens review of test quality and coverage for a change — does the test actually exercise the new behavior AND its failure modes, or does it merely pass? Hunts happy-path-only coverage, weak assertions, tests that pass even when the code is wrong, over-mocking that tests the mock, missing negative/edge/error cases, flakiness, and tests coupled to implementation detail. Every finding asks "would this test fail if I broke the code?", cites file:line, a severity, why it matters, and the concrete missing case or stronger assertion.
category: review
version: 1
---

# Test Review

You review **one thing**: the tests for this change. Not the production logic, not the
architecture, not security — those have their own lenses. Your single question is whether
the tests **actually test the change**, or just decorate it with green checkmarks.

A passing test suite is not evidence. A test that passes whether or not the code is correct
is worse than no test: it buys false confidence and it will block no regression. Your job is
to separate the tests that *catch bugs* from the tests that merely *run code*.

> This skill is **not grill.** Grill sweeps the whole change across twelve lenses and reports
> on test quality as one of them, briefly. You go *deep* on that one lens and nothing else.
> When invoked alongside grill, you own the test verdict; grill defers to you there.

> Bundled files sit alongside this SKILL.md — consult them as you work:
> - `references/falsification.md` — the central move: would this test fail if the code broke?
> - `references/coverage-and-cases.md` — the per-test hunt list (assertions, cases, mocks, flakiness)
> - `references/anti-patterns.md` — the catalogue of test smells, with the fix for each
> - `references/severity-and-evidence.md` — how to rank a test finding and the evidence each must clear
> - `assets/finding-template.md` — the exact shape of one finding + the report skeleton
> - `scripts/test-scan.sh [base-ref]` — run first to **seed** the review: lists changed source
>   with no co-changed test, weak-assertion/mock-only smells, and skipped tests (file:line).
>   Output is **hints, not findings** — it can't tell whether a test falsifies the code; only
>   your read + the mutation question can. Verify or discard every hit.

---

## Inputs

You are given a **diff** (a PR or local working-tree change) and, where available, the
surrounding test files, the production code under test, and the story/ticket. Read **both
sides**: the production change *and* its tests, together. You cannot judge whether a test
exercises a branch without reading the branch. A test reviewed blind to the code it guards
is guesswork.

Identify the **risky parts of the change first** — the new branches, the error paths, the
boundary conditions, the bug being fixed. Those are what the tests *must* cover. Then check
whether they do.

Optionally run `scripts/test-scan.sh` against the base ref to seed the read with deterministic
smells (changed-source-without-changed-test, weak-assertion tokens, skipped tests). Treat its
output as a list of places to look, never as the verdict — it finds *smells*, not *gaps*.

---

## The central move — falsification

For every test that touches the change, ask the one question this skill exists to ask:

> **If I broke the code under test, would this test fail?**

Mentally mutate the production code — invert a condition, drop an `await`, return the input
unchanged, skip the side effect, return an empty list, swallow the error. If the test would
**still pass**, it is not testing that behavior. That is a finding, regardless of coverage
numbers. Coverage says a line *ran*; falsification asks whether anyone *checked the result*.

This is the lens. Run it on every assertion. Details and worked mutations are in
`references/falsification.md`.

---

## Method — five passes

Don't do one read-through. Sweep the tests once per lens; each lens surfaces a different
class of weak test. The full hunt list per pass is in `references/coverage-and-cases.md`;
the named smells and their fixes are in `references/anti-patterns.md`.

1. **Does it test the change?** Map each risky branch / new behavior / fixed bug to the test
   that covers it. A new branch with no test that fails when the branch breaks is a gap —
   name the uncovered branch by `file:line`. For a bugfix: is there a regression test that
   *fails on the old code*? If not, the bug can silently return.

2. **Assertion strength (falsification).** For each test, run the central move. Catch weak
   oracles: asserting truthiness (`assert result`, `toBeTruthy`), asserting the call happened
   instead of the result (`expect(mock).toHaveBeenCalled` with no check of the effect),
   asserting on a value the test itself computed the same way the code does, snapshotting
   everything so nothing is really pinned, or asserting "no error thrown" as the whole test.

3. **Cases — negative, edge, error.** Happy path only is the most common gap. Is there a
   test for the *failure* path the code added (the validation that rejects, the error it
   raises, the empty/null/zero/boundary input)? A change that adds an `if err != nil` with no
   test that drives `err != nil` is half-tested. Missing the negative case is usually a
   *major*, not a nit.

4. **Mocking — is it testing the mock?** Over-mocking hollows out a test until it only
   verifies the test's own stubs. Flag: asserting on `*-mock` elements, mocking the very unit
   under test, a mock so loose it would satisfy a broken implementation, a mock whose
   stubbed return *is* the thing being asserted, and partial mocks missing fields the real
   code consumes. Ask: after these mocks, what real code is left to exercise?

5. **Coupling & flakiness.** Tests bound to implementation detail (private methods, internal
   call order, exact log strings, DOM structure) break on safe refactors and erode trust —
   flag them, but rank honestly (usually minor/nit unless they hide a real gap). Flakiness is
   sharper: real wall-clock `sleep`, dependence on test execution order or shared mutable
   state, unseeded randomness, real network/time/filesystem, `Date.now()`/`now()` without a
   fixed clock. A flaky test is a *major* — it trains the team to ignore red.

**Then — the completeness pass.** Ask: *Which risky line of the change has no test that would
fail if it broke? Which assertion did I take on trust without running the mutation? Is the
one test that looks solid actually pinned to a real oracle?* Re-open the riskiest hunk and
its test, and run falsification once more.

---

## Evidence before assertion (non-negotiable)

A test finding is a claim — back it. See `references/severity-and-evidence.md`.

- **Name the mutation.** Don't say "weak test" — say *"if `applyDiscount` returned the price
  unchanged, `cart.test.ts:42` still passes, because it only asserts the call happened, not
  the resulting total."* The mutation is your proof.
- **Cite `file:line`** for the test, and the `file:line` of the production code it fails to
  guard. A gap finding names the **uncovered line**.
- **Show the fix.** The missing test case, or the stronger assertion — concrete, not "add
  more tests." Give the assertion that *would* fail on the broken code.
- **Don't invent gaps.** If a case is genuinely covered elsewhere, or the "missing" edge case
  can't occur given upstream guarantees, don't flag it. Go check before you claim.

---

## Output

Produce a single, ranked findings list, ordered by severity then by file. Use
`assets/finding-template.md`. Severities (full ladder in `references/severity-and-evidence.md`):

- **blocker** — the change's core new behavior or a bugfix has *no* test that would fail if it
  broke; or a test asserts the wrong thing and would pass on a broken implementation that
  ships a bug. The suite is green but guards nothing that matters.
- **major** — a real branch/error/edge case of the change is untested, a key assertion is too
  weak to catch the likely regression, or a test is flaky.
- **minor** — a narrow case is missing, or an assertion is weaker than it should be but would
  still catch the obvious break.
- **nit** — implementation-coupling, a brittle exact-string match, a redundant test, a name
  that doesn't describe the behavior. Real, but low-stakes.

Open with a one-line **verdict** (`Block` / `Approve with fixes` / `Approve`) and counts by
severity. If the tests genuinely cover the change — every risky branch has a test that fails
when it breaks — **say so plainly** and name the branches you mutation-checked. A clean
verdict you can defend is a real result. If there are **no tests at all** for a non-trivial
change, that is a single, loud blocker — don't bury it in nits.

---

## Anti-patterns (reviewer's own)

| Anti-pattern | Why it fails |
|---|---|
| Treating coverage % as the verdict | 100% coverage with truthiness asserts catches nothing. Falsify, don't count. |
| "Add more tests" | Vague. Name the exact missing case and the assertion that would fail on broken code. |
| Reviewing tests blind to the code | You can't judge if a branch is covered without reading the branch. |
| Duplicating grill's full sweep | Stay in your lane: test quality only. Don't re-review the production logic. |
| Flagging coupling as a blocker | Brittle ≠ broken. A test coupled to internals still catches the bug — rank it a nit/minor. |
| Demanding a test for every trivial line | Right-size. A one-line getter doesn't need a guard. Focus on the change's *risk*. |
| Claiming a gap without checking elsewhere | The case may be covered in another file. Go look before you assert. |

## Quality bar

A great test review ends with the author able to point at **one specific test that now
catches a bug it didn't before** — because you named the mutation it missed and the assertion
that closes it. Every finding survives the question *"show me the broken code this test would
let through"*; nothing flagged is mere style dressed as a gap; and the verdict on whether the
change is safely tested is one you would defend when the regression *doesn't* happen in prod.
