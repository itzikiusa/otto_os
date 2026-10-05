## Lens: Accessibility — re-score

**Score: 9.6/10.** Arithmetic: 10.0 − 0.1 (finding 2, Partial, minor) − 0.1 (finding 4, Partial, minor) − 0.1 (finding 5, Open, minor) − 0.03 (finding 6, Open, nit) − 0.03 (new nit N1) = 9.64, rounded to 9.6. The one major from last time is fixed.

| # | Previous finding | Status | Evidence |
|---|---|---|---|
| 1 | Canvas editors pointer-only (major) | Fixed | `WorkflowCanvas.svelte:160-175` has a "Connect…" menu of target nodes, wired from the output port button at `:432-440` (click opens the menu). Edges are focusable with a label and Enter/Delete handling (`:214-222`, `:357-358`, hint at `:455`). Nodes take arrow-key moves and Delete (`:397`, hint `:454`). `QueryBuilder.svelte:677` adds the "Add join…" keyboard path (button `:1033`, Modal `:1282`). The remaining pointer-only handles at `:1001` and `:1013` are now just the mouse shortcut. |
| 2 | 160 suppressions and no ratchet (minor) | Partial | The ratchet exists: `ui-guards.mjs:431` has an `a11y-ignore` rule, with its message at `:227`. The count is only down from 160 to 156 in `ui/src` (158 counted across `ui/`, which includes the 2 matches inside `ui-guards.mjs`). The autofocus, tabindex and static-element suppressions are mostly untouched. |
| 3 | `role="tab"` wrapping buttons in Database conn-tabs (minor) | Fixed | `DatabasePage.svelte:1100-1105`, `:1137` and `:1153`: the main `<button role="tab">` has roving `tabindex` via `connTabStop`, `onTabKey` is on the tablist, and the comment at `:1097-1099` says the glyph, ⋯ menu and close are siblings. |
| 4 | Mouse-only click wrappers with suppressions (minor) | Partial | `CanvasPanel.svelte:266-282` is fixed: it is now one real toggle `<button>` with the actions as siblings. Terminal (`:2838`), Composer (`:362`), ToolStep (`:284`), NoteView (`:449`), Markdown (`:183`), ReaderView and FloatingBar still carry their suppressions. Several of these are legitimate combobox/option or delegated-click cases, but the option rows still lack a documented pattern. |
| 5 | `outline-removed` guard has a file-wide escape hatch (minor) | Open | `ui-guards.mjs:394-396` still computes `fileHasFocusRing`, and `:422` still exempts every `outline: none` in a file when it is true. |
| 6 | Palette hit pills are mouse-only spans (nit) | Open | `Palette.svelte:556-559` is unchanged, with both suppressions still there. |
| 7 | `div` inside `button` on workflow nodes (nit) | Fixed | `WorkflowCanvas.svelte:381-426`: the card is a `div`, the node is a `button.node-main` containing only `span`s, and the port is a sibling `button` (`:432`). |
| 8 | Run status by colour plus title only (nit) | Fixed | `WorkflowsPage.svelte:1913-1914` adds an `sr-only` status label. `:2017-2018` shows a visible label. `RunSteps.svelte:342` adds a text `StatusBadge` next to the `aria-hidden` dot. |

### New findings
- `[nit]` N1: the `<div class="node">` wrapper in `WorkflowCanvas.svelte:380-389` is a non-interactive element with `onpointerdown` that needs its own `a11y_no_static_element_interactions` ignore. This adds to the suppression count that finding 2 is trying to reduce. It is the drag-to-move handle, with the keyboard equivalent on the inner button. Moving the pointer handler onto `button.node-main` would drop the ignore.

No regressions found in the other changes I re-read (the Database tab, CanvasPanel and WorkflowCanvas changes).

### Remaining to reach 9.8
- Pay down the 156 suppressions, mainly autofocus (use `dialogFocus`) and scroll regions (`role="region"` plus a label). Then tighten the ratchet baseline so the count cannot creep back.
- Give the option-row listboxes in Terminal find, Composer slash and Palette a documented combobox pattern (`aria-activedescendant`) instead of a blanket ignore. Convert the Palette pills to `<button tabindex="-1">` inside the `aria-hidden` block.
- Make the `outline-removed` guard match per selector instead of per file (`ui-guards.mjs:394-422`).
- Remove the extra `a11y_no_static_element_interactions` ignore on the workflow node wrapper (`WorkflowCanvas.svelte:380`).
