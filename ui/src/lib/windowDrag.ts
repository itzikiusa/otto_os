// Manual window dragging for the Tauri desktop shell.
//
// With `titleBarStyle: "Overlay"` + `transparent: true` + window-vibrancy on
// macOS, `data-tauri-drag-region` alone doesn't move the window — we call
// `startDragging()` ourselves on mousedown (the `core:window:allow-start-dragging`
// permission is granted in the capability). A double-click on the bar toggles
// maximize, matching native macOS title-bar behaviour.
//
// The handler can sit on a bare strip OR on a container that holds buttons (the
// mobile top bar): we bail out when the mousedown lands on an interactive
// control, so tabs/buttons/inputs keep working and only the empty title-bar
// surface drags.

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const INTERACTIVE = 'button, a, input, textarea, select, label, [role="button"], [role="tab"], [contenteditable], [data-no-drag]';

export async function startWindowDrag(e: MouseEvent): Promise<void> {
  if (!isTauri) return;
  if (e.button !== 0) return;
  // Don't hijack clicks meant for a control inside the drag surface.
  const target = e.target as HTMLElement | null;
  if (target?.closest(INTERACTIVE)) return;
  try {
    const mod = await import('@tauri-apps/api/window');
    const win = mod.getCurrentWindow();
    if (e.detail === 2) {
      await win.toggleMaximize();
      return;
    }
    await win.startDragging();
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error('[window-drag] startDragging failed:', err);
  }
}
