---
name: vault-runtime-review
description: Review Otto Vault repository documentation specifically for runtime flows, messages, schedules, reconciliation, startup/shutdown, failure handling, retries, idempotency, observability, and external side effects. Use as a focused post-scan reviewer; do not author or repair docs.
category: development
metadata:
  version: "1.0.0"
---

# Vault Runtime Review

Perform a read-only, source-backed runtime lens after a Vault scan.

## Workflow

1. Read [checklist.md](references/checklist.md).
2. Independently inventory registered API/RPC triggers, broker consumers and
   producers, schedules/timers, reconciliation loops, commands, startup, and
   shutdown. A function name is not registration evidence.
3. Trace happy path, data access, side effects, boundaries, errors, retries,
   idempotency, concurrency, and observability; reconcile with coverage/flows.
4. Emit only the finding JSON array or `[]`. Later iterations start from the
   repaired bundle and omit resolved findings.

Caller focus adds depth but never waives source registration or failure-path
evidence.

## Output contract

Output only the shared JSON finding array; an empty clean verdict is exactly `[]`.

## Non-trigger example

Do not use for runtime debugging, implementation, or initial authoring.
