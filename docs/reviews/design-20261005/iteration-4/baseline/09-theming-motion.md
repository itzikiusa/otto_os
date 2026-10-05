## Lens: Light/dark theming, vibrancy, motion, polish

**Score: 8.6/10** (10.0 − 0.3 for 1 major − 1.0 for 10 minors − 0.15 for 5 nits = 8.55). The token and ratchet sweeps clearly landed. Three things keep it from 9.8: the forceDark terminal islands only recolor the terminal body, motion and shadow values are still hand-typed instead of tokenized, and the hand-rolled glass and scrim values bypass the tokens.

Verified from code only. I did not build or screenshot anything. Paths are relative to `/Users/itziklavon/claude_ade-design/ui/src`.

### Credits and regression checks
- **Accent text:** no `color: var(--accent)` remains anywhere in `*.svelte`. This pass is clean.
- **Global reduced-motion:** `app.css:700-710` collapses animations and transitions. Specific overrides exist for the nav pending bar, `.spinner`, the agent-target ring, Drawer, Toasts and FloatingBar.
- **z-index tokens:** `tokens.css:40-50` defines the layers. `Lightbox` and the other overlays use `--z-modal`, and no stray three-digit z-index remains except the injected `browser/overlay.js:103`.
- **Reduced transparency:** it is handled through the glass tokens (`tokens.css:318-337`). The vault `GraphView`, `FloatingBar` and `NotificationBell` all honor it.
- **Canvas palettes:** Mermaid and D2 re-render on a scheme change (`canvas/MermaidCanvas.svelte:124,318`). The vault `GraphView` re-reads CSS variables through a `MutationObserver` (`vault/GraphView.svelte:1250`). The panels `FileTree` preview has a per-scheme palette (`panels/FileTree.svelte:229-236`). The git graph lane palette uses series tokens.
- **No regressions** seen in the shared components (Switch, Drawer, Skeleton, StatusDot).

### Findings

1. **[major] forceDark terminal islands only recolor the terminal body.**
   - Locations:
     - `lib/components/Terminal.svelte:3046-3048` and `3064-3066` (the only force-dark rules).
     - Overlays that stay on app-scheme tokens: `.find-bar` 3084-3098, `.find-results` 3127-3141, `.term-overlay` 3172-3186, `.badge` 3200-3207, `.ro-strip` 3216-3231, `.lat-hud` 3293-3311, `.desk-toolbar` 3325-3341, `.phone-btn` 3261-3280.
     - `browser/AgentDock.svelte:283` (`.agent-shell` is `var(--bg)`, so it flashes light before the dark terminal mounts).
   - Problem: in a light scheme these render as light chips and light find bars on a near-black terminal. The `.find-bar` input is dark text on a light surface that sits over dark glyphs, which looks like a bug. Foundations §1.1 says surfaces must follow the tokens, and forceDark breaks that. The `.force-dark-wrap` background is also hard-coded `#131318`, twice, instead of a token.
   - Fix: when `forceDark` is set, scope the wrapper to a dark token set. Add a `.force-dark-wrap` block that re-declares `--surface`, `--surface-2`, `--border`, `--text`, `--text-dim`, `--hover` and `--shadow` to their dark values, or reuse `html[data-scheme='dark']` values from `tokens.css`. Set the background from `--term-bg`.

2. **[minor] Four different "terminal black" panes.**
   - Locations: `modules/kubernetes/ExecView.svelte:155` (`#000`), `modules/kubernetes/ClusterWorkspace.svelte:817` (`#000`), `modules/product/AnalysisTab.svelte:1098` (`#1b1b1b`), `lib/components/Terminal.svelte:3047,3065` (`#131318`).
   - Problem: foundations says `--term-bg` is for terminal and log panes only. These hosts paint their own black behind a forceDark terminal, so the gutters and edges mismatch.
   - Fix: use one dark terminal token (such as `--term-bg-dark`) everywhere, or make `Terminal` fill its host.

