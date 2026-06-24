// Manual window dragging for the Tauri desktop shell.
//
// With `titleBarStyle: "Overlay"` on macOS, the native title bar is hidden and
// `data-tauri-drag-region` is unreliable (tauri-apps/tauri#9503) — the window
// often won't move. The robust, documented workaround is to call
// `getCurrentWindow().startDragging()` ourselves on mousedown. (This still needs
// the `core:window:allow-start-dragging` capability permission.)
//
// Attach as `onmousedown={startWindowDrag}` to a drag surface. We only start a
// drag when the press lands on the surface ITSELF (not an interactive child),
// so tabs/buttons inside a drag region keep working. A double-press toggles
// maximize, matching native title-bar behavior.

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function startWindowDrag(e: MouseEvent): Promise<void> {
  if (!isTauri) return;
  if (e.button !== 0) return; // primary button only
  // Only the bare drag surface drags — a press on a child (a tab, a button)
  // must not move the window.
  if (e.target !== e.currentTarget) return;
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const win = getCurrentWindow();
    if (e.detail === 2) {
      await win.toggleMaximize();
    } else {
      await win.startDragging();
    }
  } catch {
    /* not in Tauri / API unavailable — no-op */
  }
}
