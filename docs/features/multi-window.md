# Multi-window

Open any number of Otto windows, each an independent workspace surface — its
own current module, workspace selection, session tabs, split panes and view
mode — and get the exact same window set back after a relaunch: same frames
(position/size/screen/fullscreen), same workspace, same tabs, same view.

> TL;DR — **File → New Window** (`⌘⇧N`) opens another full Otto window.
> Closing a window forgets it; **quitting** (`⌘Q`, or closing the last window)
> snapshots the whole set and the next launch restores it. Sessions always live
> in the daemon — windows only hold *references*, so nothing restarts or
> duplicates when windows come and go.

---

## How it works

```
apps/desktop/src-tauri/src/windows.rs      window registry + lifecycle
  ~/Library/Application Support/Otto/windows.json
  { "next_id": 3, "windows": [ {label,x,y,w,h,fullscreen}, … ] }

ui/src/lib/win.ts                          per-window identity + key namespacing
  windowId  = ?win=<id> (browser/E2E)  ||  window.__OTTO_WIN__ (Tauri)  ||  'main'
  winKey(k) = k                 for the main window (legacy keys, zero migration)
            = otto_win_<id>::k  for secondary windows
```

- **Registry (Rust).** The shell persists every window's label + physical frame
  in `windows.json` (atomic temp-file + rename; a corrupt/missing file degrades
  to a single main window). On launch it restores the main window's frame and
  recreates each secondary window — clamped back on-screen if a monitor
  disappeared — injecting `window.__OTTO_WIN__='<label>'` before the SPA loads.
- **Per-window SPA state.** All layout keys the workspace store persists
  (`otto_workspace`, `otto_tabs_<ws>`, `otto_view_mode`) plus the router's
  `otto_last_route` go through `winKey()`. Windows share one localStorage, so
  namespacing is what keeps two windows from clobbering each other. State for
  windows that no longer exist is GC'd on boot via the `windows_registry`
  command.
- **Close vs quit.** A `CloseRequested` on a non-last window removes it from
  the registry (that window is gone for good). `⌘Q` — and closing the *last*
  window, which exits the app — flips a `QUITTING` flag first, so the teardown
  closes don't "forget" anything, then snapshots all frames.
- **Menu routing.** Menu accelerators are app-wide in Tauri, so the shell
  emits `otto://menu` to the **focused** window only, and `menu.ts` listens
  per-webview-window. Otherwise `⌘W` would close a tab in every window at once.
- **Embedded browser.** Browser-tab child webviews anchor to the window whose
  Browser panel opened them; their `otto://browser-url` / `otto://browser-new-tab`
  events are emitted to that window only, and hide-all/close-all are scoped per
  window.

## Capabilities & limits

- Every window is a full workspace surface: any module, any workspace, any
  session — including the same session in two windows (both attach to the one
  daemon PTY, like two viewers of the same terminal).
- The dock badge (working-agent count) is written by the **main** window only —
  it's app-global, so one writer keeps it consistent.
- The `main` window keeps the pre-multi-window localStorage keys, so upgrading
  (or ignoring the feature entirely) changes nothing.
- Browser/E2E contexts select a window identity with `?win=<id>` — that's how
  the Playwright spec (`ui/e2e/desktop-multiwindow.spec.ts`) exercises the
  namespacing without a Tauri shell.
- Window state is macOS-desktop-only (the shell owns it); the web/remote UI is
  untouched.

## Side by side

Inside one window, any two sidebar sections can sit side by side — e.g. Agents
beside Connections. Right-click a sidebar row → **Open side by side** (or
⌥-click it, or `⌘\` for a picker); `⌘\` again, or the pane's ✕, closes it.

```
ui/src/lib/sidePane.ts                  pure rules + the postMessage protocol (unit/sidePane.test.ts)
ui/src/lib/stores/sidePane.svelte.ts    host: route/split/placement, router delegate, message bridge
ui/src/lib/embedGuest.ts                the pane's half (?embed=1): hands window verbs to the host
ui/src/shell/SidePane.svelte            the iframe + its loading/error cover
ui/src/shell/SplitDivider.svelte        the resizable, keyboard-operable divider
```

- **The pane is the app in a same-origin iframe** (`?embed=1#/<route>`) — its
  own router, stores and event socket, the same token. It renders chrome-less
  and keeps this window's keys (it is part of the window, not a new one).
- **One module per pane.** Either document's router hands a navigation to the
  module the OTHER pane shows over to that pane (`RouteDelegate` in
  `router.svelte.ts`); back/forward skip those entries.
- **Keys.** Window verbs pressed in the pane (⌘K, ⌘T, ⌘1, ⌘,, zoom…) run in the
  window; find, history and terminal zoom stay in the pane; session verbs stay
  there while it shows Agents. ⌘W / ⌘A / End Session from the native menu act
  on the pane when it has focus (⌘W closes a non-Agents pane). The pane's own
  ⌘K commands are mirrored into the window's palette.
- **Sync.** The pane follows the window's workspace; theme/terminal settings
  changed in one pane reach the other (`storage` events); whichever pane shows
  Agents owns the session tabs and the other adopts them.
- **Persisted per device** (`otto_side_pane`): the pane's last route, the
  split (25–75%, 360px floor per pane) and which side it's on. Swap is visual
  (CSS order) — neither pane reloads.
- **Once per window, never in the pane:** the native menu bridge, native
  notifications, WS notice toasts, last-route restore, window-key GC, the
  service worker, git auto-fetch (unless the pane shows Git), the palette's
  own command sets.
- **Limits.** Desktop width and the main window only (not pop-outs, phone or
  tablet). The pane's document has no Tauri bridge: it can't drag the window,
  and native-only features in it fall back to their web behaviour (external
  links are opened by the window). Each pane is a full app instance — a second
  event socket and store set while it's open.

## Troubleshooting

- **A window reopened off-screen** — shouldn't happen (frames are clamped to
  the available monitors at restore); if a frame is somehow bad, delete
  `~/Library/Application Support/Otto/windows.json` and relaunch.
- **Windows don't restore** — check the file above exists and is valid JSON;
  the shell logs registry save failures to the app's stderr log.
- **Same tab layout in every window** — that's the pre-feature behavior; make
  sure the app was rebuilt (secondary windows need `__OTTO_WIN__` injected by
  the shell).
- **Secondary window won't drag / double-click-maximize** — the custom
  titlebar drives both over IPC (`startDragging` / `toggleMaximize`), which the
  Tauri capability must grant to `w*` labels (`capabilities/default.json`,
  pinned by a regression test in `windows.rs`). Capabilities are baked at build
  time — rebuild the app (`deploy.sh`), a daemon-only rebuild is not enough.
