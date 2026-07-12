---
type: API Endpoint
title: Create widget
description: Creates one widget.
resource: "POST /v1/widgets"
timestamp: 2026-07-12T09:00:00Z
---

# Overview

Creates a widget for an authenticated caller.

# Authentication

Requires a bearer token with `widgets:write`.

# Parameters

No path or query parameters.

# Validation

The name must be non-empty.

# Side Effects

Inserts one widget row.

# Flow

The handler calls the widget service and repository.

# Citations

[1] [Route source](https://example.invalid/source/widgets#L10)
