# Correctness review — output template

Fill this in. One finding, one defect. Every finding is anchored to **intended behavior**
and backed by a **trace or repro** — that's what makes it a correctness finding and not a
preference. Drop the placeholder lines; keep the shape.

---

**Verdict:** Block | Approve with fixes | Approve
**Counts:** blocker N · major N · minor N · nit N · question N
**Intended behavior reviewed against:** <the contract / ticket / test names you anchored on>
**Scope traced:** <files + which inputs & branches you actually hand-traced or reproduced —
e.g. "compute(): empty, single, n=len boundary, null userId, error path">

---

## Findings

<ordered: blockers first, then by file>

### [blocker] <short, specific title>   — `path/to/file.ext:line`

**What:** <the wrong behavior, in one or two sentences>
**Intended:** <what it's supposed to do — the oracle you judged against>
**Why it matters:** <consequence — what breaks, for whom, when>
**Evidence:** confirmed — <the trace: the exact input, the line where actual diverges from
intended, actual vs intended result; or the reproduction command + its pasted output>
**Fix:**
```diff
- <the wrong line>
+ <the corrected line>
```
<or a precise instruction if a diff doesn't fit>

### [major] <title>   — `path/to/file.ext:line`

**What:** …
**Intended:** …
**Why it matters:** …
**Evidence:** confirmed | likely — …
**Fix:** …

### [minor] <title>   — `path/to/file.ext:line`

**What:** …
**Intended:** …
**Why it matters:** …
**Evidence:** likely — …
**Fix:** …

## Questions (could not verify from the code alone — not defects)

### [question] <title>   — `path/to/file.ext:line`

**Ask:** <phrased as a question — e.g. "Is `userId` guaranteed non-null at line 88? If a
caller can pass null, this derefs nil.">
**What I'd need to confirm:** <the caller / invariant / input you couldn't see>

## Smallest things (nits)

<the latent / narrow / confusing-but-correct items, collected so none are lost — each with
file:line and a one-line why>

## Off-lens (one line each — NOT the correctness lane)

<anything non-correctness you happened to notice — a leak, a missing index, a secret in a
log. Hand off to security / performance / grill. Do not expand these.>

---

<If the change is correct, replace the Findings section with:>

**No correctness defects found.** Traced these inputs through `<units>` and each matches
intended behavior:
- empty / zero-length input → <result>
- single element → <result>
- boundary (first / last / exactly-at-limit) → <result>
- null / None / nil / absent → <result>
- error / retry / second-call path → <result>
