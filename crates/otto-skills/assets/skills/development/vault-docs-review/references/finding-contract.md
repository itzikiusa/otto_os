# Shared finding contract

Return one JSON array and no surrounding prose. Each actionable finding is:

```json
{
  "severity": "blocker|major|minor",
  "category": "coverage|api|data|runtime|evidence|quality",
  "summary": "Concise, unique problem",
  "doc": "bundle-relative/path.md",
  "source": "repo/relative/path.rs:42",
  "evidence": "What the docs claim or omit and what the source proves",
  "repair": "Concrete change the author can verify"
}
```

`doc` may be `coverage.md` or the missing destination path. `source` must be a
real `path:line`; use an empty string only for a bundle-internal structural gap.
Severity: blocker means completion is materially false or unsafe; major means a
reader cannot use the concept reliably; minor is bounded quality loss.

Findings must be independently actionable, deduplicated, and source-backed.
Return `[]` for a clean round. Malformed JSON, prose outside the array, or a
finding without evidence is a reviewer failure, not a clean verdict.
