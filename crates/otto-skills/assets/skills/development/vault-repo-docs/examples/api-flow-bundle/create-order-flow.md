---
type: flow
description: Traces order creation from HTTP request through outbox publication.
resource: /work/acme-orders
tags: [flow, orders]
timestamp: 2026-07-12
---

# Trigger

`POST /orders` enters the handler at `src/http.rs:42`.

# Steps

1. Validate the request DTO (`src/dto.rs:8`).
2. Insert the order and outbox row atomically into MySQL `orders` + `outbox`
   (`src/orders.rs:66`).
3. Return the persisted status (`src/http.rs:55`).

# Request example

```json
{"id": "o1", "sku": "ABC-1", "quantity": 2}
```

# Response example

```json
{"status": "accepted", "id": "o1"}
```

# Failure and retry

Validation failures return `400`; duplicate IDs return `409`. Retried requests
use the order ID as the idempotency boundary (`tests/orders.rs:88`).

```d2
api: "POST /orders"
db: "MySQL orders + outbox" { shape: cylinder }
api -> db: atomic insert
db -> api: persisted status
```
