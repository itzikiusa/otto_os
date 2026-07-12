# Fixture: unavailable contract

`src/router.rs:12` registers `POST /orders`, but the request and response DTOs
come from an unavailable generated dependency. The evaluator must not accept a
plausible body invented from the route name.
