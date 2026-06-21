# Remote / mobile access — Cloudflare tunnel, PWA, scoped share links & email-OTP

> **Summary.** Otto is a macOS desktop app whose daemon (`ottod`) binds
> **`127.0.0.1` only** by default — nothing is reachable off the machine until
> *you* opt in. Two opt-ins exist. **(1) Owner remote control:** expose the
> loopback daemon through a **Cloudflare tunnel** (outbound-only, edge TLS, no
> open router ports) and install the served app as a **PWA** ("Add to Home
> Screen") to drive the full UI from a phone or iPad — every session still runs
> on the Mac; the device is a thin client. **(2) Guest share links:** mint a
> **scoped, expiring, revocable** capability URL (`https://<host>/#/s/<session>/<token>`)
> that reaches exactly **one** session as **viewer** (read-only) or **editor** —
> never root, never another session. Optionally gate a share behind an
> **email-OTP**: Otto mails a 6-digit code (via your Gmail App Password sender) to
> a **locked recipient**, and the token reaches *nothing* until the guest redeems
> it — so a leaked link alone is useless. As an alternative to the tunnel the
> daemon can also expose a **built-in `0.0.0.0` TLS listener** for LAN / self-hosted
> reach. This page is the end-user + operator guide; the original operator
> notes live in [`../remote-access-runbook.md`](../remote-access-runbook.md).

---

## 1. Overview

Remote access is **off by default and entirely opt-in**. The architecture never
changes — the phone/tablet is a browser client; agents, PTYs, git, and DBs all
run on the Mac:

```
 phone / iPad (PWA in Safari/Chrome)
        │  HTTPS + WSS  (edge TLS at Cloudflare, or the built-in 0.0.0.0 TLS listener)
        ▼
   Cloudflare edge ──outbound tunnel──▶  ottod on 127.0.0.1:7700  (loopback)
                                              │
                                              ├─ Owner login token  → full UI
                                              └─ Scoped share token  → ONE session (viewer/editor)
```

There are two independent capabilities:

| Capability | Who | Reaches | Auth | Surface |
|---|---|---|---|---|
| **Owner remote control** | you (the workspace owner / a real user) | the full UI, all your sessions/features (subject to RBAC) | your normal login bearer token | the whole SPA, served same-origin from `ottod` |
| **Guest share link** | anyone you send a link to | **one** session, capped to `viewer` or `editor` | a scoped share token in the URL fragment (+ an email OTP when gated) | the full-screen `SharePage` guest view only |

Per-user RBAC and per-session data isolation apply to *both* (see
[`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md)). A share token can
never escalate past `editor`, never reach a second session, and never carry
root.

### Where it lives

| Concern | Location |
|---|---|
| Loopback + optional `0.0.0.0` TLS listener | `crates/ottod/src/main.rs` (loopback bind ~L655; `network_listener` block ~L660-714; `load_or_make_tls_config` ~L806) |
| SPA served same-origin (rust-embed) | `crates/otto-server/src/spa.rs` (`#[folder = "../../ui/dist"]`, `embed-ui` feature) |
| CORS allowlist (loopback / LAN / `*.ts.net`) | `crates/otto-server/src/lib.rs::is_allowed_origin` |
| Share token mint / list / revoke | `crates/otto-server/src/routes/share.rs` |
| OTP verify / extend (public) | `crates/otto-server/src/routes/share.rs::{verify_share,extend_share}` |
| Token issue / revoke / OTP gen / evict + constants | `crates/otto-rbac/src/tokens.rs` (`generate_otp`, `SHARE_*` consts, `AuthRepo`) |
| Scoped-token / OTP-pending feature guard | `crates/otto-server/src/feature_guard.rs` (`"share requires email-OTP verification"`) |
| Public-route exemptions (`/share/verify`, `/share/extend`) | `crates/otto-server/src/policy.rs` |
| Email sender routes (Gmail App Password) | `crates/otto-server/src/routes/email_sender.rs` (`email-sender-{user_id}`) |
| Gmail SMTP transport + verify probe | `crates/otto-channels/src/email.rs` (`GmailSender`, `smtp.gmail.com:587`) |
| IP rate-limiter (the "share throttle") | `otto_sessions::share_throttle::global()` |
| Guest view (full-screen share UI) | `ui/src/modules/share/SharePage.svelte` |
| Create-share modal (role/expiry/recipient + QR) | `ui/src/modules/agents/ShareModal.svelte` |
| Email-sender settings page | `ui/src/modules/settings/EmailSenderSetup.svelte` (Settings → Sharing) |
| Share API client (guest, scoped token) | `ui/src/lib/api/share.ts` |
| Share-token capture from URL fragment | `ui/src/lib/router.svelte.ts` (`#/s/<id>/<token>` → in-memory, stripped from history) |
| PWA manifest + service worker + SW register | `ui/public/manifest.webmanifest`, `ui/public/sw.js`, `ui/src/main.ts` |
| DB schema | migrations `0044_share_tokens`, `0045_email_senders`, `0046_share_otp`, `0053_share_listing_index` |
| Contract | `docs/contracts/api.md` — *Share-link tokens* / *Email sender* / *Email-OTP gate for share links* |

---

## 2. Enabling remote access

There are two ways to reach the loopback daemon from another device. **The
tunnel is the recommended default.** Pick exactly one.

### 2a. Serve the UI from the daemon (prerequisite for both)

The daemon serves its own SPA only when the binary is built with the `embed-ui`
Cargo feature, which bakes `ui/dist` into the binary via `rust-embed`
(`crates/otto-server/src/spa.rs`). **Build order matters — `rust-embed` reads
`ui/dist` at compile time:**

```bash
cd ui && npm run build                                  # produces ui/dist (MUST run first)
cargo build --release -p ottod --features embed-ui      # bakes ui/dist into ottod
```

Then `https://<host>/` returns the app, and `/api` + `/ws` work **same-origin**
— the UI auto-uses `location.origin` and `wss://`, so no base-URL override is
needed. Because the app is a hash router, deep links / refresh / back all resolve
to `index.html` (the SPA history-API fallback in `serve_spa`). The packaged
desktop `.app` already bundles this build; only a manual `cargo run` needs the
explicit feature flag.

### 2b. Option A — Cloudflare Tunnel (recommended)

`cloudflared` makes an **outbound** connection to Cloudflare's edge. There are
**no inbound ports**, nothing is opened on your router, and the daemon stays on
`127.0.0.1`. Leave the built-in `network_listener` **OFF** (§2c) — the tunnel
reaches loopback; you never bind `0.0.0.0`.

```bash
brew install cloudflared
cloudflared tunnel login
cloudflared tunnel create otto
# ~/.cloudflared/config.yml:
#   tunnel: <tunnel-id>
#   credentials-file: /Users/<you>/.cloudflared/<tunnel-id>.json
#   ingress:
#     - hostname: otto.<your-domain>
#       service: http://127.0.0.1:7700
#     - service: http_status:404
cloudflared tunnel route dns otto otto.<your-domain>
cloudflared tunnel run otto          # run as a launchd service for always-on
```

What you get: a stable public URL, e.g. `https://otto.<your-domain>/`.

- **TLS terminates at Cloudflare's edge** with a valid public cert → iOS/Android
  trust it and **PWA install works**. The Mac leg is loopback HTTP, and
  `cloudflared` encrypts the tunnel, so traffic never leaves the box in clear.
- **WebSockets** proxy transparently (`wss://` terminals and event streams work
  unchanged).
- **Do NOT put a Cloudflare Access (SSO) policy on the hostname.** Guest
  share-link recipients have no SSO account, and Otto's own bearer/share token
  *is* the gate. (If you want SSO only on the owner login, gate just that path and
  leave the `/#/s/...` share path open.)
- You do **not** need to add the Cloudflare hostname to Otto's CORS allowlist —
  same-origin serving means the browser's `Origin` matches the served host and no
  cross-origin request is made.

**Documented alternatives (not the default):** **Tailscale Funnel** — also no
open ports, edge TLS, and `*.ts.net` is already in Otto's CORS allowlist (§11),
but it is owner-device-centric with less per-path control. **Caddy reverse
proxy** — full control + Let's Encrypt, but needs an open inbound port + DNS +
router forwarding (the largest attack surface).

### 2c. Option B — the built-in `0.0.0.0` TLS network listener (LAN / self-hosted)

The daemon can also expose a second listener directly, controlled by the
`network_listener` setting (`crates/ottod/src/main.rs`). It is read at startup
from the settings table:

```json
{ "enabled": true, "port": 7700 }
```

When `enabled` is true the daemon binds `0.0.0.0:<port>` (default: the loopback
port) and serves it over **TLS (rustls)** — **never plain HTTP**, because a LAN
listener is reachable by other devices. The cert + key are auto-generated
(self-signed) under `<data_dir>/tls` on first use. The loopback listener on
`127.0.0.1:7700` always remains; the network listener is *additional*.

This is the right choice for trusted-LAN or self-hosted reach (e.g. behind your
own reverse proxy). Because the cert is self-signed, mobile browsers will warn on
the first connection and **PWA install may be blocked** until you trust the cert
— which is why the Cloudflare tunnel (valid public cert) is preferred for phones.
The CORS allowlist already trusts RFC-1918 LAN hosts and `*.ts.net` for this path
(§11). Keep this **OFF** when you use the tunnel.

> **Mac sleep kills remote control.** The host must stay awake — consider
> `caffeinate` or Energy-Saver settings. This is inherent: every session runs on
> the Mac.

---

## 3. Installing the PWA

Otto ships an installable Progressive Web App. The manifest
(`ui/public/manifest.webmanifest`) declares `"display": "standalone"`, the app
name/icons (192/512/maskable + a 180px Apple touch icon), and a dark
`theme_color`/`background_color` (`#111111`). `index.html` carries the
`apple-mobile-web-app-*` meta tags so iOS installs full-screen with the Otto
title. The service worker (`ui/public/sw.js`) is registered from
`ui/src/main.ts` on load.

### iOS (Safari)
1. Open `https://otto.<your-domain>/` in **Safari** and log in.
2. Share sheet → **Add to Home Screen** → **Add**.
3. Launch from the new icon — it opens full-screen, no Safari chrome, no App
   Store, no Apple Developer ID.

### Android (Chrome)
1. Open the URL in **Chrome** and log in.
2. Menu (⋮) → **Install app** / **Add to Home Screen**.
3. Launch from the icon (standalone window).

### What works offline
The service worker is deliberately conservative — it caches the **app shell only**,
never live data:

- `/api/*` and `/ws/*` are **never** cached — always the live daemon. Offline, the
  shell loads but data calls fail (you need the Mac reachable to do anything).
- Navigations / HTML are **network-first** with the cache as an offline fallback,
  so a new deploy is picked up immediately (no stale-shell trap).
- Hashed assets under `/assets/*` are immutable (content-addressed) and served
  **cache-first**.
- `CACHE_NAME` (`otto-shell-v2`) is bumped on policy changes and the old cache is
  purged on `activate`; when a new SW takes control after a deploy, the page
  reloads once so you never run a stale build.

In short: the PWA gives you an app-like icon and instant shell load, but Otto is
not a useful offline app — it is a remote control for a live daemon.

---

## 4. Email sender setup (Gmail App Password)

OTP-gated shares (§6–§7) require **one verified Gmail sender per user**. Configure
it in **Settings → Sharing** (`ui/src/modules/settings/EmailSenderSetup.svelte`).
This step is optional — plain (non-OTP) shares work without it.

### Steps
1. Create a **Gmail App Password**: Google Account → **Security → App passwords**
   (`https://myaccount.google.com/apppasswords`). Requires **2-Step Verification**.
   Generate one for "Mail"; you get a 16-character password.
2. In **Settings → Sharing**, enter your **Gmail address** and paste the
   **16-character App Password** (groups of 4, with or without spaces).
3. Click **Save and verify**.

### What Otto does
- `PUT /api/v1/email-sender` stores the **app password in the macOS Keychain**
  under `email-sender-{user_id}` (`crates/otto-server/src/routes/email_sender.rs`)
  — **never** in the DB. The `email_senders` table holds only the Gmail address
  and an opaque `secret_ref`, plus `verified_at` (migration `0045`).
- It then runs a **real Gmail SMTP login** to validate the pair —
  `smtp.gmail.com:587`, **STARTTLS + AUTH** (`GmailSender::verify` in
  `crates/otto-channels/src/email.rs`) — by sending a tiny probe mail from the
  address to itself.
- Only on success is `verified_at` recorded and the badge flips to **Verified**.
  A bad app password **fails closed** (HTTP `502`); the sender stays **Unverified**
  and the UI shows an actionable hint.

The password is **write-only**: `GET /api/v1/email-sender` returns the address +
`verified` flag and **never** the password. The form shows `●●●●` for a stored
password; **Re-verify** re-runs the SMTP check using the Keychain value without
re-entering it.

### Public link domain
Same page, **Public link domain** card: set `share_base_url` (e.g.
`https://otto.example.com`). This is the origin Otto uses to build share URLs
**and** the link emailed alongside an OTP code. If empty, links fall back to the
request `Host` header (or `127.0.0.1`). Set this to your tunnel hostname so
emailed links are clickable on the recipient's phone.

> Both `/email-sender` routes are **self-owned** — any authenticated member
> manages their *own* sender (Exempt in the feature policy, like `/auth/tokens`).

---

## 5. Creating & managing share links

Open a session's **Share…** action to launch the **Share this session** modal
(`ui/src/modules/agents/ShareModal.svelte`).

### Mint a link
1. **Permission** — `viewer` (read-only, can watch but not type) or `editor`
   (can type commands in the terminal). `admin` is rejected by the server.
2. **Recipient email** *(optional)* — leave blank for a plain link; fill it in to
   add the email-OTP gate (§7). If you have no verified sender, the field is
   disabled with a link to **Settings → Sharing**.
3. **Expiry** — for a **plain** link choose **Link expires after** (1h / 4h / 12h /
   24h). For an **OTP** link choose the **Session window (max 12h)** (30m / 1h /
   4h / 12h) — how long the guest may stay attached after entering the code.
4. **Label** *(optional)* — e.g. `for Alice`.
5. **Generate link.** The modal shows the URL, a **Copy** button, and a **QR code**
   for hand-off to a phone.

`POST /api/v1/sessions/{id}/share` returns `CreateShareResp { token, url, info }`.
The **raw token is shown exactly once** (only its SHA-256 is stored); `url` is the
ready-to-share `<origin>/#/s/{session_id}/{token}` fragment.

- **Plain share:** body carries `ttl_secs` (default `3600`, **clamped to
  `[60, 86400]`** — `SHARE_TOKEN_TTL_MIN_SECS` / `SHARE_TOKEN_TTL_MAX_SECS`). TTL
  is **FIXED**, never slid: `expires_at = created_at + ttl_secs`.
- **OTP share:** body carries `recipient_email` + `duration_secs` (default `3600`,
  **clamped to ≤ `43200`s = 12h** — `SHARE_OTP_WINDOW_MAX_SECS`).

### Mint guards (server-enforced)
The caller must **own the session or be a workspace Admin**
(`require_session_owner_or_admin`), must **NOT** be impersonated — an
impersonation overlay returns `403` *"an impersonated session cannot mint share
links"* — and must **NOT** already hold a scoped share token (a guest cannot mint
sub-shares: `403` *"a share token cannot mint further share links"*). Requesting
`role: "admin"` is rejected. A `share.mint` audit entry is written.

### List, revoke, revoke-all
The modal lists active (live, non-revoked) shares for the session via
`GET /api/v1/sessions/{id}/shares`, each showing its `token_prefix`, label, role,
and relative expiry.

- **Revoke one** — `DELETE /api/v1/auth/shares/{share_id}` (204, idempotent).
- **Revoke all** — `POST /api/v1/auth/shares/revoke-all` (revokes every share the
  caller owns).

**Revocation evicts immediately.** After revoking, `SessionManager::evict(session_id)`
is called and any still-attached guest receives a `{"type":"terminated"}` frame —
the WebSocket closes at once and the cached auth entry is dropped, so there is no
window where a revoked token still works. A `share.revoke` audit entry is written.

---

## 6. The email-OTP access gate

When a share carries a `recipient_email`, the scoped token reaches **nothing**
until the recipient redeems a one-time code mailed out-of-band — so a
leaked/forwarded link, on its own, is useless.

### On mint
Otto generates a **6-digit OTP** from `OsRng` (`generate_otp`, rejection-sampled
to avoid modulo bias), stores **only its SHA-256** (`otp_hash`) with a **~10-minute
expiry** (`SHARE_OTP_TTL_SECS = 600`), records the **locked** `recipient_email` and
the `max_expires_at` session-window end, and **emails the code** to the recipient
via the owner's verified sender (subject: *"Your Otto access code"*). A verified
sender is **required** — otherwise mint returns `400` ("set up a verified email
sender first"). All five OTP columns live on `auth_sessions` (migration `0046`).

### While OTP-pending (guest sees the gate)
The scoped token reaches **only** `/share/verify` and `/share/extend`. The feature
guard (`crates/otto-server/src/feature_guard.rs`) returns
`403 {"code":"forbidden","message":"share requires email-OTP verification"}` for
every protected route — **even `GET` the session** — and `/ws/term` refuses the
upgrade. The guest view (`SharePage.svelte`) detects this 403 and shows the
**Enter your access code** screen (a `inputmode="numeric"` `autocomplete="one-time-code"`
field).

### Redeeming
`POST /api/v1/share/verify { token, otp }` is **public/Exempt** — the share token
is the auth (`crates/otto-server/src/policy.rs`). It is **IP rate-limited** by the
share throttle (`otto_sessions::share_throttle::global()`, keyed on the real
socket peer from `ConnectInfo<SocketAddr>` — not a spoofable header — locking an
IP after **10 failures in a 15-minute window** for **15 minutes**, returning
`429 {"code":"too_many_requests","message":"too many failed code attempts; try again later"}`
+ a `Retry-After` header), checks `otp_hash == sha256(otp)` **AND** the code hasn't expired, and
on success sets `verified_at` and **clears `otp_hash`** (single-use — a new code
needs a resend). A wrong/expired/reused code records a throttle failure and returns
`401`. After verifying, the guest may attach (`/ws/term`) and `GET` the session
until `max_expires_at` (≤12h); once that window elapses the share **re-pends** and
must be re-verified.

### Extending (re-send a fresh code)
`POST /api/v1/share/extend { token }` is **public/Exempt** and re-issues a **FRESH**
OTP, re-emailed to the **LOCKED original `recipient_email` only**. **The request
body carries no email field by design** — the destination is read from the share
row, never the request, so access can never be redirected to another mailbox. It
is IP rate-limited, generates a new 6-digit code (`OsRng`), stores only its SHA-256
(~10-min expiry), **clears `verified_at`** (re-pending the share), and opens a fresh
**≤12h** window. Only `kind='share'` rows **with** a `recipient_email` are
extendable — a plain / missing / revoked share returns `400`; if the owner no
longer has a verified sender, `400`. The guest then re-verifies the new code via
`/share/verify`. The guest UI surfaces **Re-send code** on the OTP screen and an
**Extend session** overlay when the terminal window ends.

---

## 7. The responsive / mobile shell

The whole UI is built to be driven from a phone or iPad in **portrait and
landscape**, light/dark, and **RTL** — see
[`./rtl-and-responsive.md`](./rtl-and-responsive.md) for the full responsive +
RTL treatment. Highlights relevant to remote access:

- **Full responsive shell** — the rail/navigator/panels collapse into mobile
  layouts; sections are **independently scrollable** and **collapsible** so a
  small viewport never traps content.
- **Touch terminal** — the embedded terminal accepts touch input and an on-screen
  affordance for typing on a device with no physical keyboard.
- **Per-device session view** — session view state is isolated per device, so the
  phone and the desktop don't fight over which session is focused (per-device
  session isolation).
- **Guest share view** — `SharePage.svelte` is intentionally minimal for mobile:
  a slim header (session title + status pill + a **read-only** badge for viewer
  shares) over a full-bleed terminal, with the OTP and "extend" cards sized for a
  phone. It respects the app `zoom` setting.

> Known minor TODO: the phone navigation **drawer defaults open** on first paint
> in some layouts.

---

## 8. API / contract reference

Authoritative contract: `docs/contracts/api.md` — *Share-link tokens* /
*Email sender* / *Email-OTP gate for share links*. TypeScript mirrors live in
`ui/src/lib/api/types.ts`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `POST /api/v1/sessions/{id}/share` | session owner / ws admin | `CreateShareReq { role, ttl_secs?, label?, recipient_email?, duration_secs? }` | `CreateShareResp { token, url, info }` — **token shown once** |
| `GET /api/v1/sessions/{id}/shares` | session owner / ws admin | — | `ListSharesResp { shares: ShareInfo[] }` (live, non-revoked) |
| `DELETE /api/v1/auth/shares/{share_id}` | member (self-owned) | — | `204` — revokes + evicts; idempotent |
| `POST /api/v1/auth/shares/revoke-all` | member (self-owned) | — | `204` — revokes all caller's shares + evicts |
| `POST /api/v1/share/verify` | **public** (share token *is* the auth) | `VerifyShareReq { token, otp }` | `VerifyShareResp { verified: true }` (`401` bad/expired/reused; `429` throttled) |
| `POST /api/v1/share/extend` | **public** (share token *is* the auth) | `ExtendShareReq { token }` | `{ "ok": true }` (`400` non-OTP/missing/revoked or no verified sender; `429` throttled) |
| `PUT /api/v1/email-sender` | member (self-owned) | `SetEmailSenderReq { gmail_address, app_password }` | `EmailSenderResp { gmail_address, verified }` (`502` on SMTP-verify failure → not verified) |
| `GET /api/v1/email-sender` | member (self-owned) | — | `EmailSenderResp { gmail_address?, verified }` (never the password) |

Types:

- `ShareInfo = { id, session_id, role, token_prefix, label?, created_at, expires_at }`
  — `role` is `"viewer"` or `"editor"`, never `"admin"`; `token_prefix` is the first
  12 chars (the rest is unrecoverable); `expires_at = created_at + ttl_secs` (FIXED).
- `EmailSenderResp = { gmail_address?, verified }` — `gmail_address` omitted on `GET`
  when no sender is configured; `verified` is `true` once a real Gmail SMTP login
  succeeded.

**Transport details that matter for security:**

- The share token rides the URL **fragment** (`#/s/<id>/<token>`), so it is not sent
  to servers or in `Referer`. The guest UI router captures it into in-memory state
  and **strips it from the address bar + history** on arrival
  (`ui/src/lib/router.svelte.ts`, `history.replaceState`).
- The guest WebSocket carries the token via the **`otto-bearer` `Sec-WebSocket-Protocol`
  subprotocol** (`ui/src/lib/api/share.ts`), keeping it off the URL/query string and
  out of access logs; `?token=` is a fallback only.
- Guest REST/WS calls use the scoped token directly — **never** the owner's stored
  login token — so a guest's capability is strictly isolated from any owner session
  on the same device.

---

## 9. Capabilities & limitations

**Capabilities**
- Full owner remote control of every session/feature (RBAC-scoped) from a phone or
  iPad via a same-origin PWA.
- Scoped, fixed-TTL, revocable guest links to a single session at `viewer` or
  `editor`, with QR hand-off.
- Optional email-OTP second factor with a 12h post-verify window and re-send/extend.
- No inbound ports with the Cloudflare tunnel; secrets (login token, app password)
  in Keychain only.

**Limitations / deferrals**
- **TOTP (Phase-6) is deferred.** There is no authenticator-app/TOTP second factor;
  the only out-of-band factor is the **email** OTP. The OTP is **email-only** and
  delivered via **Gmail** App Password senders specifically.
- **Mac must stay awake** — sleep ends remote control (host-side sessions).
- **Self-signed `0.0.0.0` listener** warns in browsers and can block PWA install;
  for phones, prefer the Cloudflare tunnel's valid edge cert.
- A single emailed OTP lives ~10 minutes and is single-use; an expired/used code
  needs **Re-send**/**Extend**.
- Each user has **one** email sender; OTP shares require it **verified**.
- Minor UI TODO: phone nav drawer defaults open in some layouts.

---

## 10. Security posture

- **Loopback by default.** `ottod` binds `127.0.0.1:7700`; the `network_listener`
  (`0.0.0.0`) is **OFF** unless explicitly enabled, and when on it is **TLS-only**
  (self-signed under `<data_dir>/tls`). Exposure is your opt-in (tunnel or listener).
- **Scoped capability tokens.** A share token is bound to **one** session, capped at
  `viewer`/`editor` (**never admin/root**), with a **short FIXED** TTL
  (`[60, 86400]`s), only the SHA-256 stored, and an explicit `revoked` kill switch
  (migration `0044`).
- **Email-OTP second factor.** When enabled, the token reaches nothing until a
  6-digit code (SHA-256-stored, ~10-min, single-use, OsRng) mailed to a **locked**
  recipient is redeemed. The extend path can never redirect to a different mailbox.
- **IP rate-limiting.** `/share/verify` and `/share/extend` are throttled per-IP
  (`share_throttle`), returning `429` + `Retry-After`; failures (including probing a
  bad token) record throttle hits.
- **Immediate revocation.** Revoke / revoke-all evict attached guests
  (`{"type":"terminated"}`) and drop the cached auth entry — no lingering window.
- **No secrets on disk.** Login tokens and the Gmail App Password live in the macOS
  Keychain; the DB stores only opaque references.
- **Restricted CORS.** The daemon allows only Tauri origins, loopback (any port),
  RFC-1918 LAN hosts, and `*.ts.net` — arbitrary public web origins are rejected
  (`is_allowed_origin`). Same-origin tunnel serving never needs the hostname added.
- **Per-user RBAC + data isolation** apply throughout — see
  [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md).

---

## 11. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `https://<host>/` returns the "UI not embedded" placeholder | The binary was built **without** `embed-ui`, or `ui/dist` was missing at compile time. Rebuild: `cd ui && npm run build` **then** `cargo build --release -p ottod --features embed-ui`. |
| PWA won't install on iOS / "Add to Home Screen" missing | Use **Safari** (not an in-app browser) over **HTTPS with a valid cert**. The self-signed `0.0.0.0` listener cert can block install — use the Cloudflare tunnel. |
| App stuck on an old build after deploy | The SW reloads once when a new worker takes control; if stuck, fully close and reopen the PWA. The shell is network-first by design, so this should self-heal. |
| "Set up a verified email sender first" on share mint | You added a `recipient_email` but have no **Verified** sender. Configure & verify one in **Settings → Sharing** (§4). |
| Email sender saves but stays **Unverified** (`502`) | App Password is wrong/revoked, has stray spaces, wasn't generated for "Mail", or 2-Step Verification is off. Regenerate the 16-char password and **Re-verify**. |
| Guest sees "Enter your access code" but never gets the email | Check the owner's sender is **Verified**; check the recipient's spam; the code expires in ~10 min — use **Re-send code**. The address is **locked** to the one set at mint. |
| Guest: "Too many attempts" / `429` | The share throttle locked the IP after repeated bad codes. Wait for `Retry-After`, then retry / **Re-send**. |
| Emailed link points at `127.0.0.1` and won't open on a phone | Set **Public link domain** (`share_base_url`) to your tunnel hostname (§4). |
| Guest dropped with a "terminated" message | The owner **revoked** the share (or revoked all), or the ≤12h OTP window elapsed (re-pends → **Extend**). |
| Remote control dies when the Mac is idle | The Mac slept. Keep it awake (`caffeinate` / Energy-Saver); sessions run on the host. |
| Cross-origin requests rejected | Only loopback, RFC-1918 LAN, `*.ts.net`, and Tauri origins are allowed; same-origin tunnel serving avoids this entirely — don't proxy from an arbitrary public origin. |

---

## 12. Related docs

- [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) — per-user RBAC,
  per-session isolation, impersonation; the auth model share tokens build on.
- [`./rtl-and-responsive.md`](./rtl-and-responsive.md) — the responsive shell,
  light/dark, RTL, touch terminal, and per-device session view.
- [`../remote-access-runbook.md`](../remote-access-runbook.md) — the concise
  operator runbook this page expands on (tunnel/PWA/share quickstart).
- `docs/contracts/api.md` — authoritative REST/WS contract.
- Design / plan: `docs/superpowers/specs/2026-06-19-remote-mobile-access-design.md`,
  `docs/superpowers/plans/2026-06-19-remote-mobile-access-plan.md`.
