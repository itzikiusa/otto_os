## Lens: Accessibility — iteration-5 re-score

**Score: 9.87/10.** Arithmetic: 10.00 − 0.10 (finding 2, Partial, minor) − 0.03 (finding N1, Open, nit) = 9.87.

Judgement: finding 4 is marked Fixed. The four suppressions left on those files are on delegated clicks inside rendered content, which I had already accepted as legitimate. If you read that as Partial, the score is 9.77.

| # | Item | Status | Evidence |
|---|---|---|---|
| 2 | `svelte-ignore a11y_*` count and ratchet (minor) | Partial | `grep -c "svelte-ignore a11y" ui/src` gives 101 across 65 files, down from 156. The ratchet is still in place (`ui-guards.mjs:101`, `:241`, `:488`). 101 is still well above the "under 60" I called for. The biggest clusters are QueryBuilder (6), TiledView (5), MultiRunDialog (5), DatabasePage (5), Navigator (4) and SplitNode (3). |
| 4 | Mouse-only click wrappers (minor) | Fixed, with a caveat | Terminal, Composer and FloatingBar no longer carry any a11y suppression. All three drive their option lists from the input with `aria-activedescendant` (`Terminal.svelte:2802`, `Composer.svelte:402`, `FloatingBar.svelte:939`). CanvasPanel is still a real button (`CanvasPanel.svelte:269-270`). NoteView:449, ReaderView:249, Markdown:183 and ToolStep:289 keep their suppressions for delegated clicks inside rendered content. |
| 5 | `outline-removed` guard had a file-wide escape (minor) | Fixed | `ui-guards.mjs:477-480` now checks each selector part with `hasRingFor(part)`. The old `fileHasFocusRing` is gone. |
| 6 | Palette hit pills were mouse-only spans (nit) | Fixed | `Palette.svelte:560` is now `<button type="button" class="pal-hit-btn" tabindex="-1">`, and both suppressions are gone. |
| N1 | Extra suppression on the workflow node wrapper (nit) | Open | `WorkflowCanvas.svelte:384` still has `svelte-ignore a11y_no_static_element_interactions` on `div.node` with `onpointerdown` (`:392`). |

**Spot-checks of items I had marked Fixed:**
- DatabasePage still uses `button role="tab"` with the roving tab stop.
- Workflow edge and node keyboard handling is intact.
- The Add join path is intact.

### New findings
None. I found no regressions from the changes.

### Remaining to reach 9.8
- Get the suppression count from 101 to under about 60. The easiest groups to clear are the `autofocus` ones (replace with `dialogFocus`), the `noninteractive_tabindex` ones (add `role="region"` and a label to scroll areas), and the QueryBuilder, TiledView and MultiRunDialog clusters.
- Move the pointer-down handler on the workflow node wrapper onto `button.node-main` so the `WorkflowCanvas.svelte:384` suppression can go (this is N1).
