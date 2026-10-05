# Iteration 4 design review — partition 2

**Provisional bounded source score: 8.5/10. Not accepted for 9.8.** Reviewed Git/workbench, database/connections, brokers and API client at HEAD `c5d4e999767fca39db497b6eb444e4eaaa04558d`, documentation over baseline `03f2bc3e`. Concurrent partition-1 changes are outside this review. No tests, builds, servers, screenshots, source edits or commits were performed; only this report was written.

Read AGENTS.md, PLAN.md, the design overview/checklist/accessibility/components rules, Claude's consolidated findings and relevant raw reports, coordination reservations, and partition-2 correctness/performance/UX context. Existing behavior findings stay with those owners. Two additional visual/accessibility findings follow; Claude C owns their repairs.

## Additional findings

### R4-D2-01 — major — Vertical result fields cannot be edited or operated on with the keyboard

**Locations:** `ui/src/modules/database/VerticalTree.svelte:175`–`:185`, `:208`–`:219`; `ui/src/modules/database/VerticalView.svelte:288`–`:325`, `:377`, `:397`–`:410`.

**Source trace / reproduction:** Load an editable Mongo document or SQL result with a nested JSON field, switch to Vertical, and try to edit a short scalar field using only Tab/Enter. The leaf row is a plain div with no focus target or key handler. Its typed editor starts only from `ondblclick`; Edit value, Set null, Delete field, Rename field and Add field are reached through `oncontextmenu` on that same row. The parent view provides no delegated keyboard handler for fields. Containers have an expansion button, but ordinary scalar values have no button at all. The visible instructions also offer only double-click and right-click. A keyboard user can reach the record-level JSON editor at `VerticalView.svelte:389`; this finding is specifically the loss of typed field editing and field-specific actions, not the claim that all document editing is impossible.

**Rule and impact:** `docs/design/guidelines/accessibility.md` §3 requires real controls and keyboard equivalents for pointer operations. This blocks the advertised field editing workflow for keyboard users, so major is appropriate. It is a separate interaction surface from the already-assigned QueryBuilder canvas joins; changing canvas controls will not fix these rows. Count the repeated leaf/container pattern once.

**Fix direction:** Add a real, named field edit/action control, with a visible or focus/coarse-pointer-revealed menu trigger, and pass keyboard events through to the existing field menu. Preserve the expansion and text-selection behavior. Enter should enter the existing typed editor where editable; menu actions should retain their existing review gate. Do not put buttons inside another button. Update the hint to describe an available keyboard path.

**Acceptance, not executed:** With a fixture containing a nested scalar, use only the keyboard to edit it, cancel an edit, set null, and invoke Rename/Add field from the correct nested context. Assert the value becomes a pending change and no query runs before Review & apply. Confirm read-only rows still permit relevant copy actions but cannot edit. Check focused controls and menu return focus in light/dark, RTL, and coarse-pointer layouts.

### R4-D2-02 — minor — schema comparison selectors omit their selected state and version context

**Locations:** `ui/src/modules/brokers/SchemaVersionsPanel.svelte:140`–`:151`, `:252`–`:255`; selection updates at `:45`.

**Source trace / reproduction:** Open a schema subject with at least three versions and inspect/navigate its A/B controls with a screen reader. Each row repeats buttons whose visible accessible names are just A and B; the titles explain before/after but do not identify the version. Selecting a side updates only `class:active`, with color/background/border styles. There is no `aria-pressed` or equivalent selected state on either button. The later diff heading identifies the final pair, so the information is recoverable elsewhere, but the repeated controls themselves do not identify which version is selected for which side.

**Rule and impact:** `accessibility.md` §4 requires state on value selectors and names that identify controls; `components.md` §3 prescribes `aria-pressed` for value selection. This is bounded comparison friction, not a failure to load or compare schemas. It is independent of Claude's loading-state changes to this file.

**Fix direction:** Expose each selected side with `aria-pressed`, and give controls version-bearing names such as “Use version 3 as before” / “Use version 3 as after”. Group or otherwise associate controls with their version. Retain the visible A/B shorthand if desired; do not rely on color alone for selection.

**Acceptance, not executed:** Assert exactly the chosen before/after selectors expose pressed state, including default choices and a delayed detail fetch. Verify the accessible name carries both version and side; use keyboard selection and inspect the resulting diff heading. Check selected styling in light/dark.

## Existing dependencies, deduplicated

These issues remain in the inspected baseline. Claude's other worktree may contain repairs; assignment or an unintegrated patch is not evidence that this worktree is repaired. These are dependencies, not new repair requests. Severities retain the external review's classification.

