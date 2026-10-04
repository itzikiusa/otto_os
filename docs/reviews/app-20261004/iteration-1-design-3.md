# Iteration 1 — visual design partition 3

**Verdict: Request changes.** Blocker 0 · major 2 · minor 2 · nit 0.

**Scope:** Vault, Canvas, Design Hall, Product, Browser, Snip. Focused, read-only source review of current surfaces in `/Users/itziklavon/claude_ade-review`; line references below refer to that worktree. Read `AGENTS.md`, the design-guidelines README/checklist, partition-3 correctness/performance reports, and `/tmp/otto-app-review-coordination-20261004.txt`. Compared the separate design worktree's changes from `a16f4c71` through `372a8944`, including the exact diffs for each reported component. These findings remain uncovered there. Forward visual implementation to that effort; no competing source changes were made.

**Evidence limit:** Source inspection and explicit interaction/layout traces only. No screenshots, app execution, builds, tests, servers, or external writes. Proposed verification below has not been executed. No measured colour-contrast or complete responsive-compliance claim is made.

## D3-01 — Major: Creating a mockup annotation requires a pointer

**Locations:** `ui/src/modules/product/MockupAnnotations.svelte:90`, `:160`; mounted by `ui/src/modules/product/MockupViewer.svelte:222`.

**Evidence:** `pending` is created only by `onOverlayClick`, which derives the pin coordinates from a `MouseEvent`. Its only creation trigger is a `<div role="presentation" onclick={onOverlayClick}>` with no focus target or keyboard handler. The Annotate tab merely sets `mode`; there is no Add annotation button or alternate coordinate entry. The textarea and Add button mount only after `pending` exists. Tab navigation can reach the mode controls and existing-note actions, but cannot start a new note.

**Impact:** Keyboard-only users cannot perform the feature's primary authoring action, even though they can enter annotation mode. This is separate from the reported asynchronous Product data-ownership defects.

**Smallest fix:** Add a labelled, keyboard-operable Add annotation control that places a pin at a visible default position, with keyboard movement of the pending pin (or labelled position inputs). Give the editor a labelled textarea, Escape cancellation and focus restoration to the creation control. Reuse the existing save path and normalized coordinates.

**Verification:** With only Tab, Enter, arrows and Escape, open a mockup, create and reposition a pin, enter text and save; then start a second pin and cancel. Verify the saved position/body, visible focus, and returned focus. Check both light and dark.

## D3-02 — Minor: The mockup annotation editor extends outside its scroll container near an edge

**Locations:** `ui/src/modules/product/MockupAnnotations.svelte:189`, `:316`; `ui/src/modules/product/MockupViewer.svelte:289`.

**Evidence:** The editor's `left` and `top` are copied directly from the pin percentage. CSS gives it a fixed 220 px width and `translate(-50%, 14px)`; no placement clamp, available-height cap, or boundary measurement is present. Its containing `.render-wrap` has `overflow: auto` and 12 px padding. For an LTR pin 5 px inside the render box's left edge, the editor's left is approximately `12 + 5 - 110 = -93 px` relative to the scroll container, before any additional border/padding width. The negative portion is clipped on the start side; scrolling toward positive offsets does not reveal it. The right edge likewise creates overflow rather than repositioning the form. This follows from the layout declarations; it is not a screenshot observation.

**Impact:** Notes on the left edge of a design open with part of the text field obscured. The user must choose a different annotation location or work with a clipped editor. The fixed placement becomes more restrictive on phones and at enlarged text sizes.

**Smallest fix:** Keep the pin coordinates unchanged but position the editor independently. Clamp its measured rectangle to the visible render region/viewport with an inset, cap width and height to available space, and allow internal vertical scrolling. Use an above-pin position only when it fits, with a final clamp.

**Verification:** At desktop and phone widths, place pins at all four corners and enlarge the textarea. Assert the editor and both actions with `expectFullyInViewport`; also assert they fit the actual scroll container's visible rectangle. Repeat RTL and enlarged text, without requiring horizontal scrolling to edit a note.

