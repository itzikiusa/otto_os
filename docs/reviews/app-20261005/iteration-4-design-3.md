# Iteration 4 design review — partition 3

**Provisional bounded source score: 8.8/10. Not accepted for 9.8.** Reviewed Vault, Canvas, Design Hall, Product, Browser and Snip at HEAD `c5d4e999767fca39db497b6eb444e4eaaa04558d`, documentation over baseline `03f2bc3e`. Concurrent partition-1 edits were outside scope. No tests, builds, servers, browser/native sessions or screenshots were run. Only this report was written.

Read AGENTS.md, the effort PLAN.md, design overview/checklist, accessibility/layout and relevant state rules, the external fixed rubric, consolidated findings, relevant raw lens reports, and coordination reservations. Claude owns presentation/a11y/copy repairs. His unintegrated `b751ca6f` branch was consulted only to avoid duplicate requests: its diffs do not repair the additional tree/switcher findings below. Assigned work is not integrated acceptance.

## Additional findings

### R4-D3-01 — major — tree widgets omit navigation keys, and the 3D tree cannot disclose groups by keyboard

**Locations:** `ui/src/modules/product/design/scene3d/Hierarchy.svelte:143`, `:291`, `:303`; callers `ui/src/modules/design-hall/studio3d/Studio3D.svelte:524` and `ui/src/modules/product/design/DesignArena.svelte:850`. Related same-pattern location: `ui/src/modules/vault/FileTree.svelte:247`–`:255`.

**Source trace / reproduction:** Open a 3D artifact with a group and focus its hierarchy row. Left/Right and Up/Down do nothing: `onRowKey` handles only Enter/Space (select), F2 (rename), and ContextMenu. The only group-collapse toggle is the disclosure button, which has `tabindex="-1"`; the context menu offers rename/duplicate/reorder/group/delete but no disclosure action. A keyboard-only user cannot collapse or re-expand that group. Both production callers use this component. Vault likewise gives every flat tree row `tabindex="0"`, handles only Enter/Space, and supplies neither `aria-expanded` for directories nor `aria-level` for the flattened nesting. Vault can toggle a folder with Enter; do not claim folder access is wholly blocked there.

**Rule and impact:** `docs/design/guidelines/accessibility.md` §§3–4 explicitly require list/tree arrow navigation, keyboard parity, and disclosure state. Count incomplete tree behavior once across these two surfaces. Major severity reflects the unavailable 3D disclosure operation, rather than merely nonstandard key choice. The 3D Inspector has a Visible checkbox, so its hierarchy eye buttons are not reported as globally inaccessible visibility controls.

**Fix direction:** Give the tree a focused-row model with Up/Down, Home/End and direction-aware collapse/expand, preserving editable input keys. Expose a keyboard-reachable action menu; retain native controls where appropriate. In Vault, expose expanded state and nesting level and coordinate focus with VirtualList so navigation can reach off-window rows. Prevent activation from bubbling out of nested controls.

**Acceptance, not executed:** In both Design Hall and Product, create a grouped 3D fixture, collapse/expand it using only keys, navigate children, rename/cancel, and exit the tree. In a Vault with more rows than its virtual window, navigate to and open an off-window note and inspect directory expanded/level semantics. Repeat LTR/RTL, verify focus in light/dark, and ensure checkbox/text-input keys do not activate their row.

### R4-D3-02 — major — Vault lookup failures masquerade as empty results and suggest creation

**Locations:** `ui/src/modules/vault/vault.svelte.ts:1202`–`:1208`; `ui/src/modules/vault/Switcher.svelte:24`–`:30`, `:60`–`:63`, `:88`–`:92`. Related newly traced same-pattern tags surface: `vault.svelte.ts:1190`–`:1199`, `ui/src/modules/vault/TagsPanel.svelte:14`–`:19`.

**Source trace / reproduction:** Open Quick switcher, enter a title, and fail the switcher request. The store catches every error and returns `[]`; the component treats that response as a successful search with no hits. It displays “Create … / New note · Enter,” and Enter invokes `createFromQuery`. There is no error explanation or Retry. An initially pending lookup also has no loading state and can take this empty-result branch. This is a misleading suggested next action; no claim is made here that the server overwrites an existing note. Tags similarly convert a failed tags request into an empty array and display “No tags yet. Add #tags…”.

**Rule and impact:** AGENTS.md and `components.md` §§10–11 require distinct loading/empty/error/loaded states and inline Retry. The quick switcher steers an Open task toward Create when availability is unknown. This extends the already-known failure-as-empty pattern in Linked Canvases and the Design Hall Inspector; it is one systemic deduction, not a fresh deduction for each lookup.

**Fix direction:** Preserve lookup errors and request state, surface an inline retry for the current query, and offer ordinary Enter-to-create only after a successful current-query no-match result. Preserve explicit intentional creation as a distinct action. Keep last good tags on refresh failure with stale feedback. Coordinate store behavior with the behavioral owner; Claude owns the state presentation.

**Acceptance, not executed:** Delay, reject and recover a lookup for an existing title. Pending/failed lookup must not appear as a successful empty result or create a note on ordinary Enter. Retry must use the current query and produce its results; a genuinely empty successful search must still allow deliberate creation. Fail tags loading after a successful result and verify retained tags plus retry/stale feedback.

### R4-D3-03 — minor — quick-switcher arrow selection is visual only

**Locations:** `ui/src/modules/vault/Switcher.svelte:50`–`:64`, `:71`–`:86`.

**Source trace / reproduction:** Keep focus in the switcher's text input and press Down through matches. The handler changes `sel`, and the rows change only `class:sel`; focus stays in the input. It has no combobox role, controlled list, or `aria-activedescendant`, and result buttons have no selected-state semantics. A screen reader has no declared relationship between the focused input and the newly highlighted match that Enter will open. Users can still Tab to result buttons, so this is bounded accessibility friction rather than a complete inability to open a note.

