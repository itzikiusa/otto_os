# Sharing a session

A task-oriented guide to **sharing a single Otto session with someone who doesn't
have an account** — minting a scoped link, optionally gating it behind an email
one-time code, what the guest sees, and how to revoke. It pulls the mechanism
detail from two reference docs:

- [`./remote-mobile-access.md`](./remote-mobile-access.md) — the share-link token,
  email-OTP gate, Gmail sender setup, and the full API/security surface.
- [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) — how sharing fits
  the multi-user role model and per-session isolation.

> **Where it lives:** mint UI is `ui/src/modules/agents/ShareModal.svelte`; the
> guest landing page is `ui/src/modules/share/SharePage.svelte`; the daemon side is
> `crates/otto-server/src/feature_guard.rs` + the share routes; tokens are stored as
> SHA-256 only (migrations `0044`–`0046`). The raw token rides the URL **fragment**
> and never the owner's login token.

---

## 1. What it's for

Let a teammate, reviewer, or stakeholder **watch (or drive) one specific session**
in their own browser/phone — for the length of a meeting or a debugging session —
**without** creating a user account for them. A share is:

- **Scoped to one session** (not your whole workspace).
- Capped at **viewer** (watch only) or **editor** (can type) — never admin/root.
- **Time-boxed** and **revocable**, with the raw token shown exactly once.
- Optionally **email-OTP gated** so a leaked/forwarded link is useless on its own.

For sharing your *own* access across your own devices instead, see
[`./mobile-usage.md`](./mobile-usage.md) (per-device view) — that's a different thing.

## 2. Prerequisites

1. **The guest must be able to reach Otto.** A share URL points at your daemon's
   public origin, so set up remote access first (Cloudflare tunnel recommended) and
   set the **Public link domain** (`share_base_url`) so emailed/QR links are
   clickable on a phone — see [`./remote-mobile-access.md` §2/§4](./remote-mobile-access.md).
2. **For OTP-gated shares only:** a **verified Gmail sender** (Settings → Sharing,
   one per user, Gmail App Password). Plain (non-OTP) links work without it. Setup:
   [`./remote-mobile-access.md` §4](./remote-mobile-access.md).
3. **You must own the session** (or be a workspace **Admin**), and you must not be
   acting through impersonation, and you must not yourself be a guest on a shared
   token (no sub-shares).

## 3. Mint a share link

Open the session's **Share…** action → the **Share this session** modal:

1. **Permission** — **viewer** (read-only: watch but not type) or **editor** (type
   commands in the terminal). `admin` is rejected server-side.
2. **Recipient email** *(optional)* — leave blank for a plain link; fill it in to
   add the **email-OTP gate** (§4). Disabled if you have no verified sender (with a
   link to set one up).
3. **Expiry** —
   - **Plain** link: **Link expires after** 1h / 4h / 12h / 24h. The TTL is **fixed**
     (`expires_at = created_at + ttl`, clamped to `[60s, 24h]`) — it never slides.
   - **OTP** link: **Session window (max 12h)** 30m / 1h / 4h / 12h — how long the
     guest may stay attached after entering the code.
4. **Label** *(optional)* — e.g. `for Alice`.
5. **Generate link** — the modal shows the **URL**, a **Copy** button, and a **QR
   code** for handing off to a phone.

The **raw token is shown once** (only its SHA-256 is stored); the URL is the
fragment form `<origin>/#/s/{session_id}/{token}`. A `share.mint` audit entry is
written.

## 4. The email-OTP gate (recommended for anything sensitive)

When you set a **recipient email**, the link alone reaches **nothing** until the
recipient enters a **6-digit code** emailed to them out-of-band:

- On mint, Otto generates the code (`OsRng`, rejection-sampled), stores **only its
  SHA-256** with a **~10-minute expiry**, **locks** the recipient address, and emails
  the code (subject *"Your Otto access code"*) via your verified sender.
- While pending, the token reaches **only** the verify/extend endpoints — even
  `GET` the session and the terminal WebSocket are refused — so the guest sees the
  **"Enter your access code"** screen.
