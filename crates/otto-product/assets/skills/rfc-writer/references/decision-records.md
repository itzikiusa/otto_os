# Decision Records — Making Recommendations Unmistakable

An RFC is primarily a decision artifact. Its job is to make one recommendation
legible to a decision-maker who may read it once, quickly, under time pressure.
This reference covers how to capture decisions clearly, make alternatives visible,
and structure reasoning in the ADR (Architecture Decision Record) tradition.

---

## The ADR Core: Context → Decision → Consequences

ADRs were popularized for software architecture but the pattern applies cleanly to
any product or organizational decision. The structure forces writers to be honest about
what they are deciding and what it will cost.

### Context

Describe the situation that makes a decision necessary. This is similar to an RFC's
Problem Statement but tighter — it is the minimum context a future reader needs to
understand *why* any decision was made at all, without needing to read the whole
proposal.

Good context:
- Is written in the past tense (it describes a situation, not an aspiration).
- Names the constraints that were real at the time (budget, timeline, team size,
  existing systems, regulatory requirements).
- Does not already telegraph the preferred answer.

### Decision

One or two sentences. The recommendation, stated flatly.

> "We will adopt X."  
> "We will not build Y; instead we will configure Z."

Avoid hedging ("we have decided to explore", "we are leaning toward"). A decision record
captures a decision, not an intent to investigate.

### Consequences

The honest accounting of what the decision produces — both the intended benefits and the
accepted costs or risks.

Split into:
- **Positive consequences** — what gets better, what becomes possible.
- **Negative consequences / trade-offs** — what gets harder, what we are giving up,
  what risks we are accepting.
- **Neutral consequences** — things that change but are neither good nor bad.

Consequences that are merely "possible future concerns" belong in Open Questions, not
here. Only record what is reasonably expected.

---

## Making the Recommendation Unmistakable

The single most common RFC failure is a buried or ambiguous recommendation. Readers
who disagree with the options section will stop reading before they reach the proposal.
Readers who skim will miss the conclusion entirely.

**Techniques:**

1. **State it first.** The recommended decision belongs in the first sentence of the
   Proposed Decision section, not the last. "After considering three options, we recommend
   X" is weaker than "We recommend X. Here is why, and how it compares to the
   alternatives."

2. **Use a decision box.** For high-stakes RFCs, a highlighted block at the top or
   immediately after the executive summary signals the recommendation before the reader
   enters the analysis.

   ```
   DECISION: Adopt the federated event bus (Option B).
   ```

3. **Name the decisive factor.** What single trade-off, constraint, or value tipped the
   decision? State it explicitly. "The decisive factor was operational ownership: Option A
   required the Infra team to take on a new service, which they cannot staff in Q3."

4. **Acknowledge the runner-up.** Name the strongest alternative and briefly say why it
   lost. This signals that the alternatives were genuinely considered and gives skeptics
   a place to engage.

---

## Making Alternatives Visible

A decision is only trustworthy when the reader can see what it was decided *against*.
Alternatives serve two purposes:

1. They prove the author explored the space.
2. They give future readers context if circumstances change and the decision needs
   revisiting.

**What each alternative needs:**
- A fair description — describe each option as its advocate would describe it.
- Its genuine strengths — not just "it works" but *why* someone would choose it.
- Its genuine weaknesses — not just "it's harder" but *specifically what it costs* in
  the context of this decision.
- Why it was not chosen — a sentence that connects the weakness to the decision criteria.

**The "do nothing" option:**
Include it when inaction is a real choice. It is often the strongest argument for
urgency: if "do nothing" is genuinely worse, the cost of inaction should be stated.

---

## Capturing the Reasoning Chain

A decision record should allow a reader — including the author, two years later — to
reconstruct *why* a particular conclusion was reached. This requires making the
reasoning chain explicit.

The chain is:
1. We are in this situation (context).
2. We care about these outcomes (goals and criteria).
3. The options differ on these dimensions (trade-offs).
4. Given our situation and what we care about, the trade-offs favor X (decision).
5. Here is what we are giving up and accepting (consequences).

If any link in this chain is missing, a reader cannot evaluate whether the decision
still makes sense under changed circumstances.

---

## When to Write a Decision Record vs. a Full RFC

| Situation | Artifact |
|---|---|
| High-stakes, multi-team, irreversible | Full RFC |
| Moderate scope, reversible, one team | Lightweight RFC or ADR |
| Small, low-risk, easily reversed | ADR only (or PR comment) |
| Already decided, capturing for posterity | ADR, written in past tense |

Decision records can be appended to RFCs after approval, converting the proposal into a
permanent record of what was decided and why.
