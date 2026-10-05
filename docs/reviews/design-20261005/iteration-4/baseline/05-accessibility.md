## Lens: Accessibility
**Score: 9.3/10.** Arithmetic: 10.0 − 0.3 (1 major) − 0.3 (3 minor) − 0.09 (3 nit) = 9.31. Three things hold it below 9.8:
- The canvas editors are pointer-only.
- 160 `svelte-ignore a11y_*` suppressions remain and nothing ratchets them.
- A few mouse-only wrapper patterns remain.

### Verified improvements from earlier passes (credit, no regressions found)
- **Drawer:** `Drawer.svelte:84-93` has `role="dialog"`, `aria-modal`, `aria-label`, `ui.pushModal()` and `dialogFocus`. The visible titled header has a labelled close button (`:97-101`). The only suppression left is the backdrop (`:79-81`).
- **Switch:** `Switch.svelte:22` is a real `<button role="switch">`, and it is the only `role="switch"` in the tree.
- **Tab keys:** `role="tab"` appears in 54 files, and the tablists I read have roving `tabindex`, `aria-selected` and arrow/Home/End handlers. These were MissionControlPage:42-46 and 346-352, AthenaView:510, ClusterWizard:185-206 and DatabasePage:763.
- **Search/filter inputs:** all 24 inputs whose placeholder starts with "Filter", "Search" or "Find" also have an `aria-label`. The form fields I sampled (ProofPage, ScheduledTasks, ConnectionForm) use `<label for>` or a wrapping label.
- **Text size:** the `--fs-xs` token is 11px (`tokens.css:23`), and no `font-size` below 11px exists in `ui/src`.
- **Reduced motion:** `app.css:700-710` collapses animations globally, and there is a static busy-ring alternative.
- **Widget semantics:** HomeBox resize is a real `role="slider"` with value attributes and arrow keys (`HomeBox.svelte:232-244`). SplitDivider is a proper window splitter with keys and Enter to reset (`SplitDivider.svelte:67-83`). Home widget reorder has a keyboard path through the menu (`HomeBox.svelte:174-175`).
- **Non-interactive clickables:** a grep for `onclick` on `div`, `span`, `li`, `tr` or `td` finds about 8, against the 98 the audit recorded.
- **Suppressions:** 160 `svelte-ignore a11y_*` remain, down from 172.

### Findings

1. `[major]` Canvas editors are pointer-only for core operations.
   - **Locations:**
     - `WorkflowCanvas.svelte:242-250` (pan and wheel), `:259-268` (the edge hit-path is `role="button" tabindex="-1"`, so it can never take focus), `:332-336` (output port is a `span` with `onpointerdown`) and `:297` (node drag).
     - `QueryBuilder.svelte:918`, `:952` and `:964` (node drag and the join handles are `onpointerdown` spans).
     - `AgentGraph.svelte:297-306` (`role="application"` with pointer-only pan and wheel).
   - **What is wrong:** `accessibility.md §3` requires everything reachable by pointer to be reachable by keyboard. In `modules/workflows`, the only edge-creating code I found is the `graph.edges = [` assignment at `WorkflowCanvas.svelte:183`, which is the drag path. A keyboard user cannot wire or select edges, or move nodes.
   - **Fix:**
     - Add a "Connect to…" menu on the focused node (a select of target nodes) and "Remove connection" on a selected edge.
     - Make edges focusable (`tabindex=0`, Enter to select).
     - Add arrow-key nudging for the selected node.
     - Give QueryBuilder an "Add join" form as the keyboard path.
   - **Note:** I did not read the whole QueryBuilder join section, so a keyboard join editor may exist elsewhere. The WorkflowCanvas gap is firmer.

2. `[minor]` 160 `svelte-ignore a11y_*` suppressions remain and nothing stops them growing.
   - **Locations:** `grep -rn "svelte-ignore a11y" ui/src`. By kind:
     - about 38 × `a11y_autofocus`.
     - about 36 × `a11y_no_noninteractive_tabindex`.
     - about 55 × `a11y_no_static_element_interactions`.
     - about 16 × `a11y_click_events_have_key_events`.
     - the rest are `no_noninteractive_element_interactions`, `media_has_caption`, `invalid_attribute` and similar.
     - Worst files: Navigator.svelte (7), DatabasePage.svelte (6), QueryBuilder.svelte (6), WorkflowsPage.svelte (5), TiledView.svelte (5), MultiRunDialog.svelte (5).
   - **What is wrong:** `accessibility.md §3` says the count "should only go down", but `ui/scripts/ui-guards.mjs` has no rule for it. A grep for `svelte-ignore` there finds none.
   - **Fix:**
     - Add an `a11y-ignore` ratchet to `ui-guards.mjs` with a per-file baseline.
     - Pay down the cheap groups: replace `autofocus` with `dialogFocus` or `use:focusOnMount`, and give scroll regions `role="region"` plus `aria-label` so the tabindex suppressions go away.

