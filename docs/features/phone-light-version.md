# Phone version (light)

An opt-in, per-device way to make Otto thinner on a phone: five modules instead
of the whole app, opening on Threads. The full app stays the default on every
device; turn light on for the occasions a lighter app helps, and back off in
one tap.

> **Where it lives:** `ui/src/lib/lightMode.svelte.ts` (the switch, the module
> set, the phone-size test), `ui/src/shell/BottomNav.svelte` (tabs and the More
> sheet), `ui/src/shell/pages.svelte.ts` (`pageKeyOf`: landing + gating),
> `ui/src/shell/RouteGate.svelte` (the "not in the light version" page) and
> `ui/src/lib/sidebar.ts` (`setModuleScope`, read by every navigation surface).
> Pure front end: no daemon change, no setting stored on the server.

## Turning it on and off

| Where | On | Off |
|---|---|---|
| Bottom nav → **More** | **Light version** (first row) | **Full app** (first row of the ⋯ sheet) |
| Settings → Appearance → Phone | **Light version** toggle | same toggle |
| ⌘K / Commands | **Use the light version** | **Show the full app** |
| A hidden page's notice | – | **Open the full app** |

The choice is remembered in this device's browser storage
(`otto_phone_light`, `'1'` = light). Another phone, a tablet or the Mac are not
affected. Turning light on from a page it hides goes to Threads.

## What the light version shows

- **Exactly five tabs**: Threads, Connections, Git, API, Usage, in that order
  (your role still applies: a module you can't open is not shown). Your saved
  sidebar order, favorites and hidden list are not used and not changed.
- **A small ⋯** after the tabs (not a sixth tab). It opens a sheet with
  **Full app**, **Notifications** (the bell, with the unread count also on the
  ⋯), **Commands**, **Settings**, **Sign out** and the **Machine** row.
- **Threads is the landing page**: opening the app with no route (a
  home-screen launch) goes to `#/threads`.
- **⌘K "Go to"** lists only the five (plus Settings and Message Brokers, which
  is part of Connections).
- Connections keeps its own views (`#/database`, `#/brokers`); Settings and
  Help guides still open.

## Hidden modules

A link to any other module (`#/agents/…`, `#/kubernetes`, a plugin…) shows a
calm page: "*Module* isn't in the light version", with **Open the full app**
(switches off light and shows that page) and **Go to Threads**. The hidden
module's code is never downloaded: the route maps to the gate page before its
chunk is requested, and hover / idle prefetch go through the same map.

Agent sessions open on the Agents page, which the light version leaves out, so
"open session" links (from Usage, a review, a thread's work row) land on that
notice. Use **Open the full app** to follow them.

## Phone on its side

A phone in landscape gets the tablet layout (persistent sidebar, status bar, no
bottom nav), as before. With light on it keeps the five-module set: the sidebar
lists only those five. Switch back from Settings → Appearance or ⌘K. "Phone
sized" means the phone breakpoint (≤ 640 px wide) or a landscape screen no taller
than 500 px; tablets and desktops never enter light mode, and pop-out windows
and the side-by-side pane ignore it.

## Limits

- The Settings → Appearance sidebar customizer shows the five while light is on.
- The phone top bar's Navigator drawer (on pages other than Threads) stays, for
  switching workspaces; it lists the same five modules.

## Tests

- `ui/unit/lightMode.test.ts`: module set, route allow-list, scope filtering of
  `availableModules` / `goToEntries`, and `pageKeyOf` gating.
- `ui/e2e/light-mode-mobile.spec.ts` (phone projects): switch on from More,
  five tabs, Threads landing, gated deep link, back to the full app, no
  horizontal scroll on the five pages.
- `ui/e2e/desktop-light-mode-off.spec.ts`: a stored light choice changes
  nothing on desktop.
