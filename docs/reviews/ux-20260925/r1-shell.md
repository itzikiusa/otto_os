# Round 1 — independent shell / Home / sessions UX review

Reviewer scope: shell/navigation, Home, Agents session chrome and panels, command surfaces, shared dialogs/menus. Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. Original checkout and reserved branch left untouched. Read AGENTS.md and design layout/components/accessibility/review checklist. Source audit followed by actual Chromium reproduction against the isolated test daemon on 7812; no production sessions used.

## Pre-fix scores

| Layout | Interaction | Accessibility | States | Responsive | Mean |
|---|---|---|---|---|---|
| 8.8 | 7.6 | 6.8 | 8.0 | 8.5 | 7.94 |

These are scoped scores, not an app-wide certification. A 9.5 score is unjustified: panel failure states and broader real populated session flows remain unverified.

## Confirmed and repaired

1. **P1 — mobile Navigator keyboard focus stayed behind its modal backdrop.** Original `ui/src/shell/Drawer.svelte:35–49,58–79` only listened for Escape, with no move/trap/return. Browser reproduction: 390×844 Home, focus Open navigator, press Enter; focus remained outside Navigator. Fixed with shared `ui/src/lib/dialogFocus.ts:3` action, attached to Drawer; it moves focus, wraps Tab/Shift+Tab only for the top visible dialog and restores the trigger after dismissal.
2. **P1 — Plain English palette let Tab leave its aria-modal surface.** Original `ui/src/shell/Palette.svelte:261–292,419` managed only input shortcuts. Browser reproduction: enter request, focus Plan it, Tab; active element leaves dialog. Escape from option buttons also lacked a handler. Uses the same focus action; inner Escape consumes its event so clearing a proposed plan cannot accidentally also close the surface. Textarea now has a proper accessible name.
3. **P2 — shared text prompts exposed unnamed fields.** `ui/src/lib/components/ConfirmDialog.svelte:25` input had neither label nor aria-label; browser accessible name for New space input was the empty string. It now uses prompt message (fallback title) for the accessible name. This fixes create/rename prompts throughout the app without changing their visible wording.
4. **P2 — Home changed spaces behind an open modal.** Original `ui/src/modules/home/HomePage.svelte:60` unconditionally acted on global horizontal arrows outside fields. Browser reproduction: First/Second spaces, open Add widget, focus Close, ArrowRight; First became unselected while the modal remained open. Shortcut handler now respects consumed events, overlays, drawers and context menus.
5. **P2 — Home's N more links navigated away from the promised data.** Original `ui/src/modules/home/HomeToday.svelte:70–83,131` hard-coded Agents for mixed Needs-you rows, Design Hall for recent PR rows, etc. Browser reproduction: seed five MCP approvals, click 2 more; route navigates to Agents and approvals disappear. Each card now expands its remaining rows in place, with Show less and aria-expanded. Every row preserves its own source-specific navigation.

## Evidence and verification

- New `ui/e2e/desktop-ux-shell.spec.ts` first ran red: all five above failed on their stated UX assertions, not setup.
- After fixes, those same five passed in 29.4s.
- Failure screenshots and traces: `/tmp/otto-ux-shell-results/`.
- Fixed run: `/tmp/otto-ux-shell-results-fixed/`.
- `npx svelte-check --tsconfig ./tsconfig.app.json`: **0 errors, 0 warnings**, log `/tmp/otto-ux-shell-svelte-check.log`.
- Broader verification now includes existing Home creation/switch/resize/zoom tests, shared dialog/token tests, plus native light, native dark and Warm dark phone RTL screenshots with long prompt values. **Final result: 17 passed (1.6m), including all new regressions and existing Home/dialog/theme checks.**

## Additional source-only concern for a later reviewer

**P2 — Outputs artifact-list fetch errors look like successful empty results.** `ui/src/lib/stores/activity.svelte.ts:99–110` catches errors without exposing any state; `ui/src/modules/panels/OutputsPanel.svelte:192` renders “Nothing produced yet” whenever the list is empty. A failing `/sessions/{id}/artifacts` is therefore indistinguishable from a session producing no outputs, and there is no Retry for the list (preview fetches do have Retry). Reproduce by intercepting the list endpoint with 500 for a populated agent. Not repaired or claimed browser-confirmed in this pass.

## Coverage limits

No VoiceOver/manual macOS Tauri run. Browser fixtures cover the shared interactions and constrained layouts; no claim of exhaustive live session behavior or color-contrast coverage across every provider/output type. Source reviewed HomeToday/HomePage/HomeBox, Drawer/Palette/TabBar/RightPanel, Modal/ConfirmDialog/ContextMenu/FloatingBar, AgentsPage/SessionView and Files/Outputs panels. Reserved PageHeader changes were not edited.
