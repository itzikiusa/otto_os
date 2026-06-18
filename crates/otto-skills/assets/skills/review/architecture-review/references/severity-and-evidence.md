# Severity & evidence — ranking design findings by future cost

A design finding is not a bug; it is a **prediction about the cost of future change**. Rank
it by that cost — how expensive, how risky, how spread, and how hard to reverse once merged.

## Severity ladder

| Severity | Definition (future cost) | Examples |
|---|---|---|
| **blocker** | A structural decision that is expensive to reverse once merged *and* will spread. Cheapest to fix now, before callers and code depend on it. | A wrong public seam consumers will build on; a dependency cycle across layers; a transport detail leaked into a core/domain type that every new caller inherits; an abstraction that couples two things which must vary independently. |
| **major** | Real friction the *next* change to this area will hit. Localized but certain to cost. | A unit doing three jobs that the next feature must untangle first; logic on the wrong side of a boundary; a missing seam where two variants already exist; shotgun-surgery coupling (one change forces edits in N callers). |
| **minor** | A genuine design smell that won't spread far and won't block the next change — it just makes one spot slower to read or edit. | A function a bit too long; one duplicated block; a name that mildly misleads; a parameter cluster that should be a type. |
| **nit** | A small, real structural preference *with a future-cost rationale*. Report it (clean structure is the point) but label it so it's easy to skip. | Helper that belongs in the shared module; a slightly off-convention layout; a marginally shallow wrapper. |

When unsure between two levels, state the condition (how widely it spreads, how hard to back
out) and pick the **lower** — the author re-ranks with the facts in front of them. A
reversible, contained smell is rarely a blocker, however ugly.

**The reversibility test.** Ask: *if we merge this and regret it, how expensive is the
undo?* A private implementation detail is cheap to change later → at most minor/major. A
**public** interface, a cross-module **seam**, or a **dependency direction** is expensive
once depended upon → that's what blockers are made of. Bias your severity toward
*irreversibility × spread*, not toward how ugly it looks.

## Evidence bar (every finding must clear it)

1. **Location.** `file:line` for a local issue; `module`/`dir` for a structural one. A range
   is fine. No location → not a finding.
2. **The future change.** Name the concrete edit that this structure makes slow, risky, or
   duplicated — *"adding a second provider means editing these 4 files,"* not "too coupled."
   If you cannot name a plausible future change, it is **not a finding**; drop it or mark it
   a *question*.
3. **The deletion test (for any abstraction added or removed).** State whether the
   seam/layer earns its keep: deleting it *concentrates* complexity (keep it) or just
   *moves* it to callers (cut it). Don't ask for a seam unless two adapters justify it.
4. **Confidence + how you know.** One of:
   - **grounded** — you read the surrounding modules / callers / siblings and confirmed the
     coupling, the duplication, or the missing variant actually exists (say so).
   - **likely** — a strong structural read from the diff and a quick scan, not fully traced.
   - **question** — you couldn't tell from the code whether the cost is real (e.g. you don't
     know if a second variant is coming). Phrase it as a question and say what you'd need.
5. **Refactor direction, sized to the change.** A concrete move — *"extract the rule into a
   `Pricing` module the handler calls; the handler then only orchestrates"* — and a one-line
   note on what it costs. Not "improve the design," and not "rewrite it."

## Taste filter (run before you submit)

For each finding, ask:
- **Cost or preference?** Can I name a future change it makes worse, or do I just like mine
  better? If preference, cut it — or it's a nit at most, labelled as such.
- **Reversible?** If this is cheap to change later, it is not a blocker no matter how it
  looks. Re-rank.
- **Does the abstraction I'm demanding pass the deletion test and the two-adapter rule?** If
  not, I'm asking for indirection. Cut the demand.
- **Did I read the neighbours?** If I'm judging shape without having read the sibling modules
  and callers, I'm guessing. Go read them, or downgrade to a question.
- **Am I imposing a textbook over this codebase's own convention?** Consistency wins; the
  second way of doing things is itself a cost.

A short list where every item names a real future cost is worth far more than a long one
padded with taste. Padding design reviews with preferences is how authors learn to ignore
them.
