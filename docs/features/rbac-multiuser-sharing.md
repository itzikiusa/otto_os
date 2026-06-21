# Multi-User, RBAC, Session Isolation, Impersonation & Sharing

Otto runs as a single macOS daemon (`ottod`) that one **owner** installs, but it
is built to be **shared by a team**. Every account gets explicit, per-feature
permissions; each user's sessions and data are private to them; an **admin** can
oversee the whole daemon and **impersonate** a user to see exactly what they see;
and any session can be handed to an outside **guest** through a scoped, expiring,
revocable link gated by an emailed one-time code. This document is the end-user
and operator reference for that authorization surface.

> **Seed & deeper material.** This expands the operator-oriented
> [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md). The mobile/PWA/Cloudflare-Tunnel
> half of session sharing lives in [`./remote-mobile-access.md`](./remote-mobile-access.md);
> driving any of these endpoints from a script lives in
> [`./daemon-http-api.md`](./daemon-http-api.md). The authoritative contract is
> [`../contracts/api.md`](../contracts/api.md); the WebSocket auth/eviction
> behaviour is in [`../contracts/ws.md`](../contracts/ws.md).

---

## 1. Summary

Authorization in Otto has **three independent axes**, and **root bypasses all of
them**:

1. **Feature capability** — per user, per feature: `None < View < Edit < Admin`.
   Stored in `user_feature_grants`. **Default-deny** (no grant row = `None` = no
   access). Enforced by one central middleware (`feature_guard`) that maps every
   route → `(feature, capability)`.
2. **Workspace role** — `Viewer < Editor < Admin` per workspace (the pre-existing
   axis). Answers *"which workspace's data."*
3. **Ownership** — sessions, DB query history, saved queries/dashboards, and
   (optionally) connections are private to their creator. Answers *"whose data."*

Effective access = the request passes the **feature gate** **and** the
**workspace gate** **and** (for per-user resources) the **ownership gate**. Root
passes everything.

Two more facilities sit on top: an **admin overview** (a sanctioned cross-user
view of every session + forced terminate/remove) and **audited impersonation**
(an effective-user overlay, never a re-login, where every decision runs as the
target but every audit entry records the admin). **Personal Access Tokens** carry
a user's roles into scripts; **share links + email-OTP** carry one session out to
a guest with no Otto account.

---

## 2. Overview — where everything lives

| Concern | UI location | Backend | Contract |
|---|---|---|---|
| Create/disable users, workspace-role matrix, **feature-grant matrix**, impersonate | **Settings → Users** (`ui/src/modules/settings/Users.svelte`) | `otto-rbac`, `otto-state::GrantsRepo` | api.md §"User Feature Grants" + rows #7–#10 |
| Admin active-sessions overview + terminate/remove | **Settings → Sessions** (`ui/src/modules/settings/AdminSessions.svelte`) | `otto-sessions::SessionManager` | api.md §"Admin active-sessions overview + terminate" |
| Impersonation banner ("Acting as …", countdown, Stop) | App shell (`ui/src/shell/App.svelte`), driven by `auth.svelte.ts` | impersonation token (`kind='impersonation'`) | api.md §"Admin impersonation (act-as …)" |
| Personal Access Tokens | **Settings → API Tokens** (`ui/src/modules/settings/PersonalAccessTokens.svelte`) | `otto-rbac::tokens` (`kind='api'`) | api.md §"API Tokens" #87–#89 |
| Share a session (mint scoped link) | per-session **Share** modal (`ui/src/modules/agents/ShareModal.svelte`) | share token (`kind='share'`) | api.md §"Share-link tokens" |
| Email sender for OTP shares + public share domain | **Settings → Sharing** (`ui/src/modules/settings/EmailSenderSetup.svelte`) | `email_senders` + Keychain | api.md §"Email sender" / §"Email-OTP gate" |
| Audit log / security posture | **Settings → Trust & Safety** (root) | `audit_log` | api.md §"Trust & Safety" |

