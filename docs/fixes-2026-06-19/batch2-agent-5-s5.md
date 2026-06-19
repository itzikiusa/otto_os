# Batch 2 — Agent 5 — S5: login-throttle bypass + auth security tests

## The bug (user-verified)

The batch-1 login throttle keyed on `client_ip(headers)`, which trusted
`X-Forwarded-For` / `X-Real-IP`. Nothing trusted sits in front of the daemon
(Tailscale and the Tauri shell connect directly), so an attacker could rotate
the XFF header per request → a fresh `ip|username` key each time → the per-key
counter never reached `FAILURE_THRESHOLD` → the lockout was fully bypassed.

## Fix

### 1. Key on the real socket peer, never a header

The throttle was extracted out of `auth_routes.rs` into a new module
`crates/otto-server/src/login_throttle.rs` (registered in `lib.rs`). The IP no
longer comes from headers at all — `X-Forwarded-For` / `X-Real-IP` are not read
anywhere (no trusted-proxy setting exists, so honoring them would re-open the
bypass). Instead the `login` handler extracts `ConnectInfo<SocketAddr>` and uses
`peer.ip()`:

```rust
pub async fn login(
    State(ctx): State<ServerCtx>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginReq>,
) -> Response {
    handle_login(&ctx, login_throttle::global(), Some(peer.ip()), req).await
}
```

### 2. Connect-info wired into BOTH serve calls (`crates/ottod/src/main.rs`)

For `ConnectInfo<SocketAddr>` to be populated, the router must be served with
connect-info make-services. Both listeners were updated:

- Loopback (`axum::serve`):
  ```rust
  axum::serve(
      loopback,
      router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
  )
  .with_graceful_shutdown(shutdown)
  .await
  ```
- Network TLS (`axum_server::bind_rustls(addr, tls)`):
  ```rust
  axum_server::bind_rustls(addr, tls)
      .handle(handle)
      .serve(router.into_make_service_with_connect_info::<std::net::SocketAddr>())
      .await
  ```
  `axum_server` 0.8 supports `into_make_service_with_connect_info` (the
  `MakeServiceWithConnectInfo` make-service works with its `serve`).

### 3. Global per-username lockout (anti-rotation)

`handle_login` now tracks **two** keys per attempt and EITHER can lock the
request:

- `login_throttle::ip_key(peer, username)` → `"<ip>|<username>"` — per-client
  tally;
- `login_throttle::username_key(username)` → `"user:<username>"` — a **global**
  per-username tally, independent of the source IP.

```rust
let ip_key   = login_throttle::ip_key(peer, &req.username);
let user_key = login_throttle::username_key(&req.username);

// pre-attempt gate: locked iff EITHER key is locked
if let Some(retry_after) = attempts.max_locked(&[&ip_key, &user_key]) {
    return too_many_requests(retry_after);
}
// on Unauthorized: record_failure to BOTH keys, then re-check
// on success:      clear BOTH keys
```

So even if the source IP rotates every request (no `ip|username` key ever
reaches the threshold), the username key still trips after `FAILURE_THRESHOLD`
failures against any one account. Existing bounds are unchanged
(`FAILURE_THRESHOLD = 5`, `FAILURE_WINDOW = 15m`, `LOCKOUT_DURATION = 15m`,
`MAX_TRACKED_KEYS = 10_000`). Legit-user happy path is intact: a successful
login clears both keys.

The throttle logic itself (`AttemptStore`, sliding window, prune, lockout) is
unchanged behaviorally — it was only moved and given an explicit, testable
instance (`AttemptStore::default()`) alongside the process-global
`login_throttle::global()` used in production.

## Tests added

### `crates/otto-rbac/src/tokens.rs` (new `#[cfg(test)]` module — no logic changes)

Added `[dev-dependencies] tokio` to `otto-rbac/Cargo.toml`. Tests run against an
in-memory SQLite pool with the real otto-state migrations
(`sqlx::migrate!("../otto-state/migrations")`, the same pattern otto-product
uses):

- `session_token_lifecycle` — mint → authenticate → revoke → auth fails.
- `api_token_lifecycle_mint_list_auth_revoke` — mint → appears in list (prefix
  only, never the secret) → authenticate → revoke by id → auth fails → list
  empty → second revoke is a no-op.
- `api_token_revoke_is_owner_scoped` — another user cannot revoke your token.
- `revoke_all_for_user_invalidates_every_token` — credential-change revocation
  wipes the victim's session + both API tokens (returns 3), all stop
  authenticating, a bystander's token is untouched.

### `crates/otto-server/tests/auth_security.rs` (new)

Repo-level token lifecycle via `otto_rbac::AuthRepo` over a temp SQLite pool,
plus throttle property tests that drive `otto_server::login_throttle` exactly the
way the `/auth/login` handler does (same `ip_key` / `username_key` /
`max_locked` calls, via small `record_failed_attempt` / `is_blocked` mirrors of
the handler's bookkeeping):

- `token_mint_authenticate_revoke_cycle`
- `revoke_all_on_credential_change_invalidates_outstanding_tokens`
- `lockout_trips_after_threshold_and_resets_on_success` — locks at threshold;
  clearing both keys (success) resets it.
- `ip_rotation_does_not_defeat_username_lockout` — **the S5 property**: rotating
  the IP per request leaves every per-client key below threshold, yet the global
  username key locks, and a fresh IP is then blocked outright.
- `forwarding_headers_are_not_a_throttle_input` — the key is derived purely from
  socket-peer IP + username (case-insensitive), so a spoofed forwarding header
  has no key to rotate.

The `login_throttle` module also carries two in-module unit tests
(`locks_after_threshold_then_clears_on_success`,
`username_lockout_survives_ip_rotation`).

## Verification

| Command | Result |
| --- | --- |
| `cargo check -p otto-server -p ottod -p otto-rbac` | OK (warnings only in `otto-usage`, not in scope) |
| `cargo test -p otto-rbac` | 6 passed, 0 failed |
| `cargo test -p otto-server --test auth_security` | 5 passed, 0 failed |
| `cargo test -p otto-server --lib login_throttle` | 2 passed, 0 failed |

## Files touched

- `crates/otto-server/src/login_throttle.rs` (new) — throttle module + keys + unit tests
- `crates/otto-server/src/lib.rs` — `pub mod login_throttle;`
- `crates/otto-server/src/routes/auth_routes.rs` — `ConnectInfo<SocketAddr>`, dual-key gate, delegate to `login_throttle`; drop header-based `client_ip`
- `crates/ottod/src/main.rs` — `into_make_service_with_connect_info::<SocketAddr>()` on both serve calls
- `crates/otto-rbac/Cargo.toml` — add `[dev-dependencies] tokio`
- `crates/otto-rbac/src/tokens.rs` — token-lifecycle test module (tests only)
- `crates/otto-server/tests/auth_security.rs` (new) — token lifecycle + throttle property tests
