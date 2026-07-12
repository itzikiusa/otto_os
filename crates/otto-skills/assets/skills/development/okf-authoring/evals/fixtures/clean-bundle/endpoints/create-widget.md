---
type: API Endpoint
title: Create widget
description: Creates one widget.
resource: "POST /v1/widgets"
timestamp: 2026-07-12T09:00:00Z
---

# Overview

Creates a widget for an authenticated service account.

# Authentication

Requires a bearer token with `widgets:write`.

# Parameters

No path or query parameters.

# Request

```json
{"name":"blue-widget"}
```

# Success Response

```json
{"id":"wid_123","name":"blue-widget"}
```

# Error Responses

`422` is returned when `name` is empty.

```json
{"code":"invalid_name","message":"name must not be empty"}
```

# Validation and Side Effects

The handler validates `name` and inserts one widget row in the same request.

# Flow

The handler calls the widget service and repository.

# Citations

[1] [Route source](https://example.invalid/source/widgets#L10)
