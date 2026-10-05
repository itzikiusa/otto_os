# RFC Structure — Section-by-Section Depth Guide

Use this as the standard for evaluating or strengthening each section of an RFC.
Each entry describes what "good" looks like and gives a short example pattern.

---

## Problem Statement

**Purpose:** Make the pain concrete and specific. A reader who has never thought about
this problem should finish this section convinced that something needs to change.

**What makes it strong:**
- Names real users or systems that are affected.
- Quantifies the pain where possible (frequency, scale, cost, risk).
- Explains why this problem exists *now* — what changed or what threshold was crossed.
- Avoids solution language — the problem statement is solution-neutral.

**Weak example:**
> We need a better way to handle notifications.

**Strong example:**
> Support agents spend ~20 min/day triaging duplicate alerts fired by three separate
> systems with no unified view. During incidents this delay has caused missed SLA
> breaches on five occasions in Q1. The root cause is that each system fires
> independently with no deduplication layer.

**Key question to ask:** Could a reader clearly state the problem back to you after
reading this section alone?

---

## Motivation

**Purpose:** Explain the broader context and urgency. Why is this worth the organization's
attention now rather than later?

**What makes it strong:**
- Links to company goals, OKRs, or strategic priorities if relevant.
- Names the stakeholders who care and why.
- Explains why *now* — triggered by new data, a planned milestone, a customer commitment,
  or a risk that is growing.

**Key question to ask:** Would a skeptical executive understand why this is on the agenda
at this moment?

---

## Goals

**Purpose:** Define what success looks like as outcomes, not as feature deliverables.

**What makes it strong:**
- Written as observable outcomes: "Users can X", "The system does Y within Z".
- Measurable where possible — include a success metric or acceptance signal.
- Scoped to what this RFC proposes to achieve, not to a broader vision.
- Small enough to be achievable in the proposed scope.

**Weak example:**
> Build a notification service.

**Strong example:**
> - On-call engineers receive a single deduplicated alert per incident within 30 s of
>   the first event, regardless of which system detected it.
> - False-positive alert rate falls below 5 % within 60 days of rollout.

---

## Non-Goals

**Purpose:** Draw explicit scope boundaries. Non-goals are as valuable as goals because
they prevent scope creep and misaligned expectations.

**What makes it strong:**
- Names things a reader might reasonably expect to be in scope, and explicitly excludes
  them with a brief rationale or a pointer to future work.
- Does not list trivially unrelated things (no one expected those anyway).
- Distinguishes between "not in scope for this RFC" and "a deliberate product choice not
  to do."

**Example:**
> - **Not in scope:** Email and SMS notification channels. This RFC covers in-app and
>   webhook delivery only; other channels will be addressed in a follow-on.
> - **Not in scope:** Real-time analytics on notification engagement. Metrics are read-only
>   via existing dashboards.
> - **Deliberate exclusion:** We are not building a self-serve notification subscription
>   UI in this phase. Admin-managed configs are sufficient for the current user base.

---

## Options / Alternatives Considered

**Purpose:** Demonstrate that the recommendation was chosen from a real set of
alternatives, not reverse-engineered from a pre-existing preference.

**What makes it strong:**
- At least two genuine alternatives, plus "do nothing" when meaningful.
- Each option gets a brief description, its key pros, its key cons, and any
  significant unknowns or risks.
- The comparison is honest — a strong option with real weaknesses is more credible than a
  strawman that exists only to be dismissed.
- Trade-offs are concrete (cost, risk, reversibility, time, dependency) not just
  "simpler" vs "more complex."

**Structure per option:**

```
### Option A: [Name]
Brief description of what this entails.

**Pros**
- ...

**Cons / risks**
- ...

**Why not chosen (if not the recommendation)**
- ...
```

**Key question to ask:** Would someone who prefers a different option feel their choice
was considered fairly?

---

## Proposed Decision

**Purpose:** State the recommendation plainly and explain why it wins.

**What makes it strong:**
- The recommendation is in the first sentence — no burying the lede.
- The rationale refers back to the options section: "we chose X over Y because the Z
  trade-off matters more than W in our context."
- Acknowledges the most significant downside of the chosen option and explains why it
  is acceptable.
- Clearly separates the decision from the implementation plan.

**Weak example:**
> Given the above considerations, Option B seems like a reasonable approach that balances
> the various factors discussed.

**Strong example:**
> **We recommend Option B (federated deduplication via a shared event bus).**
> It is the only option that solves the problem without requiring changes to the three
> existing alert producers. The operational overhead (running one additional service) is
> lower than the engineering cost of refactoring all three producers (Option A), and the
> risk of a single point of failure is mitigated by the bus's existing HA guarantees.
> The main downside — added latency of ~200 ms — is acceptable given the SLA window
> is measured in minutes.

---

## Scope, Impact, and Rollout

**Purpose:** Tell affected parties what changes for them and how the proposal ships
safely.

**What makes it strong:**
- **Scope:** Names every system, team, or user type that is affected. Calls out
  dependencies — what must be true before this ships.
- **Rollout:** Describes the phasing or sequencing (if any), any feature flags or
  gradual rollout strategy, and migration path for existing data or behavior.
- **Backout:** Explains how to reverse the change if something goes wrong. If backout
  is difficult, says so explicitly.
- **Success signal:** States how we will know the rollout succeeded — a metric to watch,
  a threshold to cross, or an owner to sign off.

**Key question to ask:** Could an on-call engineer use this section to safely roll back
the change at 2 AM without reading the rest of the RFC?

---

## Open Questions

**Purpose:** Surface what is still unresolved so the RFC does not appear to have false
certainty.

**What makes it strong:**
- Each question is specific — not "is this the right approach?" but "Should we handle
  duplicate events at the producer or the consumer? This affects the latency budget."
- Names the owner or the forum where it will be resolved.
- Indicates whether the question blocks the decision or can be resolved in implementation.

**Example:**

| Question | Owner | Blocks decision? |
|---|---|---|
| What SLA do we offer to notification consumers on delivery latency? | PM + Infra | Yes |
| Which team owns the new deduplication service long-term? | Eng leadership | No — resolves before launch |
| Should historical deduplication look back 1 h or 24 h? | On-call team | No |
