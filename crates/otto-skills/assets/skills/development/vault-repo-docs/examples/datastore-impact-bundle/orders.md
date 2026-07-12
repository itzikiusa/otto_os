---
type: Data Asset
description: Stores one row per order and its current lifecycle status.
resource: /work/acme-orders
tags: [data, orders]
timestamp: 2026-07-12
---

# Overview

One row represents one order (`migrations/001_orders.sql:1`).

# Schema

| Field | Type | Description |
| --- | --- | --- |
| `id` | TEXT NOT NULL | Stable order identifier |
| `status` | TEXT NOT NULL | Current order lifecycle state |

# Access Paths

Create flow inserts both fields (`src/orders.rs:71`). Status lookup selects by
the primary key (`src/orders.rs:103`). Reconciliation updates `status`
(`src/reconcile.rs:48`).

# Indexes and TTL

`id` is the primary index. Rows have no TTL; retention is indefinite
(`migrations/001_orders.sql:1`).

# Transactions and Consistency

Order and outbox rows commit in one transaction (`src/orders.rs:66`).

# Field-level Impact

| Field | Writers | Readers | Change impact |
| --- | --- | --- | --- |
| `id` | `src/orders.rs:71` | `src/orders.rs:103` | Changes break API lookup and event correlation |
| `status` | `src/orders.rs:71`, `src/reconcile.rs:48` | `src/http.rs:91` | Changes affect API state and reconciliation |
