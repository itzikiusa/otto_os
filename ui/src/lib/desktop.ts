// Desktop-shell bridge: typed wrappers over the native companion layer in
// apps/desktop/src-tauri (global shortcuts, the ⌥Space assistant bar panel,
// the menu-bar item + popover, pop-out windows). Every call is a no-op (or
// resolves false/null) outside the Tauri shell, so callers can use it from
// shared code without their own `isTauri` checks.
//
// Window routes the shell loads (all chromeless):
//   #/bar            — the assistant bar panel (`otto-bar`)
//   #/tray           — the menu-bar popover (`otto-tray`)
//   ?popout=1#/<r>   — a pop-out window of route <r> (`popout-<N>`)

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

type PopoutInit = { title?: unknown };

/** True inside a pop-out window: the shell hides the sidebar + status bar and
 *  draws a slim title strip. `?popout=1` also works in a plain browser. */
export const isPopout: boolean =
  typeof window !== 'undefined' &&
  (new URLSearchParams(window.location.search).get('popout') === '1' ||
    typeof (window as { __OTTO_POPOUT__?: PopoutInit }).__OTTO_POPOUT__ === 'object');

/** Title the shell gave this pop-out (falls back to the document title). */
export function popoutTitle(): string {
  const t = (window as { __OTTO_POPOUT__?: PopoutInit }).__OTTO_POPOUT__?.title;
  return typeof t === 'string' && t.trim() !== '' ? t : document.title || 'Otto';
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

/** The current route without the leading `#/` (what `openPopout` takes). */
export function currentRoute(): string {
  return window.location.hash.replace(/^#\/?/, '');
}

/** Open `route` (e.g. `database/<id>`, `agents/<sid>`) in its own native
 *  window. One window per route: asking again focuses it. Resolves false
 *  outside the desktop app (the caller can fall back to `router.go`). */
export async function openPopout(route: string, title?: string): Promise<boolean> {
  if (!isTauri) return false;
  await call<string>('open_popout', { route, title: title ?? null });
  return true;
}

/** Bring the main Otto window forward (activating the app), optionally at a
 *  route. Hides the bar + tray popover. Outside Tauri: navigates in place. */
export async function openInOtto(route?: string): Promise<void> {
  if (!isTauri) {
    if (route) window.location.hash = `#/${route.replace(/^#?\/?/, '')}`;
    return;
  }
  await call<void>('open_in_otto', { route: route ?? null });
}

/** Applied bar frame after the shell clamped it to the screen. */
export interface BarFrame {
  width: number;
  height: number;
  radius: number;
}

export const bar = {
  show: (): Promise<void> => (isTauri ? call<void>('bar_show') : Promise.resolve()),
  hide: (): Promise<void> => (isTauri ? call<void>('bar_hide') : Promise.resolve()),
  toggle: (): Promise<void> => (isTauri ? call<void>('bar_toggle') : Promise.resolve()),
  /** Size the bar window to its content (CSS px). It grows UPWARD from the
   *  pill, capped at 560 px and the screen; `radius` reshapes the native glass
   *  (default: a full pill at pill height, 18 when expanded). */
  resize: (
    height: number,
    opts: { width?: number; radius?: number } = {},
  ): Promise<BarFrame | null> =>
    isTauri
      ? call<BarFrame>('bar_resize', {
          height,
          width: opts.width ?? null,
          radius: opts.radius ?? null,
        })
      : Promise.resolve(null),
};

export const tray = {
  /** Report counts for the menu-bar glyph: dot while work runs, amber when
   *  something needs you. */
  setStatus: (running: number, needsYou: number): Promise<void> =>
    isTauri
      ? call<void>('tray_set_status', {
          running: Math.max(0, Math.floor(running)),
          needsYou: Math.max(0, Math.floor(needsYou)),
        })
      : Promise.resolve(),
  hidePopover: (): Promise<void> =>
    isTauri ? call<void>('tray_popover_hide') : Promise.resolve(),
};

/** One system-wide chord (the Settings UI lists these). `accel` uses the
 *  global-hotkey syntax, e.g. `Alt+Space`, `Cmd+Ctrl+Shift+2`; `''` = off. */
export interface ShortcutBinding {
  id: 'snip' | 'assistant' | 'assistant.voice';
  label: string;
  accel: string;
  default: string;
  /** The OS accepted the registration. */
  active: boolean;
  /** Why it isn't active (e.g. another app holds the chord). */
  error: string | null;
}

export const shortcuts = {
  list: (): Promise<ShortcutBinding[]> =>
    isTauri ? call<ShortcutBinding[]>('shortcuts_list') : Promise.resolve([]),
  /** Rejects with a readable message on a clash or an unparseable chord;
   *  nothing is saved unless the OS accepted it. */
  set: (id: ShortcutBinding['id'], accel: string): Promise<void> =>
    isTauri ? call<void>('shortcuts_set', { id, accel }) : Promise.resolve(),
  reset: (id: ShortcutBinding['id']): Promise<void> =>
    isTauri ? call<void>('shortcuts_reset', { id }) : Promise.resolve(),
};

/** Native → page events for the bar / tray windows. */
export type DesktopEvent =
  | 'otto://bar-shown'
  | 'otto://bar-hidden'
  | 'otto://assistant-voice'
  | 'otto://tray-shown';

/** Listen for a desktop event on THIS window (the shell targets each event at
 *  one window label). Resolves a no-op unlisten outside Tauri. */
export async function onDesktopEvent<T = unknown>(
  name: DesktopEvent,
  cb: (payload: T) => void,
): Promise<() => void> {
  if (!isTauri) return () => {};
  const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  return getCurrentWebviewWindow().listen<T>(name, (e) => cb(e.payload));
}

/** Panel windows (bar, tray) are transparent so the native vibrancy behind
 *  the page shows through: clear the opaque app background for this
 *  document. Call once from the panel page. */
export function useGlassWindow(): void {
  if (typeof document === 'undefined') return;
  document.documentElement.style.background = 'transparent';
  document.body.style.background = 'transparent';
}
