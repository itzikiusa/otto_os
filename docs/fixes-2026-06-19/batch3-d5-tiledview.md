# D5 — TiledView no longer resurrects every suspended session

## Problem

`TiledView` rendered one `<SessionView>` per session in `ws.mainSessions`. Each
`SessionView` mounts a `<Terminal>`, and `Terminal` opens a live WebSocket to
`/ws/term/{id}` on its first valid fit (`Terminal.svelte:291–294 → connect()`).
On attach, the daemon's `ensure_live` **resumes** the session.

Net effect: opening the tiled view with M suspended sessions immediately opened
M WebSockets and woke all M agents at once (~200 MB/agent), defeating the
idle-suspend memory design.

## Fix — live-tile budget (IntersectionObserver + cap)

Files changed (only my two):
- `ui/src/modules/agents/TiledView.svelte`
- `ui/src/modules/agents/SessionView.svelte` (unchanged — the fix lives entirely in
  TiledView; SessionView/Terminal already tear down their WS on unmount)

A tile gets a live `SessionView` (and therefore a WS) only if it is in `liveIds`,
computed (in priority order, capped at `MAX_LIVE_TILES = 6`):

1. **The focused/active tile** — always live. Keeps the normal single-attach path
   intact: focusing a tile (or the active session) is never demoted to a placeholder.
2. **Pinned tiles** — sessions the user explicitly attached via the placeholder's
   "click to attach" button. Survive scrolling.
3. **Visible tiles** — sessions currently intersecting the grid viewport, in grid
   order, until the budget fills.

Everything else renders a lightweight **placeholder** (`.tile-placeholder`): a pane
header (StatusDot + title + provider chip) and a body with a terminal icon and a
"Click to attach" call-to-action. The placeholder opens **no terminal, no WebSocket,
no resume**.

### Visibility tracking
An `IntersectionObserver` rooted on the grid scroll container watches each tile slot
(`use:observeTile`, keyed by `data-tile-id`). Tiles entering/leaving the viewport
update a `visible` Set, which feeds `liveIds`. The observer effect re-creates when the
grid element binds and disconnects on teardown. Pins/visibility for sessions that no
longer exist are pruned so the Sets never leak or count stale ids against the budget.

### Attach affordance
Clicking a placeholder calls `attach(id)`: it **pins** the session (so it stays live
even if scrolled off-screen) and `ws.openSession(id)` + `ws.focusedPane = 0` so it
becomes the focused, always-live tile. The placeholder is a real `<button>` with a
title tooltip, so the affordance is obvious and keyboard-accessible.

The maximized single-tile path and the empty-state path are unchanged.

## How WS teardown reclaims memory

When a tile leaves `liveIds` (scrolled off-screen, evicted by the cap, not
pinned/active), `{#if liveIds.has(s.id)}` flips to the placeholder branch, unmounting
`SessionView` → `Terminal`. Terminal's main `$effect` cleanup
(`Terminal.svelte:311–326`) runs: `closedByUs = true; sock?.close(); sock = null;
term?.dispose()`. The WebSocket closes, the daemon connection drops, and the session
can re-suspend — memory is actually reclaimed, not just hidden.

## Result

- Opening TiledView with M suspended sessions now opens **at most ~6 WebSockets**
  (visible tiles within the cap, plus the focused tile) instead of **M**. The other
  M−6 tiles are inert placeholders that wake nothing.
- Scrolling a live tile off-screen (when over budget) tears down its WS; scrolling a
  placeholder into view (within budget) lazily opens one.
- The normal single-SessionView attach path (focus / active tile / explicit attach)
  is preserved and always live.

## Verification

`cd ui && npm run check` → **0 errors / 0 warnings** (480 files).
