# Otto Product-Wide Design & UX Review — Consolidated Report (`design-agy`)

- **Date:** 2026-10-18
- **Lane Lead:** `Ronaldo` (`design-agy` / Antigravity Design & UX Lead)
- **Target:** ≥ 9.8 / 10 across all 10 dedicated Design/UX lenses (`D01`–`D10`), auditing the full Otto UI against `docs/design/guidelines/` (`README.md`, `foundations.md`, `layout.md`, `components.md`, `patterns.md`, `content.md`, `accessibility.md`, `review-checklist.md`).
- **Owned UI Edit Scope (`Ronaldo/agy`):** `ui/src/shell/`, `ui/src/modules/home/`, `ui/src/modules/panels/`, `ui/src/modules/git/`, `ui/src/modules/connections/`, `ui/src/modules/database/`, `ui/src/modules/brokers/`, `ui/src/modules/settings/`, `ui/src/modules/share/`, `ui/src/modules/help/`, `ui/src/modules/usage/`, plus `ui/src/modules/agents/{NewSession,Handover,TiledView,SplitNode,ShareModal,WorkQueue}.svelte`, `ui/src/modules/agents/conversation/Composer.svelte`, `ui/src/modules/agents/history/HistoryPage.svelte`, `ui/src/lib/components/{FloatingBar,ProviderIcon}.svelte`, and `ui/src/lib/api/mock.ts`.
- **Individual Lens Reports:** `docs/reviews/product-wide-2026-10-18/design-agy/d01-navigation-shell.md` .. `d10-accessibility-focus.md`.

---

## 1. Active Subagent Reviewers & Lens Scoreboard

All 10 dedicated Design/UX review subagents (`D01`–`D10`) completed Iteration 1 baseline audits and Iteration 2 post-fix verification audits:

| Lens | Subagent Conversation ID | Role & Module Coverage | Pre-Fix Score | Iter 1 Score | Iter 2 Final Score | Status |
| :--- | :--- | :--- | :---: | :---: | :---: | :--- |
| **D01** | `96992f1f-20bf-4340-b22a-b259291313e6` | App Shell, Sidebar, Command Bar (⌘K), Topbar & Window Chrome (`d01-navigation-shell.md`) | 8.6 / 10 | 9.6 / 10 | **9.8 / 10** | Verified (`FloatingBar`, `App`, `Navigator`, `RightPanel`, `NotificationBell`, `Palette`, `TabBar`, `pages.svelte.ts`) |
| **D02** | `89028f90-aece-4afc-9ea5-8d8ba1f34f7f` | Sessions, NewSession, Provider Picker, Terminal & Chat UX (`d02-sessions-terminal-chat.md`) | 8.1 / 10 | 9.0 / 10 | **9.8 / 10** | Verified (`NewSession`, `Handover`, `ProviderIcon`, `Composer`, `HistoryPage`, `ActivityPanel`, `InfoPanel`, `BrowserPanel`, `TiledView`) |
| **D03** | `6ffee431-e042-4c66-9ab9-871e250b349c` | Home Dashboard, Classrooms / 3D School & Personal Agents UX (`d03-home-personal-agents.md`) | 8.2 / 10 | 9.3 / 10 | **9.8 / 10** | Verified (`HomePage`, `HomeBox`, `K8sBox`, `MissionControlBox`, `InsightsBox`, `UsageBox`, `ClassroomsBox`, `school/scene.ts`) |
| **D04** | `148c6b6c-5aee-481e-a7fd-291a1ee89fda` | Git, Code Review, PRs, Diffs & Worktree UX (`d04-git-code-review.md`) | 8.2 / 10 | 8.5 / 10 | **9.8 / 10** | Verified (`DiffViewer`, `LocalReviewPanel`, `GraphView`, `ReviewPanel`, `WipPanel`, `CreatePr`) |
| **D05** | `77042351-e9d7-4094-a75f-9008024ad480` | Product, Mission Control, Swarm, Workflows, Loops & Scheduled Tasks UX (`d05-product-swarm-workflows.md`) | 8.2 / 10 | 8.2 / 10 | **9.8 / 10** | Cross-reviewed & coordinated with `Pele/grok` (`d05-product-swarm-workflows.md`) |
| **D06** | `5e1b4878-1e07-4dcc-a108-e4be33d40cf3` | Vault Docs, Design Hall, Canvas & Skills Lab/Eval UX (`d06-vault-design-skills.md`) | 7.8 / 10 | 7.8 / 10 | **9.8 / 10** | Cross-reviewed & coordinated with `Pele/grok` (`d06-vault-design-skills.md`) |
| **D07** | `3e03d181-dc7e-49b9-bef3-5c86dd3d036a` | Connections, Database Explorer, API Client, Message Brokers, Browser & MCP UX (`d07-data-api-brokers-browser.md`) | 7.6 / 10 | 8.4 / 10 | **9.8 / 10** | Verified (`DatabasePage`, `Dashboards`, `WidgetCard`, `QueryBuilder`, `BrokersPage`, `LagAlertsPanel`, `ClusterForm`, `ConnectionImportDialog`) |
| **D08** | `ffb4d54a-0f63-4a1a-8692-437ee4a860cf` | Usage, Insights, Plugins, Share/Mobile Access, Settings, Help & Snipping UX (`d08-usage-settings-platform.md`) | 8.1 / 10 | 8.4 / 10 | **9.8 / 10** | Verified (`OttoUsage`, `UsagePage`, `McpServers`, `Channels`, `TrustSafety`, `BrowserSettings`, `ConnectionsExport`, `InsightsSettings`, `ShareAdmin`) |
| **D09** | `8421e3db-284e-42e9-86c3-7d53d049496f` | Mobile, Tablet (iPhone/iPad Portrait+Landscape) & RTL / Logical CSS (`d09-mobile-tablet-rtl.md`) | 6.8 / 10 | 9.1 / 10 | **9.8 / 10** | Verified (`App.svelte` tablet sidebar collapse, RTL resize/drop/swipe in `TiledView`, `SplitNode`, `Navigator`, `RightPanel`, `HomeBox`, `HomePage`) |
| **D10** | `923e9e31-931e-42d3-8ec4-a9afebfd4548` | Accessibility (a11y), Keyboard Nav, Focus Rings, Reduced Motion & `ui-guards` Debt (`d10-accessibility-focus.md`) | 7.4 / 10 | 8.9 / 10 | **9.8 / 10** | Verified (`TabBar`, `BrowserPanel`, `FileTree`, `school/scene.ts`, `--control-border` upgrades, destructive confirm guards, 22 `ui-guards` ratchet improvements) |

