---
description: Use when creating or refining a 2D DESIGN for an Otto Product story from an agent session — the Design tab's Figma/Canva-style side — an Excalidraw BOARD (`design.excalidraw`: frames as artboards, wireframes, user flows, component sheets, moodboards) or an HTML SCREEN (`design.html`: a self-contained, script-free, high-fidelity UI screen shown in a device frame). Covers Excalidraw JSON authoring rules (document shape, element fields, frames, bound text, arrows with bindings, 8-pt grid, palette, reusable components), HTML screen rules (no network, no JS, realistic content, light/dark), and the quality bar for each.
category: development
version: 1
---

# Otto Design — 2D screens & boards — in-place agent

Otto's **Product → Design** tab hosts design artifacts on a story. For 2D work you
edit ONE file **in place** (the assist prompt names it and includes its current
contents); Otto previews every save live (`mockup_updated`) and commits the file
as a `kind:"design"` attachment when the turn ends:

| you're asked for | format | file you edit | renders as |
|---|---|---|---|
| a UI screen, dashboard, settings page, landing page, email | `html` | **`design.html`** | sandboxed iframe inside a device frame (iPhone / iPad / desktop), light or dark |
| a wireframe set, user flow, artboards, moodboard, component sheet, anything freeform | `excalidraw` | **`design.excalidraw`** | the Excalidraw board — the user keeps editing it by hand |
| a flow/sequence/ER/state diagram (strict, auto-laid-out) | `mermaid` | **`design.mmd`** | Mermaid (see `otto-mockup` for the rules) |

Write the WHOLE file each time. Keep it always-valid so every intermediate save
renders. Reply with ONE short sentence describing what you changed.

---

## A. Excalidraw boards — `design.excalidraw`

### Document shape

```json
{
  "type": "excalidraw",
  "version": 2,
  "source": "otto",
  "elements": [ /* … */ ],
  "appState": { "viewBackgroundColor": "#ffffff", "gridSize": 8 },
  "files": {}
}
```

`type`/`version`/`elements` are required. `files` stays `{}` — never embed
base64 images (the file has a size cap and the board would bloat); draw
placeholders instead. Do not put comments in the JSON.

### Element fields (write these; Excalidraw defaults the rest on load)

Every element:

```json
{ "id": "home-frame", "type": "rectangle", "x": 0, "y": 0, "width": 390, "height": 844,
  "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
  "fillStyle": "solid", "strokeWidth": 1, "strokeStyle": "solid", "roughness": 0,
  "opacity": 100, "groupIds": [], "frameId": null, "roundness": { "type": 3 },
  "seed": 1, "version": 1, "versionNonce": 1, "isDeleted": false,
  "boundElements": null, "updated": 1, "link": null, "locked": false }
```

- `type` ∈ `rectangle` | `ellipse` | `diamond` | `text` | `arrow` | `line` | `frame` | `freedraw`.
  Use `rectangle`/`ellipse`/`text`/`arrow`/`frame` — skip `freedraw`.
- `id`: unique, readable slug (`home-frame`, `home-cta`, `arrow-home-checkout`). Stable across edits.
- `roughness`: **`0`** (architect) for product wireframes/UI; `1` only for
  deliberately sketchy moodboards. `roundness: {"type": 3}` = rounded corners
  (cards, buttons); `null` = sharp (frames, tables, dividers).
- `fillStyle`: `"solid"`; `hachure`/`cross-hatch` read as sketch — avoid in UI wireframes.
- `strokeWidth` 1 for UI, 2 for emphasis/arrows; `strokeStyle` `"dashed"` for optional/future states.

**Text** adds: `"text"`, `"originalText"` (same string), `"fontSize"` (12/14/16/20/24/32/40),
`"fontFamily"` (**2** = Helvetica-like sans for UI; 3 = monospace for code; 1 = hand-drawn),
`"textAlign"` (`left`|`center`|`right`), `"verticalAlign"` (`top`|`middle`),
`"lineHeight": 1.25`, `"containerId": null`, and `width`/`height` roughly
`0.55 × fontSize × chars` / `1.25 × fontSize × lines` (Excalidraw re-measures on load).

**Bound label** (text centred inside a shape): give the shape
`"boundElements": [{ "id": "<textId>", "type": "text" }]` and the text
`"containerId": "<shapeId>"`, `textAlign: "center"`, `verticalAlign: "middle"`.
Excalidraw positions it on load — this is how buttons, cards with titles, and
flow nodes get labels.

**Arrow** adds: `"points": [[0, 0], [dx, dy]]` (relative to `x,y`; add mid-points
for elbows: `[[0,0],[120,0],[120,80]]`), `"startBinding"`/`"endBinding"`:
`{ "elementId": "<shapeId>", "focus": 0, "gap": 8 }` or `null`,
`"startArrowhead": null`, `"endArrowhead": "arrow"` (`"triangle"`, `"bar"`, `"dot"` also exist),
`"elbowed": false`. Place `x,y` on the source shape's edge and end the last point on
the target's edge — bindings keep it attached when the user drags things. Also list the
arrow in both shapes' `boundElements` as `{ "id": "<arrowId>", "type": "arrow" }`.

