# Shared finding contract

Return one JSON array and no surrounding prose. Each actionable finding is:

```json
{
  "severity": "blocking|major|minor",
  "category": "coverage|api|data|runtime|evidence|quality",
  "summary": "Concise, unique problem",
  "evidence": [
    {"repo_path": "repo/relative/path.rs", "line": 42},
    {"doc_path": "bundle-relative/path.md", "section": "Section name"}
  ],
  "missed_item": "The exact contract, flow, field, or proof that is absent",
  "required_fix": "Concrete change the author can verify"
}
```

Evidence entries use either `repo_path` with a positive `line`, or `doc_path`
with a non-empty `section`. Include both when comparing source to docs. A
bundle-internal structural gap may use only doc evidence. Severity: `blocking`
means completion is materially false or unsafe; `major` means a reader cannot
use the concept reliably; `minor` is bounded quality loss.

Findings must be independently actionable, deduplicated, and source-backed.
Return `[]` for a clean round. Malformed JSON, prose outside the array, or a
finding without evidence is a reviewer failure, not a clean verdict.
