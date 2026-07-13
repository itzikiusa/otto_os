---
name: vault-docs-review
description: Review an Otto Vault repository-documentation run for missed surfaces, shallow concepts, unsupported claims, broken coverage, and incomplete API/data/runtime detail. Use as the default general reviewer after a Vault scan; do not use to author or silently repair the bundle.
category: development
version: 2
metadata:
  version: "2.0.0"
---

# Vault Documentation Review

Independently review the current bundle against the repository and the scan's
manifest/coverage ledger. You are a read-only reviewer: report actionable gaps;
do not edit docs or source.

## Workflow

1. Read [finding-contract.md](references/finding-contract.md) and
   [checklist.md](references/checklist.md).
2. Read the request, manifest, coverage ledger, bundle index, and relevant
   source. Do not trust the author's completion statement.
3. Reconcile every manifest candidate and coverage row. Inspect every in-scope
   documented concept against its core API/data/runtime/quality contract, plus
   all `uncertain`, `irrelevant`, and generated rows. Uninspected eligible rows
   prevent a clean verdict.
4. Check completeness, cross-links, examples, API/data/runtime depth, and
   consistency between Markdown and text artifacts exhaustively. Sampling is
   allowed only for secondary claim-level citation auditing after every concept
   passes its structural and type-specific contract; expand the sample whenever
   it reveals a systematic evidence gap.
5. Emit only the JSON array required by the finding contract. Return `[]` only
   when no actionable finding remains. Never invent a finding to consume an
   iteration.

Use the caller's focus as an additional lens, not permission to waive evidence
or completeness. A later iteration reviews the repaired bundle from scratch and
does not repeat resolved findings.

## Output contract

Output only the shared JSON finding array; an empty clean verdict is exactly `[]`.

## Non-trigger example

Do not use this skill to write the initial bundle, review application code, or
answer a one-off repository question.
