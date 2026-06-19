# Agent 3 — WS edge hardening (T5 / T3b / T-CORS)

Hardening pass on the Otto repo against `docs/deep-dive-improvements-2026-06-19.md`
§2 (audit items S9, S3) and the CORS finding. Studied the existing
notification handlers, repo, event flow, and WS upgrade path before changing
anything; all changes match existing patterns.

## Summary

- [x] **T5 — Notifications per-user (audit S9)**
- [x] **T3b — WS bearer token out of `?token=` (audit S3)**
- [x] **T-CORS — Tighten CORS off `permissive()`**

`cargo check --workspace` passes clean (no warnings introduced).
`cd ui && npx svelte-check` → 0 errors / 0 warnings across 480 files.
Migration 0031 verified to apply over 0011 on a real SQLite DB, and the
scoping/dedupe SQL was functionally tested (see "Migration note" below).

---

## T5 — Notifications per-user (audit S9)

**Problem (audit):** notification handlers ignored `CurrentUser`; every
`Event::Notification` was broadcast to all WS clients (`ws_events.rs:93`); any
user could clear/alter global notification state.

**Design.** Notices gain an optional owner. `user_id IS NULL` = a global /
system notice (the credential monitor, session-event hooks, and skill-eval
producers all emit these — genuinely system-wide, so kept broadcast-to-all and
backward compatible). `user_id = <id>` scopes a notice to one user. A new
`NoticeAccess` enum expresses the caller's scope:
- `NoticeAccess::All` — root operator / daemon-internal: sees & mutates everything.
- `NoticeAccess::User(id)` — sees global + own; may **only** mark-read / dismiss /
  clear its **own** rows. Global/shared notices are read-only to a non-root user,
  so one user can't alter another's (or the system's) state — the exact leak S9 flags.

**Files & functions changed:**

- `crates/otto-state/migrations/0031_notifications_user.sql` (NEW) — `ALTER TABLE
  notifications ADD COLUMN user_id TEXT`; new `idx_notifications_user`; drop the
  global `idx_notifications_source_key` and replace with a per-`(user_id,
  source_key)` unique index.
- `crates/otto-state/src/notifications.rs`
  - `NewNotice` — added `user_id: Option<Id>`.
  - new `NoticeAccess { All, User(Id) }` enum (with policy doc).
  - `create` — dedupe lookup now scoped by `(source_key, user_id)` using NULL-safe
    `user_id IS ?`; INSERT writes `user_id`.
  - `list`, `unread_count`, `mark_read`, `mark_all_read`, `dismiss`, `clear` — all
    take `&NoticeAccess` and branch SQL: `All` = unrestricted; `User(id)` = read
    `WHERE user_id IS NULL OR user_id = id`, mutate `WHERE user_id = id`.
- `crates/otto-state/src/lib.rs` — re-export `NoticeAccess`; declared the new
  migration is picked up automatically by `sqlx::migrate!()` (embeds `migrations/`).
- `crates/otto-core/src/event.rs` — `Event::Notification` now carries
  `user_id: Option<Id>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`
  so global notices serialize byte-identically to before (wire-compatible).
- `crates/otto-server/src/routes/notifications.rs` — handlers now enforce the
  authenticated user: `access_for(&user)` (root → `All`, else `User(user.id)`)
  is passed into every list/mark-read/mark-all-read/dismiss/clear call. Settings
  stay a single daemon-wide row (correctly global; unchanged).
- `crates/otto-server/src/ws_events.rs` — `allowed()` splits `Notice` (free-form
  toast, still everyone) from `Notification`: a global notice (`user_id == None`)
  goes to everyone; an owned notice is delivered only to that user's connections
  (root sees all), mirroring the REST `NoticeAccess` policy.

**Additive cross-file edits (outside my owned set, caused by the schema change).**
Adding `user_id` to `NewNotice` and to `Event::Notification` broke 7 struct
literals in files owned by other agents. All current producers create global
notices, so each needed `user_id: None`; `state.rs` forwards the owner into the
event. These are purely additive (no logic touched), each annotated with an
inline comment. Applied to unblock the whole team's `cargo check`:
- `crates/otto-server/src/monitor.rs` (4 sites) — `user_id: None`
- `crates/otto-server/src/routes/activity.rs` (1 site) — `user_id: None`
- `crates/otto-server/src/skill_eval.rs` (1 site) — `user_id: None`
- `crates/otto-server/src/state.rs` (`NotificationService::create`) — captures the
  owner before consuming `new` and forwards it on `Event::Notification`.

**UI:** `ui/src/lib/stores/notifications.svelte.ts` and the notification UI
(`NotificationBell.svelte`, `settings/Notifications.svelte`) need no change —
the server now filters per-user, and the wire shape is unchanged for the
fields the UI reads. `ui/src/lib/api/types.ts` — the `notification` event type
gained `user_id?: string | null` for contract accuracy (lead-approved; additive,
no consumer depends on it).

## T3b — WS bearer token out of `?token=` (audit S3)

**Problem (audit):** the WS bearer token traveled in the `?token=` query string,
which lands in access logs everywhere.

**Change.** `/ws/events` now reads the token from the `Sec-WebSocket-Protocol`
header. The browser offers `["otto-bearer", "<token>"]`; the server pulls the
token from the second subprotocol value, validates it, and echoes back the
fixed `otto-bearer` marker so the handshake completes (the token itself is
never echoed). A legacy `?token=` query is still accepted as a graceful
fallback, so the change is self-consistent and backward compatible.

Otto tokens are 64-char hex (`[0-9a-f]`), which are valid `Sec-WebSocket-Protocol`
token values (no spaces/commas/control chars) — verified against
`otto-rbac/src/tokens.rs`.

**Files & functions changed:**
- `crates/otto-server/src/ws_events.rs` — `events_ws` reads `HeaderMap`, prefers
  the subprotocol token, echoes `otto-bearer` via `ws.protocols([..])` only when
  the subprotocol path was used; new `token_from_subprotocol()` helper.
- `ui/src/lib/api/client.ts` — new `wsConnect(path)` + `WS_BEARER_SUBPROTOCOL`
  export. `wsConnect` opens `new WebSocket(url, [WS_BEARER_SUBPROTOCOL, token])`
  (no `?token=`). The existing `wsUrl()` (still used by the terminal WS and other
  non-owned consumers) is left untouched so I don't break files I don't own.
- `ui/src/lib/events.svelte.ts` — single-line swap: `new WebSocket(wsUrl('/ws/events'))`
  → `wsConnect('/ws/events')` (the only `/ws/events` consumer). Flagged to the
  team lead since this file wasn't in my exclusive set.

**Note:** the other `?token=` WS endpoints (`/ws/term/*` in otto-sessions,
`/ws/api-client/stream`, LSP, the module proxy) are owned by other crates/agents
and were intentionally **not** changed.

## T-CORS — Tighten CORS

**Change.** Replaced `CorsLayer::permissive()` (echoed any origin) with a
restricted allowlist in `crates/otto-server/src/lib.rs` (`cors_layer()` +
`is_allowed_origin()` + `is_private_lan_host()`). Allowed origins:
- Tauri native shell (`tauri://localhost`, `http://tauri.localhost`),
- loopback at any port (`localhost`, `127.0.0.1`, `[::1]`, `*.localhost`) — covers
  the SPA served by the daemon and `vite` in dev,
- RFC-1918 private LAN IPv4 + Tailscale (`*.ts.net`) hosts — covers the
  remote/mobile access feature.

Methods pinned to GET/POST/PUT/PATCH/DELETE/OPTIONS; headers to
`Authorization` + `Content-Type`. `allow_credentials` stays off (auth is a
bearer header, not a cookie), which keeps a non-wildcard origin list valid and
avoids the credentialed-wildcard footgun. Arbitrary public web origins are now
rejected; the SPA, native app, and remote access all keep working.

---

## Migration note (read before deploying)

`0031_notifications_user.sql` **adds a column** (`user_id TEXT`, nullable) and
**rebuilds an index**:
- `ALTER TABLE notifications ADD COLUMN user_id TEXT` — nullable, no default; safe
  on existing rows (they become global notices, preserving today's behavior).
- Drops `idx_notifications_source_key` (global unique-on-source_key from 0011) and
  creates `idx_notifications_user_source_key` UNIQUE `(user_id, source_key) WHERE
  source_key IS NOT NULL`, plus a plain `idx_notifications_user`.

**Caveat (handled):** SQLite treats each NULL as distinct in a UNIQUE index, so
the new index does NOT enforce uniqueness for global (`user_id IS NULL`) notices.
The authoritative de-dupe therefore lives in `NotificationsRepo::create`, whose
lookup uses NULL-safe `user_id IS ?` — this is documented in both the migration
and the code. Verified on a scratch DB: global dedupe still collapses to one row,
per-user notices with the same `source_key` don't collide across owners, and a
non-root user cannot dismiss a global notice.

## Verification

- `cargo check --workspace` → clean, no warnings.
- `cargo check -p otto-server -p otto-state -p otto-core` → clean.
- `cd ui && npx svelte-check --tsconfig ./tsconfig.app.json` → 0 errors / 0 warnings.
- Migration 0031 applied over 0011 on a real SQLite DB; schema + indexes confirmed.
- Scoping/dedupe SQL functionally tested (global dedupe, per-user isolation,
  per-user visibility, non-root can't delete global).
