---
name: vault-api-review
description: Review Otto Vault repository documentation specifically for complete HTTP/RPC/WebSocket contracts, real request and response bodies, errors, auth, examples, OpenAPI parity, and runtime side effects. Use as a focused post-scan reviewer; do not author or repair docs.
category: development
metadata:
  version: "1.0.0"
---

# Vault API Review

Perform a read-only, source-backed API lens after a Vault scan.

## Workflow

1. Read [checklist.md](references/checklist.md) and the shared JSON contract it contains.
2. Inventory route/RPC/socket registration independently; reconcile it with API
   coverage, Markdown operations, OpenAPI paths, and flow notes.
3. For every operation trace middleware, handler, request DTO, response DTO,
   errors, service calls, and tests. Type names and empty schemas are not bodies.
4. Emit only the finding JSON array. Return `[]` when clean. In a repair
   iteration, re-read the changed artifacts and omit resolved findings.

Caller focus narrows which APIs deserve extra depth; it never waives evidence,
contract bodies, or consistency checks.

## Output contract

Output only the shared JSON finding array; an empty clean verdict is exactly `[]`.

## Non-trigger example

Do not use for general code review or initial documentation authoring.
