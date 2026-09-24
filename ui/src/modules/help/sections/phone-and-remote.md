---
id: phone-and-remote
title: Phone and remote
group: Basics
summary: Drive Otto from a phone or iPad, install it as an app, and share a single session with someone through a link.
---
## What it's for

Your sessions always run on your Mac. A phone, an iPad or another computer is a remote control for them: it shows the same pages, and you can watch agents, type into terminals and approve work from wherever you are.

There are 2 separate ways to use Otto away from your Mac:

- **Remote control for you**: reach your own Otto from another device and use the full app with your normal sign-in.
- **Share links for someone else**: send a link that reaches exactly one session, read-only or with typing allowed, for a limited time.

Both are off by default. The daemon only listens on `127.0.0.1`, so nothing is reachable from another device until you set it up.

## Getting started

1. **Make Otto reachable.** The recommended way is a Cloudflare tunnel from your Mac to `http://127.0.0.1:7700`. It needs no open ports and gives you a trusted HTTPS address. On a trusted local network, a root admin can instead turn on **Settings → Daemon → Enable network listener**.
2. **Open Otto on your phone.** Go to your tunnel's address in Safari or Chrome and sign in as usual.
3. **Install it as an app (optional).** In Safari, use the Share sheet → **Add to Home Screen**. In Chrome, use the menu → **Install app**. Otto then opens full screen from its own icon.
4. **Open a session.** Tap **Agents** in the bottom bar, open a session and tap the terminal to type.
5. **Share one session.** Right-click a session tab, choose **Share…**, pick **Viewer** or **Editor** and an expiry, and send the link or let the other person scan the QR code.

## Everything it can do

**Layouts for every screen**
- **Phone** (up to 640 px wide): one page at a time. The sidebar is a drawer from the left, the right panel is a drawer from the right, and a bottom bar holds your modules.
- **Tablet** (641–1024 px): a narrow sidebar column stays visible and the right panel is a drawer.
- **Desktop** (1025 px and wider, including an iPad in landscape): the full 3-pane layout.
- Rotating the device switches layouts straight away. Pages show the same content at every size; only the surrounding chrome changes.

**Phone navigation**
- **Bottom bar**: your first 4 modules, in your sidebar order, as large buttons. **More** opens the rest and Settings. The Agents button shows how many agents are working, and the Assistant button shows how many things need you.
- **Quick-action bar**: buttons for the shortcuts you can't press on a touch screen: **Palette** (`⌘K`), **New** (`⌘T`), **Close** (`⌘W`), **Find** (`⌘F`) and, when sessions exist, **Broadcast** (`⌘⇧B`).
- **Drawers**: close with a tap on the backdrop, the close button or `Esc`.
- The command palette sheet replaces the floating bar on phones and tablets.

**Terminals on touch**
- Tap the terminal to raise the keyboard. Text never renders smaller than 15 px on a phone.
- Tap the keyboard button for a key bar with the keys a soft keyboard lacks: **Esc**, **Tab**, **Ctrl** (tap, then a letter), **Ctrl-C**, **⇧↵** (a new line in an agent's prompt), the 4 arrows, and `|`, `/` and `~`.

**Per-device settings**
- **Isolate sessions to this device** (Settings → Appearance) shows only the sessions you started on this device, so a phone and a Mac don't fight over which session is focused. Other sessions keep running; they're just hidden here.
- Theme, scheme and layout direction (including right-to-left) are saved per device.

**Share links**
- A share link reaches one session only. **Viewer** can watch but not type. **Editor** can type into the terminal. A link can never reach another session or your account.
- Expiry: 1, 4, 12 or 24 hours. You can add a label to tell links apart.
- **Email code (optional)**: enter the recipient's email address and Otto emails them a 6-digit code. The link does nothing until they enter it. Code-protected links last 30 minutes, 1, 4 or 12 hours. When one ends, the guest can choose **Extend** to get a fresh code sent to the same address.
- The share dialog shows the link and a QR code, lists the session's active links and lets you **Revoke** any of them.
- The guest sees a full-screen view of just that session: its title, status and terminal, with a **read-only** badge for viewers.

**Sharing settings** (Settings → Sharing)
- Set up a Gmail sender (with an App Password) so Otto can email codes.
- **Public link domain**: the address Otto uses when it builds share links, such as your tunnel's domain. Without it, links use the address you opened Otto on.

## Keyboard shortcuts

None specific to this page. On a phone, the quick-action bar and the terminal key bar stand in for the keyboard shortcuts.

## Tips and limits

- Your Mac must be on, awake and running Otto's daemon. If it sleeps, remote sessions stop responding.
- Otto doesn't work offline. An installed app opens without a connection, but it can't do anything until it reaches your Mac again.
- The network listener uses a self-signed certificate, so phones show a warning and may refuse to install the app. Use a Cloudflare tunnel for phones.
- The network listener setting takes effect when the daemon restarts. Turn it on only on networks you trust: anyone on the network can reach the sign-in page.
- Don't put a single sign-on gate (such as Cloudflare Access) in front of the whole address: share-link guests don't have accounts. Otto's own sign-in and share tokens are the gate.
- Remote users see only the modules and sessions their role allows. Share links are capped at editor and can't be raised.
- Only a root admin can turn on the network listener. Email codes need a Gmail sender set up first.

## Related

- [Getting started](#/walkthroughs/getting-started)
- [Agents](#/walkthroughs/agents)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
- [Desktop app](#/walkthroughs/desktop-app)
