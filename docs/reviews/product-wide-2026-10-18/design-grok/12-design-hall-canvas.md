# Design Hall and Canvas — UX review

Read of `ui/src/modules/design-hall/` and `ui/src/modules/canvas/` against `docs/design/guidelines/`. No diff, no Cargo, no npm, no screenshots. Light/dark judged from `ui/src/lib/tokens.css` and the module styles (semantic tokens, not a rendered pass). Phone ≤640 and tablet ≤1024 traced from the layout code.

| Module | Score | Defects |
|---|---|---|
| Design Hall | 8.4 | 7 |
| Canvas | 8.6 | 3 |
| **Together** | **8.5** | **10** |

Target 9.8 is not met. Lobby, brand kit, learning, artifact states, and the desktop boards are in good shape. The misses are controls that disappear or cannot be hit.

## Design Hall — 8.4

### 1. 3D scene column is gone on a normal laptop

`ui/src/modules/design-hall/studio3d/Studio3D.svelte:1089-1096`

Below a 980px studio container, `.left` is `display: none`. That column is the object hierarchy, environment preset, saved cameras, and the only GLB import (`importModel` anchors on `[data-testid="s3d-left"]`, line 373). Nothing else opens it. The artifact rail is 320px and the app sidebar is 220px, so the studio is under 980px until the window is about 1520px wide. A 1440px desktop, and a 14" MacBook, open a 3D design with no hierarchy and no way to import a model. The viewport click-select and the right-hand inspector do not replace those controls.

Fix: stop hiding the column with `display: none`. Below 980px, stack it above the viewport or put it behind a header toggle (same pattern as Canvas “Show scenes”), and keep Import / Environment / Views reachable. Do not use a breakpoint the center pane cannot clear while the details rail is open.

### 2. Site section toolbar clips off the stage

`ui/src/modules/design-hall/site/SiteCanvas.svelte:468-473` and `:684-711`

The toolbar is `white-space: nowrap` (Ask Otto, Variants, five icon buttons) and the overlay is `overflow: hidden` (line 642). Placement does `x = max(8, min(viewW - tbW - 8, …))`. When the bar is wider than the stage, `x` stays 8 and the right-hand actions (hide on mobile, More) are clipped. Buttons are bare `<button>`s at 26×26, so the coarse-pointer `::after` in `app.css` (which only grows `.icon-btn` / `.btn`) does not apply.

Fix: cap the bar at `viewW - 16`, wrap or move Ask Otto and Variants into More under 640px, and size the remaining buttons to at least `var(--hit-min)` on a coarse pointer.

### 3. On a narrow site, Blocks sits under the inspector

`ui/src/modules/design-hall/site/SiteStudio.svelte:870-876` and `:606-609`

Under a 699px studio the grid is canvas (fixed 460px), then the inspector, then Pages/Layers/Blocks. Adding a section means scrolling past the inspector. The “N to fix” chip calls `select(null, null)`, which only changes the inspector selection. It does not scroll `.insp` into view, so on a phone the chip appears to do nothing.

Fix: on that breakpoint use one segmented switch (Page / Blocks / Design) instead of three stacked panes. Point the checks chip at the Design pane and scroll it into view.

### 4. Two filled primaries on a site

`ui/src/modules/design-hall/ArtifactView.svelte:751` and `ui/src/modules/design-hall/site/SiteStudio.svelte:613`

Header Save and the studio Publish button are both `.btn.small.primary` on the same view. Publish already opens a sheet that names the target, the version, and who can open it. The second accent button fights Save.

Fix: drop `primary` from the studio Publish button. Leave Save as the one filled action; Publish stays a labeled menu that leads to the sheet.

### 5. Variant Compare and Reject are 20px and clipped

`ui/src/modules/design-hall/assist/VariantsTray.svelte:91-95`, `:96-103`, `:206-208`

The tray is `minmax(92px, 1fr)`, so a 360px Otto panel fits three cards. `.card` is `overflow: hidden`, which clips the coarse-pointer hit slop. `.acts .icon-btn` is `width: 20px`. Compare and Reject sit in that strip next to Apply. A miss taps Apply or nothing.

