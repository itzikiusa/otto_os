# Cost model & severity

A performance finding is a claim about **work as a function of input size and concurrency**.
Build the model first, then rank by it. A finding without a cost model is an opinion and
will be ignored (rightly).

## Step 1 — size the input (`n`)

Before anything else, name where the size comes from. The same line is a blocker or a
non-issue depending entirely on this.

| Where `n` comes from | Treat as | Example |
|---|---|---|
| Compile-time constant / small fixed enum | **bounded — usually no finding** | iterating 5 currencies, 3 retry attempts |
| Config list, feature set, brand count | **small but check growth** | per-brand loop (tens), per-provider list |
| `WHERE pk = ?` / `WHERE player_id = ?` | **bounded per entity** — but watch per-row fan-out | one player's open orders |
| Unbounded table / no `LIMIT` | **grows forever** — top suspect | scan of `GSS_activities`, full `SELECT` with no page |
| User/API-supplied count or page size | **adversarial** — assume the max; DoS-adjacent | client sends `pageSize=100000` |
| Per-request × concurrency | **multiply** — 10k concurrent users × the per-request cost | hot endpoint under load |

If you cannot determine `n` from the code, the finding is a **question**, not a blocker:
"if `items` is unbounded this is O(n²); what bounds it?"

## Step 2 — state the per-unit cost

Pick the dimension that dominates and quantify it *per unit of `n`*:

- **Round-trips** (the big one here): DB queries, Redis calls, HTTP requests. `1 + N` is
  the N+1 signature — count both numbers.
- **Complexity class**: O(n), O(n log n), O(n²). Name the nested operation that makes it.
- **Bytes / memory**: rows or objects held in memory; allocation size × frequency.
- **CPU**: serialize/deserialize, hashing, compression, regex per item.

"This is slow" is not a cost. "1 query + 1 query per order, ~30 orders/player → ~31
round-trips per page load" is.

## Step 3 — is the path hot?

Cost only matters where the path runs often or `n` grows. Classify it:

- **Hot** — per-request, per-row, per-event, per-message, in a tight loop, on the
  login/balance/bet path. Cost here is real.
- **Warm** — per-report, per-batch-job, per-admin-action. Cost matters at large `n`.
- **Cold** — once at startup, in a migration, a one-off admin script, an error path that
  almost never fires. **Micro-optimizing a cold path is not a finding.** Mention it only to
  say it's *not* a problem, if at all.

## Step 4 — rank by cost at scale

| Severity | Definition | Examples |
|---|---|---|
| **blocker** | Will not scale: falls over at production size or under load. | N+1 on a hot endpoint; missing index on a large-table query → full scan; unbounded memory growth across requests; unpaginated query over a growing table; per-request fan-out that's O(users). |
| **major** | Real cost that bites as the system grows or on a heavy-but-plausible input. | O(n²) over a list that reaches thousands; serial fan-out of dozens of network calls; re-running the same query every request; loading a whole large result set to compute one aggregate. |
| **minor** | Genuine inefficiency, narrow or only at large `n`, on a warm-ish path. | Redundant clone of a medium struct per request; an extra round-trip that's usually cache-warm; `SELECT *` pulling a few unused fat columns. |
| **nit** | Real but tiny cost on a path that isn't hot. | A needless allocation in a rarely-hit branch; a micro-inefficiency on a cold/admin path. Report it labeled; never inflate it. |

When torn between two levels, **state the trigger size and pick the lower** — then the
author re-ranks with the facts. A blocker that only triggers at `n > 1M` on a table capped
at 10k is really a minor; say so.

## Step 5 — false-positive filter (run before submitting)

For each finding, ask:

- **Did I size `n`?** If `n` is provably bounded and small, cut it — quadratic over 8 items
  is free.
- **Is the path actually hot?** If it's startup/migration/admin one-off, downgrade to nit
  or drop it. No premature optimization.
- **Did I count, or pattern-match?** "Looks like an N+1" → go count the queries. If you
  can't, mark it a question.
- **Is there already an index / cache / bound** upstream that I didn't read? Check the
  migration and the schema before asserting "missing index."
- **Does my fix actually pay off, and not break correctness?** State the new cost (N→1,
  O(n²)→O(n)). A "fix" that trades a tiny win for a correctness risk is not a finding — that
  belongs to another lens.

A short list where every item survives this filter beats a long one that doesn't. The whole
value of this lens is that the author *trusts* a flagged perf issue is real.
