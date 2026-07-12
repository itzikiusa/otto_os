# API reviewer checklist and output

Check route inventory; exact method/path/operation ID; authentication and
authorization; all parameters; request content type/schema/validation/example;
successful and material error response bodies/examples; idempotency/retries;
side effects and flow links; route/DTO/handler/test citations; and parity with
OpenAPI operations/components/examples. For WebSockets/streams check handshake,
directional frames/events, termination, and errors.

Return only `[]` or objects with keys `severity`, `category`, `summary`,
`evidence`, `missed_item`, `required_fix`. Evidence is an array containing
`{"repo_path":"path","line":42}` and/or
`{"doc_path":"path","section":"heading"}`. Category is `api` or `coverage`;
severity is `blocking`, `major`, or `minor`. Do not report
stylistic preferences or infer a missing body when source explicitly has none.
