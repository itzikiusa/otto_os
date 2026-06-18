# Finding template — test review

Emit each finding in this shape. Keep it tight — one finding, one test weakness. The
load-bearing line is **Mutation**: the broken code this test would let through.

```
### [SEVERITY] <short, specific title>   — test `path/to/test.ext:line` · code `path/to/code.ext:line`

**What:** <the weakness — missing case, weak assertion, tests the mock, flaky, coupled>
**Mutation:** <the concrete break that exposes it — "if X returned Y, this test still passes">
**Why it matters:** <the regression that would ship green because of this gap>
**Evidence:** <confirmed | likely | question> — <how you know: the trace, the mutation you ran, the sibling file you checked>
**Fix:**
```diff
+ <the missing test case, or the stronger assertion that WOULD fail on the broken code>
```
<or a precise instruction if a diff doesn't fit>
```

For a pure **gap** (no test exists), cite the uncovered production line as `code:line`, state
the mutation it leaves unguarded, and give the test to add as the fix.

## Report skeleton

```
**Verdict:** Block | Approve with fixes | Approve
**Counts:** blocker N · major N · minor N · nit N · question N
**Scope reviewed:** <test files + the production change they guard; how far into the code you read>
**Falsification-checked:** <the risky branches you ran the mutation against by name>

<findings, ordered: blockers first, then by file>

### Coupling / brittleness (nits)
<low-stakes test-quality nits, collected so none are lost>

<if clean: "Tests cover the change: every risky branch (list them) has a test that fails under
mutation. No test-quality defects found after passes 1–5 + completeness.">

<if NO tests at all for a non-trivial change: a single, loud blocker — state it first, plainly,
and do not pad with nits.>
```