## D3-03 — Major: Mermaid and D2 previews cannot be panned with the keyboard

**Locations:** `ui/src/modules/canvas/D2Canvas.svelte:188`, `:350`; `ui/src/modules/canvas/MermaidCanvas.svelte:179`, `:347`.

**Evidence:** Both previews expose `role="application"` on their diagram surface, but the surface has no `tabindex` or key handler. Translation changes through pointer dragging and wheel events; the real toolbar buttons offer zoom, Fit and export but no directional movement. The surface clips overflow, so native keyboard scrolling cannot move the content. After using the reachable Zoom in button on a wide diagram, off-centre nodes are clipped and only a pointer/wheel can bring them into view at that readable scale. The parent CanvasPage has no alternate keyboard pan handler.

**Impact:** Keyboard-only users can enlarge a diagram but cannot inspect its outer parts at that zoom. Fit is an available workaround for seeing its overall shape, not for reading labels on a large diagram.

**Smallest fix:** Make the preview focusable with a visible focus ring, document arrow-key panning and implement it while the surface owns focus. Keep code-editor keys unaffected; set `userAdjusted` consistently with pointer panning. Provide a labelled Fit shortcut or retain the existing Fit button. Factor the shared handler if useful for keeping D2 and Mermaid equivalent.

**Verification:** Load a diagram wider and taller than the preview, focus the board by keyboard, zoom in, and pan to each corner without a mouse; assert translation changes and formerly clipped labels enter the visible area. Confirm arrows in Code still edit/navigate text and Fit restores the complete diagram. Repeat readonly preview.

## D3-04 — Minor: Product import search displays request failures as empty results

**Locations:** `ui/src/modules/product/SourceSearch.svelte:84`, `:98`, `:185`, `:286`; used by `ui/src/modules/product/ImportDialog.svelte:184`.

**Evidence:** Project/space loading catches discard the failure and render selectors containing only All projects/All spaces. A failed non-append search emits a temporary toast and clears both result arrays. Once `searching` becomes false, the template renders “No results found.” for a nonempty query, with no persistent error or Retry control. Thus a rejected request and a successful empty result converge to the same visible state. The separate design branch changes only loading copy and a radius token here.

**Impact:** An account or temporary service failure looks like the requested issue/page does not exist; users lose the immediate cause and recovery action once the toast disappears. This violates the documented four-state data-region pattern.

**Smallest fix:** Retain separate filter-load/search errors and render a compact inline error with Retry for the exact failed request. Show the empty-results message only after a successful search. Preserve current query/filter context and existing results when a Load more request fails.

**Verification:** Stub project/space failure, initial search failure and Load more failure independently. Confirm each has a persistent error/retry path, Retry retains the query/filter, and a successful empty response alone shows No results found. Do not require closing and reopening Import to recover.

## Coverage and exclusions

- **Vault:** Sampled main layout, link previews, recovery, switcher and existing design changes. Did not duplicate Claude's tabs/resizers, graph work or refine-attribution changes. No additional ranked Vault finding from this pass.
- **Canvas:** Sampled main/list states and Mermaid/D2 interaction/compact layouts. Excluded an unclamped shape-menu suspicion after finding it in the old tool-rail surface rather than establishing a current-page entry point.
- **Design Hall:** Sampled ArtifactView, SiteStudio/SiteCanvas, brand controls and tab handling. Existing compact details navigation and narrow site layout are present. No additional ranked finding; full site/3D studios remain outside this finite sample.
- **Product:** Sampled import search, mockup viewing/annotations and main-page coordination changes. Draft-loss and delayed-response ownership findings remain with correctness partition 3.
- **Browser:** Sampled main/tab controls, notes and design-branch changes. Excluded CredentialsPanel's load-error suspicion because no current mount was found. No ranked Browser finding.
- **Snip:** Sampled toolbar, loading/error states, keyboard annotation controls and phone rules. Existing keyboard insertion/help and responsive tool grid are present. Explicit Copy behavior remains with correctness partition 3.

Only this report was authored. Runtime/light-dark/phone evidence is still required before claiming the proposed fixes are verified.
