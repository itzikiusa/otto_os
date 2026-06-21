// Shell UI state: rail, right panel, palette, theme, zoom. Persisted bits go
// to localStorage.

export type ThemeName = 'native' | 'pro-dark' | 'warm';
export type SchemePref = 'auto' | 'light' | 'dark';
export type Direction = 'ltr' | 'rtl';
export type RightTab = 'git' | 'files' | 'notes' | 'activity' | 'info' | 'browser' | 'api';

/** Terminal font choice. 'Cousine' supplies Hebrew glyphs in every stack so RTL
 *  text stays crisp; 'system' keeps SF Mono primary (no change for English). */
export type TermFontKey = 'system' | 'cousine' | 'menlo';

export const TERM_FONT_OPTIONS: { id: TermFontKey; name: string; desc: string }[] = [
  { id: 'system', name: 'System', desc: 'Apple SF Mono — Hebrew via Cousine' },
  { id: 'cousine', name: 'Cousine', desc: 'Uniform mono with full Hebrew coverage' },
  { id: 'menlo', name: 'Menlo', desc: 'Classic macOS mono — Hebrew via Cousine' },
];

const TERM_FONT_STACKS: Record<TermFontKey, string> = {
  // 'Cousine' sits after the Latin fonts: the browser only falls through to it
  // for codepoints SF Mono/Menlo lack (i.e. Hebrew), so Latin/code is unchanged.
  system: "'SF Mono', SFMono-Regular, Menlo, Monaco, 'Cousine', 'Courier New', monospace",
  cousine: "'Cousine', 'SF Mono', SFMono-Regular, Menlo, monospace",
  menlo: "Menlo, Monaco, 'Cousine', 'Courier New', monospace",
};

const LS = {
  rail: 'otto_rail_expanded',
  right: 'otto_right_open',
  rightTab: 'otto_right_tab',
  rightWidth: 'otto_right_width',
  railWidth: 'otto_rail_width',
  theme: 'otto_theme',
  scheme: 'otto_scheme',
  direction: 'otto_direction',
  accent: 'otto_accent',
  zoom: 'otto_zoom',
  termFont: 'otto_term_font',
  termFontFamily: 'otto_term_font_family',
  rtlBidi: 'otto_term_rtl_bidi',
  termCopyOnSelect: 'otto_term_copy_on_select',
  termToolbar: 'otto_term_toolbar',
  dbDock: 'otto_db_dock',
  dbDockWidth: 'otto_db_dock_width',
  clientId: 'otto_client_id',
  sessionIsolation: 'otto_session_isolation',
};

export const RIGHT_MIN = 260;
export const RIGHT_MAX = 760;
export const RAIL_MIN = 190;
export const RAIL_MAX = 420;

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

function lsGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function lsSet(key: string, val: string): void {
  try {
    localStorage.setItem(key, val);
  } catch {
    /* private mode */
  }
}

/** Stable per-DEVICE id. Persisted once in localStorage and reused thereafter;
 *  identifies which browser/device started a session (see meta.client_id) so the
 *  optional session-isolation filter can show only this device's sessions. */