3. **[minor] Hand-rolled `backdrop-filter` behind terminal output ignores reduced transparency.**
   - Locations: `lib/components/Terminal.svelte:3278-3279` (`.phone-btn`) and `3337-3338` (`.desk-toolbar`), both `blur(4px)`.
   - Problem: foundations §7 says vibrancy is for chrome only, never behind logs or editors. Raw `blur(4px)` bypasses `--glass-blur-raised`, so `data-transparency='reduced'` and `prefers-reduced-transparency` do not turn it off.
   - Fix: drop the blur and use opaque `--surface` at about 92%. Or use `var(--glass-tint-raised)` with `var(--glass-blur-raised)`.

4. **[minor] Duration and easing tokens are almost unused.**
   - Locations: `tokens.css:57-59` defines `--dur-fast`, `--dur-enter` and `--ease-out`, but only `lib/components/Toasts.svelte:68,80`, `app.css` and `modules/home/HomePage.svelte` use them. Literals elsewhere include:
     - Shell: `lib/components/Modal.svelte:181,200` (140/160 ms), `shell/Drawer.svelte:121,180,185`, `shell/Palette.svelte:643` (120 ms) and `663` (150 ms), `shell/NotificationBell.svelte:583`, `lib/components/FloatingBar.svelte:1007-1010` (180 ms).
     - Hover and press: about 150 literal `transition:` declarations across modules, ranging 80, 90, 100, 110, 120, 130, 140 and 150 ms (for example `modules/product/OverviewTab.svelte:2211-3320`, `modules/git/GraphView.svelte:4105-4481`, `lib/components/Switch.svelte:41,52`, `lib/components/StatusDot.svelte:40`).
   - Problem: foundations §8 sets hover and press at 120–140 ms and enter at 140–160 ms. The page-level product, git and database files run faster (80–110 ms) or slower, which gives uneven feel.
   - Fix: add `--dur-hover: 130ms` and replace the literals. Add a ui-guards ratchet on `transition:[^;]*\d+m?s` that does not use `var(--dur-`.

5. **[minor] Data-driven width and flex transitions break the motion rules.**
   - Locations (the ≤200 ms and "don't animate data changes" rules):
     - `modules/swarm/SwarmPage.svelte:957` (0.3 s) and `modules/kubernetes/MetricsView.svelte:230` (300 ms), both over the 200 ms cap.
     - `modules/product/PlanTab.svelte:732`, `modules/panels/ActivityPanel.svelte:535`, `modules/agents/history/HistoryPage.svelte:790`, `modules/agents/MissionControl.svelte:852`, `modules/usage/UsagePage.svelte:1447`.
     - `modules/git/GraphView.svelte:4380` (`flex`), `4749` and `4968`.
   - Problem: progress and usage bars animate on data updates, against foundations §8.
   - Fix: remove the transition on data bars. Keep it only for user-initiated panel resizes, at 130–160 ms.

6. **[minor] Local keyframes duplicate the global ones.**
   - Locations:
     - `spin`: `modules/vault/RefineDrawer.svelte:390`, `modules/git/ReviewPanel.svelte:1769`, `modules/api/ApiPage.svelte:604` (`req-tab-spin`).
     - `pulse`: `modules/skills-eval/SkillsEvalPage.svelte:567`, `modules/loops/LoopDetail.svelte:483`, `modules/git/RepoView.svelte:692`, `modules/design-hall/studio3d/Studio3D.svelte:893`, plus `pb-pulse` and `rail-pulse`.
     - `blink`: `modules/kubernetes/LogsView.svelte:453`, `modules/agents/conversation/LiveDraft.svelte:89`.
     - `slide`: `modules/kubernetes/InstallPanel.svelte:196`, `modules/aws/InstallPanel.svelte:127`.
     - Indeterminate bars: `modules/database/ExportDialog.svelte:376` (`exp-sweep`), `modules/database/ImportDialog.svelte:346` and `modules/connections/ConnectionImportDialog.svelte:504` (`imp-indet`).
     - `fade-in` redeclared in `lib/components/Modal.svelte:233` and `shell/Palette.svelte:910`, while `otto-fade-in` exists at `app.css:657`.
   - Problem: foundations says `otto-spin` and `otto-pulse` are the global primitives. Divergent copies drift in timing and in the reduced-motion shape.
   - Fix: use `otto-spin`, `otto-pulse`, `otto-fade-in` and one shared `otto-indeterminate`. Delete the local copies. Add a ui-guards rule for `@keyframes (spin|pulse|blink|slide|fade-in)`.

