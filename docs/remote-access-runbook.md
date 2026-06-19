# Otto Remote Access — Operator Runbook

How to reach Otto from a phone/tablet over the internet, securely. The phone is a
thin browser client; every session runs on the Mac. See the design at
`docs/superpowers/specs/2026-06-19-remote-mobile-access-design.md`.

## What you get

- **Owner remote control** — log into the full Otto UI from your phone and drive
  everything (open a Claude session → it runs on the Mac → you watch/type from the
  phone).
- **Guest share links** — generate a scoped `https://<host>/#/s/<session>/<token>`
  link that lets someone view (or, if you grant it, drive) **one** session and
  nothing else. Links are short-lived, revocable, rate-limited, never carry root,
  and (when the email-OTP gate is enabled) require a one-time code mailed to a
  locked recipient.

## 1. Serve the UI from the daemon

The daemon can serve its own SPA when built with the `embed-ui` Cargo feature:

```bash
cd ui && npm run build              # produces ui/dist  (MUST run first)
cargo build --release -p ottod --features embed-ui   # bakes ui/dist into the binary
```

Then `https://<host>/` returns the app and `/api`, `/ws` work same-origin (the UI
auto-uses `location.origin` + `wss://`, no `otto_base` override needed). Because the
UI is a hash router, deep links / refresh / back all resolve to `index.html`
automatically. (Build order matters — `rust-embed` reads `ui/dist` at compile time.)

## 2. Expose it — Cloudflare Tunnel (recommended)

`cloudflared` makes an **outbound** connection to Cloudflare's edge — **no inbound
ports**, nothing opened on your router, the daemon stays on `127.0.0.1`.

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
cloudflared tunnel run otto          # (run as a launchd service for always-on)
```

- **TLS terminates at Cloudflare's edge** (valid public cert → iOS/Android trust it
  and PWA install works). The Mac leg is loopback HTTP — traffic never leaves the box
  unencrypted (cloudflared encrypts the tunnel).
- **WebSockets** proxy transparently (`wss://` terminals/events work unchanged).
- **Do NOT put a Cloudflare Access (SSO) policy on the hostname** — guest share-link
  recipients have no SSO account, and Otto's own bearer/share token IS the gate. (If
  you want SSO on the *owner login* only, gate just that path and leave the share
  path open.)
- Keep the built-in `network_listener` setting **OFF** — the tunnel reaches the
  loopback daemon; you never bind `0.0.0.0`.

### Alternatives (documented, not default)
- **Tailscale Funnel** — also no open ports, edge TLS; `*.ts.net` is already trusted
  by Otto's CORS allowlist. Simpler, but owner-device-centric and less per-path control.
- **Caddy reverse proxy** — full control + Let's Encrypt, but needs an open inbound
  port + DNS + router forwarding (largest attack surface).

## 3. Install on the phone (PWA)

Open `https://otto.<your-domain>/` in Safari (iOS) or Chrome (Android), log in, then
**Add to Home Screen**. The app installs full-screen with an icon — no App Store, no
Apple Developer ID.

## 4. Share a session securely

1. In a session's menu choose **Share…**; pick **viewer** (read-only) or **editor**,
   a **duration**, and (if email-OTP is set up) a **recipient email**.
2. Send the recipient the link. With email-OTP on, they must also enter the one-time
   code mailed to them — so a leaked link alone is useless.
3. **Revoke** any share at any time (Settings → the session's shares, or "revoke all")
   — an attached guest is dropped immediately.

## Security posture (defaults)

- Daemon bound to `127.0.0.1`; `network_listener` OFF; exposure only via the tunnel.
- Share tokens: scoped to one session, **never root**, short fixed TTL, single-use OTP
  (when enabled), IP rate-limited, revocable, token carried in the URL **fragment**
  (not sent to servers/Referer) and over the `otto-bearer` WS subprotocol (not the URL).
- Per-user RBAC + data isolation apply (`docs/MULTI-USER-RBAC.md`).

## Watch-items

- **Mac sleep** kills remote control (inherent — the host must be awake). Consider
  `caffeinate` / Energy-Saver settings.
- Treat the Cloudflare hostname as public: rely on Otto's auth + the share-link
  scoping; don't add the hostname to the daemon's CORS allowlist (same-origin serving
  means it's never needed).
