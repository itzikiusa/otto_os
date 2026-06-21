# Using Otto on a phone or iPad

A task-oriented guide to **driving Otto from a mobile device** — installing the
app, what the touch UI looks like, how to run and type into a session on a
touchscreen, and the limits to expect. This is the "how do I actually use it on
my phone" companion to the two reference docs it pulls from:

- [`./remote-mobile-access.md`](./remote-mobile-access.md) — *how to make Otto
  reachable* from a phone (Cloudflare tunnel / PWA / network listener) and the
  share-link + email-OTP mechanics.
- [`./rtl-and-responsive.md`](./rtl-and-responsive.md) — *how the responsive
  shell, touch terminal, theming, and RTL are built* (the full mechanism).

> **Where it lives:** the responsive shell is `ui/src/App.svelte` +
> `ui/src/shell/` driven by `viewport.svelte.ts`; the touch terminal is
> `ui/src/lib/components/Terminal.svelte`; the guest share view is
> `ui/src/modules/share/SharePage.svelte`. The daemon is unchanged — mobile is a
> pure front-end concern talking to the same `ottod` API.

---

## 1. First, make Otto reachable

Otto's daemon (`ottod`) binds **loopback only (`127.0.0.1:7700`) by default**, so a
phone can't reach it out of the box. Pick one of the paths in
[`./remote-mobile-access.md`](./remote-mobile-access.md):

| Path | Best for | Note |
|---|---|---|
| **Cloudflare Tunnel** (recommended) | Reaching your Mac from anywhere | Valid edge TLS cert (PWA installs cleanly); no inbound ports. |
| **`0.0.0.0` TLS network listener** | Same-LAN / self-hosted | Self-signed cert → browser warning; can block PWA install. |
| **Vite dev server on the LAN** | Local development only | Talks to a running daemon; not for real use. |

Your Mac must stay **awake** while you drive it remotely — the sessions run
host-side.

## 2. Install the app (PWA) — optional but nice

Otto ships an installable PWA (`ui/public/manifest.webmanifest`, `"display":
"standalone"`, dark theme, 192/512/maskable + 180px Apple-touch icons). You can
always just use the browser; installing gives you an app icon and full-screen
chrome-less launch.

**iOS (Safari):** open `https://otto.<your-domain>/`, log in → Share sheet →
**Add to Home Screen** → **Add**. Launch from the icon (full-screen, no Safari
chrome, no App Store / Developer ID needed).

**Android (Chrome):** open the URL, log in → menu (⋮) → **Install app** / **Add to
Home Screen** → launch from the icon (standalone window).

**Offline:** the service worker (`ui/public/sw.js`) caches the **app shell only** —
`/api/*` and `/ws/*` are *never* cached. Offline, the shell loads but nothing works
until the Mac is reachable again. Otto is a remote control for a live daemon, not
an offline app. New deploys are picked up immediately (navigations are
network-first; a new service worker reloads the page once).

## 3. What the UI looks like on a touch device

The layout re-buckets live by width (`viewport.svelte.ts`, kept in sync with
`app.css`), so **rotating** a device re-flows instantly:

| Mode | Width | Shell |
|---|---|---|
| **Phone** | ≤ **640px** | Single content pane. Navigator is an off-canvas **left drawer** (hamburger in the top bar); the right Activity panel is a **right drawer**; a phone **quick-action bar** sits under the top bar; a **bottom nav** sits at the bottom. |
| **Tablet / iPad portrait** | **641–1024px** | Single content pane with a persistent narrow **Navigator column** (220px); the right panel becomes a **right drawer**; compact top bar, no bottom nav. |
| **Desktop / iPad landscape** | ≥ **1025px** | The full 3-pane desktop shell (an iPhone in landscape ≈932px is treated as *tablet*). |

Desktop and mobile render **byte-for-byte identical content** — only the
surrounding chrome differs — so nothing is "missing" on mobile.

**Navigation affordances on phone:**

- **Bottom nav** (`BottomNav.svelte`) — your first 4 viewable modules as big tap
  targets (filtered by your RBAC `view` grants) plus a **More** sheet for the rest +
  Settings. The Agents tab shows a working-count badge. Respects the iOS home
  indicator safe-area.
- **Quick-action bar** (`MobileActionBar.svelte`) — the desktop chords a touch user
  can't press: **Palette (⌘K), New (⌘T), Close (⌘W), Find (⌘F), Broadcast (⌘⇧B)**.
  Each calls the exact same function as the keyboard shortcut. Broadcast only shows
  when sessions exist.
- **Drawers** (`Drawer.svelte`) — Navigator (left) and right panel slide over;
  backdrop tap or **Esc** dismisses, plus an explicit ✕ button. Width is
  `min(86vw, 320px)`; animations honour `prefers-reduced-motion`.

