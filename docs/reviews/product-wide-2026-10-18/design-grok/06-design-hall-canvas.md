# Design Hall and Canvas — design/UX review

Reviewer: grok (design-hall + canvas). Source-traced only. No Cargo, npm, browser, or screenshots, so contrast in all five themes and real overflow at 390/768/1440 are unverified. Chrome was checked against `docs/design/guidelines/` and `ui/src/lib/tokens.css`. Brand hex and `site/engine/site.css` are the artifact’s own theme, not Otto chrome. Excalidraw’s board is a third-party surface.

Inspected: lobby (search, hero, studios, continue, projects, linked, rail, spatial), artifact view (load / gone / error / read-only / dirty / newer-head / imported), Otto / links / references, version strip, compare, site studio + publish sheet, 3D studio (header only), brand kit (empty, redirect, editor, impact), learning log (pending, rules, memory, signals, settings, off), canvas scene list (load, error, empty, filter, sections), Mermaid, D2, Excalidraw host, assistant panel. Phone ≤640 and tablet ≤1024 via `viewport` and the container queries in these files.

## Scores

| Surface | Score | Why it is not 9.8 |
|---|---|---|
| Design Hall overall | **8.6** | Phone sub-page chrome and two filled primaries on the main screens. States, drafts, and confirmations are in good shape. |
| Lobby | 8.9 | Search, empty, and error exist. New and Generate are both primary. Search failure prints the raw error. |
| Artifact view | 8.4 | Phone title is eaten by badges. Version chips are 24 px and scroll the wrong way in RTL. Site Publish is a second primary. |
| Brand kit | 8.6 | No phone back. Editor is a centered island. Preset swatches are 24 px. |
| Learning log | 8.8 | No phone back. Memory filter is `dir="ltr"`. Header and every Accept are primary. |
| Canvas overall | **8.2** | Phone opens the board read-only and unnamed. |
| Scene list | 9.0 | Load, error, empty, and filter are real. Filter field is LTR-only. |
| Mermaid | 8.1 | Same phone lock. Empty copy tells you to open Code, which is hidden. Zoom targets are 30 px and there is no pinch. |
| D2 | 8.1 | Same as Mermaid, plus Sketch hidden on the phone. |
| Excalidraw | 8.3 | Editor chrome is Excalidraw’s. The host forces `viewMode` on the phone. |

**Defects: 16.** No nits below.

## Defects

1. **Phone canvas is view-only, and the empty state lies.** `ui/src/modules/canvas/CanvasPage.svelte:40` sets `readonly` from `viewport.isPhone` and passes it into every editor (`:268`). Excalidraw turns on `viewModeEnabled` (`ExcalidrawCanvas.svelte:427`). Mermaid and D2 hide Code and Sketch (`MermaidCanvas.svelte:405`, `D2Canvas.svelte:408`). The empty copy still says “open Code” (`MermaidCanvas.svelte:392`, `D2Canvas.svelte:395`). A phone user cannot draw or edit source, and the empty board tells them to press a control that is not there. Ask Otto still works. **Fix:** use `isPhone` only to hide the list. Do not pass it as `readonly`. If Excalidraw’s toolbar cannot fit, keep view mode for that editor only, and still show Code on Mermaid and D2. Change the empty sentence when Code is actually hidden.

2. **Open scene has no name on the phone.** `CanvasPage.svelte:174` always titles the page “Canvas”. The list is `display: none` once a scene is open (`:236`). **Fix:** `title={canvas.scene?.title ?? 'Canvas'}` while a scene is open. Keep the Design Hall crumb.

3. **Diagram zoom is a 30 px target and pinch does nothing.** `MermaidCanvas.svelte:525` sets `touch-action: none`. Zoom buttons are 30×30 (`:626`) and are not `.icon-btn`, so the coarse-pointer hit slop in `app.css` does not apply. Pointer handlers (`:212`) track one pointer, so pinch never zooms. D2 matches (`D2Canvas.svelte:525`, `:626`, pointer handlers beside `:380`). **Fix:** make the buttons `.icon-btn` with `min-block-size: var(--hit-min)` under a coarse pointer, and scale from a second pointer.

4. **Scene and memory filters force LTR.** `SceneList.svelte:138` and `LearnedPage.svelte:402` use `dir="ltr"` on fields that hold human titles. A Hebrew scene name or memory types left-to-right. **Fix:** `dir="auto"`. Keep `dir="ltr"` on hex, URLs, slugs, and source.

5. **Lobby shows two primary buttons.** Header New is `.btn.primary` (`Lobby.svelte:281`) and the hero Generate is `.btn.primary` (`:369`) on the same first screen. **Fix:** while the hero is visible, New is `.btn` (it already opens a menu). Generate stays the one primary. When search replaces the hero, New can be primary again.

