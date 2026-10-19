# D02 — Sessions, NewSession, Provider Selection, Terminal & Chat UX

- **Reviewer Subagent ID:** `89028f90-aece-4afc-9ea5-8d8ba1f34f7f` (`D02 Sessions & Terminal Chat UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** `ui/src/modules/panels/*`, `ui/src/modules/agents/{NewSession,Handover,TiledView,SplitNode,ShareModal,WorkQueue}.svelte`, `ui/src/modules/agents/conversation/*`, `ui/src/modules/agents/history/*`, `ui/src/lib/components/ProviderIcon.svelte`
- **Iteration 1 Baseline Score:** **8.4 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D02-01 (High — Fixed) — Global `.row` class collision in `ActivityPanel.svelte`, `InfoPanel.svelte`, `HistoryPage.svelte`, and `TiledView.svelte`
- **Location:** `ui/src/modules/panels/ActivityPanel.svelte:169`, `ui/src/modules/panels/InfoPanel.svelte:220`, `ui/src/modules/agents/history/HistoryPage.svelte:198`, `ui/src/modules/agents/TiledView.svelte:293`
- **Issue:** `ui/src/app.css:209` defines `.row { display: flex; align-items: center; gap: var(--sp-2); }`. In `ActivityPanel.svelte`, `<li class="row">` inherited horizontal flex layout, forcing expanded `<pre class="row-detail">` blocks side-by-side with `.row-head` instead of stacking vertically.
- **Fix Applied:** Renamed `.row` to `.trail-row` (`ActivityPanel.svelte`), `.info-row` (`InfoPanel.svelte`), `.hist-row` (`HistoryPage.svelte`), and `.tgut.r-gut` (`TiledView.svelte`).

### D02-02 (Medium — Fixed) — `NewSession.svelte` arrow-key navigation lands on disabled providers, missing `ProviderIcon`, and unguarded dirty prompt discard
- **Location:** `ui/src/modules/agents/NewSession.svelte:246-254, 345, 535`
- **Issue:** `onProviderRadioKey` cycled through `visibleProviders` without skipping `!p.available` entries; provider cards rendered raw letter initials instead of `<ProviderIcon id={p.id} />`; and clicking Cancel discarded a multi-line initial prompt without confirmation.
- **Fix Applied:** Updated `onProviderRadioKey` to skip unavailable providers, replaced letter badges with `<ProviderIcon id={p.id} size={18} />`, and added `requestClose()` confirmation when `initialPrompt.trim()` is non-empty.

### D02-03 (Medium — Fixed) — `Handover.svelte` unguarded dirty note discard, missing `ProviderIcon`, and missing `:focus-visible` ring
- **Location:** `ui/src/modules/agents/Handover.svelte:88, 100, 148`
- **Issue:** Pressing `Escape` or clicking backdrop/Cancel discarded typed handover instructions immediately; target provider buttons lacked `<ProviderIcon>` and `:focus-visible` rings.
- **Fix Applied:** Added `requestClose()` dirty-check via `confirmer.ask`, rendered `<ProviderIcon id={p.id} size={15} />`, set `width={600}`, and added `:focus-visible` styling.

### D02-04 (Medium — Fixed) — `BrowserPanel.svelte` background polling when hidden and `FileTree.svelte` keyboard tree navigation
- **Location:** `ui/src/modules/panels/BrowserPanel.svelte:64`, `ui/src/modules/panels/FileTree.svelte:274, 348`
- **Issue:** `BrowserPanel.svelte` used raw `setInterval(refresh, 4000)` even when the document was hidden; `FileTree.svelte` lacked `aria-expanded`, arrow-key row navigation, and `loadErrorText(e)` formatting, and attached `ondblclick` to `<nav>`.
- **Fix Applied:** Switched `BrowserPanel.svelte` to `pollWhileVisible(refresh, 4000)`, added `aria-expanded` and `ArrowRight`/`ArrowLeft`/`ArrowDown`/`ArrowUp` handling in `FileTree.svelte`, moved `ondblclick` to `.ft-crumb` buttons, and formatted errors via `loadErrorText(e)`.

### D02-05 (Low — Fixed) — `ProviderIcon.svelte` duplicate static SVG `<linearGradient id="agy-g">` across multiple instances
- **Location:** `ui/src/lib/components/ProviderIcon.svelte:47-55`
- **Issue:** Every `agy` icon rendered static `id="agy-g"` and `id="agy-b"`, causing DOM ID collisions and broken fills when an ancestor splits or hides.
- **Fix Applied:** Scoped SVG gradient IDs per component instance using `$props.id()`.

### D02-06 (Low — Fixed) — `Composer.svelte` slash/mention popup `max-height` stale after attachment changes
- **Location:** `ui/src/modules/agents/conversation/Composer.svelte:418-425`
- **Issue:** `popMax` only recomputed when `slashOpen`, `mentionOpen`, or `session.id` changed, not when `attachments.length` or window height changed.
- **Fix Applied:**Tracked `attachments.length` and `resize` listener in `Composer.svelte`'s `popMax` effect.

### D02-07 (Medium — Fixed & Browser-Verified) — `NewSession.svelte` `.cbtn` (`+/-`) provider stepper buttons below `36×36px` touch target on mobile/coarse viewports
- **Location:** `ui/src/modules/agents/NewSession.svelte:808-840`
- **Issue:** `.cbtn` defaulted to `20px × 20px` and only grew to `30px × 30px` under `@media (pointer: coarse)` — both below the `36px × 36px` (`--hit-min`) touch target requirement in `docs/design/guidelines/layout.md`, and remaining `20px × 20px` on `390×844` mobile viewports when tested without `pointer: coarse`.
- **Fix Applied:** Updated `ui/src/modules/agents/NewSession.svelte:834-840` to `@media (pointer: coarse), (max-width: 640px) { .cbtn { width: var(--hit-min); height: var(--hit-min); font-size: var(--fs-l); } }`.
- **Browser Verification:**
  - Playwright `e2e/desktop-new-session-sizing.spec.ts` (4 cases across desktop + phone `390×844` light/dark on isolated slot 19) **PASSED**, asserting actual rendered width/height `>= 36px` (`36px × 36px`) for every visible counter control on phone.
  - Playwright `e2e/desktop-ux-r4-shell.spec.ts` (`-g "Outputs refreshes|Activity applies|mounted Activity and Outputs watchers"`, isolated slot 21) **PASSED (3/3, 7.4s)**, verifying mounted `ActivityPanel` and `OutputsPanel` late-GET vs WS-delta reconciliation, WS `resync` recovery, and unmount cleanup.
