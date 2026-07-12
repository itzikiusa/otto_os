---
name: vault-data-review
description: Review Otto Vault repository documentation specifically for complete relational, document, analytical, and Redis schemas; real reads/writes; indexes/TTL; consistency; relationships; and field-level impact. Use as a focused post-scan reviewer; do not author or repair docs.
category: development
metadata:
  version: "1.0.0"
---

# Vault Data Review

Perform a read-only, source-backed datastore lens after a Vault scan.

## Workflow

1. Read [checklist.md](references/checklist.md).
2. Independently search migrations/schema, raw and built queries, ORM access,
   stored procedures, collection/key builders, call sites, and tests.
3. Reconcile every confirmed store asset and access path with coverage and its
   concept. Trace destination structs when queries use wildcards or dynamic SQL.
4. Emit only the finding JSON array; return `[]` when clean. Recheck repaired
   paths during later iterations and do not repeat resolved findings.

Caller focus may emphasize one store, but does not waive evidence or impact
analysis.

## Output contract

Output only the shared JSON finding array; an empty clean verdict is exactly `[]`.

## Non-trigger example

Do not use to optimize queries, change schemas, or author the bundle.
