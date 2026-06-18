# Trace and reproduce — the gate every finding must pass

A correctness finding is a claim that the code produces the wrong result. You earn the right
to make that claim by **proving it** — by hand-tracing a concrete input or by building a
check that goes red on the bug. This file is the technique. It is the thing that separates
this review from guesswork.

The rule: **no trace and no repro → no blocker.** An untraced suspect is at most a
*question*.

---

## Hand-tracing: walk the failing input line by line

The cheapest proof, and usually enough.

1. **Pick the input that breaks it** — not a generic one, *the* one. The empty list. `n=0`.
   The last index. `null`. The value exactly at the boundary. The second call. Choose the
   input your hunt-list suspect predicts will fail.
2. **State the intended result** for that input (from the ticket / contract / a sibling
   test). This is your oracle — what *should* happen.
3. **Walk the code with that value held in your head**, line by line, branch by branch.
   Track the state that matters (the index, the accumulator, the flag) as it changes.
4. **Name where it diverges** — the exact line where actual ≠ intended, the wrong value it
   computes, and why.
5. **Check reachability as you go** — if an upstream guard means your input can never reach
   the suspect line, the bug is dead. Kill it; don't report a guarded non-bug.

Write the trace into the finding so the author can replay it:

> "With `items=[]`: line 12 sets `total=0`; the `for` on line 14 never iterates; line 19
> returns `total/len(items)` → **division by zero**. Intended (per the docstring): empty
> input returns `0`. Reachable — `compute()` is called from `report()` with the unfiltered
> list, which can be empty."

A trace this concrete is irrefutable. "This might divide by zero" is not.

---

## Reproducing: build a check that goes red on *this* bug

Stronger than a trace when it's cheap. The goal is a **red-capable** signal — one that fails
*now* because of this bug and passes once it's fixed. Reach for it in roughly this order:

1. **A failing unit test** at whatever seam reaches the bug. Often you can add one assertion
   to an existing test file. Assert on the *specific* wrong result, not "doesn't crash."
2. **A REPL / one-liner** that calls the function with the breaking input and prints actual
   vs. expected.
3. **A curl / HTTP call** against a running dev server, if the bug is endpoint-level.
4. **Replay a captured payload** through the code path in isolation if setup is heavy.

Make the signal **sharp** (assert the exact symptom — the wrong number, the missing field,
the thrown error), **deterministic** (pin time, seed RNG, fix ordering), and **fast**
(seconds). Then **run it** and paste the invocation + output into the finding. A pasted red
test is the highest-confidence evidence you can give.

If reproducing would take more than a few minutes of setup, a clean hand-trace is the right
trade — don't over-invest. The trace and the repro are alternatives, not both-required.

---

## Confidence ladder — label every finding by how you know

| Confidence | You did | Use it as |
|---|---|---|
| **confirmed** | Hand-traced the input to the divergent line, **or** reproduced it (test/REPL/curl). State which. | A defect (blocker/major/minor by impact). |
| **likely** | Strong read of the code, but you did not execute the path or trace a full input. | A defect, but say it's unexecuted; invite a quick check. |
| **question** | You could not verify from the code alone (depends on a caller you can't see, an external invariant). | Phrase as a question + what you'd need. **Never** a blocker. |

Promote a `likely` to `confirmed` by tracing it. Demote anything you can't defend to a
`question` or drop it. The integrity of the whole review rests on this label being honest.

---

## The untested-path move

The author ran the happy path; the bug is in the path they didn't. Before you finish, list
the paths with **no test and no trace** — the `else`, the empty input, the error branch, the
boundary, the second call — and trace at least the riskiest one. That deliberate trace of the
unrun path is, more often than not, where the real bug is.

---

## False-positive filter (run before you submit)

For each finding, ask:
- **Reachable?** Did I confirm the breaking input can actually arrive here, or is it guarded
  upstream by something I didn't read? (Go read it.)
- **Real, or my preference?** A confusing-but-correct condition is a `nit` at most — or cut.
- **Verified, or pattern-matched?** If I can't paste a trace or a repro, it's a `question`,
  not a `blocker`.

A finding list where every item survives this filter is worth ten times a longer list that
doesn't.