**Frame** (`type: "frame"`) adds `"name": "Home — iPhone 15"`. Children carry
`"frameId": "<frameId>"`. Frames clip and export as units — **one frame per
artboard/screen/state**. Nothing bound to a frame should sit outside its box.

### Layout rules (this is what makes a board look designed)

1. **8-pt grid** — every `x`, `y`, `width`, `height` a multiple of 8 (4 for icon
   nudges). `appState.gridSize: 8`.
2. **Artboards as frames**, laid out in a **row per flow, column per step**, with
   **80 px gutters** between frames and a `text` title (fontSize 20) 40 px above
   each row. Device sizes: iPhone 390×844, Android 412×915, iPad 820×1180,
   desktop 1440×900, email 600×(n). Add a 44-pt status bar strip and a 34-pt
   home-indicator area on phones.
3. **Type scale**: 32/24 titles, 20 section, 16 body, 14 secondary, 12 caption.
   Left-align body text; centre only labels inside shapes.
4. **Spacing**: 16 outer padding, 12 between cards, 8 inside components,
   48-pt touch targets (buttons `height: 48`), 56-pt list rows, 64-pt nav bars.
5. **Palette (default; obey the story's brand if given)** — ink `#1e1e1e`,
   secondary text `#6b7280`, hairline `#e5e7eb`, surface `#ffffff`, canvas
   `#f8fafc`, accent `#4f46e5` (white text on it), success `#16a34a`, warning
   `#f59e0b`, danger `#dc2626`, and flow-chart node fills `#e0e7ff` / `#dcfce7` /
   `#fef3c7` / `#fee2e2`. Max 1 accent + 1 neutral family per board.
6. **Components, not doodles**: build a button/input/card/nav/list-row/chip once
   as a `groupIds: ["cmp-button-primary"]` cluster and repeat it with the same
   sizes. Consistency beats variety. Icons: draw 24×24 ellipses/rectangles as
   placeholders or use a single glyph character (`☰ ⌕ ← → ✓ ✕ ★ ♥ ⚙ ⋯`) in a
   text element — never fake icons with many strokes.
7. **Realistic content**: real-looking names, prices, dates, counts, states
   ("3 of 12 shipped", "Visa •••• 4242"). No lorem ipsum, no "Label".
8. **Flows**: nodes 200×72 rounded rectangles with bound labels; decisions as
   `diamond` 200×120; arrows with **bindings** and a short bound label ("yes" /
   "no" / "timeout"); swimlanes as tall dashed rectangles behind them
   (`strokeStyle: "dashed"`, `opacity: 60`).
9. **Annotations**: red-ish (`#dc2626`) callout numbers in 24×24 ellipses with a
   matching numbered list in a `text` element beside the frame — reviewers read
   those.
10. **Keep the board small**: ≤ 400 elements. One artboard row per flow; open a
    second board (another design artifact) for another feature area.

### Editing in place

Preserve the user's hand edits: keep element ids, keep elements you weren't asked
to change, only append/modify what the request covers. Deleting is `isDeleted:
true` (or simply omit the element). If the file is already a full Excalidraw
document with fields you don't know, **copy them through unchanged**.

### Minimal worked example (two-screen flow)

```json
{ "type": "excalidraw", "version": 2, "source": "otto",
  "appState": { "viewBackgroundColor": "#f8fafc", "gridSize": 8 }, "files": {},
  "elements": [
    { "id": "row1-title", "type": "text", "x": 0, "y": -56, "width": 320, "height": 24, "text": "Checkout — happy path", "originalText": "Checkout — happy path", "fontSize": 20, "fontFamily": 2, "textAlign": "left", "verticalAlign": "top", "lineHeight": 1.25, "containerId": null, "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "transparent", "fillStyle": "solid", "strokeWidth": 1, "strokeStyle": "solid", "roughness": 0, "opacity": 100, "groupIds": [], "frameId": null, "roundness": null, "seed": 1, "version": 1, "versionNonce": 1, "isDeleted": false, "boundElements": null, "updated": 1, "link": null, "locked": false },
    { "id": "f-cart", "type": "frame", "name": "1 · Cart", "x": 0, "y": 0, "width": 390, "height": 844, "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "#ffffff", "fillStyle": "solid", "strokeWidth": 1, "strokeStyle": "solid", "roughness": 0, "opacity": 100, "groupIds": [], "frameId": null, "roundness": null, "seed": 2, "version": 1, "versionNonce": 1, "isDeleted": false, "boundElements": null, "updated": 1, "link": null, "locked": false },
    { "id": "cart-cta", "type": "rectangle", "x": 16, "y": 764, "width": 358, "height": 48, "angle": 0, "strokeColor": "#4f46e5", "backgroundColor": "#4f46e5", "fillStyle": "solid", "strokeWidth": 1, "strokeStyle": "solid", "roughness": 0, "opacity": 100, "groupIds": ["cmp-button-primary"], "frameId": "f-cart", "roundness": { "type": 3 }, "seed": 3, "version": 1, "versionNonce": 1, "isDeleted": false, "boundElements": [{ "id": "cart-cta-label", "type": "text" }, { "id": "a-cart-pay", "type": "arrow" }], "updated": 1, "link": null, "locked": false },
    { "id": "cart-cta-label", "type": "text", "x": 120, "y": 778, "width": 150, "height": 20, "text": "Pay $148.00", "originalText": "Pay $148.00", "fontSize": 16, "fontFamily": 2, "textAlign": "center", "verticalAlign": "middle", "lineHeight": 1.25, "containerId": "cart-cta", "angle": 0, "strokeColor": "#ffffff", "backgroundColor": "transparent", "fillStyle": "solid", "strokeWidth": 1, "strokeStyle": "solid", "roughness": 0, "opacity": 100, "groupIds": ["cmp-button-primary"], "frameId": "f-cart", "roundness": null, "seed": 4, "version": 1, "versionNonce": 1, "isDeleted": false, "boundElements": null, "updated": 1, "link": null, "locked": false },
    { "id": "f-pay", "type": "frame", "name": "2 · Payment", "x": 470, "y": 0, "width": 390, "height": 844, "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "#ffffff", "fillStyle": "solid", "strokeWidth": 1, "strokeStyle": "solid", "roughness": 0, "opacity": 100, "groupIds": [], "frameId": null, "roundness": null, "seed": 5, "version": 1, "versionNonce": 1, "isDeleted": false, "boundElements": [{ "id": "a-cart-pay", "type": "arrow" }], "updated": 1, "link": null, "locked": false },
    { "id": "a-cart-pay", "type": "arrow", "x": 374, "y": 788, "width": 96, "height": 0, "points": [[0, 0], [96, 0]], "startBinding": { "elementId": "cart-cta", "focus": 0, "gap": 8 }, "endBinding": { "elementId": "f-pay", "focus": 0, "gap": 8 }, "startArrowhead": null, "endArrowhead": "arrow", "elbowed": false, "angle": 0, "strokeColor": "#6b7280", "backgroundColor": "transparent", "fillStyle": "solid", "strokeWidth": 2, "strokeStyle": "solid", "roughness": 0, "opacity": 100, "groupIds": [], "frameId": null, "roundness": { "type": 2 }, "seed": 6, "version": 1, "versionNonce": 1, "isDeleted": false, "boundElements": null, "updated": 1, "link": null, "locked": false }
  ] }
```

---

## B. HTML screens — `design.html`

One complete, self-contained document rendered in a **sandboxed iframe with
scripts disabled**, inside the arena's device frame with a light/dark toggle.

Rules:
- `<!doctype html>`, `<meta charset="utf-8">`, `<meta name="viewport" content="width=device-width, initial-scale=1">`, a `<title>`.
- **All CSS inline** in one `<style>`. **Zero network**: no `<link>`, no web fonts,
  no remote images/scripts/iframes. `system-ui` fonts, CSS gradients/shapes,
  emoji, and inline `<svg>` for icons and illustrations.
- **No JavaScript** runs. Convey state visually: the active tab, a filled form,
  a selected row, a hover-looking primary button, an open dropdown drawn as a
  static list, a toast already on screen.
- **Fit the frame**: design mobile-first at 390 px wide and let it stretch to
  desktop with flex/grid; never rely on a fixed body width. `body { margin: 0 }`.
  Avoid horizontal scroll.
- **Light + dark**: define tokens on `:root` and override them under
  `@media (prefers-color-scheme: dark)` (the device frame flips the scheme).
  `color-scheme: light dark` on `:root`.
- **Realistic content** and a **small cohesive palette** (see the Excalidraw
  palette above), a clear type scale (28/20/16/14/12), 8-pt spacing, rounded
  cards with hairline borders, one accent. It should look like a screenshot of a
  shipped product, not a wireframe.
- Keep it under ~60 KB. One screen per file; another screen = another artifact
  (the user can ask for "Checkout — step 2" as a new one).

Skeleton:

```html
<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Orders — Acme Admin</title>
<style>
  :root { color-scheme: light dark;
    --bg:#f8fafc; --card:#fff; --line:#e5e7eb; --ink:#1e1e1e; --dim:#6b7280; --accent:#4f46e5; }
  @media (prefers-color-scheme: dark) {
    :root { --bg:#0f172a; --card:#1e293b; --line:#334155; --ink:#f1f5f9; --dim:#94a3b8; } }
  body { margin:0; font:15px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif; background:var(--bg); color:var(--ink); }
  /* header, nav, cards, table, buttons … */
</style></head>
<body><!-- realistic UI here --></body></html>
```

## Related

- 3D scenes (`scene.json`) → skill **`otto-design-3d`**.
- The Design-tab mechanics (formats, files, commit, live preview) → skill **`otto-mockup`**.