Fix: `minmax(160px, 1fr)`, clip only `.pic`, and do not set icon buttons below `var(--hit-min)`.

### 6. Spatial frames have no visible name

`ui/src/modules/design-hall/SpatialView.svelte:99-103` and `:292-298`

Each frame’s title is in `.sr` (1px clipped). `title=` shows on hover only. On a phone or tablet the wall is a row of unnamed thumbnails. The plaque names the project, not the design.

Fix: put one ellipsized line of `a.title` under each frame and on the plinth. Keep the screen-reader text.

### 7. Brand kit title is the crumb trail on a phone

`ui/src/modules/design-hall/brand/BrandEditor.svelte:487-493`

The header always renders “Design Hall / Brand Kit /” inside the nowrap `h1`, then the kit switcher. Artifact view clears crumbs on a phone and puts Back in `leading` (`ArtifactView.svelte:728-733`). Brand Kit does not, so the switcher ellipsizes and there is no back control that is not a crumb.

Fix: on `viewport.isPhone`, pass `crumbs={[]}`, put a Back button to `design/brand` in `leading`, and leave the switcher as the title.

Lobby, collection, learned, and brand-kit list already use `PageHeader`, `EmptyState` / `LoadState`, `Modal`, and `confirmer`. Publish states where, what, and who. Agent variants stay drafts until Apply. Those are not defects.

## Canvas — 8.6

### 1. Phone layout flag turns the board read-only

`ui/src/modules/canvas/CanvasPage.svelte:39-40` and `:268-272`

`readonly` is `viewport.isPhone`. The comment only describes collapsing the scene list. The same flag is passed into the editors: Excalidraw `viewModeEnabled` (`ExcalidrawCanvas.svelte:427`), and Mermaid/D2 hide Code (`MermaidCanvas.svelte:405`, `D2Canvas.svelte:408`). Empty Mermaid/D2 copy still says “open Code” (`MermaidCanvas.svelte:392`, `D2Canvas.svelte:395`). The hero still says you can draw by hand (`CanvasPage.svelte:305`). On a phone you can create a board, then cannot draw or edit, with no reason except a “Preview” chip on Mermaid and D2. Excalidraw has no such chip. Ask Otto still works.

Fix: split `isPhone` from `readonly`. Keep push navigation and the Back button. Pass `readonly` only when the scene is actually read-only. If preview-on-phone is intentional, say so on every format and change the empty copy so it does not mention Code.

### 2. Zoom controls stay 30px on touch

`ui/src/modules/canvas/MermaidCanvas.svelte:619-623` and `ui/src/modules/canvas/D2Canvas.svelte:630-634` (same rule)

Zoom, fit, download, and copy are bare buttons at `height: 30px` / `min-width: 30px`. They are not `.icon-btn` or `.btn`, so `app.css` does not grow them on a coarse pointer. They sit on the board corner on a phone, under the raised Ask Otto pill.

Fix: use `.icon-btn` (or set `min-width` / `height` to `var(--hit-min)` under a coarse pointer) and keep the bar inset so it does not meet Ask Otto.

### 3. Assistant header overflows while Otto is working

`ui/src/modules/canvas/ConversationPanel.svelte:215-221`

`.head` is a nowrap flex row: title, provider `<select>`, “Working…”, Stop, Restore, Close. On the phone overlay (`CanvasPage.svelte:399-407`, width 100%) that row is wider than the screen. The panel has no `min-width: 0` and no wrap, so the page scrolls sideways. Restore is `margin-inline-start: auto` (line 269), which does not wrap.

Fix: `flex-wrap: wrap` on `.head`, let the provider select shrink (`min-width: 0`), and keep Close on the first line.

Scene list, empty workspace, list error, open error, and save-failure alert are real states with Retry. Desktop list/detail remembers a scene and does not open on a blank. New scene is the only `.primary`. Excalidraw follows `ui.resolvedScheme`. Those are not defects.

## Not scored as defects

Brand hex values and canvas sticky/export fills are document colors, not chrome. Diagram pan surfaces may keep their own board. Uppercase eyebrows at `--fs-xs` (11px) are on the type floor. `outline: none` on search fields and the canvas composer is paired with a `:focus-within` or `:focus` ring.
