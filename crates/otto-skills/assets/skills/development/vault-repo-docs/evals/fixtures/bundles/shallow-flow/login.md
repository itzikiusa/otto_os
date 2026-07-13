---
type: API Endpoint
description: Player login operation.
resource: /work/acme-login
tags: [api]
timestamp: 2026-07-13
---

# POST /login/{brand_id}/player

# Authentication

`x-auth-token` header validated at `src/auth.rs:9`.

# Request Body

```json
{"user_name": "player1", "password": "S3cret12"}
```

Required `password`; see `src/dto.rs:4`.

# Success Response

```json
{"status": "ok", "token": "abc"}
```

# Errors

400 returns `{"error":"invalid"}`.

# Flow

[Login flow](flows/login.md)

# Citations

`src/http.rs:12` `src/dto.rs:4` `src/handler.rs:8`
