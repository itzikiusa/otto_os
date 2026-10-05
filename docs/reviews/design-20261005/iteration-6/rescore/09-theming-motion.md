## Lens: Light/dark theming, vibrancy, motion, polish — iteration-6 re-score

**Score: 10.00/10** (10.00 − 0 blockers − 0 majors − 0 minors − 0 nits = 10.00)

Judgement: I'd hold this at about 9.9 in practice. The 10.00 above is the fixed-rubric arithmetic. The remaining 75 `var(--accent) N%` mixes that still use a non-transparent base are an unaudited residual. Judgement is recorded but not used.

Checked by grep in `/Users/itziklavon/claude_ade-design6/ui/src` (paths below are relative to it), read-only.

### Items left open or partial in the iteration-5 re-score

| # | Item | Status | Evidence |
|---|---|---|---|
| 5 | Data-bar width transitions | **Fixed** | `transition: width` and `flex` now match only `modules/git/GraphView.svelte:4406`. It is marked user-driven (the detail pane opening) at `--dur-fast`, so it is legitimate. `UsagePage`, `AttributionDrilldown`, `HistoryPage` and `MissionControl` no longer animate their data bars. |
| 8 | Hand-rolled accent mixes (partial) | **Fixed** | `color-mix(in srgb, var(--accent) N%, transparent)` has 0 matches, down from 240. The `--accent-soft` (14%) and `--accent-soft-strong` (22%) tokens are defined at `lib/tokens.css:108,111`, and re-declared in the force-dark island at `392-393`. |
| New | `DoneContractMeter` literal 240 ms transition (nit) | **Fixed** | No `transition: stroke-dasharray` or `stroke-dasharray <n>` match remains in `*.svelte`. |

### Spot-checks on earlier Fixed items
- **Hover vocabulary:** `:hover { background: var(--surface-2) }` has 0 matches, down from 4. The four `aws` and `kubernetes` leftovers were converted.
- **Keyframes:** the only `@keyframes` in `*.svelte` is `git/GraphView.svelte:4541` (`row-pulse`), with a ui-guards allow.
- **Terminal panes:** no `background: #000` or `#1b1b1b` remains.
- **Shadows and mask gradients:** `rgba(0,0,0` appears only in the mask gradients (`agents/conversation/Markdown.svelte:360-361`) and the `product/design/DeviceFrame.svelte:85` device mock. Both are acceptable.
- **Backdrop blur:** `backdrop-filter` appears only in `lib/components/FloatingBar.svelte:1044-1045,1070-1079` (token-driven with an `@supports` fallback) and a comment in `shell/NotificationBell.svelte:10`. No hand-rolled blur remains.
- **forceDark island:** `lib/tokens.css:369` is intact. It is applied at `lib/components/Terminal.svelte:2793`, and `modules/browser/AgentDock.svelte:179` now puts `otto-force-dark` on `.agent-shell`, so the pre-mount frame is already dark (my iteration-3 point is covered).
- **Duration tokens:** no regression found. No literal `transition:` durations appear in `*.svelte`.

### New findings
None. One thing to watch, not scored: the residual 75 `color-mix(in srgb, var(--accent) N%, <solid>)` mixes (for example `git/DiffViewer.svelte:1921,1968`) blend onto `--bg` or `--surface` rather than `transparent`. They are a different, mostly one-off pattern and are not covered by the `--accent-soft` tokens.

### Remaining to reach 9.8
The lens is at ceiling by the fixed rubric, so there is nothing required. Optional polish:
- Audit the 75 solid-base accent mixes and move the repeated ones (5–8% on `--bg` / `--surface`) to a shared `--accent-wash` token.
- Add a ui-guards ratchet on `@keyframes` outside `app.css`, `transition:` with literal ms, and `scrollbar-width: thin`, so the clean state holds.
- Do the light and dark screenshot pass on terminal-heavy pages (AgentDock, k8s exec and k9s, product Analysis) to confirm the force-dark island visually.
