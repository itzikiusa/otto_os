---
id: snip
title: Snipping tool
group: Basics
route: settings/snipping
summary: Capture a screen region, annotate it, and paste it straight into an agent session — the image is on your clipboard at every step.
---
## What it's for

The snipping tool turns "screenshot → annotate → paste into the agent" into one
gesture. You press a shortcut, drag a region with the native macOS crosshair,
and Otto's annotation editor opens. The capture is already on your clipboard
the moment you release the mouse, and every edit you make re-copies the
annotated image, so whatever you paste is always the latest version.

Otto's daemon does the capture, storage and clipboard work, so the flow is the
same in the desktop app and in a browser tab pointed at Otto. The desktop app
adds the system-wide shortcut and opens the editor in its own window.

## Getting started

1. Press **⌘⌃⇧2** anywhere on your Mac while Otto is running (desktop app), or
   **⌘⇧S** inside Otto.
2. The first time, macOS asks you to allow **Screen Recording** for `ottod`.
   Grant it in System Settings → Privacy & Security → Screen Recording, then
   capture again.
3. Drag a region. Press **Space** to switch to capturing a whole window, or
   **Esc** to cancel (cancelling is silent).
4. The editor opens with the capture already copied. Paste now if a plain
   screenshot is all you need.
5. Pick a tool, colour and size, and annotate. About 0.8 seconds after your
   last change the toolbar shows **Copied ✓** — paste the result into your
   agent session.
6. Choose **Close** when you're done, or **Delete** to remove the snip now.

## Everything it can do

**Ways to start a capture**
- The global shortcut, **⌘⌃⇧2** by default (desktop app only; works while any
  other app is in front).
- **⌘⇧S** anywhere in Otto, including the browser.
- ⌘K → **Take screenshot (snip)**.
- The desktop app's menu: **Take Snip**.
- **Settings → Snipping → Take a snip now**.

**Annotation tools** (toolbar, left to right)
- **Select** — click an annotation to select it, drag to move it, drag its
  handles to resize.
- **Box** (rectangle) and **Ellipse**.
- **Arrow** and **Line**.
- **Pen** — freehand drawing.
- **Mark** — a translucent highlighter.
- **Text** — click to place a text box and type; double-click existing text to
  edit it. The text commits when you click away or press ⌘↩.
- **Blur** — pixelates a region to hide private details.
- **Step** — numbered badges (1, 2, 3…) for step-by-step callouts.

**Style**
- 8 colours: red (default), orange, yellow, green, blue, purple, near-black and
  white.
- 3 sizes, **S / M / L**, which set both the stroke width and the text size.

**Editing**
- Undo and redo for every change, including moves and resizes.
- Nudge a selected annotation with the arrow keys (hold ⇧ for 10 px steps).
- Delete a selected annotation with Delete or Backspace.

**Clipboard**
- The original capture is copied as soon as it lands.
- Every change schedules a re-copy of the flattened, annotated PNG; the status
  chip reads **Copying…**, **Copied ✓** or **Copy failed**.
- **Copy** (or ⌘C with nothing selected) copies immediately.

**Snips on disk**
- Each snip is saved by the daemon with its original and annotated PNGs.
- **Delete** in the editor removes the snip immediately; otherwise snips are
  removed automatically after 14 days.
- Pasting an image into an Otto terminal session also stores it as a snip and
  types the file path into the terminal, so agent CLIs can read it even when
  you drive Otto from another machine.

**Settings → Snipping** (desktop app)
- See the current global shortcut, **Change** it by pressing a new chord
  (it needs at least one modifier; Esc cancels recording), **Reset** it to
  ⌘⌃⇧2, or **Disable** it.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| ⌘⌃⇧2 | Take a snip from anywhere on your Mac (desktop app, default; change it in Settings → Snipping) |
| ⌘⇧S | Take a snip from inside Otto |
| Space | While selecting: toggle window capture |
| Esc | While selecting: cancel the capture |
| ⌘Z | Undo |
| ⇧⌘Z | Redo |
| ⌘C | Copy the annotated image now (when no annotation is selected) |
| Delete or Backspace | Delete the selected annotation |
| ←  →  ↑  ↓ | Nudge the selected annotation 1 px |
| ⇧ + arrow key | Nudge the selected annotation 10 px |
| Esc | Deselect; while typing text, discard the text box |
| ⌘↩ | Commit the text you're typing |

See [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts) for Otto's global
shortcuts.

## Tips and limits

- **macOS only.** Capture uses the system `screencapture` tool, and clipboard
  writes use macOS scripting.
- **Screen Recording permission** is needed once for `ottod`. After a daemon
  update macOS may ask again. Until it is granted, capture fails with a
  message telling you what to do.
- **The global shortcut needs the desktop app running** (it doesn't need to be
  in front). In a plain browser, use ⌘⇧S, ⌘K or Settings → Snipping.
- If the global shortcut does nothing, another app may own that chord — pick a
  different one in Settings → Snipping; registration errors show inline there.
  Otto doesn't use ⌘⌃⇧3–6 because macOS reserves them for its own screenshot
  tools.
- **One capture at a time.** Starting another while the crosshair is up shows
  "A screen capture is already in progress"; finish or cancel the first one.
  A capture waits at most 120 seconds for your selection.
- **Pasted the un-annotated version?** You pasted before the re-copy finished.
  Wait for **Copied ✓**, or press ⌘C first.
- **Annotations are not editable later.** Reopening a snip shows the original
  image with a fresh canvas; the annotated PNG you copied is the lasting result.
- If the editor says "This snip no longer exists", it was deleted or passed the
  14-day limit. Close the window.
- Clipboard writes are best effort: if one fails, the snip is still saved and
  the chip shows **Copy failed** — use **Copy** to try again.
- Images must be PNG and at most 25 MB.
- Snipping uses the **Agents** permission: viewing needs Agents view, and
  capturing, annotating or deleting needs Agents edit.
- The toolbar tooltips show single-letter hints (V, R, O…), but those letter
  keys are not wired up yet — pick tools with the mouse.

## Related

- [Agents](#/walkthroughs/agents)
- [Settings](#/walkthroughs/settings)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