| Pattern | Severity / deduction | Exact evidence, rule and reproduction | Integration acceptance |
|---|---:|---|---|
| QueryBuilder custom joins require a pointer | major / 0.3 | `QueryBuilder.svelte:952`, `:964` use pointer-only join handles; `:548` implements their pointer path. Accessibility §3. Add two tables without an automatically suggested FK and attempt to create a join by keyboard. Existing edge badges can edit/remove joins, but do not create arbitrary joins. Claude `05-accessibility.md` finding 1. | Integrate the named Add join form and create a custom join using only keyboard; retain edge editing. |
| PR detail duplicates title/navigation chrome | major / 0.3 | `PrDetail.svelte:424` uses generic title, `:443` custom body, `:460` repeats the real PR title, `:479` renders tabs in body. Layout §§3.1–3.2. Open a loaded PR and compare its hierarchy with other detail pages. Claude `02-layout.md`. | Item title in PageHeader, tabs in its snippet and PageBody wrapper; inspect long titles at phone/tablet/desktop. |
| Broker refresh replaces loaded regions with loading text | major / 0.3 | `TopicsTab.svelte:323`, `GroupsTab.svelte:301`, `:334` choose loading before loaded data. Components §11: retain stale content during refresh. Refresh loaded topics/group detail with a delayed response. Claude `04-states.md` finding 3. | Retain the table/detail while refreshing; first load uses shared loading state, failed refresh preserves data with retry/stale feedback. |
| Database connection tabs wrap focusable descendants | minor / 0.1 | `DatabasePage.svelte:1116` has tab role on wrapper, `:1117` focuses inner ordinary button, `:1130` close button is another descendant. Accessibility §§3–4. Tab through two connections and inspect focused role/selection. Claude `05-accessibility.md` finding 3. | Real main button owns tab semantics; close control is its sibling, arrow navigation and menu actions keyboard-accessible. |
| List-pane sizing/resizing remains inconsistent | minor / 0.1 | `ApiPage.svelte:330`–`:338` bespoke separator/legacy handler and 520px maximum; `EnvironmentsView.svelte:306` fixed 220px list; `WorkbenchPage.svelte:651` fixed 200–250px list. Layout §4.1. Resize a narrow workbench/API pane containing long names. Claude `02-layout.md` pane findings; combined as one systemic partition deduction. | Adopt shared PaneDivider/LIST_PANE where appropriate, retain phone stack, verify keyboard resizing and long-name readability. |

Full external catalogue coverage is broader than this focused pass. Shared token, copy, hover-reveal and general command-surface work remains represented in Claude's ten-lens reports; this report does not claim to independently re-audit or clear every instance. Do not add partition and external lens deductions together as though they were disjoint findings.

## Score and evidence coverage

Fixed design rubric: **10 − (4 majors × 0.3) − (3 minors × 0.1) = 8.5**. The four majors are Vertical field access, QueryBuilder joins, PR hierarchy and broker refresh. The three minors are schema selection semantics, database tab semantics and list-pane sizing. No blockers or nits added. Repeated instances of a pattern are counted once; no credit taken for unintegrated repairs. A remaining major prevents 9.8 acceptance regardless of arithmetic.

| Design dimension | Source inspected | Rendered/native acceptance outstanding |
|---|---|---|
| Shared hierarchy | PR title/tabs/body, workbench file tabs and pane composition, API sidebar, broker schema sections | Long PR titles, toolbar fit, actual list/detail proportions |
| Tokens/consistency/readability | Token-scaled detail text, schema selection CSS, database row detail, API response header/body/cookie/trace layouts, SFTP names and metadata | Computed contrast in all schemes/custom accent, zoom, long translated labels |
| Responsive layout | Workbench phone stack, ConnectionComparison container stack, API environment stacking, SFTP phone row layout and 36px actions, RowDetail constraints | Phone/tablet overflow, nested-pane widths, two zoom steps, RTL screenshots |
| Keyboard/accessibility | Vertical field menu/edit paths, schema selectors, database tabs, Workbench arrow handling, Git ref popover sibling action buttons/dialogFocus, API JSON tree and response tab handlers | Keyboard-only journeys, accessibility tree/axe, VoiceOver, overlay focus restoration |
| Inspected rendered states | **No rendered execution in this pass.** Existing verified baseline is acknowledged, but does not validate these newly traced states. | Light/dark screenshots, loading/error/refresh transitions, native WKWebView and actual provider/database journeys |

Positive source observations worth preserving: Git's expanded ref popover provides sibling action buttons and dialogFocus; SFTP distinguishes load error, empty directory and filtered empty, with a Retry and a phone layout; RowDetail reveal controls already include focus and coarse-pointer rules; ConnectionComparison uses a container stack and labels the submitted query separately; Workbench file-tab arrows already account for RTL. None is asserted as an executed runtime pass.

Discarded candidate: EnvSelector's unlabeled variable inputs are behind its noncompact branch, while its sole production caller passes `compact`; this pass does not report unreachable markup as a user-facing defect. C2 async ownership/write-confirmation/broker replay, P2 history/diff costs, and U2 import/save/recovery behavior are excluded from design findings and deductions.

**Release status:** Source pass complete. Two additional findings forwarded to root for Claude C; only this report changed. Read-shell slot released. Rendered/native acceptance remains open.
