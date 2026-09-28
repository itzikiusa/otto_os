# Detachable panes

In the macOS app, either side of a side-by-side layout can become a separate
native window. Keep the agent floating while Connections fills the original
window, or move Connections to another display while the agent stays in place.
The floating window can be resized, maximized or made fullscreen.

## Using it

1. Open two sections side by side: right-click a sidebar section and choose
   **Open side by side**, or ⌥-click it.
2. Open **Pane window** in either pane's header. Choose **Detach main pane**
   to float the primary view, or **Detach side pane** to float its companion.
   These actions are also available from the section's context menu and ⌘K.
3. Drag the native window to another display, or use **Move to display…**.
   **Fill display**, **Enter fullscreen**, and **Keep on top** control the
   detached target window.
4. Choose **Return to split**, or close a detached window, to bring the pair
   back together.

If native creation fails, the original pane remains available and the side
surface offers **Retry**. A failed window action leaves an error toast with a
retry instruction; it does not start another session.

## State and agent control

Detaching moves the existing side webview. When floating the primary pane,
Otto moves its companion and resizes the original window. Neither document is
recreated by the move: open sessions, editor drafts, query results and scroll
position remain in the same live views. Detaching does not rerun a query or
restart an agent.

The pair keeps its original logical host identity. Existing agent-control
permissions still apply, and Stop still revokes the grant. Moving a pane does
not grant an agent additional access. Each Otto window owns its own pair.

Return restores the split, including its placement and divider ratio. Closing
a detached native window also returns the pair; it does not terminate a
backend session. Quitting Otto remains a normal application quit. Separate
windows are temporary; restarting the app restores the saved split layout.

## Capabilities and limits

- Native detachment is available in the macOS desktop app. Browser clients
  retain the existing side-by-side iframe layout.
- One companion window belongs to each split. Return before switching which
  pane floats.
- Keep on top is optional and initially off.
- Display selection uses the displays currently reported by macOS. Window
  placement is bounded to available displays.
- The in-app browser inside the side pane uses its remote renderer, as it does
  in an embedded browser pane. This avoids leaving native browser surfaces in
  the original window when the pane moves.
- A host menu or dialog temporarily hides an attached native side surface so
  it cannot cover the controls. A detached surface remains in its own window.

## Implementation boundary

This is local desktop functionality; it adds no daemon HTTP or WebSocket
endpoint. Native pane commands derive ownership from the invoking local
webview, validate the existing side-pane message protocol, and target events
to that pair. Child navigation remains confined to the bundled application.
Remote browser content cannot choose another host or send arbitrary scripts.

The UI bridge is `ui/src/lib/nativePane.ts`; native lifecycle and ownership
live in `apps/desktop/src-tauri/src/panes.rs`. Browser regression tests cover
the existing split and agent-control paths. The detachable host tests use a
controlled IPC fixture; actual native document continuity requires the
standalone desktop probe as well.

An agent's state query to the side document returns that module's live state.
The host's nested side summary includes route, visibility and focus metadata;
it does not synchronously read the separate native document's module state.
