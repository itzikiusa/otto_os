# Visual design review — partition 2

Baseline: `a16f4c71`. Scope: Git/workbench, Database Explorer/connections/brokers, and API client. **Two findings: one major accessibility issue, one minor readability issue.** Both are source-grounded; neither is claimed as a current-screen visual or VoiceOver reproduction.

Read `AGENTS.md`, design foundations, layout, components, accessibility and the review checklist; checked the coordination file, partition-2 correctness/performance reports, and the other design worktree's changed-file list and relevant diffs. The findings below are absent from those reviewed fixes. Forward them to the existing visual-design owner; no competing source changes were made.

## D2-01 — Major: the database keyboard cursor has no accessible cell identity

**Location:** `ui/src/modules/database/GridView.svelte:753`, `:656`, `:699`; `ui/src/modules/database/grid-cells.ts:111`.

**Evidence (source-only):** The focusable results container has `role="group"` and a static instruction label. Arrow keys update `focusCell`, scroll the target into view, and prevent the default event. The effect at line 656 only moves the `kbd-focus` CSS class. Generated data cells receive data attributes and a CSS class, but no stable cell ID, active-descendant relationship, or DOM focus. There is no live announcement of the new row, column or value. The native table remains browseable, but that separate reading cursor is not connected to the application cursor used by Enter to edit and Command-C to copy.

**User impact:** A user relying on a screen reader cannot determine which cell the advertised arrow-key interaction has selected before copying or editing it. The visible selection ring conveys information unavailable through the accessibility tree. This is distinct from input labels, tablist navigation and resizer keyboard support already owned by the design effort.

**Guideline:** `docs/design/guidelines/accessibility.md` §§3–4: the whole flow works with the keyboard, grids provide cell navigation, and controls expose their states. A visual cursor alone does not expose the current cell.

**Smallest coherent fix:** Connect the existing cursor to an accessible grid focus model. Give rendered cells stable IDs and expose the active cell using a supported grid/active-descendant structure, with row/column positions preserved through virtualization; alternatively move actual focus to a roving cell. Keep the current virtualized renderer and pointer interactions. Do not attach `aria-activedescendant` to the existing generic group without adopting a supported owning role.

**Verification:** With a small result containing different values per column, use VoiceOver and keyboard only to enter the grid, move right/down, and copy/open a cell. Confirm the spoken row, column and value agree with the visible ring and action target. Repeat across a virtual row/column boundary and after sorting/filtering. Capture light/dark screenshots confirming that the same focus ring remains visible; inspect the accessibility tree to ensure it references a mounted cell. No such runtime check was performed in this pass.

## D2-02 — Minor: row-detail type labels fade below the text contrast minimum

**Location:** `ui/src/modules/database/RowDetail.svelte:192` (rendered at `:84`; background at `:122`).

**Evidence (source plus token calculation):** `.rd-type` is 11px `--text-dim` with `opacity: 0.8`, drawn over the row-detail panel's opaque `--surface`. Alpha compositing those checked-in theme tokens and calculating WCAG relative luminance produces:

| Theme | Effective text contrast |
| --- | ---: |
| Native light | 3.82:1 |
| Native dark | 4.03:1 |
| Pro Dark | 4.28:1 |
| Warm light | 3.66:1 |
| Warm dark | 3.83:1 |

Values use `ui/src/lib/tokens.css:155`, `:168`, `:184`, `:200`, and `:214`, with each block's `--text-dim`. This is a source-color calculation, not a browser pixel measurement. The other design worktree does not change `RowDetail.svelte`.

**User impact:** Type hints such as `UInt64`, `varchar` or timestamp types are meaningful information when inspecting query values. At the minimum text size, the extra fade makes that information unnecessarily hard to read in every shipped theme.

**Guideline:** `docs/design/guidelines/accessibility.md` §1 requires at least 4.5:1 for UI text; `foundations.md` §§1.4 and 2 reserve the 11px floor for readable metadata. `--text-dim` already supplies the secondary hierarchy.

**Smallest fix:** Remove the additional `opacity: 0.8` from `.rd-type`, keeping the existing type scale and semantic text token.

**Verification:** Open row detail for a result with type hints in all five theme/scheme combinations and measure the composited text contrast (at least 4.5:1). Capture Native light/dark screenshots at desktop and narrow widths to confirm type names remain readable without overtaking field names. No live visual check was performed here.

## Coverage and limits

- **Git/workbench:** Sampled repository toolbar/tab structure, graph row controls, blame presentation, conflict controls, workbench file list/menu and editor layout. Viewed `docs/reviews/ux-20260925/evidence/git-header-2048.png`; it is historical, light-theme, sparse-data evidence and does not establish current rich-state correctness. No additional uncovered layout finding asserted.
- **Database/connections:** Inspected results grid keyboard/rendering behavior, row detail, relevant results/structure layout, SFTP listing, transfer toolbar and dialog use; sampled connection-form source. The two findings above are the concrete remaining issues. Did not repeat SQL mutation correctness, assistant read-only behavior, insert/close confirmations, resizers, or the other owner's RTL changes.
- **Brokers:** Sampled cluster/topic/group presentation and schema list/detail styles, auto-selection, narrow/short viewport rules, load/error handling and version tabs. Existing source includes container/viewport accommodations and error-retry affordances. No fresh broker overflow or contrast defect was established.
- **API client:** Inspected page pane structure, request and response controls, responsive rules, collection/history rows and menus, and JSON tree presentation. Existing request/response tabs already have keyboard handlers and selected state. No additional substantive finding asserted; script-variable concurrency remains in the correctness report.
- **Verification limit:** No builds, servers, interactive browser runs, new screenshots, or source edits. The available load-test screenshots depict agent sessions rather than these reviewed screens, so they were not used as evidence. The historical Git screenshot is context only. Rich data at phone/tablet widths, custom accents, current light/dark rendering, and actual VoiceOver behavior still need the targeted checks above; this report is not a visual acceptance certificate for the full partition.
