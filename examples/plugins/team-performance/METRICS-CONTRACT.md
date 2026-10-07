# Estimate accuracy in overview responses

`GET /overview` returns `estimate_accuracy`, computed from completed, in-scope,
non-subtask tickets in the requested period. Eligible pairs require a finite,
positive effective estimate and a finite, nonnegative measured actual. Effective
estimates include saved lead corrections. Missing actuals are not zeroes.

Existing fields remain compatible:

- `histogram`: counts keyed by the existing ratio bucket IDs (`<0.5`, `0.5-0.8`,
  `0.8-1.25`, `1.25-2`, `>2`). Boundaries are lower-inclusive and upper-exclusive;
  the legacy `>2` key includes 2.
- `pct_within_25`: fraction within the inclusive range 0.75–1.25, or null without
  eligible pairs. `median_ratio` retains its existing definition.

The UI contract also includes:

- `bins`: ordered `{label, n}` entries for those same histogram counts.
- `within_25`: the same fraction as `pct_within_25`; `n`: eligible pair count.
- `worst`: at most 30 tickets outside the inclusive ±25% band, ordered by
  `abs(log(actual / estimate))` descending, then ticket key for ties. Each row
  includes `key`, `project`, `summary`, `assignee_id`, `assignee_name`, `est_days`,
  `actual_days`, and `ratio`. Missing descriptive fields are null. An observed
  zero actual has ratio zero and ranks first; no nonfinite value is serialized.

These rows drive the Estimates page's existing Correct action, which submits
`PUT /override` with account, project, key, corrected estimate and optional reason.
No eligible pairs yields zero-count bins, `n: 0`, null fractions, and `worst: []`.
Coverage/sample guardrails remain on the surrounding metric envelope.

## View periods and All time

A positive explicit `since` cutoff is retained exactly. With no cutoff (All time),
the view starts at the earliest valid creation/start/commit/completion/deployment
activity carried by selected records, rounded down to its UTC day boundary.
This includes `first_deployed_at`, and the existing opted-in Git feature records,
so window and metrics use the same population. Raw global PR and tag caches do
not establish scope ownership: Jira keys can collide across accounts, and a
record's aggregate repository list cannot identify its deployment's repository.
Their timestamps are excluded; this is record-derived history, not all Git history.
UTC matches the existing day-granular capacity calculation. Team and person
metrics use the same whole-scope observation window, including when one person's
history starts later. Missing, invalid and future timestamps do not enlarge it;
without valid history, `since` equals `until` and capacity is zero. There is no
implicit 90-day cutoff or epoch-1970 fallback.

The view observation instant is shared for the scope, cutoff and current UTC day,
matching its existing daily metric-cache cadence. Changing data rebuilds the
scope; explicit report start/end timestamps retain exact cache identities.
