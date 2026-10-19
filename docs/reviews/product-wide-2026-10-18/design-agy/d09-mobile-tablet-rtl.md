# D09 — Responsive Layout, Mobile/Tablet (iPhone/iPad) & RTL UX

- **Reviewer Subagent ID:** `8421e3db-284e-42e9-86c3-7d53d049496f` (`D09 Mobile Tablet & RTL UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** Mobile (375px–430px), Tablet (768px–834px), and RTL (`dir="rtl"`) across `ui/src/app.css`, `ui/src/lib/tokens.css`, `ui/src/shell/*`, `ui/src/modules/home/*`, `ui/src/modules/panels/*`, `ui/src/modules/agents/*`
- **Iteration 1 Baseline Score:** **8.5 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D09-01 (High — Fixed) — Physical horizontal drag deltas and keyboard resize directions inverted under `dir="rtl"`
- **Location:** `ui/src/shell/Navigator.svelte:293-305`, `ui/src/shell/RightPanel.svelte:218-230`, `ui/src/modules/agents/TiledView.svelte:160-241`, `ui/src/modules/agents/SplitNode.svelte:92-145, 195-210`, `ui/src/modules/home/HomeBox.svelte:62-75`
- **Issue:** Dragging sidebar resizers, tiled session column gutters, split-pane vertical dividers, or Home box width handles used physical `e.clientX - startX` and `ArrowLeft`/`ArrowRight` without checking `document.documentElement.dir === 'rtl'`. In RTL layout, dragging toward the inline-end shrank panes instead of growing them, and `SplitNode.svelte` drop zones mapped `'left'`/`'right'` to inverted halves.
- **Fix Applied:** Added RTL direction detection (`document.documentElement.dir === 'rtl'`) across `Navigator.svelte`, `RightPanel.svelte`, `TiledView.svelte`, `SplitNode.svelte`, and `HomeBox.svelte`, inverting horizontal pointer deltas, arrow-key resize steps, and drop-zone horizontal fractions, plus logical `nesw-resize` cursor in RTL on `.tile-corner`.

### D09-02 (High — Fixed) — `TiledView.svelte` allowed 3–4 columns on 375px phone viewports
- **Location:** `ui/src/modules/agents/TiledView.svelte:91-98`
- **Issue:** `cols` was clamped to `1..4` regardless of viewport width, squashing 4 terminal/chat tiles into ~85px columns on mobile phones.
- **Fix Applied:** Added `effectiveCols` clamping (`1` column on phone, at most `2` columns on tablet, `1..4` on desktop).

### D09-03 (Medium — Fixed) — Physical `border-left` on `.session-row.needs-you` and swipe direction in `HomePage.svelte`
- **Location:** `ui/src/shell/Navigator.svelte:1058`, `ui/src/modules/home/HomePage.svelte:182`
- **Issue:** `.session-row.needs-you` used physical `border-left: 2px solid var(--warning)`, and `HomePage.svelte` horizontal swipe navigation between tabs did not invert swipe delta in RTL.
- **Fix Applied:** Converted `.session-row.needs-you` to `border-inline-start: 2px solid var(--warning)` and made `HomePage.svelte` swipe navigation RTL-aware.