7. **[minor] Hover background vocabulary is split between `--hover` and `--surface-2`.**
   - Locations: `:hover{background:var(--surface-2)}` appears in about 84 declarations across 59 files, for example `modules/git/DiffViewer.svelte`, `modules/git/GitTabs.svelte`, `modules/aws/*`, `modules/kubernetes/*`, `shell/TabBar.svelte`. `:hover{background:var(--hover)}` appears in about 160 declarations across 122 files, for example the whole `modules/vault/*` set.
   - Problem: the same row hover reads differently in different modules. A row already sitting on `--surface-2` also gets a hover that is invisible.
   - Fix: row and list hover uses `--hover`. Keep `--surface-2` for buttons only, per foundations §1.2. Add a ratchet for `:hover` rules that set `background: var(--surface-2)` on rows.

8. **[minor] Selected and soft-accent fills are hand-rolled at varying opacities.**
   - Locations: about 431 `color-mix(in srgb, var(--accent) N%, transparent)` across 185 files, against about 129 uses of `var(--accent-soft)`. Examples: `modules/vault/FileTree.svelte:410` (18%) and `452,505` (12%), `modules/vault/NoteView.svelte:620` (16%) and `644` (8%), `modules/vault/SearchPanel.svelte:132` (30%), `modules/vault/GraphView.svelte:1731` (45%).
   - Problem: selection and active-row strength varies per module (8, 12, 14, 16, 18, 22, 30, 45). `--accent-soft` is 14%. Layout.md specifies about 11% for a nested active row.
   - Fix: add tokens `--accent-soft` (14%), `--accent-soft-strong` (22%, which is also the focus ring) and `--accent-hover`. Codemod the common values to them.

9. **[minor] Raw `rgba(0,0,0,…)` shadows are not scheme-aware.**
   - Locations:
     - `modules/product/ProductPage.svelte:1212`
     - `modules/product/MockupAnnotations.svelte:453,483`
     - `modules/design-hall/SpatialView.svelte:211,241`
     - `modules/design-hall/brand/ColorsSection.svelte:225`
     - `modules/design-hall/brand/AppliedPreview.svelte:197,246,268`
     - `modules/database/ResultsGrid.svelte:2354`
     - `modules/git/FocusView.svelte:921`
     - `modules/product/design/scene3d/Inspector.svelte:704` (`color-mix(black 25%)`)
     - `modules/design-hall/studio3d/StatesBar.svelte:188` (`color-mix(var(--text) 18%)`)
   - Problem: shadows should come from `--shadow`, `--shadow-card` or `--glass-shadow`, which `tokens.css:289-313` already tunes per scheme. A fixed 25% black on dark surfaces is nearly invisible, and `FocusView:921` is a floating panel with its own shadow.
   - Fix: replace each with `var(--shadow)` or `var(--shadow-card)`. For tiny pin or thumb shadows add a `--shadow-xs` token.

10. **[minor] Scrims are hard-coded although tokens exist.**
    - Locations: `modules/agents/conversation/Composer.svelte:644` (`rgba(0,0,0,0.6)`), `modules/agents/conversation/Lightbox.svelte:35` (`color-mix(#000 78%)`).
    - Problem: `--scrim-media: rgba(0,0,0,0.6)` is defined at `tokens.css:302`, and the Composer value equals it. Lightbox is a bespoke 78%.
    - Fix: use `var(--scrim-media)` in both. If the Lightbox needs a darker value, add `--scrim-strong`.

11. **[minor] z-index literals outside the in-pane range.**
    - Locations: `modules/agents/TiledView.svelte:603` (20) and `653` (21), `modules/agents/SplitNode.svelte:292` (25) and `367` (30).
    - Problem: foundations §6 restricts literals to 1–10 plus `--z-sticky`. 20–30 collide conceptually with `--z-floating-bar: 40` and the drawer layers.
    - Fix: renumber to 1–10 within a local stacking context (add `isolation: isolate` on the pane), or use `--z-sticky`.

