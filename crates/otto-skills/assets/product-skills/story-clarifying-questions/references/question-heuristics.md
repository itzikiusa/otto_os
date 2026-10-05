# Question Heuristics

How to find the questions that matter most and phrase them so a non-engineer PO
can answer them without interpretation.

---

## The decision-unblocking test

Before writing any question, state the decision it unlocks:

> "If the PO says X, we do A. If the PO says Y, we do B."

If you cannot fill in that sentence, the question is not high-leverage. Discard it.

**Example (passes):**
Question: "Should the export include rows the user has manually deleted, or only
currently visible rows?"
Decision: Determines whether we need a soft-delete flag in the query — a scope and
data decision with direct implementation impact.

**Example (fails):**
Question: "What should the button label say?"
Decision: Copy is usually the team's call during build. Unless the label is a legal
or brand constraint, this is implementation trivia.

---

## Ambiguity hotspots

These are the areas where stories most often leave dangerous gaps. Scan each one
before finalising your question set.

### Scope
- What's explicitly in vs. out? Stories often describe the happy path and leave
  adjacent cases undefined.
- "Also" and "including" are signals: they hint at list items the author may not
  have enumerated fully.
- Watch for implicit assumptions: "the admin can do X" — which admin role?

### Data
- Where does the data come from? Existing tables, third-party APIs, user input?
- What's the shape and volume? Pagination, limits, and stale-data tolerance often
  aren't stated.
- Identifiers: which ID unambiguously refers to the entity? Stories often use
  informal names that map to multiple database concepts.
- Retention, deletion, and access: does the story touch data the user can later
  delete, export, or that has a regulatory retention requirement?

### UX
- What happens in error states? The story usually describes the happy path only.
- Empty states: what does the screen show when there's no data yet?
- Permission states: what does a user without the right role see?
- Mobile / responsive: is the behavior on small screens the same?

### Edge cases (meaningful ones)
- Concurrency: what if two users act at the same time on the same entity?
- Zero / one / many: does behavior change at the boundary of the collection?
- Partial failure: if the story calls multiple services, what's recoverable?

Ask about an edge case only if it is plausible in production *and* the answer
materially changes design. Skip theoretical ones.

### Dependencies
- Does this story require another team to deliver something first?
- Is there a feature flag, API version, or third-party contract that must be in
  place?
- Are there legal, compliance, or brand constraints that gate the behavior?

---

## Phrasing for a non-engineer PO

| Avoid | Prefer |
|---|---|
| "Which endpoint should we call?" | "Where does this data come from — is it already in our system or pulled from the payment provider?" |
| "What's the DB retention policy?" | "How long do we need to keep this transaction history — does it have a legal or compliance shelf life?" |
| "Should we paginate this?" | "If a user has thousands of entries, should they scroll through all of them or search and filter instead?" |
| "What's the idempotency key?" | "If the user clicks Submit twice quickly, should the second click be silently ignored, show an error, or create a duplicate?" |

The reframe: talk about *the user experience or business rule*, not the
technical mechanism. The PO answers the business question; the team picks
the mechanism.

---

## Reducing to the minimum

After listing all open uncertainties, do a final pass:

1. **Can we assume a reasonable default?** If the answer is obvious from context
   or industry convention, document the assumption and skip the question.
2. **Does it block sprint delivery?** If the team can build a sensible version
   and ask later, park it as a follow-up note, not a blocker question.
3. **Are two questions really one?** Sometimes scope and UX questions about the
   same feature can be merged if the answer to one answers the other.

Target: 3–8 questions. If you have 10+, you haven't applied the filter above.