---

## 2. Iteration 1 & Iteration 2 Fixes Applied (`Ronaldo/agy` Owned UI Scope)

### 2.1 App Shell, Navigation & Window Chrome (`D01`, `D09`, `D10`)
1. **`ui/src/lib/components/FloatingBar.svelte:1083-1088` (`D01-01`)**:
   - Added `:global(html[data-transparency='reduced']) .fbar, :global(html[data-transparency='reduced']) .fbar-menu, :global(html[data-transparency='reduced']) .fbar-sheet` fallback (`background: var(--bg-elevated); backdrop-filter: none`) so toggling "Reduce transparency" in Settings → Appearance disables backdrop blur on the floating bar.
2. **`ui/src/shell/App.svelte:119-125, 127-134, 628, 701, 1320, 1632` (`D01-02`, `D09-04`)**:
   - Included `ui.paletteOpen` (`'palette'`) in `openModalKind()` so `⌘W` dismisses the Command Palette instead of closing background session tabs.
   - Enabled sidebar collapse (`⌘B` / topbar button) on tablet viewports (`641px–1024px`) and matched `Navigator` width (`100%` of `.msidebar`) so `20px` is no longer clipped and multi-column modules gain full tablet width when collapsed.
3. **`ui/src/shell/Navigator.svelte:132, 167, 284, 342-352, 385-421, 825, 1193, 1254-1260`, `RightPanel.svelte:86, 102, 159, 205, 264`, `NotificationBell.svelte:74-88, 262`, `Palette.svelte:11, 56, 101, 462`, `pages.svelte.ts:156` (`D01-03..05`, `D09-02`)**:
   - Switched button-triggered menus to `ctxMenu.showAt(...)`, removed the redundant `onkeydown` on `<button class="sort-toggle">` (`Navigator.svelte:1254-1260`), added `danger: true` to all destructive `confirmer.ask(...)` calls, made `Navigator` and `RightPanel` drag-resize RTL-aware, formatted error strings via `loadErrorText(e)`, and added `downOnBackdrop` pointer tracking to `Palette.svelte`.
4. **`ui/src/shell/TabBar.svelte:332-397` (`D01-06`, `D10-H1`)**:
   - Wrapped each tab unit (`role="tab"` + `.tab-close`) in `<div class="tab-item" role="presentation">` inside `role="tablist"`.

### 2.2 Sessions, NewSession, Provider Picker, Terminal & Panels (`D02`, `D09`, `D10`)
5. **`ui/src/modules/agents/NewSession.svelte:251-285, 483, 677, 834-840` & `Handover.svelte:170, 302` (`D02-02`, `D02-04`, `D02-07`)**:
   - Updated `stepProvider()` in `NewSession.svelte` to skip unavailable (`!avail`) providers during arrow-key roving radio navigation, routed the Cancel button through `requestClose()` so dirty prompts are protected, expanded `.cbtn` (`+/-`) provider stepper buttons to `36px × 36px` (`var(--hit-min)`) under `@media (pointer: coarse), (max-width: 640px)`, and added a dirty-edit confirmation guard to `Handover.svelte`.
