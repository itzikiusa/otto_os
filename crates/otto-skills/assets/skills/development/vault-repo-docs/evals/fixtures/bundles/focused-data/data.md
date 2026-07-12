---
type: Data Asset
description: Stores one row per order.
resource: /repo
tags: [data]
timestamp: 2026-07-12
---

# Schema

| Field | Type | Description |
| --- | --- | --- |
| `id` | text | Stable order identifier |

# Access Paths

Read with `SELECT` at `src/dao.rs:2`; write with `INSERT` at `src/dao.rs:8`.

# Indexes and TTL

The primary index is `id`; retention has no TTL.

# Transactions and Consistency

Writes use one atomic transaction.

# Field-level Impact

`id` is read at `src/dao.rs:2` and written at `src/dao.rs:8`.
