## Lens: Shared component usage & quality — iteration-6 re-score

**Score: 9.80/10** (10.00 − 2 minors × 0.10 = 9.80). Both deductions are new items I missed earlier; every item carried over from iteration 5 is now fixed.

I read the code in `/Users/itziklavon/claude_ade-design6` and ran no builds or tests.

### Findings carried over from iteration 5

| Item | Status | Evidence |
|---|---|---|
| 6. Local spinners (5 rules) | Fixed | A grep for `animation:…otto-spin` or `…spin` in `ui/src/**/*.svelte` returns no matches. `SharePage` and the four conversation spinners are gone, and no forked keyframes remain. |
| 7. Local pill, chip and badge rules | Fixed | Only two rule blocks are left: `lib/components/Badge.svelte:41` (the shared component itself) and `vault/StructuredNote.svelte:162`. The latter is a clickable `.tag` link style (`cursor: pointer`), not a status pill. The K8s, swarm and tray pills are gone. |
| 19. `Modal` named by `aria-label` only | Fixed | `lib/components/Modal.svelte:21` defines `titleId = $props.id()`, `:161` has `aria-labelledby={titleId}` and `:166` has `<h2 id={titleId}>`. The `aria-label={title}` kept at `:162` is documented at 153-155 as there for specs that locate a sheet by that attribute. |

Spot-checks of earlier Fixed items found no regressions:

- `DockedDrawer` is still used (`kubernetes/ResourceDrawer.svelte` has 4 matches).
- `Switch` is imported in `vault/VaultPage`, `vault/DocsAgentsView` and `workflows/TriggersPanel`.
- The `.fld`, `Couldn't` and glyph-button (+ − ↑ ↓) patterns return no matches in `modules`. The `class="fld"` form-field pattern is also still gone.

### New findings

- **[minor] Hand-rolled empty blocks.**
  - `product/RefineChat.svelte:119` has a `<div class="empty-state">`.
  - `database/DatabasePage.svelte:1054,1512,1583,1585,1639,1641` use `.list-empty` text blocks for empty and filtered-empty states.
  - Rule: components §11 requires `EmptyState`, or `LoadState` with an `emptyView`.
  - Fix: use `EmptyState variant="panel"`, with a "Clear filter" action for the filtered cases.
- **[minor] A `div role="button"` row remains.**
  - `panels/ActivityPanel.svelte:361-368`: the `.row-main.clickable` disclosure is a `div` with `role="button"`.
  - Rule: components §1 says to use a real `<button>`. A wrapping `<button class="row-main">` that renders `rowMain()` inside it (content permitting) gives native Enter and Space handling and focus.

### Remaining to reach 9.8

- The score is already 9.80 on the rubric arithmetic.
- To get comfortably clear of that, convert the `RefineChat` and `DatabasePage` empties to `EmptyState`.
- Turn the `ActivityPanel` disclosure into a real `<button>`.
