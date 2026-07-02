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

## Troubleshooting

- **A window reopened off-screen** — shouldn't happen (frames are clamped to
  the available monitors at restore); if a frame is somehow bad, delete
  `~/Library/Application Support/Otto/windows.json` and relaunch.
- **Windows don't restore** — check the file above exists and is valid JSON;
  the shell logs registry save failures to the app's stderr log.
- **Same tab layout in every window** — that's the pre-feature behavior; make
  sure the app was rebuilt (secondary windows need `__OTTO_WIN__` injected by
  the shell).
