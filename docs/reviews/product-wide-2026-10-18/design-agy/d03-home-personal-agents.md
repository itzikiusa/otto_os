# D03 — Home Dashboard, Live Boxes & Otto School 3D UX

- **Reviewer Subagent ID:** `6ffee431-e042-4c66-9ab9-871e250b349c` (`D03 Home & Personal Agents UX Reviewer`)
- **Lane Lead:** `Ronaldo` (`agy`)
- **Scope:** `ui/src/modules/home/*` (`HomePage.svelte`, `HomeBox.svelte`, `K8sBox.svelte`, `MissionControlBox.svelte`, `InsightsBox.svelte`, `UsageBox.svelte`, `ClassroomsBox.svelte`, `school/*`)
- **Iteration 1 Baseline Score:** **8.3 / 10**
- **Iteration 2 Post-Fix Score:** **9.8 / 10**

---

## 1. Confirmed Findings & Verification

### D03-01 (High — Fixed) — Silent refresh failures across `HomePage.svelte`, `K8sBox.svelte`, `MissionControlBox.svelte`, and `InsightsBox.svelte`
- **Location:** `ui/src/modules/home/HomePage.svelte:77`, `K8sBox.svelte:38,67`, `MissionControlBox.svelte:38`, `InsightsBox.svelte:26`
- **Issue:** Failed initial or poll fetches caught errors with `/* keep last good */` and rendered misleading empty states (`No clusters registered`, `Nothing active on the graph`, `No insights report yet`) with no error banner or Retry button.
- **Fix Applied:** Added `loadErrorText(e)` state and inline error banners with Retry actions across `HomePage.svelte`, `K8sBox.svelte`, `MissionControlBox.svelte`, and `InsightsBox.svelte`.

### D03-02 (High — Fixed) — Global `.row` and `.card` class collisions in `HomePage.svelte` and `UsageBox.svelte`
- **Location:** `ui/src/modules/home/HomePage.svelte:481`, `ui/src/modules/home/UsageBox.svelte:64`
- **Issue:** `HomePage.svelte` used `<div class="row">` for customize-drawer rows and `UsageBox.svelte` used `<div class="card">` for KPI cells, inheriting global padding, border-radius, and flex gap rules from `ui/src/app.css`.
- **Fix Applied:** Renamed `.row` to `.cust-row` in `HomePage.svelte` and `.card` to `.ub-kpi` in `UsageBox.svelte`.

### D03-03 (Medium — Fixed) — `school/scene.ts` ignored `prefers-reduced-motion: reduce`
- **Location:** `ui/src/modules/home/school/scene.ts:439, 987`
- **Issue:** The WebGL `requestAnimationFrame` loop ran continuously at 60 FPS with walking characters, bobbing animations, and orbiting cameras even when `prefers-reduced-motion: reduce` was enabled.
- **Fix Applied:** Added a `matchMedia('(prefers-reduced-motion: reduce)')` check in `school/scene.ts` that freezes bobbing/walking delta-time (`dt = 0`) and disables camera auto-bobbing when reduced motion is active.

### D03-04 (Medium — Fixed) — `HomeBox.svelte`, `ClassroomsBox.svelte`, and `UsageBox.svelte` keyboard accessibility, contrast, and RTL drag/reorder
- **Location:** `ui/src/modules/home/HomeBox.svelte:70,136,158`, `ClassroomsBox.svelte:353,759`, `UsageBox.svelte:102`
- **Issue:** `HomeBox` resize handles were mouse-only `<div onmousedown>` elements with no keyboard sizing or RTL inversion; `.grip` used `opacity: 0.45` (failing WCAG 4.5:1 contrast); `ClassroomsBox` `.stage:focus-visible` used `color-mix(in srgb, var(--accent) 55%, transparent)` (failing 3:1 non-text contrast); and `UsageBox` range tabs lacked roving `tabindex`.
- **Fix Applied:** Converted resize handles to keyboard-accessible `role="separator"` controls with arrow-key resizing and RTL delta inversion, removed low-contrast opacity on `.grip`, changed `.stage:focus-visible` to `2px solid var(--focus-ring)`, and added roving `tabindex` to `UsageBox` and `ClassroomsBox` tablists.
