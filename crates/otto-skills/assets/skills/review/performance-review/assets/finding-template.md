# Finding template

Emit each finding in this shape. Keep it tight — one finding, one cost problem. The
**Cost** line is what makes this a performance finding rather than a vibe: it must state
`n`, the per-unit cost, and the scale at which it bites.

```
### [SEVERITY] <short, specific title>   — `path/to/file.ext:line`

**What:** <the pattern — N+1 / full scan / O(n²) / unbounded growth / blocking I/O / …, in one or two sentences>
**Cost:** n = <where the size comes from>; per unit = <round-trips | complexity class | bytes | allocs>; total ≈ <the math>. Path is <hot: per-request/row/event | cold: startup/migration/admin>. Bites at <input size / concurrency>.
**Evidence:** <confirmed | likely | question> — <how you know: counted the queries, read the schema/migration for the index, sized the table, hand-traced the loop>
**Fix:** <concrete change> → new cost <N→1 | O(n²)→O(n) | scan→index seek | bounded>.
```diff
- <the costly line>
+ <the cheaper line>
```
<or a precise instruction if a diff doesn't fit — e.g. "add index `idx_player_created (player_id, created_at)` in the migration; without it this is a full scan">
```

## Report skeleton

```
**Verdict:** Block | Approve with fixes | Approve
**Counts:** blocker N · major N · minor N · nit N · question N
**Scope sized:** <which inputs you sized, the worst-case n you assumed, and which hot paths you traced>

<findings, ordered: blockers first, then by file>

### Smallest things (nits)
<the cold-path / tiny-cost nits, collected and labeled so none masquerade as urgent>

### Out of scope — noticed in passing
<one line each for any non-perf defect (correctness/security/style) — for grill, not this lens>

<if it scales: "No scaling problems found. Sized: <inputs + worst-case n>. Why it holds: <indexed lookups, bounded n, batched access, …>.">
```