> **Role/capability model in code:** `Capability` and `Feature` enums live in
> `crates/otto-core/src/domain.rs` (Capability lines 1436–1466, Feature lines
> 1469–1541); the central guard is `crates/otto-server/src/feature_guard.rs`; the
> route → policy table is `crates/otto-server/src/policy.rs`. The UI mirror is
> `ui/src/lib/api/types.ts` (`Feature`, `Capability`) + `auth.svelte.ts` (`can()`).

---

## 3. The role model & per-feature grants

### 3.1 The capability ladder

`Capability` is an **ordered** enum — comparisons are by strength:

| Capability | String | Meaning (general) |
|---|---|---|
| `None` | `"none"` | No access. **Never stored** — the *absence* of a grant row reads back as `none`. |
| `View` | `"view"` | Read-only: GET surfaces (list, inspect, read history). |
| `Edit` | `"edit"` | Read + mutate/run (POST/PUT/PATCH/DELETE on the feature's data). |
| `Admin` | `"admin"` | Manage: privileged config / lifecycle for that feature. |

The check is "**at least**": a route requiring `View` is satisfied by `View`,
`Edit`, or `Admin`. The UI uses the identical ladder (`CAP_ORDER` in
`auth.svelte.ts`); `auth.can(feature, required)` returns true when
`index(granted) >= index(required)`, with **root always true**.

### 3.2 The gated features (18)

Every feature is an independently-gatable axis. These are the exact enum variants
and their snake_case strings (used as the `feature` key in grants & in
`/auth/capabilities`):

| Feature (enum) | String | UI label | Typical capability meaning |
|---|---|---|---|
| `Agents` | `agents` | Agents | View: see/attach sessions you own · Edit: create/drive · (cross-user is via Users:Admin) |
| `Connections` | `connections` | Connections | View: list · Edit: open/create/edit · Admin: manage all / global |
| `Database` | `database` | Database | View: `SELECT`/`SHOW`/`EXPLAIN`, schema, read history/saved · Edit: mutating SQL, manage saved queries/dashboards · Admin: manage connections |
| `Git` | `git` | Git | View: repos/diffs · Edit: commit/PR ops |
| `Issues` | `issues` | Issues | View / Edit Jira & Confluence |
| `Product` | `product` | Product | View / Edit product workflows |
| `Swarm` | `swarm` | Swarm | View / Edit agent swarms |
| `ApiClient` | `api_client` | API Client | View / Edit the HTTP API client |
| `Workflows` | `workflows` | Workflows | View / Edit / run workflows |
| `Channels` | `channels` | Channels | View / Edit Slack/Telegram bridges |
| `SkillEval` | `skill_eval` | Skills Evaluator | View · Edit start/cancel · (promote = root) |
| `Skills` | `skills` | Skills | View library · Admin manage |
| `Insights` | `insights` | Insights | management-only surface |
| `Usage` | `usage` | Usage | management-only surface |
| `SelfImprovement` | `self_improvement` | Self-Improvement | View · run = Edit · config = Admin |
| `Context` | `context` | Context | View · materialize = Edit · config = Admin |
| `Settings` | `settings` | Settings | `Settings:Admin` ≈ the old root-only config gate (now grantable) |
| `Users` | `users` | Users | **Admin only**: user CRUD, grant management, the active-sessions overview, terminate, and impersonation |

> The grant matrix in **Settings → Users** renders these 18 features against the
> four capabilities (`none / view / edit / admin`) as a segmented control per row
> (`ALL_FEATURES` / `FEATURE_LABELS` / `CAP_OPTIONS` in `Users.svelte`).

**Custom plugins** add a parallel, *string-keyed* RBAC axis: a plugin's **slug**
behaves like a feature key on `/users/{id}/plugin-grants` and is returned by
`/auth/capabilities` so the UI can gate the plugin's nav entry. See the Custom
Plugins section of `../contracts/api.md`.

### 3.3 How the gate maps routes → `(feature, capability)`

The `feature_guard` middleware (layered right after auth) reads the matched route
template + HTTP method and consults the **policy table** (`policy.rs`). It yields
one of three decisions:

- **`Exempt`** — pass through (self-owned, public, or workspace-axis routes; see
  §8 and §9).
- **`Require(feature, capability)`** — allow **iff** the caller is root **or**
  `capability_of(user, feature) >= capability`; otherwise **403**
  (`"requires <feature>:<capability>"`).
- **`Deny`** — fail closed (403) for anything unmapped.

The default method→capability convention is: **GET ⇒ View**, **mutating
(POST/PUT/PATCH/DELETE) ⇒ Edit**, **privileged config/lifecycle ⇒ Admin**.
Deliberate exceptions are encoded per-route (e.g. self-improvement/context
*config* `PUT` requires `Admin` while *run/materialize* `POST` requires `Edit`;
skill-eval *promote* requires root; library *writes* require `Admin`).

Root bypass is explicit in the guard (`if user.is_root { return next … }`), so a
root account always sees the full nav and every API returns 200-class.

---

## 4. Managing users

### 4.1 The root account & local accounts

- Otto stores **local accounts** (username + bcrypt-hashed password via
  `otto-rbac::passwords`; `MIN_PASSWORD_LEN` enforced — the UI rejects passwords
  shorter than 6 chars at create time).
- The **first** account, created during onboarding (`POST /onboarding/root`,
  allowed only while zero users exist), is **`root`**. Root **bypasses all three
  axes**. There is **no promote-to-root** endpoint — delegate elevated power via a
  `Users:Admin` grant instead.
- `DELETE /users/{id}` is a **soft** delete (sets `disabled`); the root user
  **cannot** be disabled (→ 400).

### 4.2 Creating a user (Settings → Users)

1. As **root** (or a `Users:Admin`), open **Settings → Users** and click **New
   User**.
2. Provide **username**, optional **display name**, and a **password** (≥ 6 chars)
   → `POST /users` (409 on duplicate username).
3. New users start with **no grants** (default-deny). Assign capabilities in the
   **Feature grants** matrix (select the user, set per-feature capability, **Save
   grants** → `PUT /users/{id}/grants`, which **atomically replaces** all grants
   and is audited).
4. Add them to the relevant **workspace** with a **role** in the **Workspace
   roles** matrix (`PUT /workspaces/{id}/members`) so they can reach that
   workspace's connections/repos.

**Example — a "Database-only" user:** set `Database → View` (or `Edit`), leave
everything else `None`, and give them `Editor` in the workspace whose connections
they need. Result: they log in and see **only Database** in the nav; every other
feature's API returns **403**; they see only **their own** query history and saved
queries.

**Disable / enable** toggles `disabled` via `PATCH /users/{id}`; a disabled user
cannot log in (login returns 401) and their tokens stop authenticating.

> **Contract divergence to note.** The frozen core rows #7–#10 (`GET/POST/PATCH/
> DELETE /users`) are documented as **root**-gated, while the **grant** endpoints
> (`GET/PUT /users/{id}/grants`), the active-sessions overview, and impersonation
> are **`Users:Admin` *or* root**. The UI surfaces the whole **Settings → Users**
> page (and the **Sessions** tab) behind `auth.can('users','admin')`, so a
> non-root `Users:Admin` sees the page — but creating/patching the *user rows*
> themselves still flows through the root-gated `/users` CRUD. In practice,
> delegate day-to-day user administration to `Users:Admin` for grants/sessions/
> impersonation; keep account creation/deletion with root.

---

## 5. Per-session isolation & data ownership

Non-admin users are confined to **their own** resources. The owning user is
recorded in `Session::created_by` (and the DB-history `user_id` column added in
migration `0042`).

**What a non-admin user can and cannot see:**

- **Sessions** — a user sees, attaches to, and controls only sessions they
  created (`created_by == user.id`). Workspace-admins and root see all. The
  ownership predicate is `session_owner_or_admin` (`otto-core/src/auth.rs`):
  `is_root || created_by == user.id || ws-Admin`.
- **Live terminal** (`/ws/term/{id}`) — only the owner / workspace-admin / root
  (or a valid scoped guest, §9) may attach; others are refused **before** the WS
  upgrade (403). See `ws.md`.
- **Event stream** (`/ws/events`) — session status / trail / task events reach
  only the owner / ws-admin / root.
- **Activity** (trail / tasks / summary) — owner-scoped; admins/root see the full
  roll-up.
- **DB query history & saved queries / dashboards** — private to the user who
  ran/saved them (legacy rows with `user_id = NULL` predate multi-user and don't
  appear in per-user filtered views); **root** sees all.
- **`/app/kill-sessions`** — **root only**.

### 5.1 Shared vs private connections

By default connections are **shared**: an admin provisions a company DB
connection; any user with `Connections:Edit` opens it; each user's *queries* still
stay private. To make **connections themselves** private to their creator, set the
daemon setting:

```
connections.owner_private = true     # default: false
```

When ON, non-root users **list and open only their own** connections
(`require_conn_owner_or_root` guards open/test/pin/patch/delete; the read is
`owner_private_enabled` in `otto-connections/src/http.rs`). Root is always exempt.

---

## 6. Admin: active-sessions overview & terminate

**Settings → Sessions** (gated `Users:Admin` *or* root) is the **sanctioned
cross-user view** — a daemon-wide list of **every** session across all workspaces
and users, deliberately bypassing the per-session owner gate. Each row
(`AdminSessionRow`) shows: **owner** (`owner_username`, resolved from
`created_by`), **kind/provider**, **title**, **status**, **live** dot, and
**viewers** (attached `/ws/term` count). Rows are persisted sessions enriched with
in-memory state (`live = is_live(id)`, `viewers = attached_count(id)`).

Two destructive actions (single or bulk, with confirm), plus a "Remove all exited"
sweep:

| Action | Endpoint | Effect |
|---|---|---|
| **Terminate** | `POST /admin/sessions/{id}/terminate` | Kills the PTY → `exited`, **keeps** the row + history (non-destructive), then **evicts** attached `/ws/term` viewers — each gets a `{"type":"terminated"}` frame and the socket closes (including a shared mobile guest). Audited `session.terminated`. |
| **Remove** | `POST /admin/sessions/{id}/remove` | Kills the PTY **and deletes** the session row + history, emits `SessionRemoved`. Audited `session.removed`. |

"Remove all exited" prunes background/ephemeral sessions (insights, analysis, …)
that accumulate without bound. A session **owner** can still self-terminate via the
owner-gated `DELETE /sessions/{id}`.

---

## 7. Audited impersonation ("act as")

Impersonation is an **effective-user overlay**, *not* a re-login. From **Settings
→ Users**, an admin clicks **Impersonate** on a non-root, non-admin user.

### 7.1 How it works

- `POST /admin/impersonate/{user_id}` mints a **short-lived** impersonation token
  (`kind='impersonation'`, **30-minute fixed TTL, never slid**). Its row records
  `user_id` = the **admin** (real) and `acting_as_user_id` = the **target**
  (effective).
- `authenticate()` resolves it to `AuthContext { real_user: admin, effective_user:
  target, scope: None }`. **Every authorization decision runs against the target**;
  **every audit entry records the admin.** `is_root` is enforced from the effective
  user (so the admin's root powers do not leak into the overlay).
- The UI (`auth.svelte.ts → impersonate()`) **saves the admin token** to
  `localStorage['otto_admin_token']`, swaps the active bearer to the impersonation
  token, and re-loads identity + capabilities as the target. A sticky banner
  renders **"Acting as `<username>` (you are `<admin>`)"** with a live
  **30-minute countdown** and a **Stop impersonating** button (`App.svelte`).

### 7.2 Ending it

**Stop impersonating** calls `POST /admin/impersonate/stop` (self-scoped /
`Exempt` — the effective user mid-overlay is a plain user, so it can't be
`Users:Admin`-gated or "Exit" would be impossible). It **revokes the presented
token** (which then returns 401), restores the saved admin token, clears the
persisted key, and re-boots as the admin. The token also simply **times out** at
30 minutes if you forget.

### 7.3 Anti-escalation guardrails (403 on violation)

1. **No up/sideways** — the target may not be root, nor hold `Users:Admin`.
2. **No nesting** — an impersonation token may not start another impersonation.
3. **No self** — the target may not be the caller (404 if absent; 403 if
   disabled).
4. **Impersonation cannot mint credentials** — `POST /auth/tokens` is **403** when
   the request is impersonated (`real_user != effective_user`); the same guard
   covers share-link minting. An admin acting-as a user **cannot forge a
   long-lived credential** as that user.

### 7.4 What is recorded

| Event | `user_id` | `target` | `detail` |
|---|---|---|---|
| `impersonate.start` | admin (real) | target (effective) | `{real_user_id, effective_user_id, effective_username}` |
| `impersonate.stop` | real | effective | `{real_user_id, effective_user_id}` |

---

## 8. API tokens & RBAC over the API

**Personal Access Tokens (PATs)** let scripts/CLIs/CI drive the daemon over HTTP
with the **same authorization** as the issuing user.

- Manage at **Settings → API Tokens** (`PersonalAccessTokens.svelte`). The raw
  secret is shown **exactly once** at creation (only its SHA-256 hash is stored);
  copy it then. Pass it as `Authorization: Bearer <token>` on any route, or
  `?token=<token>` on the WS endpoints; the conventional env var is
  `OTTO_API_TOKEN`.
- A PAT (`kind='api'`) has a **~10-year fixed lifetime** whose expiry is **never
  slid** (unlike the 30-day sliding login token).
- **Scope = the owner's roles.** A token minted by **root** has root; otherwise it
  carries that user's **workspace roles** and is subject to the **same feature
  guard and ownership gates** as the user in the UI. There is no way for a PAT to
  exceed its owner.
- **DELETE** revokes only the **caller's own** tokens (scoped by `user_id` +
  `kind='api'`); `last_seen_at` updates on use (throttled to ≤ once/hour).
- **Impersonation sessions cannot mint PATs** (see §7.3).

| # | Method & path | Auth | Notes |
|---|---|---|---|
| 87 | `POST /api/v1/auth/tokens` | member (Exempt; **not** while impersonating) | `{label?}` → `{token, info}` (secret once) |
| 88 | `GET /api/v1/auth/tokens` | member (Exempt) | `ApiTokenInfo[]`, newest first, never the secret |
| 89 | `DELETE /api/v1/auth/tokens/{id}` | member (self-owned) | 204 (404 if not found / not owned) |

`ApiTokenInfo` = `{id, label?, token_prefix (first 12 chars), created_at,
last_seen_at, expires_at}`. The token routes are **`Exempt`** in the policy table
(self-owned, like `/auth/me`, `/auth/logout`, `/auth/capabilities`), so any authed
member manages their **own** tokens — but the token then carries that member's
RBAC into every other route.

> Bootstrap pattern: log in once, mint a PAT, store it in `OTTO_API_TOKEN`, then
> script against `http://127.0.0.1:7700/api/v1/...`. See
> [`./daemon-http-api.md`](./daemon-http-api.md).

---

## 9. Session sharing (links + email-OTP) — overview

Sharing hands **one session** to an outside **guest** who has no Otto account, via
a **scoped, expiring, revocable capability token** bound to that session. It is the
guest-access primitive for the mobile remote-access feature — the **full
mobile/PWA/Cloudflare-Tunnel setup is in
[`./remote-mobile-access.md`](./remote-mobile-access.md)**; this section covers the
authorization model.

### 9.1 The scoped share token

- Minted via the per-session **Share** modal → `POST /api/v1/sessions/{id}/share`
  (`kind='share'`). The raw token is shown **once** (only its SHA-256 hash is
  stored); the response `url` is the ready-to-share fragment
  `<origin>/#/s/<session_id>/<token>`.
- **`role`** is `"viewer"` (read-only) or `"editor"` (read + input) — **never
  `"admin"`** (rejected). TTL for a plain share is **fixed**, clamped to
  `[60, 86400]` seconds (`expires_at = created_at + ttl_secs`, never slid).
- **Deny-by-default scope:** the token's `AuthContext.scope` pins it to that single
  session — it cannot enumerate other sessions, reach `/ws/events`, or touch any
  non-session route; its role caps what it may do.
- **Mint/list guards:** the caller must own the session or be a workspace Admin,
  must **not** be impersonated, and must **not** themselves hold a share token (a
  guest cannot mint sub-shares).
- **Revocation evicts:** `DELETE /api/v1/auth/shares/{share_id}` (or
  `POST /api/v1/auth/shares/revoke-all`) revokes **and** evicts — any still-attached
  viewer immediately gets `{"type":"terminated"}` and the WS closes.

### 9.2 The email-OTP gate (recommended for anything outward-facing)

So a **leaked/forwarded link alone is useless**, the owner can require an emailed
one-time code:

1. **One-time setup (Settings → Sharing):** configure a Gmail **App Password**
   sender (`PUT /api/v1/email-sender`). The app password is verified by a real
   `smtp.gmail.com:587` STARTTLS login and stored in the macOS **Keychain**
   (`email-sender-{user_id}`) — **never** in the DB (which holds only the opaque
   `secret_ref`). A bad password fails closed (502, stays unverified). The same
   page sets `share_base_url` (the public domain used to build share links/emails).
2. **Mint an OTP share:** call `POST /sessions/{id}/share` **with**
   `recipient_email` (LOCKED for the share's life) + `duration_secs` (clamped to
   ≤ 43200s = **12h**). Requires a **verified** sender (else 400). Otto generates a
   **6-digit OTP** (`OsRng`), stores only its `sha256` (`otp_hash`, ~10-min
   expiry), and **emails the code** to the recipient. Omitting `recipient_email`
   mints a plain (no-OTP) share.
3. **Redeem (guest):** while OTP-pending, the scope reaches **nothing** except
   `/share/verify` (every protected route 403s; `/ws/term` refuses the upgrade).
   The guest submits `POST /api/v1/share/verify {token, otp}` (public — the token
   *is* the auth; **IP rate-limited**). On success Otto sets `verified_at` and
   **clears `otp_hash`** (single-use); a wrong/expired/reused code returns 401 and
   records a throttle failure. The guest may then attach until `max_expires_at`
   (≤12h); once the window elapses the share **re-pends**.
4. **Extend (guest/owner):** `POST /api/v1/share/extend {token}` re-issues a fresh
   OTP and re-emails it to the **LOCKED original `recipient_email` only** (the
   request carries **no email field by design** — destination is read from the
   share row, never the request). It clears `verified_at`, opens a fresh ≤12h
   window, and requires the owner still have a verified sender (else 400). The
   guest re-verifies the new code.

See [`./remote-mobile-access.md`](./remote-mobile-access.md) for the device-side
PWA, touch terminal, responsive shell, and Cloudflare-Tunnel exposure runbook.

---

## 10. API / contract reference

All routes are under `/api/v1` with bearer auth (`Authorization: Bearer <token>`
or `?token=` on WS). Role column meaning: `member` = any authed user; `Users:Admin
or root`; `root`. Authoritative source: [`../contracts/api.md`](../contracts/api.md).

### Accounts & identity
| Method & path | Auth | Notes |
|---|---|---|
| `POST /onboarding/root` | public (only while 0 users) | first/root account → `LoginResp` |
| `POST /auth/login` · `POST /auth/logout` | public / member | 401 on bad creds / disabled |
| `GET /auth/me` | member | `MeResp {user (effective), real_user, impersonating}` |
| `GET /auth/capabilities` | member (Exempt) | `{capabilities: {feature-or-slug → capability}}`; root ⇒ `admin` for all |
| `GET /users` · `POST /users` · `PATCH /users/{id}` · `DELETE /users/{id}` | root | CRUD; delete is soft-disable; root can't be disabled |

### Grants & workspace membership
| Method & path | Auth | Notes |
|---|---|---|
| `GET /users/{id}/grants` · `PUT /users/{id}/grants` | Users:Admin or root | PUT **atomically replaces** all grants; audited `grant.changed` |
| `GET /users/{id}/plugin-grants` · `PUT /users/{id}/plugin-grants` | root | string-keyed plugin axis (`feature` = slug) |
| `GET /workspaces/{id}/members` · `PUT /workspaces/{id}/members` | ws admin | the workspace-role matrix |

### Admin overview, terminate, impersonation
| Method & path | Auth | Notes |
|---|---|---|
| `GET /admin/sessions` | Users:Admin or root | `AdminSessionsResp {sessions: AdminSessionRow[]}` |
| `POST /admin/sessions/{id}/terminate` | Users:Admin or root | kill PTY + evict viewers; 204; audited |
| `POST /admin/sessions/{id}/remove` | Users:Admin or root | kill + delete row/history; 204; audited |
| `POST /admin/impersonate/{user_id}` | Users:Admin or root | `{token}` (once); 30-min fixed TTL; guardrails; audited `impersonate.start` |
| `POST /admin/impersonate/stop` | impersonating session (Exempt/self) | revokes presented token; 204; audited `impersonate.stop` |

### Tokens & sharing
| Method & path | Auth | Notes |
|---|---|---|
| `POST /auth/tokens` · `GET /auth/tokens` · `DELETE /auth/tokens/{id}` | member (self; mint blocked while impersonating) | PATs, secret shown once |
| `POST /sessions/{id}/share` · `GET /sessions/{id}/shares` | session owner / ws admin | mint/list scoped shares (role viewer/editor) |
| `DELETE /auth/shares/{share_id}` · `POST /auth/shares/revoke-all` | member (self-owned) | revoke + evict; idempotent |
| `PUT /email-sender` · `GET /email-sender` | member (self-owned) | Gmail App-Password sender; pw in Keychain |
| `POST /share/verify` · `POST /share/extend` | **public** (the share token is the auth) | OTP redeem / re-send; IP rate-limited |

### Audit & posture (root)
| Method & path | Auth | Notes |
|---|---|---|
| `GET /audit-log` | root | filter `from/to/action/user_id/limit/offset`; newest first |
| `GET /security-posture` | root | `{network_listener, network_listener_port?, loopback_only, active_api_tokens}` |

---

## 11. Capabilities & limitations

**Capabilities**
- Per-user, per-feature authorization across **18 features** × 4 capabilities,
  enforced centrally by one middleware (no per-handler drift).
- Default-deny; root bypass; `Users:Admin` is a **grantable** admin (delegate
  without sharing root).
- Per-session, per-history, per-saved-query ownership isolation; optional
  per-connection privacy (`connections.owner_private`).
- Sanctioned admin overview + non-destructive **terminate** and destructive
  **remove**, with viewer eviction.
- Audited, time-boxed, non-nesting impersonation that **cannot escalate or mint
  credentials**.
- PATs that inherit (never exceed) their owner's roles; scoped + OTP-gated guest
  shares.

**Limitations / by-design constraints**
- **No promote-to-root** endpoint; the root account cannot be disabled.
- Impersonation is capped at **30 minutes** (re-impersonate to continue) and
  **cannot target root or another `Users:Admin`**, cannot nest, and cannot mint
  PATs/shares.
- Share **role** can never be `admin`; share/impersonation/PAT tokens with a guest
  scope cannot create sub-shares.
- Legacy DB-history rows (pre-`0042`, `user_id = NULL`) are excluded from per-user
  filtered views (visible to root/admin only).
- Account **CRUD** (`/users`) remains **root-gated** even though grants/sessions/
  impersonation are `Users:Admin`-grantable (see the divergence note in §4.2).
- The email-OTP gate requires a **verified Gmail App-Password sender** per owner;
  without one, only **plain** (no-OTP) shares can be minted.

---

## 12. Security

- **Audit log (root-only `/audit-log`).** Sensitive actions are recorded:
  `grant.changed`, `user.created` / `user.disabled`, `impersonate.start` /
  `impersonate.stop`, `session.terminated` / `session.removed`. Impersonation
  entries carry **both** the real admin and the effective user, so an admin can
  never act invisibly as someone else.
- **Least privilege.** Default-deny grants; "at least" capability checks; the gate
  fails **closed** (`Deny`) on unmapped routes; root bypass is explicit and the
  only blanket bypass.
- **Ownership boundaries.** Sessions, live terminals, event streams, activity, and
  DB history are owner-scoped before any data flows; `/ws/term` and `/ws/events`
  are refused **before** the upgrade for non-owners; revoke/terminate **evicts**
  attached sockets immediately.
- **Secrets in the Keychain, never the repo/DB.** Passwords are bcrypt-hashed;
  token secrets are stored only as SHA-256 hashes and shown once; the Gmail App
  Password lives in the macOS **Keychain** (`otto-keychain`, `email-sender-{user_id}`),
  with the DB holding only an opaque `secret_ref`.
- **Loopback by default.** `ottod` listens on `127.0.0.1:7700`; any outward
  exposure (Cloudflare Tunnel) is TLS-only and deliberate — see
  [`./remote-mobile-access.md`](./remote-mobile-access.md). `GET /security-posture`
  reports the listener/loopback state and active token count.
- **Brute-force / abuse throttling.** Login is rate-limited; `/share/verify` and
  `/share/extend` are **IP rate-limited** (429 with `Retry-After`); OTPs are
  6-digit, hashed, ~10-min, **single-use**.

---

## 13. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| User sees an empty nav / 403 on every feature | No grants (default-deny) | Assign capabilities in **Settings → Users → Feature grants**, **Save grants** |
| Feature works in UI but `403 requires <feature>:<cap>` from a script | PAT inherits the user's (insufficient) capability | Grant the needed capability to the **token owner**, or use a root-owned token |
| User can't reach a workspace's connections despite a `Connections` grant | Not a member of that workspace | Add them in **Workspace roles** (`Editor`/`Admin`) |
| User can't see another user's session | Working as designed — ownership isolation | Use **Settings → Sessions** (Users:Admin/root) for the cross-user view, or impersonate |
| **Impersonate** button disabled / 403 | Target is root or a `Users:Admin`, is yourself, is disabled, or you're already impersonating | Pick a non-root, non-admin, enabled target; exit any active impersonation first |
| Impersonation banner gone / 401 mid-session | The 30-min token expired (never slid) | **Stop**, then re-impersonate |
| `POST /auth/tokens` returns 403 | You're impersonating (real ≠ effective) — mint blocked | Stop impersonating, then mint as yourself |
| Minting an OTP share → 400 "set up a verified email sender first" | No verified Gmail sender | Configure + verify in **Settings → Sharing**; or omit `recipient_email` for a plain share |
| Sharing → 400 on `PUT /email-sender` / 502 | Bad/expired Gmail **App Password** or 2-Step not enabled | Regenerate the 16-char App Password (Google Account → Security → App passwords) and re-save |
| Guest stuck on the OTP screen | Code expired (~10 min), already used, or wrong | `POST /share/extend` to re-send to the **locked** recipient; guest re-verifies |
| Guest's live terminal suddenly closed (`terminated`) | Owner/admin terminated the session **or** the share was revoked | Expected; re-share if intended |
| Can't disable / delete the root user (400) | Root cannot be disabled by design | Delegate via `Users:Admin`; there is no way to remove root |
| Shared connection shows another user's queries | `connections.owner_private` off (shared mode) | Queries are already private per user; to hide the *connection* set `connections.owner_private = true` |

---

## 14. Related docs

- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — the operator/deployment seed
  this document expands.
- [`./remote-mobile-access.md`](./remote-mobile-access.md) — guest/owner mobile
  access: PWA, responsive shell, touch terminal, Cloudflare-Tunnel exposure, and
  the device-side half of share links + email-OTP.
- [`./daemon-http-api.md`](./daemon-http-api.md) — driving `ottod` over HTTP/WS
  with a Personal Access Token.
- [`../contracts/api.md`](../contracts/api.md) — **authoritative** REST contract
  (User Feature Grants, Admin sessions overview + terminate, Admin impersonation,
  API Tokens, Share-link tokens, Email sender, Email-OTP gate).
- [`../contracts/ws.md`](../contracts/ws.md) — WebSocket auth, attach gating, and
  the `{"type":"terminated"}` eviction frame.
