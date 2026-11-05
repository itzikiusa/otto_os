# Phone access — Tailscale + QR pairing + "Continue on phone"

> **Summary.** Reach Otto from **your own phone** over your **Tailscale
> tailnet** — never the public internet. Settings → **Phone access** turns
> `tailscale serve` on for Otto (HTTPS when the Mac can get a certificate,
> otherwise HTTP inside WireGuard), shows a **QR** to pair a phone, and the
> phone is signed in only after you **approve it on the Mac** with a matching
> 6-digit code. The phone gets its own revocable **device token** (no password
> typed). Every terminal session and every thread has **Continue on phone**: a
> QR that opens exactly that session/thread on the phone — pairing first when
> the phone is new.

---

## 1. Setup

1. **Install Tailscale on the Mac** — the Mac App Store app, the standalone
   app from tailscale.com, or `brew install --cask tailscale`. Sign in.
2. **Install Tailscale on the phone** (App Store / Play Store) and sign in to
   the **same tailnet**.
3. In Otto on the Mac: **Settings → Phone access**. Otto detects the CLI at
   `/usr/local/bin/tailscale`, `/opt/homebrew/bin/tailscale` or
   `/Applications/Tailscale.app/Contents/MacOS/Tailscale` (fixed paths only)
   and shows the tailnet, this Mac's MagicDNS name and address, and your
   tailnet devices.
4. **Turn on** (root, on the Mac). Otto runs
   `tailscale serve --bg --yes --https=443 http://127.0.0.1:7700` and verifies
   it by fetching `/api/v1/health` over HTTPS through the Mac's own tailnet
   address. If that fails, the HTTPS entry is removed and Otto serves
   `--http=80` instead, with a note explaining why (see §4).
5. **Pair phone** → scan the QR with the phone's camera → both screens show
   the same 6-digit code → **Approve** on the Mac. The phone is signed in.

Turning **off** removes only Otto's own serve entries (those whose `/`
handler proxies to this daemon's port). Other `tailscale serve` configs are
listed as "other serves" and never touched; Otto refuses to turn on over a port
another config already uses. Otto **never runs `tailscale funnel`** — and if it
finds Funnel enabled on its own entry it reports it as an error ("Turn off"
removes it).

## 2. Continue on phone

In a terminal session's pane header (⋯ menu and a small phone button) and in
every Threads thread header, **Continue on phone** shows a QR for
`<served url>/#/agents/<id>` or `#/threads/<id>`:

- **A phone is already paired** → the QR deep-links straight to that
  session/thread. The phone opens the same live terminal (multi-viewer attach)
  or the same thread (same runtime event stream). Threads are the main phone
  surface: the page works at phone width with drawers for the sidebar and
  inspector.
- **No phone paired yet** (or "Pair & open") → the QR is a pairing link that
  carries the route as `next`; after you approve on the Mac the phone lands on
  that session/thread. `next` is validated on both sides against an allow-list
  (`agents`, `threads`, `home`, `agents/<id>`, `threads/<id>`).
- **Phone access is off** → the dialog explains how to turn it on.

Deep links resolve the session's workspace first and switch to it, so opening
a session from another workspace never lands on an empty view; a session you
cannot read shows a clear error.

## 3. Security model

The goal: nobody but you, on a device you approved, can reach this Mac.