**Scrolling never traps you:** every pane owns its own scroll (`min-height: 0` flex
chain); on phone/tablet, module side-panels become **collapsible accordions** while
the primary content (diff, results grid, graph) is its own independent scroll
container. The document itself never scrolls horizontally.

## 4. Running and typing into a session on touch

The embedded terminal (`Terminal.svelte`) is the part that needed the most care for
touch — it's solid now:

- **Always visible.** On phone the terminal uses xterm's **DOM renderer** (not
  WebGL) — mobile WebViews often fail to create/keep a GL context, which used to
  leave a black void. The DOM renderer has no GPU dependency.
- **Readable.** Phone terminals never render below **15px** (`PHONE_MIN_FONT`), even
  if your configured font size is smaller.
- **Type by tapping.** Tap the terminal to focus it and raise the soft keyboard. A
  floating **⌨ button** toggles a **key accessory bar** with the keys a soft keyboard
  can't produce — **Esc / Ctrl / arrows / Ctrl-C** — each a ≥44px target, using the
  same input path as a physical keyboard.
- The desktop terminal toolbar (font zoom, copy-on-select) is hidden on phone.

So a typical phone flow: bottom nav → **Agents** → open a session → tap the terminal
→ type, or use ⌨ for Esc/Ctrl-C → watch output stream live. Use the quick-action bar
for ⌘K/⌘T/⌘F/Broadcast. See [`./agent-sessions.md`](./agent-sessions.md) for the
session lifecycle itself.

## 5. Per-device session view

If you bounce between a phone and a desktop, turn on **"Isolate sessions to this
device"** (`otto_session_isolation`) so each device only shows the sessions it
started and they don't fight over which session is focused. This is a **client-side
view filter only** — the daemon never reads it, and it is orthogonal to (and never
weakens) server-enforced RBAC/ownership. Details:
[`./rtl-and-responsive.md` §5](./rtl-and-responsive.md).

## 6. Light/dark + RTL on mobile

Theme (`data-theme` × `data-scheme`) and **direction** (LTR/RTL) are per-device
settings in **Settings → Appearance**; they apply identically on mobile. RTL flips
`<html dir>` and mirrors the layout via CSS logical properties; there's also an
experimental terminal-bidi mode. Full treatment:
[`./rtl-and-responsive.md`](./rtl-and-responsive.md).

## 7. Sharing what you're looking at

To let someone *else* watch or drive a single session from their own phone — without
giving them an account — mint a **share link** (optionally email-OTP gated). That's
its own task: see [`./session-sharing.md`](./session-sharing.md). The guest gets a
deliberately minimal, phone-sized view (`SharePage.svelte`): a slim header
(title + status + a **read-only** badge for viewer shares) over a full-bleed
terminal.

## 8. Capabilities & limitations

**You can**
- Drive every feature/session you have RBAC access to, from a phone or iPad, via a
  same-origin PWA or the browser.
- Run, watch, and type into agent sessions on touch (with the key accessory bar for
  Esc/Ctrl/arrows).
- Rotate freely — the shell re-buckets live; an iPad in landscape gets the full
  desktop shell.

**You cannot / caveats**
- Use Otto offline — the shell loads but every data call needs the live daemon.
- Keep working if the Mac sleeps — host-side sessions stop.
- Install the PWA cleanly behind the self-signed `0.0.0.0` listener (browser cert
  warning) — prefer the Cloudflare tunnel for phones.
- Known minor TODO: the phone navigation **drawer defaults open** on first paint in
  some layouts.

## 9. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Can't reach Otto at all | Daemon is loopback-only; enable a tunnel/listener — [`./remote-mobile-access.md` §2](./remote-mobile-access.md). |
| PWA won't install on iOS | Self-signed cert (network listener). Use the Cloudflare tunnel's valid cert. |
| Terminal is a black void | Should be fixed (DOM renderer on phone). Hard-reload to pick up the latest build (service worker is network-first for HTML). |
| Can't type special keys | Tap the floating **⌨** to open the key accessory bar (Esc/Ctrl/arrows/Ctrl-C). |
| Phone and desktop fight over focus | Enable **Isolate sessions to this device** (§5). |
| Content runs off-screen / pane is 0-height | Shouldn't happen (regression-guarded by E2E). Report with device + orientation. |

## 10. Related docs

- [`./remote-mobile-access.md`](./remote-mobile-access.md) — make Otto reachable (tunnel/PWA/listener), email-OTP.
- [`./session-sharing.md`](./session-sharing.md) — share one session with a guest.
- [`./rtl-and-responsive.md`](./rtl-and-responsive.md) — the responsive shell, touch terminal, theming, RTL (mechanism).
- [`./agent-sessions.md`](./agent-sessions.md) — the sessions you drive.
- [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) — roles, per-session isolation, the sharing model.
