# D01 — Global Navigation, Shell, Command Bar & Window Chrome

- **Reviewer Subagent ID:** `96992f1f-20bf-4340-b22a-b259291313e6` (`D01 Shell & Navigation UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** `ui/src/App.svelte`, `ui/src/shell/*`, `ui/src/lib/sidebar.ts`, `ui/src/lib/components/{FloatingBar,PageHeader,PageBody,Modal,ContextMenu,ToastHost,ConfirmHost,EmptyState}.svelte`
- **Iteration 1 Baseline Score:** **8.6 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D01-01 (High — Fixed) — `FloatingBar.svelte` missing `html[data-transparency='reduced']` fallback
- **Location:** `ui/src/lib/components/FloatingBar.svelte:1076-1088`
- **Issue:** `.fbar`, `.fbar-menu`, and `.fbar-sheet` handled `@media (prefers-reduced-transparency: reduce)` (`1076-1082`) when the OS toggle was on, but ignored `:global(html[data-transparency='reduced'])` when the user enabled **"Reduce transparency"** inside Otto's Settings → Appearance (`ui/src/lib/stores/prefs.svelte.ts:288`).
- **Fix Applied:** Added `:global(html[data-transparency='reduced']) .fbar, :global(html[data-transparency='reduced']) .fbar-menu, :global(html[data-transparency='reduced']) .fbar-sheet { background: var(--bg-elevated); backdrop-filter: none; -webkit-backdrop-filter: none; }` at `ui/src/lib/components/FloatingBar.svelte:1083-1088`.

### D01-02 (High — Fixed) — `Palette.svelte` (`⌘K`) omitted from `modalKeyVerdict`, allowing `⌘W` to close background session tabs
- **Location:** `ui/src/shell/App.svelte:119-125, 628, 701`
- **Issue:** `openModalKind()` checked `ui.newSessionOpen`, `ui.handoverSessionId`, `ui.shareSessionId`, and `ui.shortcutsOpen`, but omitted `ui.paletteOpen`. Pressing `⌘W` while `⌘K` was open closed the active background session tab instead of closing the command palette.
- **Fix Applied:** Added `ui.paletteOpen` (`'palette'`) to `openModalKind()` and handled `'palette'` in `App.svelte` key routing so `⌘W` cleanly dismisses the palette.

### D01-03 (Medium — Fixed) — Destructive confirmations in `Navigator.svelte` and `NotificationBell.svelte` missing `danger: true`
- **Location:** `ui/src/shell/Navigator.svelte:385, 398, 421`; `ui/src/shell/NotificationBell.svelte:74-78`
- **Issue:** Closing all tabs in a workspace, deleting a workspace, killing a terminal session, and clearing all notifications called `confirmer.ask(...)` without `danger: true`, rendering primary blue confirmation buttons instead of `--danger` red.
- **Fix Applied:** Added `danger: true` to all four destructive confirmation calls.

### D01-04 (Medium — Fixed) — `Navigator.svelte`, `RightPanel.svelte`, and `NotificationBell.svelte` context menus opened at cursor coordinates instead of button anchor
- **Location:** `ui/src/shell/Navigator.svelte:342, 1254-1260`, `ui/src/shell/RightPanel.svelte:205`, `ui/src/shell/NotificationBell.svelte:88`
- **Issue:** Trigger buttons called `ctxMenu.show(e.clientX, e.clientY, ...)` rather than anchoring to `e.currentTarget` via `ctxMenu.showAt(...)`, causing keyboard activation (`Enter`/`Space`, where `clientX === 0, clientY === 0`) to open the menu at `(0, 0)`. Additionally, `.sort-toggle` in `Navigator.svelte:1254-1260` had a redundant `onkeydown` handler passing a `KeyboardEvent` to `openSortMenu(e: MouseEvent)`.
- **Fix Applied:** Switched all button-triggered menus to `ctxMenu.showAt(e.currentTarget as HTMLElement, ...)` and removed the redundant `onkeydown` handler on `<button class="sort-toggle">` (`Navigator.svelte:1254-1260`) since native `<button onclick>` fires on `Enter` and `Space`.

### D01-05 (Low — Fixed) — `Palette.svelte`, `RightPanel.svelte`, and `pages.svelte.ts` raw error strings and backdrop drag-close
- **Location:** `ui/src/shell/Palette.svelte:11, 56, 101, 462`, `ui/src/shell/RightPanel.svelte:159`, `ui/src/shell/pages.svelte.ts:156`
- **Issue:** Error banners interpolated raw `String(e)` instead of `loadErrorText(e)`, and `Palette.svelte` closed when a text selection drag started inside `.pal` and released on `.pal-backdrop`.
- **Fix Applied:** Routed errors through `loadErrorText(e)` across `Palette.svelte`, `RightPanel.svelte`, and `pages.svelte.ts:156`, and added `downOnBackdrop` pointer tracking on `.pal-backdrop`.

### D01-06 (Low — Fixed) — `TabBar.svelte` ARIA hierarchy and `App.svelte` icon-button label/title parity
- **Location:** `ui/src/shell/TabBar.svelte:332-397`, `ui/src/shell/App.svelte:441`
- **Issue:** `<div class="tab">` sat directly inside `role="tablist"` without `role="presentation"` wrapping both the `role="tab"` button and `.tab-close`; `.pb-dismiss` had mismatched `title` and `aria-label`.
- **Fix Applied:** Wrapped each tab unit in `<div role="presentation">` in `TabBar.svelte:332-397` and aligned `title`/`aria-label` across `App.svelte`, `Navigator.svelte`, `RightPanel.svelte`, and `NotificationBell.svelte`.
