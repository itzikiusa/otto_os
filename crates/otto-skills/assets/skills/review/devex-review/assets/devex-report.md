# Devex review report

Fill this in. Lead with the verdict; back every finding with a shown friction and an
improvement (see `references/severity-and-finding.md`). If no developer-facing surface
changed, say so in one line and stop — don't manufacture findings.

---

**Verdict:** Block | Approve with fixes | Approve
**Counts:** blocker N · major N · minor N · nit N · question N
**Surface reviewed:** <the endpoints / signatures / flags / errors / docs you walked, + how
far into the callers and neighbors you read>

## Exposed surface inventory

What this change adds or alters that another developer calls, reads, or imports — and whose
experience each affects.

| Surface | Kind (fn / endpoint / flag / error / config / doc) | Caller / audience |
|---|---|---|
| `mod::fn(...)` | public fn | library consumers |
| `POST /v1/...` | endpoint | service integrators |
| `--flag` | CLI flag | humans at the terminal |
| `XError` | emitted error | on-call debuggers |

<if empty: "No developer-facing surface changed — nothing to review for ergonomics.">

## Findings

Ordered: blockers first, then by surface. Use the finding template from
`references/severity-and-finding.md`.

<findings>

## Smallest things (nits)

The nits collected here so none are lost.

<nits, or "none">

## Off-lens (FYI)

Issues outside the devex lens — correctness, concurrency, leaks, internal coupling. One line
each; not devex findings. Hand to grill / correctness-review / architecture-review.

- `file:line` — <one clause>
- <or "none noticed">

## Journey-stage check (optional)

Which stage of the developer journey this change touches, and whether it leaves a hole.

| Stage | Touched? | Gap? |
|---|---|---|
| Discover (can they find it?) | | |
| First call (TTHW — can they make one correct call fast?) | | |
| Integrate (real use, not the toy path) | | |
| Debug (does the error help when it fails?) | | |
| Upgrade (migration path for existing callers?) | | |

<if clean: "Walked the surface as the caller — wrote the N common call sites, read the M error
paths, checked the new flag's `--help` and the README example. Pleasant to use; no friction
found.">