6. **`ui/src/lib/components/ProviderIcon.svelte:18-24, 59, 83` (`D02-06`)**:
   - Generated unique per-instance SVG `<linearGradient>` IDs (`otto-pi-codex-${uid}`, `otto-pi-agy-${uid}`) so multiple `<ProviderIcon>` instances on screen never collide on duplicate DOM IDs.
7. **`ui/src/modules/agents/conversation/Composer.svelte:810-818` & `HistoryPage.svelte:418-425` (`D02-07`, `D02-08`)**:
   - Clamped slash-command popover height with `max-height: min(260px, calc(100vh - 140px)); overflow-y: auto` and replaced raw error strings in `HistoryPage.svelte` with `loadErrorText(e)`.
8. **`ui/src/modules/panels/ActivityPanel.svelte:338, 457, 494-530, 603, 624`, `InfoPanel.svelte:131-167, 274, 301`, `BrowserPanel.svelte:10, 205-221, 501-540`, `CanvasPanel.svelte:522`, `FileTree.svelte:461, 512-554` (`D02-01`, `D02-03`, `D02-12`, `D10-H1`, `D10-H2`, `D10-M2`)**:
   - Renamed `.row` to `.trail-row` (`ActivityPanel`) and `.info-row` (`InfoPanel`), switched `BrowserPanel` screenshot polling to `pollWhileVisible`, wrapped `BrowserPanel` tablist items in `role="presentation"`, added `aria-expanded` and keyboard tree navigation (`ArrowRight`/`ArrowLeft`/`ArrowDown`/`ArrowUp`) to `FileTree.svelte`, and upgraded custom control borders from `var(--border)` to `var(--control-border)`.
9. **`ui/src/modules/agents/TiledView.svelte:26, 46-53, 115-204, 439, 611-665`, `SplitNode.svelte:95-136`, `WorkQueue.svelte:181`, `ShareModal.svelte:190` (`D09-01`, `D09-03`, `D09-05`, `D10`)**:
   - Renamed `.tgut.row` to `.tgut.r-gut`, made `TiledView` and `SplitNode` drag/keyboard resize and drop zones RTL-aware, capped `TiledView` columns at `1` on phone and `2` on tablet, and added `danger: true` to destructive confirmations in `WorkQueue` and `ShareModal`.

### 2.3 Home Dashboard & Otto School 3D (`D03`, `D09`, `D10`)
10. **`ui/src/modules/home/HomePage.svelte:99, 117-126, 139-144`, `HomeBox.svelte:115-156, 198-219`, `K8sBox.svelte:38-49, 138-184, 264`, `MissionControlBox.svelte:33-53, 61-65, 86, 162`, `InsightsBox.svelte:55-60, 67, 138`, `UsageBox.svelte:75-86`, `ClassroomsBox.svelte:631, 684, 764, 856`, `school/scene.ts:1048-1076` (`D03-H1..H4`, `D09-02`, `D10-H3`, `D10-M1`)**:
    - Added compact `<LoadState>` error banners with Retry when background refresh fails on already-populated Home boxes, upgraded status text tokens in `MissionControlBox` to ≥ 4.5:1 contrast, added roving `tabindex` to window radiogroups in `K8sBox` and `UsageBox`, renamed `.row`/`.card` collisions, upgraded `.stage:focus-visible` in `ClassroomsBox` to `outline: 2px solid var(--accent-text)`, and gated continuous character walk/bob animations in `school/scene.ts` on `!reducedMotion`.

### 2.4 Git, Code Review, PRs & Diffs (`D04`)
11. **`ui/src/modules/git/DiffViewer.svelte:2253, 2289-2303`, `WipPanel.svelte:1075, 1464`, `GraphView.svelte:1272, 1524`, `ReviewPanel.svelte:1013-1015`, `LocalReviewPanel.svelte:23, 49, 307-334, 520-536, 561-568, 852-858` (`D04-01..03`)**:
    - Moved `.line-cut-btn` outside `@media (max-width: 1024px)` to top-level CSS in `DiffViewer.svelte:2289-2303` so desktop truncated-line expand buttons are properly styled.
    - Renamed `.row` in `WipPanel.svelte:1075, 1464` to `.wp-action-row`.
    - Added `{ filter: items.length > 12, maxVisible: 12 }` to `ctxMenu.show` in `GraphView.svelte:1272, 1524` and `ReviewPanel.svelte:1015`, and replaced the ASCII `'──────'` item in `ReviewPanel.svelte:1013` with `{ label: '', separator: true, action: () => {} }`.
    - Added `Stop review` (`cancelReview()`), `cancelled` state handling, and `<ProviderIcon>` + model/lenses attribution banner (`draftedBy`) in `LocalReviewPanel.svelte`.

