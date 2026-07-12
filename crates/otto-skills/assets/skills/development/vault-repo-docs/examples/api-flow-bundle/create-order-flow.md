---
type: Runbook
description: Traces order creation from HTTP request through outbox publication.
resource: /work/acme-orders
tags: [flow, orders]
timestamp: 2026-07-12
---

# Trigger

`POST /orders` enters the handler at `src/http.rs:42`.

# Steps

1. Validate the request DTO (`src/dto.rs:8`).
2. Insert the order and outbox row atomically (`src/orders.rs:66`).
3. Return the persisted status (`src/http.rs:55`).

# Failure and retry

Validation failures return `400`; duplicate IDs return `409`. Retried requests
use the order ID as the idempotency boundary (`tests/orders.rs:88`).
