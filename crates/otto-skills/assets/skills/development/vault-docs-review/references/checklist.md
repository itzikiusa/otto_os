# General review checklist

- Every manifest candidate appears exactly once in coverage with a defensible status.
- Every documented row links to a concept that satisfies its type's contract.
- Overview, index, log, scan marker, links, and non-Markdown artifacts agree.
- API request/response/error bodies and examples match route/DTO/handler/tests.
- Data schema, reads, writes, indexes/TTL, consistency, and field impact are traced.
- Producers, consumers, schedules, reconciliation, startup, shutdown, retries,
  idempotency, and side effects are covered.
- Every OpenAPI `{placeholder}` and auth header resolves to a declared
  parameter (follow `$ref`s into `components.parameters` yourself).
- Flow notes fill the required skeleton: numbered steps naming each store as
  engine + table/collection/key-pattern, request/response examples on
  HTTP-triggered flows, and a diagram whose store nodes carry the engine name.
  A flow whose diagram shows only generic service boxes is shallow.
- Load-bearing claims have precise source citations and examples are not invented.
- No remaining uncertainty is hidden behind a completion claim.

Common false positives: generated code intentionally reconciled with provenance;
an explicit evidence-backed no-body/no-TTL/no-write statement; docs outside the
requested focus marked irrelevant with a concrete focus reason.
