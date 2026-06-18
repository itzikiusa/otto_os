# Severity & the finding shape

Shared severity model with `grill` — same ladder, narrowed to the correctness lens. Rank
honestly: the author triages by severity, so a miscalibrated label costs trust both ways (a
nit marked blocker cries wolf; a real bug marked nit ships).

## Severity ladder

| Severity | Definition (correctness lens) | Examples |
|---|---|---|
| **blocker** | Produces a wrong result, corrupts data, crashes, or loses a write on a **real, reachable** path. Do not merge. | Division by zero on empty input; off-by-one that drops the last record; inverted condition that lets the wrong branch run; nil deref on a path a real caller hits; double-apply on retry. |
| **major** | Likely-wrong behavior or a real branch/edge gap that will bite in normal use. | Unhandled `else` that returns a stale value; missing-default that silently computes `0`; rounding error that accumulates; enum case left unhandled. |
| **minor** | Genuinely wrong, but narrow or unlikely to trigger. | Off-by-one only at `MAX_INT`; wrong result only for an input the type system nearly forbids. |
| **nit** | Small but real. Report it, label it so it's easy to skip. | A correct-but-confusing condition that invites a future bug; a latent truncation that can't trigger today; a magic boundary constant. |

When torn between two levels, **state the trigger condition and pick the lower** — the author
re-ranks with the facts in front of them.

## Evidence bar (every finding must clear it)

1. **Location** — `path/to/file.ext:line` (a range is fine). No location → not a finding.
2. **Mechanism** — *why* it's wrong, concretely: the input, the line where actual diverges
   from intended, and the wrong value it produces.
3. **Confidence + how you know** — `confirmed` (traced or reproduced — say which) ·
   `likely` (strong read, unexecuted) · `question` (couldn't verify — ask, don't accuse).
   Never present a *question* as a *confirmed* defect. See `trace-and-reproduce.md`.
4. **Fix** — a concrete diff/snippet or a precise instruction. Not "handle this better."

## Finding shape

The exact fill-in template — the finding shape and the report skeleton — lives in
`assets/correctness-report.md`. Populate that; don't reinvent the layout here.

One field there is specific to this lens and load-bearing: **Intended.** Every finding names
what the code is *supposed* to do, then shows where actual diverges. That named oracle is
what makes a correctness finding a defensible claim and not a preference — you are always
asserting a gap between supposed-to and actual.

A clean verdict naming the inputs and branches you traced is a real, defensible result. A
fabricated bug to look diligent is not — and one untraced false positive costs you every
finding that follows it.
