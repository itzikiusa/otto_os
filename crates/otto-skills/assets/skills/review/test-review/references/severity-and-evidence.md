# Severity & evidence — for test findings

Same discipline as any review: rank honestly, prove it before you claim it. The twist is that
your "bug" is usually a *gap* — a test that should exist and doesn't, or an assertion that
should fail on broken code and doesn't. The proof is the **mutation**: the broken code this
test would let through.

## Severity ladder

| Severity | Definition | Examples |
|---|---|---|
| **blocker** | The suite is green but guards nothing that matters: the change's core new behavior — or a bugfix — has no test that would fail if it broke, or a test asserts the wrong thing and would pass on a shipping bug. Also: no tests at all for a non-trivial change. | New payment branch with only a truthiness assert; bugfix with no regression test that reds on the old code; the unit under test is mocked so nothing real runs. |
| **major** | A real branch, error, or edge case of the change is untested; a key assertion is too weak to catch the likely regression; or a test is flaky. | New `if err != nil` path never driven; validation tested only with valid input; a `sleep(500)` race that flakes CI. |
| **minor** | Genuinely wrong but narrow: a rare case missing, or an assertion weaker than it should be that would still catch the obvious break. | Boundary tested at `min+5` not `min`; loose `toContain` where exact equality was knowable. |
| **nit** | Real but low-stakes: implementation-coupling, brittle exact-string match, a redundant test, a name that doesn't describe the behavior. | Asserts an internal log string; snapshot of a stable banner; `test('works')`. |

When unsure between two levels, state the trigger condition and pick the lower, then let the
author re-rank with the facts. But do **not** soften a blocker into a nit to seem agreeable:
a bugfix with no regression test *is* a blocker, however small the fix looked.

## Evidence bar (every finding must clear it)

1. **Location — both sides.** `path/to/test.ext:line` for the test, and `path/to/code.ext:line`
   for the production line it fails to guard. A *gap* finding names the **uncovered line** of
   production code, not just "missing tests."
2. **The mutation.** State the concrete break that exposes the gap: *"if `parseAmount` returned
   0 on bad input, `amount.test.ts:30` still passes."* The mutation is the mechanism — it's
   what turns "weak test" from an opinion into a fact.
3. **Confidence + how you know.** One of:
   - **confirmed** — you traced the test against the code, or ran the mutation in your head and
     are certain the assertion can't catch it (say which mutation).
   - **likely** — strong read, not fully traced.
   - **question** — you couldn't tell from the code alone whether the case is covered elsewhere
     or can even occur. Phrase it as a question and say what you'd check.
   Never present a *question* (a maybe-missing case) as a *confirmed* gap.
4. **Fix.** The exact missing test case or the stronger assertion — concrete enough to paste.
   "Add a test for the empty-cart case asserting `total === 0`," not "improve coverage."

## False-positive filter (run before you submit)

For each finding, ask:
- Is the case **actually uncovered**, or is there a sibling test file I didn't read? (Go check
  before claiming a gap.)
- Can the "missing" edge case **actually occur**, or is it impossible given an upstream
  guarantee? (An unreachable case needs no test.)
- Is the weak assertion **really weak**, or does a later line in the same test pin the value?
  (Read the whole test, not the one line.)
- Is this a **real protection gap** or my **testing-style preference**? If preference, it's a
  nit at most — or cut it.
- Did I run the mutation, or am I pattern-matching on a smell? If I can't name the broken code
  it lets through, downgrade to *question* or drop it.

A short list where every finding names the bug it would let ship beats a long list of
"could use more tests."