12. **[nit] Vault `GraphView` glass floats over a live canvas.**
    - Locations: `modules/vault/GraphView.svelte:1632-1633` (legend) and `1671-1672` (panel).
    - Problem: it uses the right tokens and honors reduced transparency, but the blur sits over an animated canvas on every frame.
    - Fix: use an opaque `--surface` for the legend and panel, or keep the blur and accept the cost.

13. **[nit] Hard-coded canvas fallbacks in vault `GraphView`.**
    - Locations: `modules/vault/GraphView.svelte:198-203`, `211-214`, `1026` (`ghostFill` `#77777f` / `#9a9aa2`).
    - Problem: the dark-theme defaults are hex, and the `--surface` fallback differs from the initial `surface` (`#1c1c1e` vs `#2a2a30`). `dark` is derived from `data-scheme !== 'light'` instead of `ui.resolvedScheme`.
    - Fix: read the ghost fill from `--text-dim` at lower alpha and unify the defaults.

14. **[nit] `scrollbar-width: thin` overrides the custom scrollbar on some containers.**
    - Locations: `modules/vault/VaultPage.svelte:691`, `modules/browser/TabStrip.svelte:80`, `modules/product/DiscoveryChat.svelte:311`, `modules/database/QueryEditor.svelte:1575`, `modules/api/ApiPage.svelte:536`.
    - Problem: `app.css:74-91` styles `::-webkit-scrollbar` globally. In newer WebKit, setting `scrollbar-width` makes the `::-webkit-scrollbar` rules not apply to that element, so these use the native thin bar while the rest use the 10 px custom bar. This depends on the webview version, which I did not check.
    - Fix: drop `scrollbar-width: thin` where the global bar is acceptable. Otherwise add a global `scrollbar-width`/`scrollbar-color` so both paths match.

15. **[nit] A few focus rings use `--accent` instead of the global `--accent-text` ring.**
    - Locations: `modules/agents/conversation/ConversationView.svelte:1087-1089` (inset `box-shadow` with `var(--accent)`), `modules/workflows/WorkflowsPage.svelte:3408-3412` (`outline: none` on `:focus-visible`, with a gradient background instead).
    - Problem: accessibility.md §2 specifies the 2 px opaque `--accent-text` ring. In Warm light, `--accent` is the lower-contrast value.
    - Fix: use `outline: 2px solid var(--accent-text)` with `outline-offset: -2px` for inset cases.

16. **[nit] `ContextMenu` has no enter animation.**
    - Locations: `lib/components/ContextMenu.svelte` (only a hover transition at `366`). `NotificationBell` has `nb-in` (140 ms, `shell/NotificationBell.svelte:583-586`) and `Palette` has `pal-in`.
    - Problem: popovers fade in, but context menus pop instantly.
    - Fix: add a shared 120–140 ms `otto-fade-in` to the menu.

### What would get this lens to 9.8
- Make forceDark a real scoped token island: re-declare the dark token set on `.force-dark-wrap`, move the `#131318` and `#000` panes to one terminal token, and remove the blurred chips over terminal output (findings 1, 2, 3).
- Tokenize motion: `--dur-hover`, `--dur-enter` and `--ease-out` everywhere, one global keyframe set, no animated data bars. Add ui-guards ratchets for literal `transition` durations and local keyframe names (4, 5, 6).
- Collapse the elevation, scrim and overlay values to tokens: `--shadow-xs`, `--scrim-media`, `--scrim-strong` and a single hover token (7, 9, 10).
- Add `--accent-soft-strong` and `--accent-hover`, then codemod the 400-plus hand-written accent mixes so selection strength is consistent (8).
- Move the stray z-index values into the pane range or onto tokens, and unify popover enter motion (11, 16).
- After these changes, run the light and dark screenshot checklist on terminal-heavy pages (AgentDock, k8s exec and k9s, product Analysis).
