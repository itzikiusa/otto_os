---
name: vault-evidence-review
description: Review Otto Vault repository documentation specifically for citation precision, claim-to-source support, source hierarchy, invented examples, stale contradictions, coverage-status evidence, and honest uncertainty. Use as a focused post-scan reviewer; do not author or repair docs.
category: development
metadata:
  version: "1.0.0"
---

# Vault Evidence Review

Use when a completed Vault scan needs a read-only claim-to-source audit.

## Workflow

1. Read [checklist.md](references/checklist.md).
2. Inspect every coverage exception and all load-bearing claims. Sample
   additional claims across API, data, and runtime concepts; expand if a sample
   exposes systematic citation drift.
3. Open each cited location and decide whether it directly supports the claim.
   Prefer executable source/migrations over tests, generated contracts, config,
   and prose. Record contradictions rather than choosing convenient evidence.
4. Emit only the finding JSON array or `[]`. Re-review repaired claims during
   later iterations without repeating resolved findings.

Caller focus chooses claims to prioritize, not a lower proof threshold. Do not
use for citation formatting alone, initial authoring, or general code review.

## Output contract

Output only the shared JSON finding array; an empty clean verdict is exactly `[]`.

## Non-trigger example

Do not use merely to reformat citations without checking their claims.
