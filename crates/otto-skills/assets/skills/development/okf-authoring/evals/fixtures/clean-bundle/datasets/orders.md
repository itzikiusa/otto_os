---
type: Data Asset
title: Orders
description: Stores one row per accepted order.
resource: "postgresql://commerce/orders"
timestamp: 2026-07-12T09:00:00Z
---

# Overview

One row represents one accepted order retained for seven years.

# Schema

| Field | Type | Description |
|---|---|---|
| id | uuid | Stable order identifier. |
| status | text | Current order state. |

# Access Paths

The create-order repository inserts rows; order lookup reads by `id`.

# Indexes and TTL

The primary-key index covers `id`; retention is enforced by an archive job rather than database TTL.

# Transactions and Consistency

The order and outbox rows are inserted in one transaction; readers use committed data.

# Relationships

`orders.id` is referenced by `order_items.order_id`.

# Impact

Create order writes `id` and `status`; cancel order updates only `status`; reporting reads both fields.

# Examples

```sql
SELECT id, status FROM orders WHERE id = $1;
```

# Citations

[1] [Migration source](https://example.invalid/source/migrations/orders.sql#L1)
