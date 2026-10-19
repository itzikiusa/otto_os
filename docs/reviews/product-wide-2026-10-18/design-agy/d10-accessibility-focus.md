# D10 — Accessibility, Keyboard Navigation, Focus Rings, Reduced Motion & Design Guards

- **Reviewer Subagent ID:** `923e9e31-931e-42d3-8ec4-a9afebfd4548` (`D10 Accessibility & Design System Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** `scripts/ui-guards.mjs`, `ui/src/lib/tokens.css`, `ui/src/app.css`, `ui/src/shell/*`, `ui/src/modules/home/*`, `ui/src/modules/panels/*`, `ui/src/lib/components/*`
- **Iteration 1 Baseline Score:** **8.5 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D10-01 (High — Fixed) — Keyboard-inaccessible tree rows, resize separators, and tablists across `FileTree.svelte`, `HomeBox.svelte`, `BrowserPanel.svelte`, `UsageBox.svelte`, and `ClassroomsBox.svelte`
- **Location:** `ui/src/modules/panels/FileTree.svelte:274, 348`, `ui/src/modules/home/HomeBox.svelte:158`, `ui/src/modules/panels/BrowserPanel.svelte:205`, `ui/src/modules/home/UsageBox.svelte:102`, `ui/src/modules/home/ClassroomsBox.svelte:353`
- **Issue:**
  1. `FileTree.svelte` attached `ondblclick` to `<nav class="ft-crumbs">` (flagged by `ui-guards.mjs` `click-on-non-interactive`) and lacked `aria-expanded` and arrow-key navigation on `.tree-row`.
  2. `HomeBox.svelte` resize handles were mouse-only `<div onmousedown>` elements with no keyboard operable `role="separator"`.
  3. `UsageBox.svelte`, `ClassroomsBox.svelte`, and `BrowserPanel.svelte` tablists lacked roving `tabindex={active ? 0 : -1}`.
- **Fix Applied:** Moved `ondblclick` onto `.ft-crumb` buttons, added `aria-expanded` and arrow-key tree navigation in `FileTree.svelte`, converted `HomeBox.svelte` resize handles into keyboard-operable `role="separator"` elements, and added roving `tabindex` across `UsageBox.svelte`, `ClassroomsBox.svelte`, and `BrowserPanel.svelte`.

### D10-02 (Medium — Fixed) — Non-text contrast (WCAG 1.4.11) on custom form controls and focus rings
- **Location:** `ui/src/modules/panels/ActivityPanel.svelte:218`, `InfoPanel.svelte:337`, `CanvasPanel.svelte:258`, `ui/src/modules/home/ClassroomsBox.svelte:759`, `ui/src/modules/agents/Handover.svelte:232`
- **Issue:** Custom `<input>` and `<select>` controls used `border: 1px solid var(--border)` (`#d1d1d6` on `#ffffff` = 1.53:1 contrast) instead of `var(--control-border)` (3.08:1 contrast); `ClassroomsBox` `.stage:focus-visible` used 55% transparent accent outline; `Handover.svelte` target buttons lacked `:focus-visible` outlines.
- **Fix Applied:** Switched form control borders to `var(--control-border)`, updated `.stage:focus-visible` to `2px solid var(--focus-ring)`, and added `:focus-visible` outlines in `Handover.svelte`.

### D10-03 (Medium — Fixed) — Icon-only button `aria-label` and `title` parity across `shell/`, `home/`, `panels/`, and `agents/`
- **Location:** `App.svelte`, `Navigator.svelte`, `RightPanel.svelte`, `NotificationBell.svelte`, `FileTree.svelte`, `BrowserPanel.svelte`, `HistoryPage.svelte`
- **Fix Applied:** Audited and aligned `title` and `aria-label` on all icon-only controls across `shell/`, `home/`, `panels/`, and `agents/`.

### D10-04 — Design Guard Ratchet (`node scripts/ui-guards.mjs`)
- Running `node scripts/ui-guards.mjs` passes with **zero regressions** and reduces guard debt below baseline in **9 file/rule pairs** (including `FileTree.svelte:click-on-non-interactive` 1 → 0 and `no-global-class` reductions across `ActivityPanel.svelte`, `InfoPanel.svelte`, `HistoryPage.svelte`, `TiledView.svelte`, `HomePage.svelte`, `UsageBox.svelte`).
