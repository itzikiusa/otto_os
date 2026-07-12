# Fixture: polyglot repository

- `src/api.rs:12` registers `POST /orders` and names request/response DTOs.
- `src/orders.sql:1` creates `orders`; DAO code selects, inserts, and updates it.
- `src/events.go:30` consumes `order.requested` and publishes `order.created`.
- `src/reconcile.py:17` registers an hourly reconciliation worker.
- Tests contain representative HTTP and event payloads.
