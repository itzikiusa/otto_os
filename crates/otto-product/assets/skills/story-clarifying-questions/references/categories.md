# Question Categories

A taxonomy for labelling clarifying questions. Each question gets exactly one
category — pick the one that best describes the primary decision the answer unlocks.

---

## `scope`

**What it covers:** The boundary of the feature — what is included, what is
explicitly excluded, which users or roles are affected, and which cases are
covered by this story vs. a follow-up.

**Ask scope questions when the story leaves open:**
- Which user roles or personas can perform the action?
- Whether adjacent, similar cases are handled the same way or deferred.
- What "done" means — partial rollout, full feature, MVP subset?
- Whether the story replaces or coexists with existing behavior.

**Example questions:**
- "Does this apply to all account types, or only accounts on the Pro plan?"
- "Is exporting in CSV format in scope for this story, or a follow-up?"
- "When an admin and a regular user both have access, should they see the same
  view or a role-differentiated one?"

---

## `data`

**What it covers:** Where data comes from, how it is structured, what volumes
are expected, identifiers, accuracy tolerances, retention rules, and any
data the story reads or writes that isn't clearly defined.

**Ask data questions when the story leaves open:**
- The source of a dataset (our DB, third-party API, user input, calculated).
- Volume or growth characteristics that would affect design.
- How stale the data can be (real-time vs. cached vs. daily snapshot).
- Which identifier uniquely addresses the entity.
- Whether historical records are preserved, updated in place, or discarded.

**Example questions:**
- "Is the transaction history pulled live from the payment provider on each
  page load, or cached in our system? How fresh does it need to be?"
- "When we say 'active users', is that defined by login in the last 30 days,
  or by a status flag in the database?"
- "After a refund is processed, should the original transaction record be
  updated or should a new record be appended?"

---

## `ux`

**What it covers:** User flows, screen states, copy, feedback messages, error
presentation, empty states, loading states, and how the feature behaves for
users in non-happy-path situations.

**Ask UX questions when the story leaves open:**
- What the user sees when no data exists yet (empty state).
- How errors are communicated — inline, toast, modal, redirect?
- Whether there is a confirmation step before a destructive action.
- How the feature behaves on mobile or smaller viewports (if applicable).
- Whether the user can undo or recover from an action.

**Example questions:**
- "If the export takes more than a few seconds, should the user see a progress
  indicator, or is a 'we'll email you the file' approach acceptable?"
- "If the user doesn't have permission to view a record, should we show a
  'no access' message or redirect to a different page?"
- "Is there a confirmation step before a user permanently deletes their account,
  or is the delete action immediate?"

---

## `edge-case`

**What it covers:** Meaningful boundary conditions and failure modes that are
plausible in production and whose handling isn't implied by the story — not
theoretical worst cases.

**Ask edge-case questions only when:**
- The scenario is genuinely likely to occur (not a 0.001% case).
- The behavior would be visibly different from the happy path.
- Different reasonable answers lead to meaningfully different implementations.

**Example questions:**
- "What should happen if two team members try to approve the same request at the
  same time — should the second one see an error, or is one silent win okay?"
- "If the third-party payment API is unavailable, should the checkout page show
  an error or hide the payment option entirely?"
- "If a user uploads a file that exceeds the size limit, should we reject it
  upfront with a clear message, or truncate silently?"

---

## `dependency`

**What it covers:** Other teams, services, feature flags, external APIs,
compliance or legal gates, and sequencing constraints that this story relies
on but doesn't control.

**Ask dependency questions when the story assumes:**
- Another team's feature is already complete or available.
- A third-party integration is live or has a known API contract.
- A feature flag exists that controls rollout.
- Legal, compliance, or brand approval is required before launch.

**Example questions:**
- "This story mentions the new notification service — is that already live in
  staging, or does this story block on that team's delivery?"
- "Does this change require a legal review before we ship, given it affects
  how we display billing information?"
- "Is there a feature flag for this rollout, and which markets or user segments
  should see it first?"

---

## `other`

**What it covers:** Anything that materially affects scope, behavior, or
correctness but doesn't fit the categories above. Use sparingly — most
questions fit one of the five categories above.

**Example questions:**
- "Is there a target launch date that constrains what we can build in this
  story, or is timing flexible?"
- "Are there any A/B test variants running on this surface that we need to
  account for?"
