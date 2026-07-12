# API reviewer checklist and output

Check route inventory; exact method/path/operation ID; authentication and
authorization; all parameters; request content type/schema/validation/example;
successful and material error response bodies/examples; idempotency/retries;
side effects and flow links; route/DTO/handler/test citations; and parity with
OpenAPI operations/components/examples. For WebSockets/streams check handshake,
directional frames/events, termination, and errors.

Return only `[]` or objects with keys `severity`, `category`, `summary`, `doc`,
`source`, `evidence`, `repair`. Category is `api` or `coverage`; severity is
`blocker`, `major`, or `minor`. `source` is a real `path:line`. Do not report
stylistic preferences or infer a missing body when source explicitly has none.
