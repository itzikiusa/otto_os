# Severity & evidence

## Severity ladder

Rank every finding honestly. The author triages by severity, so a miscalibrated label
costs trust both ways (a nit marked blocker cries wolf; a blocker marked nit ships a bug).

| Severity | Definition | Examples |
|---|---|---|
| **blocker** | Ships a bug, data loss, security hole, or breaks a consumer. Do not merge. | Nil deref on a real path; SQL injection; non-backward-compatible migration; lost write under concurrency. |
| **major** | Likely-wrong behavior or a real gap that will bite. | Unhandled error that drops data; missing the documented edge case; N+1 on a hot endpoint. |
| **minor** | Genuinely wrong, but narrow or unlikely to trigger. | Off-by-one only at `MAX`; leak only on a rare error path. |
| **nit** | Small but real. Report it — the brief is to catch even the smallest things — but label it so it's easy to skip. | Stale comment; redundant allocation; name breaks local convention; missing test for a trivial branch. |

When unsure between two levels, state the trigger condition and pick the lower — then the
author can re-rank with the facts in front of them.

## Evidence bar (every finding must clear it)

1. **Location.** `path/to/file.ext:line` (a range is fine). No location → not a finding.
2. **Mechanism.** *Why* it's wrong, concretely. For races: the two interleavings and the
   corrupted field. For a nil/None: the path where the value is absent at that line. For a
   perf issue: the loop and the per-iteration cost.
3. **Confidence + how you know.** One of:
   - **confirmed** — you traced it or reproduced it (say how: "hand-traced with `n=0`",
     "the test on line X would fail").
   - **likely** — strong read, not executed.
   - **question** — you could not verify from the code alone. Phrase it as a question, not
     an accusation, and say what you'd need to confirm.
   Never present a *question* as a *confirmed* defect.
4. **Fix.** A concrete diff/snippet or a precise instruction. Not "handle this better."

## False-positive filter (run before you submit)

For each finding, ask:
- Is it **actually reachable**, or guarded upstream by something I didn't read? (Go check.)
- Is it a **real defect** or my **style preference**? If preference, it's a nit at most —
  or cut it.
- Did I verify, or am I pattern-matching? If I can't defend it, downgrade to *question* or
  drop it.

A list where every item survives this filter is worth more than a longer list that doesn't.