- The code is **single-use**; a wrong/expired/reused code fails and is **IP
  rate-limited** (10 failures / 15-min window → 15-min lockout, `429`).
- After verifying, the guest may attach until the **session window** ends (≤12h),
  after which the share **re-pends**.
- **Extend / re-send:** the guest can request a fresh code; it is **only ever
  re-emailed to the locked original recipient** (the request carries no email field
  by design), so access can never be redirected to another mailbox.

## 5. What the guest experiences

The guest opens the link to a deliberately minimal, mobile-friendly page
(`SharePage.svelte`) — no rail, no navigator, no side panels — that walks through:

| State | What they see |
|---|---|
| **No / stripped token** | "Link invalid or expired — ask the owner for a new link." |
| **Loading** | A spinner / "Connecting…". |
| **OTP required** | "Enter your access code" — a 6-digit `one-time-code` field, **Verify**, and a **Re-send code** link. |
| **Attached** | A slim header (session title + status pill + a **read-only** badge for viewer shares) over a **full-bleed terminal**. Editor shares can type; viewer shares can't. |
| **Window ended** | An **Extend session** overlay to request a fresh code and re-attach. |

The token lives in the URL **fragment** and is **stripped from the address bar +
history** on arrival; the guest WebSocket carries it via the `otto-bearer`
subprotocol (off the URL/query/logs). Guest calls use the **scoped token only** —
never your login token.

## 6. Manage & revoke shares

The Share modal lists the session's active (live, non-revoked) shares — each with
its `token_prefix`, label, role, and relative expiry:

- **Revoke one** — removes that link.
- **Revoke all** — removes every share you own.

**Revocation evicts immediately:** any still-attached guest gets a
`{"type":"terminated"}` frame, the WebSocket closes at once, and the cached auth is
dropped — there is no window where a revoked token still works. A `share.revoke`
audit entry is written.

## 7. Guards, capabilities & limitations

**You can**
- Hand someone a scoped, fixed-TTL, revocable link to **one** session at viewer or
  editor, with QR hand-off and optional email-OTP second factor.
- Re-send/extend an OTP window (to the locked recipient) up to 12h.

**You cannot / caveats**
- Share at **admin** (rejected) or share your whole workspace — one session per link.
- Mint a share **while impersonating**, or mint a **sub-share** from a guest token
  (both `403`).
- Slide a plain link's TTL — it's fixed at mint; mint a new one to extend.
- Use OTP without a **verified Gmail sender**; OTP is **email-only** (TOTP/authenticator
  is deferred — Phase 6).
- Keep a guest connected if the Mac sleeps or the session ends.

## 8. API reference (condensed)

Authoritative: `docs/contracts/api.md` — *Share-link tokens* / *Email-OTP gate*.

| Method & path | Auth | Purpose |
|---|---|---|
| `POST /api/v1/sessions/{id}/share` | session owner / ws admin | Mint a link → `{ token, url, info }` (**token shown once**). |
| `GET /api/v1/sessions/{id}/shares` | session owner / ws admin | List live shares for the session. |
| `DELETE /api/v1/auth/shares/{share_id}` | member (self-owned) | Revoke one (204, idempotent, evicts). |
| `POST /api/v1/auth/shares/revoke-all` | member (self-owned) | Revoke all your shares (evicts). |
| `POST /api/v1/share/verify` | **public** (token *is* the auth) | Redeem the 6-digit OTP (`401` bad, `429` throttled). |
| `POST /api/v1/share/extend` | **public** (token *is* the auth) | Re-send a fresh code to the locked recipient; open a new ≤12h window. |

Full table, request/response types, and transport-security notes:
[`./remote-mobile-access.md` §8](./remote-mobile-access.md).

## 9. Related docs

- [`./remote-mobile-access.md`](./remote-mobile-access.md) — share/OTP mechanics, Gmail sender, reachability, full API.
- [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) — roles, per-session isolation, impersonation, audit.
- [`./mobile-usage.md`](./mobile-usage.md) — what the guest's (and your) mobile view looks like.
- [`./agent-sessions.md`](./agent-sessions.md) — the sessions you share.
