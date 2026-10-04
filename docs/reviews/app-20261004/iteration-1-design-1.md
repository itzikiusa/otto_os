# Iteration 1 — design reviewer 1: shell and sessions

Baseline: `a16f4c71`. Read-only review of shell/navigation, Agents, session headers, tiled sessions, conversation composer and previews. No application changes, builds, servers or tests were run by this reviewer.

Read `AGENTS.md` and the design guidelines README, foundations, layout, components, accessibility and review checklist. Read the coordination record and compared the relevant files in `/Users/itziklavon/claude_ade-design` against the baseline. The findings below are not covered by that branch's changes at review time. Shared styles, RTL, labels/tablist work, shared dialogs, Lightbox, navigation overlays, status/copy and trust confirmations are deliberately excluded.

## Evidence and coverage

Visually inspected `/tmp/otto-review-load-baseline-fixed/shot-n0-tiled.png` and `shot-n1-tiled.png`: Native light, 1600×1000, expanded navigation, empty Agents tiled state and one terminal session. Those frames show quiet grouped navigation, a clear empty-state action, readable terminal content and a compact session header. They do not demonstrate the three defects below; those findings are source-supported and need the targeted visual/keyboard checks described below.

The original `/tmp/otto-review-load-baseline` run had no screenshots; the parent supplied the corrected location. Dark/Warm themes, phone, VoiceOver, zoom and populated chat were not visually tested in this pass. This is not a visual sign-off of the whole session experience.

## Findings

### D1-01 — P2: a long draft can consume or overflow a short chat tile

**Location:** `ui/src/modules/agents/conversation/Composer.svelte:91`, `:372`, `:414`; `ui/src/modules/agents/SessionView.svelte:1086`; `ui/src/modules/agents/TiledView.svelte:92`.

**Evidence:** textarea autosizing caps against `window.innerHeight * 0.4`, and CSS also uses `max-height: 40vh`. The composer cannot shrink (`flex-shrink: 0`). However, tiled rows can be only 220px high, and the session pane clips overflow. At a 900px window height, a sufficiently long draft can request a 360px textarea alone, before the tool row, status line and two chat/session headers. A short tile cannot contain it: the conversation list loses its available height and the controls can extend beyond the clipped pane. Resizing a tile also does not itself trigger `autosize`, which runs on input/draft changes.

**Impact/guideline:** users drafting a longer instruction in tiled view lose the preceding conversation and can lose pointer access to Send/Stop. Workbench layouts must adapt to their pane, keep internal content scrollable and preserve reachable controls (`layout.md` §4.2, accessibility §7).

**Fix direction:** calculate the textarea budget from the chat pane's available block size, reserve space for the headers/tool/status rows and a useful conversation viewport, and let the textarea scroll at that cap. Recalculate when the pane resizes, including restoring a multiline draft. Remove the window-relative CSS cap or align it with the pane budget.

**Verification:** extend `ui/e2e/desktop-conversation-chat.spec.ts:461` beyond its current width-only six-tile assertions. In 3×2 and 3×3 layouts at 1280×900, fill one composer with at least 30 wrapped lines; assert the conversation list retains usable height and Send/Stop remain completely inside the tile and hit-testable. Repeat after shrinking a row and restoring the draft. Capture light/dark screenshots. No reproduction run was performed in this review.

### D1-02 — P2: slash completion is not vertically clamped to its clipping pane

**Location:** `ui/src/modules/agents/conversation/Composer.svelte:475`; `ui/src/modules/agents/SessionView.svelte:1086`.

**Evidence:** the popup is absolutely placed above the composer with `bottom: calc(100% + 6px)` and `max-height: min(320px, 50vh)`. No measurement restricts it to the space between the composer and the enclosing pane's top. The pane clips overflow. With a full 12-item result list in a short tiled row, the popup extends above that clipping boundary; its initial/top options can be hidden even though the popup itself has a scrollbar. This is a different trigger from D1-01: a single `/` in an otherwise empty composer is sufficient.

**Impact/guideline:** visible command choices and keyboard selection diverge because the selected option can be outside the visible clip. This violates the explicit clamp-and-cap rule in `AGENTS.md` and `components.md` §9. The source comment saying the popup is clamped to the pane is not implemented vertically.

**Fix direction:** measure available space above the composer against both the pane and viewport, and cap the scrollable popup to it; alternatively portal the popup and clamp its coordinates to the viewport. Keep the selected option scrollable into the actual visible region.

**Verification:** extend `ui/e2e/desktop-conversation.spec.ts:357`, which currently opens `/comp` and checks a single match. Seed 12 commands, type `/` in a top-row 220–280px chat tile, and assert the popup's full bounds are inside its clipping pane as well as the window. Move selection through all options and check the active row is visible/hit-testable. Capture light/dark screenshots. No reproduction run was performed in this review.

### D1-03 — P2: narrow file previews leave keyboard focus in the covered conversation

**Location:** `ui/src/modules/agents/conversation/ConversationView.svelte:189`, `:762`, `:1045`; `ui/src/modules/agents/conversation/PreviewPanel.svelte:91`.

**Evidence:** below a 760px conversation width, `.pv-slot` becomes an absolute overlay covering `.conv-main`. `openPreview` only assigns the request. The underlying `.conv-chat` remains mounted and focusable, without `inert` or another focus exclusion. The preview does not move focus on mount. Activating a file reference with Enter therefore leaves focus on that now-covered reference; subsequent Tab navigation still encounters covered conversation controls before reaching the preview later in document order. Closing always focuses the list rather than the initiating reference.

**Impact/guideline:** a keyboard user sees a file preview but continues navigating an invisible chat. Focus is not visibly aligned with the displayed work surface. Accessibility §2 requires visible focus and sensible focus order; §3 requires keyboard access to the whole flow. This is the inline narrow preview, separate from the already-assigned full-view Modal and Lightbox work.

**Fix direction:** for the covering layout, move focus into the preview, exclude the covered chat from focus/accessibility navigation, and restore the initiating control on close where it still exists. Handle crossing the width breakpoint while a preview is open. Preserve normal access to both columns in the side-by-side layout.

**Verification:** extend `ui/e2e/desktop-conversation-chat.spec.ts:324`, which currently checks pointer-opened preview width and pointer close only. Open a file reference with Enter in a narrow pane; assert focused controls are visible in the preview, Tab never enters the covered composer, Escape closes, and focus returns to the trigger. Resize across 760px while open and verify both-column keyboard access returns in the wide layout. Add a VoiceOver spot check. No keyboard reproduction was run in this review.

## Handoff

Forward these three issues to the coordinated visual-design owner. No new shell/navigation redesign is proposed. The existing screenshot observations are limited to light desktop empty/terminal states; targeted chat screenshots and keyboard verification remain pending.
