# Agent 2 — Auth hardening + doctest (2026-06-19)

Hardening pass on Otto auth (audit S5, §1/§2). Files owned & touched:
`crates/otto-server/src/routes/auth_routes.rs`, `crates/otto-server/src/routes/users.rs`,
`crates/otto-rbac/src/tokens.rs`, `crates/otto-rbac/src/lib.rs`,
`crates/otto-rbac/src/passwords.rs`.

## Checklist

- [x] **T4a — Login rate-limiting / lockout (S5)**
  - `crates/otto-server/src/routes/auth_routes.rs`
  - Added an in-memory throttle store (`static OnceLock<Mutex<HashMap<String, Attempts>>>`,
    std only — no new crate). New helpers: `login_attempts`, `client_ip`, `throttle_key`,
    `check_locked`, `record_failure`, `clear_failures`, `prune_expired`, `too_many_requests`.
  - `login` now takes `HeaderMap`, returns `Response`, and delegates the credential check to a
    new `try_login`. Key = `<client-ip>|<username-lowercased>`. After **5** failures in a
    **15 min** window the key is locked for **15 min** → `429 Too Many Requests` with a
    `Retry-After` header and `{"code":"too_many_requests"}` body. A successful login clears the
    key; the attempt that crosses the threshold is itself answered with the lockout.
  - Client IP comes from `X-Forwarded-For` (first hop) then `X-Real-IP`; falls back to `"local"`
    when neither is present (the daemon is served without connect-info — `axum::serve(listener,
    router)` in ottod/main.rs, which I don't own — so on direct loopback only the per-username
    half of the key throttles, which is still effective). Map is bounded (`MAX_TRACKED_KEYS =
    10_000`, expired entries pruned first).

- [x] **T4b — Revoke sessions on credential change / disable**
  - `crates/otto-rbac/src/tokens.rs`: added `AuthRepo::revoke_all_for_user(&self, user_id)` —
    `DELETE FROM auth_sessions WHERE user_id = ?` (kills both `kind='session'` login tokens and
    `kind='api'` PATs), returns rows deleted.
  - `crates/otto-server/src/routes/users.rs`: `update` revokes all sessions when the password
    changed **or** `disabled == Some(true)`; `remove` (soft-delete = disable) revokes all
    sessions too. So a changed/disabled credential invalidates every outstanding token.

- [x] **T4c — Password policy in users::create / users::update**
  - `crates/otto-rbac/src/passwords.rs`: new `pub const MIN_PASSWORD_LEN: usize = 10` (same value
    onboarding already enforced) + `pub fn validate_password(&str) -> Result<()>` (returns
    `Error::Invalid("password must be at least 10 characters")`). Re-exported from
    `crates/otto-rbac/src/lib.rs`.
  - `users::create` and `users::update` now call `otto_rbac::validate_password(...)` instead of
    the old "must not be empty" check, so they match onboarding's rule.
  - NOTE: `onboarding.rs` (not owned by me) still has its own local `MIN_PASSWORD_LEN = 10`
    constant — the **value matches**, so the policies are in sync. A follow-up could switch
    onboarding to `otto_rbac::validate_password` to make the shared helper the single call site.
  - Added a `password_policy` unit test in `passwords.rs`.

- [x] **T8 — `cargo test --workspace` doctest failure (otto-rbac / `ApiTokenInfo`)**
  - **No code change needed.** On the current working tree the issue is already resolved:
    `ApiTokenInfo` is `pub` at `otto-core/src/api.rs:74` and imported as a normal
    `use otto_core::api::ApiTokenInfo;` at `tokens.rs:8` (not inside a doctest). The audit
    captured a transient mid-edit state (the API-token feature was being added — those three
    files show as modified in git status). Verified `cargo test -p otto-rbac --doc`,
    `cargo test -p otto-rbac`, and `cargo test --workspace --doc` all pass (0 doctest failures).

## Command results

```
cargo check -p otto-core -p otto-rbac        # clean
cargo test -p otto-rbac                       # 2 passed (roundtrip, password_policy); 0 failed
cargo test -p otto-rbac --doc                 # ok. 0 passed; 0 failed (no doctests; compiles)
cargo test --workspace --doc                  # all crates: ok, 0 failed
cargo check -p otto-server                    # 8 errors — ALL in non-owned files (user_id field
                                              #   on NewNotice/Event::Notification added by another
                                              #   agent): monitor.rs, routes/activity.rs,
                                              #   skill_eval.rs, state.rs. ZERO in auth_routes.rs /
                                              #   users.rs (verified by grep on my symbols).
```

`otto-server` cannot fully compile until the concurrent per-user-notification work lands, but
none of those errors come from my files — the type-checker reaches and accepts `auth_routes.rs`
and `users.rs` with no diagnostics referencing my introduced symbols.
