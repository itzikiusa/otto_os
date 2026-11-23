# Keep Awake

A lighter `caffeinate`: keep this Mac and its display awake for a while, or
indefinitely, from the menu bar, ⌘K or a global shortcut. Nothing else.

It lives entirely in the desktop shell (`apps/desktop/src-tauri/src/keep_awake.rs`):
the shell holds the two power assertions `caffeinate -d -i` holds
(`PreventUserIdleDisplaySleep` and `PreventUserIdleSystemSleep`). The daemon is
not involved, so there is no API route for it.

## Setup

Nothing to set up and no permission to grant. It is off at every launch.

## Walkthrough

1. **Turn it on** — any of:
   - right-click Otto's menu-bar icon → **Keep Awake** → **For 15 Minutes**,
     **For 1 Hour**, **For 2 Hours**, **For a Custom Time…**, or
     **Indefinitely**;
   - ⌘K → "Keep awake for 15 minutes" / "…1 hour" / "…2 hours" / "…a custom
     time…" / "Keep awake indefinitely";
   - **⌘⌃⇧C** from any app (desktop app) turns it on indefinitely, or off when
     it is on. Change or turn off the chord in **Settings → Keep awake**.
   A custom time takes minutes or hours: `45`, `45m`, `3h`, `1h 30m`, `1:30`.
   (From the menu bar, **For a Custom Time…** opens Otto to ask.)
2. **See it** — while it is on, the menu-bar icon shows the time left beside it
   (`42m`, `1h 5m`, or `∞` when indefinite), refreshed at least every 30 s,
   and Otto's status bar shows **Awake · 42m**.
3. **Turn it off** — click **Awake** in the status bar, the menu bar's **Keep
   Awake** → **Turn Off**, ⌘⌃⇧C, or ⌘K → "Stop keeping awake". A timed run turns
   itself off when its time is up. Starting a new duration replaces the old one.

## Capabilities and limits

- Same effect as `caffeinate -d -i`: idle system sleep and display sleep are
  prevented. Closing the lid, choosing Sleep, or a low battery still sleeps the
  Mac (as with `caffeinate`). The deadline is checked against the wall clock,
  so a forced sleep doesn't extend a timed run.
- Works with the main window closed while Otto stays in the menu bar.
- **Quitting or a crash ends it.** macOS ties the assertions to Otto's process
  and drops them when it exits; there is no helper process to leave behind.
- State is in memory only: after a relaunch it is off.

## Privacy

Nothing is stored or sent anywhere.

## Troubleshooting

- **Is it really on?** `pmset -g assertions` lists two lines for Otto's
  desktop process named `Otto keep awake` (`PreventUserIdleDisplaySleep` and
  `PreventUserIdleSystemSleep`). They disappear when it turns off.
- **The Mac slept anyway** — the lid was closed, Sleep was chosen, or the
  battery ran low; those override idle-sleep assertions, as with `caffeinate`.
- **No ⌘K commands or status-bar item** — keep awake is in the desktop app
  only, not in a browser tab pointed at `ottod`.