export function clientId(): string {
  let id = lsGet(LS.clientId);
  if (!id) {
    id =
      typeof crypto !== 'undefined' && 'randomUUID' in crypto
        ? crypto.randomUUID()
        : `dev-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    lsSet(LS.clientId, id);
  }
  return id;
}

function clampRight(px: number): number {
  return Math.max(RIGHT_MIN, Math.min(RIGHT_MAX, Math.round(px)));
}
function clampRail(px: number): number {
  return Math.max(RAIL_MIN, Math.min(RAIL_MAX, Math.round(px)));
}

class UiStore {
  railExpanded = $state(lsGet(LS.rail) !== '0');
  rightOpen = $state(lsGet(LS.right) === '1');
  rightTab: RightTab = $state((lsGet(LS.rightTab) as RightTab) ?? 'git');
  rightWidth = $state(clampRight(Number(lsGet(LS.rightWidth)) || 300));
  railWidth = $state(clampRail(Number(lsGet(LS.railWidth)) || 240));
  // DB dock: the DB Explorer docked beside the agent panes (side-by-side).
  dbDockOpen = $state(lsGet(LS.dbDock) === '1');
  dbDockWidth = $state(Math.min(900, Math.max(320, Number(lsGet(LS.dbDockWidth)) || 480)));
  paletteOpen = $state(false);
  /** Which mode the palette should open in. */
  paletteMode: 'commands' | 'english' = $state('commands');
  /** Optional text to pre-fill the plain-English box with. */
  palettePrefill = $state('');
  newSessionOpen = $state(false);
  newWorkspaceOpen = $state(false);
  /** Dedicated broadcast composer (separate from the ⌘K AI orchestrator). */
  broadcastOpen = $state(false);

  /** Count of open modal-style overlays (Modal.svelte registers itself). Used
   *  so the native browser webview can hide while a modal is up — a native
   *  webview always paints above the HTML and would otherwise cover it. */
  modalCount = $state(0);
  pushModal(): void {
    this.modalCount += 1;
  }
  popModal(): void {
    this.modalCount = Math.max(0, this.modalCount - 1);
  }

  /** True when any full-screen overlay (palette / modal / new-* sheet) is open. */
  get overlayOpen(): boolean {
    return (
      this.paletteOpen ||
      this.modalCount > 0 ||
      this.newSessionOpen ||
      this.newWorkspaceOpen ||
      this.broadcastOpen
    );
  }

  /** Open the palette, optionally in plain-English mode with a prefill. */
  openPalette(mode: 'commands' | 'english' = 'commands', prefill = ''): void {
    this.paletteMode = mode;
    this.palettePrefill = prefill;
    this.paletteOpen = true;
  }

  /** Open the dedicated broadcast composer. */
  openBroadcast(): void {
    this.broadcastOpen = true;
  }

  theme: ThemeName = $state((lsGet(LS.theme) as ThemeName) ?? 'native');
  scheme: SchemePref = $state((lsGet(LS.scheme) as SchemePref) ?? 'auto');
  direction: Direction = $state((lsGet(LS.direction) as Direction) ?? 'ltr');
  accent: string = $state(lsGet(LS.accent) ?? '');

  /** app-level zoom, 1 = 100% */
  zoom = $state(Number(lsGet(LS.zoom) ?? '1') || 1);
  /** terminal font size in px */
  termFontSize = $state(Number(lsGet(LS.termFont) ?? '13') || 13);
  /** terminal font family choice (see TERM_FONT_STACKS) */
  termFontFamily: TermFontKey = $state((lsGet(LS.termFontFamily) as TermFontKey) ?? 'system');
  /** experimental: reorder RTL (Hebrew) runs right-to-left via the browser's
   *  bidi engine. Forces xterm's DOM renderer (no WebGL) — may distort TUIs. */
  rtlBidi = $state(lsGet(LS.rtlBidi) === '1');
  /** Copy-to-clipboard automatically on text selection in the terminal. */
  termCopyOnSelect = $state(lsGet(LS.termCopyOnSelect) === '1');
  /** Show the desktop terminal toolbar (font zoom + copy-on-select). */
  termToolbar = $state(lsGet(LS.termToolbar) !== '0'); // default on

  /** Resolved CSS font-family stack for the terminal, per the current choice. */
  get termFontStack(): string {
    return TERM_FONT_STACKS[this.termFontFamily] ?? TERM_FONT_STACKS.system;
  }

  /** Per-DEVICE session isolation: when on, the sessions list/tabs/Navigator
   *  only show sessions this device started (meta.client_id === clientId()).
   *  Other devices' sessions stay hidden here but still run on the daemon.
   *  Default off (current behavior = see all). Persisted per device. */
  sessionIsolation = $state(lsGet(LS.sessionIsolation) === '1');

  setSessionIsolation(on: boolean): void {
    this.sessionIsolation = on;
    lsSet(LS.sessionIsolation, on ? '1' : '0');
    // Re-apply the filter immediately against the already-loaded sessions.
    void import('./workspace.svelte').then(({ ws }) => ws.refreshSessions());
  }

  /** resolved light|dark after applying `auto` */
  resolvedScheme: 'light' | 'dark' = $state('dark');

  private media: MediaQueryList | null = null;

  toggleRail(): void {
    this.railExpanded = !this.railExpanded;
    lsSet(LS.rail, this.railExpanded ? '1' : '0');
  }

  toggleRight(): void {
    this.rightOpen = !this.rightOpen;
    lsSet(LS.right, this.rightOpen ? '1' : '0');
  }

  openRight(tab: RightTab): void {
    this.rightTab = tab;
    this.rightOpen = true;
    lsSet(LS.right, '1');
    lsSet(LS.rightTab, tab);
  }

  setRightWidth(px: number): void {
    this.rightWidth = clampRight(px);
    lsSet(LS.rightWidth, String(this.rightWidth));
  }

  setRailWidth(px: number): void {
    this.railWidth = clampRail(px);
    lsSet(LS.railWidth, String(this.railWidth));
  }
  toggleDbDock(): void {
    this.dbDockOpen = !this.dbDockOpen;
    lsSet(LS.dbDock, this.dbDockOpen ? '1' : '0');
  }
  setDbDockWidth(px: number): void {
    this.dbDockWidth = Math.min(900, Math.max(320, Math.round(px)));
    lsSet(LS.dbDockWidth, String(this.dbDockWidth));
  }

  setTheme(theme: ThemeName): void {
    this.theme = theme;
    lsSet(LS.theme, theme);
    this.applyTheme();
  }

  setScheme(scheme: SchemePref): void {
    this.scheme = scheme;
    lsSet(LS.scheme, scheme);
    this.applyTheme();
  }

  setAccent(accent: string): void {
    this.accent = accent;
    lsSet(LS.accent, accent);
    this.applyTheme();
  }

  setDirection(direction: Direction): void {
    this.direction = direction;
    lsSet(LS.direction, direction);
    this.applyTheme();
  }

  zoomIn(): void {
    this.setZoom(Math.min(2, Math.round((this.zoom + 0.1) * 10) / 10));
  }
  zoomOut(): void {
    this.setZoom(Math.max(0.6, Math.round((this.zoom - 0.1) * 10) / 10));
  }
  zoomReset(): void {
    this.setZoom(1);
  }
  private setZoom(z: number): void {
    this.zoom = z;
    lsSet(LS.zoom, String(z));
    void this.applyNativeZoom();
  }

  /**
   * In Tauri, scale the page via the native WKWebView zoom instead of CSS
   * `zoom`. CSS `zoom` rasterizes the WebGL terminal canvas at the unscaled
   * size and then stretches it → blurry text at any zoom ≠ 100%. Native page
   * zoom re-rasterizes everything (terminal included) at the proper resolution,
   * so text stays crisp. No-op (and harmless) in the browser build, where the
   * shell falls back to CSS `zoom`. macOS support relies on `macos-private-api`.
   */
  async applyNativeZoom(): Promise<void> {
    if (!isTauri) return;
    try {
      const { getCurrentWebview } = await import('@tauri-apps/api/webview');
      await getCurrentWebview().setZoom(this.zoom);
    } catch {
      /* zoom permission/API unavailable — CSS fallback still applies in web */
    }
  }

  termZoomIn(): void {
    this.setTermFont(Math.min(28, this.termFontSize + 1));
  }
  termZoomOut(): void {
    this.setTermFont(Math.max(8, this.termFontSize - 1));
  }
  termZoomReset(): void {
    this.setTermFont(13);
  }
  private setTermFont(px: number): void {
    this.termFontSize = px;
    lsSet(LS.termFont, String(px));
  }

  setTermFontFamily(key: TermFontKey): void {
    this.termFontFamily = key;
    lsSet(LS.termFontFamily, key);
  }

  setRtlBidi(on: boolean): void {
    this.rtlBidi = on;
    lsSet(LS.rtlBidi, on ? '1' : '0');
  }

  setTermCopyOnSelect(on: boolean): void {
    this.termCopyOnSelect = on;
    lsSet(LS.termCopyOnSelect, on ? '1' : '0');
  }

  setTermToolbar(on: boolean): void {
    this.termToolbar = on;
    lsSet(LS.termToolbar, on ? '1' : '0');
  }

  /** Apply data-theme/data-scheme attrs and accent override on <html>. */
  applyTheme(): void {
    if (typeof document === 'undefined') return;
    if (!this.media) {
      this.media = window.matchMedia('(prefers-color-scheme: dark)');
      this.media.addEventListener('change', () => this.applyTheme());
    }
    const resolved: 'light' | 'dark' =
      this.scheme === 'auto' ? (this.media.matches ? 'dark' : 'light') : this.scheme;
    this.resolvedScheme = resolved;
    const el = document.documentElement;
    el.setAttribute('data-theme', this.theme);
    el.setAttribute('data-scheme', resolved);
    // Document direction (RTL support). CSS uses logical properties so the
    // layout mirrors automatically when this flips to 'rtl'.
    el.dir = this.direction;
    if (this.accent) el.style.setProperty('--accent', this.accent);
    else el.style.removeProperty('--accent');
  }
}

export const ui = new UiStore();