**Rule:** `accessibility.md` §§3–4 require usable composite keyboard behavior and exposed names/roles/states. The matched-result state must be available beyond its color.

**Fix direction:** Use the existing searchable-picker/combobox pattern with a named result list and stable option IDs, keeping the active descendant synchronized with selection and visibility, or move actual focus through named result controls with a coherent keyboard model. Keep Create distinct from a matched result.

**Acceptance, not executed:** Assert accessible roles, controlled-list relationship and active-result changes on arrows; verify the Enter target matches the announced selection after a query refresh. Spot-check with VoiceOver and a result list requiring scrolling. Preserve ordinary text editing and Escape/focus return.

## Existing dependencies, deduplicated

These remain in the inspected baseline. They are already Claude-owned; no competing changes or new requests are made here.

| Pattern | Severity / deduction | Evidence and rule | Required integration acceptance |
|---|---:|---|---|
| Failure shown as no items in Linked Canvases and Site Inspector | Included in D3-02's major; **no extra deduction** | `product/LinkedCanvases.svelte:25`–`:34`, `:171`–`:174`; `design-hall/site/Inspector.svelte:173`–`:188`, `:222`–`:238`. Failed lookup empties collections. Components §§10–11; external `04-states.md` findings 1/6. | Integrate their error/retry states and fail each lookup in-place without an empty-data claim. |
| Product Plan/Rewrite omit reachable Stop while generating | major / 0.3 | `product/PlanTab.svelte:464`–`:478`, `:549`; `product/RewriteTab.svelte:235`–`:250`. Disabled generation controls and polling feedback have no Stop. Patterns §1; external `08-agent-patterns.md` finding 2. | Integrate neutral Stop controls, cancel an active generation, and verify honest post-stop status. |
| Browser close controls lack the coarse-pointer hit-area treatment | minor / 0.1 | `browser/TabStrip.svelte:43`–`:49`, `:122`–`:135`: bespoke `.close` declares 18×18 and has no local coarse-pointer growth. Layout §5; external `06-responsive-rtl.md`. Source declaration, not a measured rendered hit target. | Integrate shared/coarse hit-area treatment; inspect actual target reachability with several open tabs on phone. |
| Canvas chrome stays on physical sides in RTL | minor / 0.1 | `canvas/MermaidCanvas.svelte:585`, `:626`, `canvas/D2Canvas.svelte:588`. Mode/zoom chrome uses physical offsets. Accessibility §6; external `06-responsive-rtl.md`. D2 zoom already uses `inset-inline-end` at `:632`; do not re-report that line. | Integrate logical chrome offsets and inspect both directions without changing the canvas coordinate system. |

The external catalogue is broader than this focused pass: token/copy/reveal utilities, shared component migration, commands and agent vocabulary remain covered by Claude's ten-lens audit. This report does not clear every catalogue entry or add its total to this partition score. C3 metadata/browser/Excalidraw, P3 transcript-body/diff bounds, and U3 publish preview/destination retry/canvas-assist drafts are excluded from findings and deductions here.

## Score and coverage

Fixed rubric: **10 − (3 majors × 0.3) − (3 minors × 0.1) = 8.8/10**. Majors: incomplete tree keyboard model, failed lookup presented as empty (all listed instances counted once), Product generation Stop. Minors: switcher active-result semantics, Browser coarse targets, Canvas RTL chrome. No blockers or nits added. Any remaining major prevents 9.8 acceptance. Score is bounded source judgment, not a rendered/native pass.

| Design dimension | Source inspected | Rendered/native acceptance still needed |
|---|---|---|
| Shared hierarchy | Vault pane/divider composition; Canvas PageHeader/PageBody/scene selection; Design Hall version strip/studio hierarchy; Product grouped tabs; Browser tab siblings; Snip grouped controls | Long titles, nested pane proportions, actual toolbar budgets at desktop/tablet/phone |
| Tokens/consistency/readability | Source uses of text tokens, version chips, row selected state, Snip labels and keyboard instructions | Computed contrast, all schemes/custom accent, text zoom, actual source/image readability |
| Responsive layout | Internal tab/list scroll containers, scene-list/main states, Canvas physical chrome offsets, bespoke Browser close dimensions | Phone/tablet overflow, touch targets, two zoom steps, RTL and light/dark captures |
| Keyboard/accessibility | Tree key dispatch and controls, switcher selection model, SourceSearch buttons and retries, version pressed states, Browser sibling select/close, Snip placement/selection/movement/delete handlers | Keyboard-only journeys, accessibility tree/axe, VoiceOver, modal focus restoration, native key interception |
| Inspected rendered states | **None executed in this pass.** Existing green baseline evidence is acknowledged but cannot validate these newly traced cases. | Loading/failure/stale transitions, light/dark screenshots, actual WKWebView, real external-provider and capture flows |

Positive source evidence to retain: Canvas has page/compact Retry branches that preserve the open scene on a failed replacement load; SceneList offers sibling action buttons and keyboard rename/menu actions; SourceSearch keeps independent filter/search errors and retry callbacks; Design Hall VersionStrip exposes pressed selection and stale refresh Retry; Snip supplies explicit canvas keyboard instructions, Enter placement, annotation selection, movement/deletion, and a live selected-annotation description. These are source observations, not executed successes.

**Release status:** Focused source pass complete. Three additional findings forwarded to root for Claude/behavior coordination. Only this report changed; no source edits, tests, builds or commits. Read-shell slot released. Rendered/native acceptance remains open.
