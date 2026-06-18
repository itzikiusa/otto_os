# Architecture review report

Fill this in. Emit each finding in the shape below — keep it tight, one finding, one
structural problem. The fields force a cost prediction, not a vibe. The report skeleton at
the bottom is what you hand back.

```
### [SEVERITY] <short, specific title>   — `path/to/file.ext:line` (or `module/`)

**What:** <the structural problem, in one or two sentences — use the vocabulary: shallow, leaky, low cohesion, wrong seam, coupled>
**Future cost:** <the concrete change this makes slow / risky / duplicated — name the next edit and what it forces>
**Deletion test / two-adapter check:** <if you add or remove an abstraction: does deleting it concentrate complexity (keep) or move it (cut)? Do two adapters justify the seam? — omit if the finding isn't about a layer>
**Evidence:** <grounded | likely | question> — <how you know: which neighbours/callers/siblings you read; the duplication you found; the variant that's missing>
**Refactor direction:**
<concrete, sized to this change — "extract X into module Y; the caller then only sees Z" — plus a one-line note on what it costs. Not "improve the design," not "rewrite.">
```

## Report skeleton

```
**Verdict:** Block | Approve with design notes | Approve
**Counts:** blocker N · major N · minor N · nit N · question N
**Scope reviewed:** <files/modules + which neighbours, callers, and sibling modules you read to judge shape>
**Codebase calibration:** <the existing modules you compared against — the standard for "right abstraction / right size / right convention">

<findings, ordered: blockers first, then by file>

### Smallest things (nits)
<the structural nits, collected so none are lost — each still with a future-cost rationale>

<if the design is sound: "No structural problems found after lenses 1–8 + the fit pass.
Checked: <what you read and compared against>. The change fits the codebase's existing
shape for <X>.">
```
