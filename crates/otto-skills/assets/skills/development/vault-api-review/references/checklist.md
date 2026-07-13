# API reviewer checklist and output

Check route inventory; exact method/path/operation ID; authentication and
authorization; all parameters; request content type/schema/validation/example;
successful and material error response bodies/examples; idempotency/retries;
side effects and flow links; route/DTO/handler/test citations; and parity with
OpenAPI operations/components/examples. For WebSockets/streams check handshake,
directional frames/events, termination, and errors.

Parameter completeness is a per-operation check, not a sample: every
`{placeholder}` in a path resolves to a declared `in: path` parameter; every
auth/tenant header the handler reads (`x-auth-token`, `jwt-auth`, brand/player
identifiers) appears as a header parameter or referenced security scheme; and
every parameter `$ref` resolves to a `components.parameters` entry carrying
`name` and `in`. Resolve refs yourself — a parameter that only exists as an
unresolved reference is a missing parameter.

Return only `[]` or objects with keys `severity`, `category`, `summary`,
`evidence`, `missed_item`, `required_fix`. Evidence is an array containing
`{"repo_path":"path","line":42}` and/or
`{"doc_path":"path","section":"heading"}`. Category is `api` or `coverage`;
severity is `blocking`, `major`, or `minor`. Do not report
stylistic preferences or infer a missing body when source explicitly has none.
