# Snipping Tool

One-gesture screenshots for agent work: press a shortcut, drag a region (the
native macOS crosshair), annotate in Otto's editor — and at **every** step the
image is **already on your clipboard**, so you can paste it straight into an
agent session (Claude Code accepts clipboard-image paste with `Ctrl+V`). It
replaces the two-tool loop of ⌘⌃⇧4 → separate editor → re-copy.

The daemon owns the whole pipeline (capture, storage, clipboard), so the flow
works identically from the desktop app and from a plain browser tab pointed at
`ottod`; the Tauri shell adds the system-wide shortcut and a dedicated editor
window.

## Setup

- **Nothing to install.** The capture uses `/usr/sbin/screencapture`; clipboard
  writes use `osascript`. macOS only.
- **Screen Recording permission (one-time).** The first real capture makes
  macOS prompt to grant **Screen Recording** to **`ottod`** (the capture runs in
  the daemon). Grant it under System Settings → Privacy & Security → Screen
  Recording, then capture again. Until granted, `POST /snips/capture` returns a
  500 with this exact instruction and the UI shows it as a toast.
- The global shortcut requires the desktop app to be **running** (foreground
  not required); it is registered by the Tauri shell at launch.

## Walkthrough

1. **Trigger** — any of:
   - **⌘⌃⇧2** system-wide (default; works while Chrome or anything else is
     frontmost — change or disable it in Settings → Snipping),
   - **⌘⇧S** inside Otto (works in the browser too),
   - ⌘K → “Take screenshot (snip)”, or File → **Take Snip**,
   - Settings → Snipping → **Take a snip now**.
2. **Select** — the native interactive capture starts: drag a region, press
   **Space** to toggle window-capture mode, **Esc** to cancel (cancel is silent,
   like the native tool). The daemon holds the HTTP request open while the
   crosshair is on screen (up to 120 s).
3. **Captured → clipboard (automatically).** The moment the selection lands,
   the original PNG is on the pasteboard — if all you wanted was a plain
   screenshot, paste it now.
4. **Annotate** — the editor opens (its own window in the desktop app,
   `#/snip/{id}` in a browser): rectangle, ellipse, arrow, line, freehand pen,
   highlighter, **text** (click to type, double-click to re-edit), **pixelate**
   (privacy redaction), numbered **step badges**; 8 colors, S/M/L stroke & font
   sizes; select/move/resize/delete; undo/redo (⌘Z / ⇧⌘Z); arrow-key nudge
   (⇧ = 10 px).
5. **Every edit re-copies (automatically).** 800 ms after your last change the
   flattened PNG replaces the clipboard (“Copied ✓” in the toolbar). ⌘C or the
   **Copy** button forces it immediately. Paste into your session whenever
   you're ready — the newest state is always what pastes.
6. **Done** — Close the window. Snips are pruned after 14 days (Delete removes
   one immediately).

You can also annotate an **existing** image: `POST /api/v1/snips` with
`{data_b64}` (PNG), then open `#/snip/{id}`.

## API surface

See the authoritative table in [`docs/contracts/api.md`](../contracts/api.md)
(§ Snips). Summary: `POST /snips/capture`, `POST /snips` (upload), `GET /snips`,
`GET /snips/{id}/image`, `GET+POST /snips/{id}/annotated`,
`POST /snips/{id}/copy`, `DELETE /snips/{id}`. Feature gate: **Agents**
(GET = View, mutations = Edit). No WS events. Storage:
`<data_dir>/snips/{id}.png` + `{id}.annotated.png` + `{id}.json` (no SQLite);
the bytes last placed on the clipboard are mirrored to
`<data_dir>/snips/clipboard-last.png`.

## Capabilities & limitations

- **macOS only** — capture and clipboard shell out to `screencapture` /
  `osascript`. On another OS the endpoints exist but capture fails.
- The global shortcut fires only while the **desktop app is running**; the
  browser-only setup still has ⌘⇧S / palette / API triggers.
- Editor state is **in-memory per editing session**: reopening a snip shows the
  original image plus a fresh canvas (the flattened PNG is the durable
  artifact and stays on `GET /snips/{id}/annotated`).
- One interactive capture at a time (a concurrent request gets a 409).
- Clipboard writes are best-effort: on failure the snip is still saved and the
  response carries `copied:false` (the editor chip shows “Copy failed”).
- Default chord **⌘⌃⇧2**: macOS reserves ⌘⌃⇧3/4/5/6 for the system screenshot
  service — Otto deliberately does not try to shadow those.
- Under `OTTO_E2E=1` the real pasteboard is never touched (file sink only) —
  that's how the Playwright spec (`ui/e2e/desktop-snip.spec.ts`) asserts
  clipboard content byte-exactly; capture plumbing is covered by
  `crates/otto-server/tests/snips.rs` via the `OTTO_SNIP_CAPTURE_CMD` seam.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| Toast: “screen capture was blocked by macOS…” | Grant Screen Recording to `ottod` (System Settings → Privacy & Security), retry. After a daemon **update** macOS may re-prompt. |
| Global shortcut does nothing | Otto.app not running, chord disabled/rebound (Settings → Snipping), or another app grabbed it — pick a different chord there (registration errors surface inline). |
| “A screen capture is already in progress” | A previous crosshair is still on screen (or within its 120 s window). Finish or Esc it. |
| Editor says “This snip no longer exists.” | The snip was deleted or pruned (14 days) — e.g. a restored editor window from a previous app run. Close it. |
| Paste shows the un-annotated capture | Paste happened inside the 800 ms debounce — wait for “Copied ✓” or hit ⌘C first. |
