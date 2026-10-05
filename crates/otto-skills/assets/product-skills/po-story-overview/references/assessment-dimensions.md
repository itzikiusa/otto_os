# Assessment Dimensions — Deep Guidance

Use this reference when working through Step 3 of the PO Story Overview workflow.
For each dimension, make a **clear verdict** (strong / partial / missing) and explain why.

---

## 1. Business Value & Measurable Outcome

**What you are looking for:**
The story must answer "why does this matter to the business?" with something concrete
enough that you could confirm it happened after launch.

**Strong signal:** A stated metric, a before/after comparison, a named pain point that
is currently costing users or the business in a measurable way, or a strategic goal
the story directly advances.

**Weak signal:** "Improve UX," "increase performance," "support the roadmap." These are
placeholders, not outcomes.

**Probing questions:**
- What does success look like six weeks after launch? How would we measure it?
- If this story were never built, what would users or the business miss?
- Is the value speculative ("might improve") or evidenced ("currently X% of users fail at Y")?
- Is there a hypothesis that could be falsified, or just a belief that something will be better?
- What is the cost of not doing this now?

**Common failure mode:** The story describes a feature ("add a filter to the table")
without connecting it to a user problem or business metric. A strong PO rewrites this
as "users currently cannot narrow results to their region, causing X% to abandon the
report — add region filter so they can complete the workflow."

---

## 2. Target Users / Personas

**What you are looking for:**
A named, specific audience — not "users" or "admins" but the role, context, or persona
that will feel the change. If multiple audiences are affected differently, each should
be identified.

**Strong signal:** A specific role ("casino operations manager reviewing end-of-day
settlements"), a named persona from the product's persona library, or a user group
with a clearly described context and need.

**Weak signal:** "Users," "customers," "the team," "stakeholders."

**Probing questions:**
- Who specifically does this? What is their role, frequency of use, and context?
- Are there secondary users who are affected but not the primary beneficiary?
- Does the persona's technical fluency level affect what "done" looks like?
- If this is an internal tool, which internal teams? Are their workflows documented?
- Are there users who will be negatively affected (e.g., a workflow that changes for
  a group that wasn't consulted)?

**Common failure mode:** A story improves an admin dashboard but never says which
admin team or what workflow they are performing. Engineering ships a solution for
an imagined user instead of a real one.

---

## 3. Scope Boundaries

**What you are looking for:**
Both **what is in scope** and **what is explicitly out of scope** for this story.
Unstated scope is the leading cause of rework — if something is not called out as
out of scope, engineers often assume it is in.

**Strong signal:** A bullet list of explicit inclusions and exclusions. Statements
like "this story does not cover mobile" or "reporting is a follow-on story" are
extremely valuable.

**Weak signal:** A story that describes what should be built but never draws a line
around it.

**Probing questions:**
- What adjacent functionality will users expect but is intentionally deferred?
- What related features were considered and cut? Why?
- Are there platform, browser, locale, or permission-level constraints that bound the scope?
- What happens at the edges — empty states, error states, large datasets, concurrent users?
- Is this story a slice of a larger epic? If so, what does the rest of the epic cover?

**Common failure mode:** "As a user I want to export reports." Does this mean CSV only?
All date ranges? Only my data or all data? With or without filters applied? Scoping
silences all of these questions.

---

## 4. Acceptance Criteria Completeness

**What you are looking for:**
Criteria that are **present**, **testable**, and **complete** — meaning a developer
and an independent tester would independently reach the same conclusion about whether
the story is done.

**Strong signal:**
- Given/When/Then format, or outcome-based bullets that name the observable result.
- Edge cases covered: empty state, error state, permission boundary, data at scale.
- No subjective language ("fast," "easy," "clean") without a concrete threshold.

**Weak signal:**
- Criteria written as requirements ("the system should…") instead of verifiable outcomes.
- Only happy-path criteria; no error handling, loading states, or boundary conditions.
- Subjective or unmeasurable language: "the page should load quickly," "the UI should feel responsive."

**Probing questions:**
- Could two developers independently decide "done" using only these criteria?
- Is the error path specified? What happens when an API call fails, a record is missing,
  or input is invalid?
- Are there performance thresholds? Permission rules? Locale/timezone considerations?
- What does "done" mean for an empty state (no data, first-time user)?
- Are there non-functional requirements implied but not written (audit logging, accessibility,
  data retention)?

**Concrete completeness check:**
Walk through the user journey mentally. At each step ask: "is there a criterion that
confirms this step works correctly, including when it fails?" Any step without a
criterion is a gap.

---

## 5. Dependencies & Assumptions

**What you are looking for:**
Every condition outside the team's direct control that must be true for this story to
land successfully. Hidden dependencies become last-minute blockers; hidden assumptions
become production bugs.

**Strong signal:** Named teams, services, feature flags, data migrations, external APIs,
or legal/compliance approvals that are required. Each dependency has an owner and a
status ("confirmed," "pending," or "at risk").

**Weak signal:** No dependencies listed when the story clearly touches multiple systems
or teams.

**Probing questions:**
- What other teams or services must deliver something before or alongside this?
- Are there data requirements — is the data already available, or does it need to be
  backfilled, migrated, or generated?
- Does this depend on a feature flag, a configuration change, or an infrastructure
  upgrade?
- What external parties are involved (vendors, regulators, third-party APIs)?
- What does this story assume about the current state of the system that might not be true?
- Are there timing dependencies — does this need to land before or after another release?

**Common failure mode:** A story assumes "the new user API is already returning the
`locale` field" — but that work hasn't shipped yet. The dependency is invisible until
integration testing.