6. **Search failure shows the exception.** `Lobby.svelte:230` stores `e.message`, and `:306` prints it next to “Search failed.” **Fix:** one sentence: “Search failed. Check the connection and try again.” Put the raw message on the alert’s `title` only.

7. **Lobby controls are under 36 px on touch.** Header search is 27 px tall (`Lobby.svelte:587`); phone moves it to 36 px (`:598`) but a tablet keeps 27 px. Studio, project, and variant selects are 24 px (`:674`). **Fix:** under `@media (pointer: coarse)`, set those controls to `min-block-size: var(--hit-min)`.

8. **Artifact title disappears on a phone.** The badge slot (`ArtifactView.svelte:736`) is `flex-shrink: 0` inside a nowrap title (`PageHeader.svelte:525`). Studio, status, version, and Edited sit beside Back, Save, and More. The stage toolbar already names the studio (`ArtifactView.svelte:842`). **Fix:** on `viewport.isPhone`, badge is the status pill only.

9. **Version chips are 24 px and the strip pins the wrong end in RTL.** Chip height is fixed at 24 px (`VersionStrip.svelte:136`) inside a 40 px bar (`:105`). They are not in the coarse-pointer slop list. `:33` sets `scrollLeft = scrollWidth`, which in WebKit RTL leaves the current version off-screen. **Fix:** at `max-width: 640px`, `height: auto` on the strip and `min-block-size: var(--hit-min)` on `.vchip`. Scroll the last chip with `scrollIntoView({ inline: 'nearest', block: 'nearest' })`.

10. **Brand Kit and What Otto learned have no phone back.** `BrandEditor.svelte:487` (also the error headers at `:472`, `:477`, `:482`) and `LearnedPage.svelte:323` put “Design Hall / …” in the `h1`. On a phone that line ellipsizes (`PageHeader.svelte:623`) and the crumb is not a reliable hit target. Artifact view and collection view already put a back button in `leading` and clear crumbs. **Fix:** the same leading control, `aria-label` “Back to Design Hall”, and `crumbs={viewport.isPhone ? [] : crumbs}`.

11. **Brand editor is a centered island.** `BrandEditor.svelte:621` caps the grid at 1360 px and centers it with `margin: 0 auto`, so the form does not share the header title’s inline start. **Fix:** delete the auto margin. Let `PageBody` align it.

12. **A failed kit open names the storage format.** `BrandEditor.svelte:484` says “Only otto-brand documents open in the Brand Kit editor.” **Fix:** “This design isn’t a brand kit. Open it in its studio instead.” The button already routes to the artifact.

13. **Color presets are 24 px and the name does not match the tooltip.** `ColorsSection.svelte:145` uses `aria-label="Set {name} to {n} {h}"` and `title="{n} {h}"`. `.pchip` is 24×24 (`:335`) and is not a `.chip`, so it gets no hit slop. **Fix:** box of 36 px (padding, not a larger glyph). `title` and `aria-label` both “Set {token} to {name}”.

14. **Site Publish is a second primary next to Save.** `SiteStudio.svelte:613` is `.btn.primary` while `ArtifactView.svelte:751` Save is also `.btn.primary` on the same view. Publish already confirms in `PublishModal.svelte`. **Fix:** Publish is `.btn`. Save stays the only filled button.

15. **Pending rules add more primaries.** `LearnedPage.svelte:341` “Look for rules now” is primary, and every card’s Accept is primary (`:476`). **Fix:** the header action is `.btn`. Accept stays the card primary.

16. **Send, Undo, and Redo put the shortcut in the name, and the pairs disagree.** `OttoPanel.svelte:639` is `aria-label="Send to Otto (⌘Enter)"` and `title="Send (⌘Enter)"`. `ConversationPanel.svelte:200` is `aria-label="Send"` and `title="Send (Enter)"`. `SiteStudio.svelte:604` is `aria-label="Undo"` and `title="Undo (⌘Z)"` (Redo on the next line). **Fix:** `aria-label` and `title` share the verb only. Put the chord on `aria-keyshortcuts` (`Meta+Enter`, `Meta+z`, `Meta+Shift+z`).

## What held up

Lobby, artifact, brand, and learning log all have loading, empty, error, and retry. Agent output stays a draft (variants, Accept/Reject, publish sheet names host and who can open it). Confirms go through `confirmer`, menus through `ctxMenu`. Canvas list/detail restores the last scene off the phone, and the phone list is the first screen. Mermaid and D2 pasteboards use tokens and a `:focus-visible` ring on the pan surface. Reduced motion is overridden on the lobby studios, canvas mode cards, and spatial frames.