| Layer | What it does |
|---|---|
| Tailnet only | `tailscale serve` is reachable only from your tailnet devices, over WireGuard. Funnel is never used. The daemon still listens on `127.0.0.1` only. |
| Peer check | The host guard classifies each request's peer: `Local` (the desktop app), `Tailnet` (a `100.64.0.0/10` / `fd7a:115c:a1e0::/48` peer, or a `tailscale serve` request: loopback peer + `*.ts.net` Host + no `Tailscale-Funnel-Request` + a tailnet `X-Forwarded-For` — tailscaled *sets* that header to the real source), else `Other`. A device token is refused from `Other` peers; pairing claims are accepted only from `Tailnet` peers. A process on the Mac could forge those headers — it is already on the Mac. |
| Pairing code | 32 random bytes in the QR (URL fragment — never sent to a server log), only its SHA-256 stored, **single use**, **2-minute** expiry, one outstanding pairing per user. Claimed through a POST body, rate-limited per peer (5 bad codes / 300 requests per 10 min → 429). |
| Human approval on the Mac | The Mac shows "Pair ‹iPhone · Safari› from ‹100.x.y.z›?" and the 6-digit code the phone shows; only a person's own credential, **on the Mac** (`Local` peer), not impersonating, can create, approve or deny. Agent session / MCP / share credentials and device tokens get 403 — a paired phone can never approve another device. |
| Device token | Its own table (`phone_devices`), only the hash stored, minted in the same transaction that consumes the approved pairing — the raw token exists once, in the phone's poll response. Never cached; **idle expiry 30 days** (sliding), **absolute 180 days**; renameable; revocable. Stored on the phone like a normal login token. |
| Device restrictions | A device token is the owner's credential minus: listing/minting API, MCP and share tokens; every `Secret`-class route; every `Admin`-class route (users, grants, settings, plugins, …) except human approval gates (agent permission prompts, workflow / DB-change approvals — answering those from the phone is the point); and all of Phone access except "who am I" / "unpair me". Normal use works: sessions, threads, terminals, git, DB explorer, … |
| Revoke | Settings → Phone access → Revoke / Revoke all, or "Unpair this phone" on the phone (`/auth/logout` with a device token unpairs it too). The row is deleted, so the next request fails; the revocation pulse closes the phone's `/ws/events` socket within a beat and `/ws/term` re-checks within 5 s. A password reset / account disable revokes every device. |
| Audit | `phone.serve.on/off`, `phone.pair.create/claim/refused/approve/deny`, `phone.device.paired/rename/revoke/revoke_all/unpair` — never codes, secrets or tokens. |
| Rollback safety | Device tokens are not `auth_sessions` rows: an older daemon would read an unknown `kind` as a full login token. With their own table, a rollback just signs the phone out. |

## 4. Troubleshooting

**HTTPS fails, Otto falls back to HTTP.** HTTPS needs a Let's Encrypt
certificate for `<mac>.<tailnet>.ts.net`, which `tailscaled` fetches through
Tailscale's DNS resolver. If **another VPN's DNS server sits inside
Tailscale's `100.64.0.0/10` range** (e.g. a corporate VPN at `100.96.1.177`),
tailscaled cannot resolve the ACME endpoints and the certificate never arrives.
Otto then serves over **HTTP on port 80**: still end-to-end encrypted by
WireGuard and tailnet-only, but the browser shows no padlock and the page is not
a "secure context" (no passkeys / WebAuthn, no service worker). Fixes:
disconnect the other VPN while Tailscale fetches the certificate (it is cached
for ~90 days) and Turn off → Turn on again; or ask the VPN admin to move its DNS
out of `100.64.0.0/10`; also make sure **HTTPS certificates** are enabled in the
Tailscale admin console (DNS → HTTPS Certificates).

**"pairing only works from a device on your tailnet".** The phone opened the
link without Tailscale (VPN toggle off, or a different tailnet). Turn Tailscale
on on the phone and scan again.

**The code expired.** Codes last 2 minutes; press **New code**.

**The phone was signed out after an Otto downgrade.** Expected (see rollback
safety); pair again.

## 5. API, code and limits

- Contract: `docs/contracts/api.md` → *Phone access*.
- Backend: `crates/otto-phone` (routes, Tailscale CLI wrapper behind the
  `TailscaleCli` trait, device policy), `crates/otto-rbac/src/devices.rs`
  (pairings + device tokens), `crates/otto-core/src/{peer,phone}.rs` (peer
  classification, DTOs, `next` allow-list), migration `0202_phone_access`.
  Glue in otto-server: `host_guard.rs` (publishes the peer),
  `feature_guard.rs` (device gate), `policy.rs` (`/phone/*` exemption).
- UI: Settings → Phone access, the pair page (`#/pair/<code>[/<next>]`),
  "Continue on phone" in the session pane and thread headers.
- **Passkey unlock — TODO.** A WebAuthn unlock on top of the device token is
  only possible when the phone page is a secure context (HTTPS serve). Not
  built yet: it needs a credential table, registration/assertion routes and a
  lock screen; the HTTP fallback (§4) would have to keep working without it.
- The daemon serves Tailscale on its loopback port (`$OTTO_PORT`, default
  7700). Changing the port needs Turn off → Turn on.
