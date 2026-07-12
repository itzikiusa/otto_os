# API documentation completion contract

Trace each operation from route registration through middleware, handler,
request DTO, service calls, response DTO, and material tests. Do not infer a
body from a type name alone.

Every operation documents:

- method, exact path, operation ID, purpose, authentication and authorization;
- path, query, header, and cookie parameters including requiredness/defaults;
- content type, complete request body schema, validation and a realistic body
  example, or an explicit statement that no body exists;
- every successful status with full response body and example;
- material 4xx/5xx statuses with response bodies and triggering conditions;
- side effects, idempotency/retry behavior, and a link to the runtime flow;
- citations for registration, handler, request/response types, and tests.

The OpenAPI artifact must contain the same operations and shapes. Use
`otto_vault_write_file` for `api-openapi.yaml`, validate it as YAML/OpenAPI,
and reconcile operation IDs against `api.md`. A schema name without fields,
`additionalProperties: true`, or an empty example is a stub, not documentation.

Document streaming/websocket endpoints with handshake, frames/events,
direction, closure/error behavior, and examples for both directions.
