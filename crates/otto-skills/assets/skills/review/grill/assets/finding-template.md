# Finding template

Emit each finding in this shape. Keep it tight — one finding, one defect.

```
### [SEVERITY] <short, specific title>   — `path/to/file.ext:line`

**What:** <the defect, in one or two sentences>
**Why it matters:** <the consequence — what breaks, for whom, when>
**Evidence:** <confirmed | likely | question> — <how you know: the trace, the input, the test that would fail>
**Fix:**
```diff
- <the wrong line>
+ <the corrected line>
```
<or a precise instruction if a diff doesn't fit>
```

## Report skeleton

```
**Verdict:** Block | Approve with fixes | Approve
**Counts:** blocker N · major N · minor N · nit N · question N
**Scope reviewed:** <files/areas + how far into callers you looked>

<findings, ordered: blockers first, then by file>

### Smallest things (nits)
<the nits, collected so none are lost>

<if clean: "No defects found after sweeping passes 1–12 + completeness. Checked: …">
```