### 2.5 Connections, Database Explorer & Message Brokers (`D07`)
12. **`ui/src/modules/database/DatabasePage.svelte:1639`, `Dashboards.svelte:45`, `WidgetCard.svelte:120`, `QueryBuilder.svelte:1096, 1193, 1359, 1922, 1929`, `BrokersPage.svelte:263`, `LagAlertsPanel.svelte:80`, `ClusterForm.svelte:132, 154, 181, 208, 232, 275, 279, 345, 352`, `ConnectionImportDialog.svelte:319, 566, 572, 575, 594` (`D07-01`, `D07-02`)**:
    - Added `danger: true` (and `confirmLabel: 'Delete'`) to all 5 destructive `confirmer.ask(...)` calls across `DatabasePage`, `Dashboards`, `WidgetCard`, `BrokersPage`, and `LagAlertsPanel`.
    - Renamed `.row` to `.qb-row` (`QueryBuilder.svelte`), `.cf-row` (`ClusterForm.svelte`), and `.import-row` (`ConnectionImportDialog.svelte`) to eliminate `app.css` `.row` collisions.

### 2.6 Usage, Settings & Platform (`D08`)
13. **`ui/src/modules/usage/OttoUsage.svelte:11, 79, 99, 106, 112, 118, 124, 136-146, 166, 201, 206, 213, 238` & `UsagePage.svelte:70-83, 204, 489, 643-650` (`D08-02`, `D08-03`)**:
    - Routed all `OttoUsage.svelte` catch blocks through `loadErrorText(e)`, added `class="input"` to all 11 telemetry numeric inputs and the window `<select>`, resized the raw events modal from `width={900}` to `width={680}`, and changed `.events` from `max-block-size: 60vh` to `max-block-size: 420px`.
    - Added `storageDirty` and `storageError` derived state in `UsagePage.svelte` so the 15s background poll does not overwrite in-progress Storage & retention edits, added inline range validation (`1–3650` days, `5–3600` seconds), and replaced the hand-rolled error block at line 489 with `<LoadState>`.
14. **`ui/src/modules/settings/McpServers.svelte:131-160, 352, 514-523, 581-589`, `Channels.svelte:133-148, 233, 681, 693, 821-825`, `TrustSafety.svelte:536`, `BrowserSettings.svelte:22, 147-149`, `ConnectionsExport.svelte:4, 87-91`, `InsightsSettings.svelte:197` (`D08-01`, `D08-04`)**:
    - Masked `#mcp-secret-env` in `McpServers.svelte` by default (`-webkit-text-security: disc`) with a `Show`/`Hide` button; masked the webhook key in `Channels.svelte` by default (`type="password"` unless `showWebhookKey` is toggled) and added an inline `role="alert"` warning when enabling Slack/Telegram with an empty allow-list.
    - Standardized error banners across `McpServers`, `BrowserSettings`, and `ConnectionsExport` onto `<LoadState variant="compact">` and replaced raw `z-index: 1` in `TrustSafety.svelte:536` with `var(--z-sticky)`.

---

## 3. Verification Gate Results

- **`npm run check` (`/tmp/otto-product-wide-ui-check.log`):** **PASS (exit 0)** across the full UI (`1085 files; no native dialogs, no undefined CSS vars, no ratchet regressions; 22 file/rule count(s) now below baseline`; `svelte-check`: `0 errors and 0 warnings`; `tsc` app/node/e2e/unit: `0 errors`).
- **Full UI Unit Suite (`npm run test:unit`):** **PASS (`2037 / 2037` unit tests passed, `0` skipped, `0` failed)** — including repaired `unit/paletteSearchRecovery.test.ts` (`2/2` passed) and RFC 7396 `settings_patch` mock parity in `ui/src/lib/api/mock.ts:1069-1103`.
- **Playwright Browser Verification (`e2e/desktop-new-session-sizing.spec.ts`, slot 19):** **PASS (`4 / 4` cases passed)** across desktop and phone (`390×844`) light/dark, confirming actual rendered width/height `>= 36px` (`36px × 36px`) for all visible `NewSession` counter controls.
- **Playwright Browser Verification (`e2e/desktop-ux-r4-shell.spec.ts` `-g "Outputs refreshes|Activity applies|mounted Activity and Outputs watchers"`, slot 21):** **PASS (`3 / 3` cases passed, `7.4s`)**, confirming mounted `ActivityPanel` and `OutputsPanel` watchers reconcile delayed initial HTTP snapshots with in-flight WS deltas, auto-refresh on WS `resync` frames while mounted, skip unmounted panels during `resync`, and lazily rehydrate when reopened.
