# Complete API endpoint example

The values are illustrative; replace them with cited source evidence.

````markdown
---
type: API Endpoint
title: Create widget
description: Creates one widget for the authenticated tenant.
resource: "POST /v1/widgets"
tags: [widgets, write]
timestamp: 2026-07-12T09:00:00Z
---

# Overview

Creates one widget and returns its stable identifier.

# Authentication

Requires a bearer token with `widgets:write`; the tenant comes from the verified token.

# Parameters

No path or query parameters. `Content-Type: application/json` is required.

# Request

| Field | Type | Required | Validation |
|---|---|---|---|
| name | string | yes | Trimmed, 1–80 characters. |
| color | string | no | One of `blue`, `green`; defaults to `blue`. |

```json
{"name":"daily-summary","color":"blue"}
```

# Success Response

Status: `201 Created`

| Field | Type | Description |
|---|---|---|
| id | string | Stable widget identifier. |
| name | string | Stored display name. |
| color | string | Stored color. |

```json
{"id":"wid_123","name":"daily-summary","color":"blue"}
```

# Error Responses

| Status | Trigger |
|---|---|
| 422 | A request field fails validation. |
| 409 | The tenant already owns a widget with this name. |

```json
{"code":"invalid_name","message":"name must contain 1 to 80 characters"}
```

```json
{"code":"widget_exists","message":"a widget with this name already exists"}
```

# Validation

Validation runs before persistence. Unknown JSON fields are rejected.

# Side Effects

Inserts one `widgets` row and emits `widget.created` in the same documented transaction boundary.

# Flow

The route calls the widget service, which checks uniqueness and writes through the widget repository; see [Create widget flow](../flows/create-widget.md).

# Citations

[1] [Route and DTO](../../src/api/widgets.rs#L42)
[2] [Service transaction](../../src/widgets/service.rs#L88)
````