3. `[minor]` A `role="tab"` element wraps interactive controls.
   - **Locations:** `DatabasePage.svelte:1116-1122` (a `div role="tab" tabindex="-1"` wrapping `button.conn-tab-main`), `:1144` and `:1161` (same shape).
   - **What is wrong:** a tab must not contain focusable descendants. The real tab stop is the inner button, which has no `role="tab"`, so the tablist's `aria-selected` and the arrow-key model do not line up with focus. The "open beside agents" action is also reachable only by right-click (`oncontextmenu`).
   - **Fix:** put `role="tab"`, `aria-selected` and the roving `tabindex` on the main button, and keep the spinner and close controls as siblings outside the tab element. Expose the context-menu action through a visible ⋯ button or a keyboard shortcut.

4. `[minor]` Mouse-only click wrappers with a suppression and no key handler.
   - **Locations:**
     - `CanvasPanel.svelte:280-281` (`div.ref-main`, with a duplicate `role="button"` child at `:287-293`).
     - `Terminal.svelte:2838-2846` (listbox option rows with `onclick` only).
     - `Composer.svelte:362-370` (slash-command option rows).
     - `ToolStep.svelte:284-285` (`<pre onclick>`).
     - `NoteView.svelte:449-457` (`div.read` with `onclick`, `onmouseover` and `onmouseout`).
     - `Markdown.svelte:183-184`.
     - `FloatingBar.svelte:852`.
     - `ReaderView.svelte:246-247`.
   - **What is wrong:** `accessibility.md §3` says `onclick` on a `div` is a bug. Some of these are valid (the combobox/option pattern and delegated link clicks inside rendered content). Others, such as the `CanvasPanel` row, are duplicates of a real control.
   - **Fix:** for the listbox rows, keep the combobox pattern and add `aria-activedescendant` plus a comment, rather than a blanket ignore. For `CanvasPanel`, drop the wrapper `onclick` and rely on the inner button, or make the row a `<button>`.

5. `[minor]` The `outline-removed` guard has a file-wide escape hatch.
   - **Locations:** `ui/scripts/ui-guards.mjs:369-376`, with `NumberDrag.svelte:173` as a sample site.
   - **What is wrong:** the guard flags a rule only when `!fileHasFocusRing`. Any `:focus-within` rule, or any `:focus` rule that sets a border or shadow, in the same file exempts every `outline: none` in that file. `outline: none` appears in 114 files (151 occurrences), so this is a likely source of unreviewed removals.
   - **Fix:** match rule-to-rule by selector, so an input's `.x:focus` ring covers `.x` only. Alternatively, ratchet the `outline: none` count per file.

6. `[nit]` The command palette hit action pills are mouse-only spans.
   - **Location:** `Palette.svelte:556-559` (`div aria-hidden="true"` containing `span.pal-hit-btn onclick`).
   - **What is wrong:** the comment explains the keyboard path (⏎/⌥⏎/⇧⏎), and the pills are `aria-hidden` for that reason. The suppressions are still counted debt, and touch users on the tablet layout cannot use them.
   - **Fix:** make them `<button tabindex="-1">` inside `aria-hidden`, which removes both suppressions at no cost to the pattern.

7. `[nit]` Invalid HTML content model on workflow nodes.
   - **Location:** `WorkflowCanvas.svelte:286-338` (`<button class="node">` contains `<div class="head">`, `<div class="steps">`, `<div class="until">` and an interactive `span.port.out`).
   - **What is wrong:** a `<div>` inside a `<button>` is invalid, and the nested pointer target is not a separate control.
   - **Fix:** use `<span>` with `display:block`, and move the output port out of the button as a sibling.

8. `[nit]` Run status is conveyed by colour plus a `title` only.
   - **Locations:** `WorkflowsPage.svelte:1775` and `:1890` (`span.dot` with `aria-hidden`, and the status label appears only in the button `title`). `RunSteps.svelte:340` has the same shape.
   - **What is wrong:** `accessibility.md §4` says colour is never the only signal. Neighbouring code does it right: `HistoryPage.svelte:503-504` adds an `sr-only` label.
   - **Fix:** add `<span class="sr-only">{runStatusLabel(r.status)},</span>` as `HistoryPage` does, or a visible label.

### What would get this lens to 9.8
- Give the workflow and query-builder canvases a keyboard path: connect-to menus, focusable edges and arrow-key node nudging. This is the one finding that blocks a keyboard user from a feature.
- Add the `svelte-ignore a11y_*` ratchet to `ui-guards.mjs`, then drive the 160 down by replacing `autofocus` with `dialogFocus` and naming scroll regions. The target is under 60, each with a one-line reason.
- Fix the `role="tab"` structure in the Database connection tabs, and use `aria-activedescendant` consistently for option-row listboxes (Terminal find, Composer slash, Palette).
- Tighten the `outline-removed` rule to per-selector scope.
- Add an `sr-only` status label wherever a status dot is `aria-hidden`.
- Run `expectAccessible` for `serious` `color-contrast` and `nested-interactive` in the e2e suite for Database, Workflows and Home. Today it fails only on `critical`.

Key files: `/Users/itziklavon/claude_ade-design/ui/src/modules/workflows/WorkflowCanvas.svelte`, `/Users/itziklavon/claude_ade-design/ui/src/modules/database/DatabasePage.svelte`, `/Users/itziklavon/claude_ade-design/ui/src/modules/database/QueryBuilder.svelte`, `/Users/itziklavon/claude_ade-design/ui/scripts/ui-guards.mjs`, `/Users/itziklavon/claude_ade-design/ui/src/shell/Palette.svelte`.
