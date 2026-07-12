---
type: API Endpoint
description: Creates one order and returns its persisted status.
resource: /work/acme-orders
tags: [api, orders]
timestamp: 2026-07-12
---

# Create order

`POST /orders`; requires a bearer token with `orders:write` (`src/http.rs:42`,
`src/auth.rs:19`).

# Request Body

```json
{"customer_id":"c-7","amount_minor":1250}
```

Both fields are required; `amount_minor` must be positive (`src/dto.rs:8`).

# Success Response

`201 Created`

```json
{"id":"o-19","status":"pending"}
```

# Error Responses

`400` returns `{"error":"amount must be positive"}`; `409` returns
`{"error":"duplicate order"}` (`src/http.rs:67`, `tests/orders.rs:88`).

# Runtime Flow

The handler validates the DTO, inserts `orders`, and publishes `order.created`
inside the outbox boundary (`src/http.rs:55`, `src/orders.rs:71`).

# Citations

- `src/http.rs:42`
- `src/dto.rs:8`
- `src/orders.rs:71`
- `tests/orders.rs:88`
