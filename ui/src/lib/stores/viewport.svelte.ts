// Viewport store: a `matchMedia`-backed reactive store exposing the current
// responsive breakpoint as `mode` ('phone' | 'tablet' | 'desktop'), with
// convenience getters. The mobile shell (BottomNav, drawers, single-pane
// content) is gated behind these so the desktop layout stays untouched at
// ≥1025px.
//
// Breakpoints (kept in sync with the tokens in app.css):
//   phone   ≤ 640px
//   tablet  641–1024px
//   desktop ≥ 1025px
//
// Implementation mirrors the existing prefers-color-scheme listener in
// ui.svelte.ts: two MediaQueryList objects whose `change` events flip a single
// `$state` mode. SSR-safe: if `window`/`matchMedia` is absent we default to
// 'desktop' (the historical, full-chrome layout) and never touch the DOM.

export type ViewportMode = 'phone' | 'tablet' | 'desktop';

/** Phone ends at this width (inclusive). */
export const PHONE_MAX = 640;
/** Tablet ends at this width (inclusive); desktop is ≥ TABLET_MAX + 1. */
export const TABLET_MAX = 1024;
/** A touch screen this short is a phone on its side (≈ 932×430), whatever
 *  its width says. Width-only bucketing gave it the tablet shell: a fixed
 *  Navigator, a status bar and an empty top bar in ~400 px of height. */
export const SHORT_QUERY = '(max-height: 500px) and (pointer: coarse)';

function computeMode(): ViewportMode {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return 'desktop';
  }
  // Probe widest-first. Using matchMedia (not innerWidth) keeps this consistent
  // with the CSS media queries the components rely on.
  if (window.matchMedia(`(max-width: ${PHONE_MAX}px)`).matches) return 'phone';
  if (window.matchMedia(`(max-width: ${TABLET_MAX}px)`).matches) return 'tablet';
  return 'desktop';
}

/** The tablet's Navigator / Rail choice (design.md §2: Rail by default). */
export const TABLET_NAV_KEY = 'otto.nav.tablet';
function loadTabletFolded(): boolean {
  try {
    return localStorage.getItem(TABLET_NAV_KEY) !== 'navigator';
  } catch {
    return true;
  }
}

function computeShort(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false;
  return window.matchMedia(SHORT_QUERY).matches;
}

class ViewportStore {
  /** Current responsive mode; reactive. */
  mode: ViewportMode = $state(computeMode());
  /** Short touch screen (phone landscape). Only the shell chrome follows it
   *  (`navDrawer`); module layouts keep their width-based `mode`. */
  short: boolean = $state(computeShort());
  /** Tablet-only (641-1024px): the Rail (true, the default — design.md §2)
   *  or the 220 px Navigator; the person's choice, persisted on this device. */
  tabletNavFolded: boolean = $state(loadTabletFolded());

  toggleTabletRail(expanded?: boolean): void {
    this.tabletNavFolded = expanded !== undefined ? !expanded : !this.tabletNavFolded;
    try {
      localStorage.setItem(TABLET_NAV_KEY, this.tabletNavFolded ? 'rail' : 'navigator');
    } catch {
      /* storage unavailable: the choice lasts this session */
    }
  }

  // Hold references so the listeners can be (re)attached idempotently. Two
  // queries cover the three buckets: phone (≤640) and tablet (≤1024). When
  // neither matches we're on desktop.
  private phoneMq: MediaQueryList | null = null;
  private tabletMq: MediaQueryList | null = null;
  private shortMq: MediaQueryList | null = null;
  private attached = false;

  constructor() {
    this.attach();
  }

  /** Lazily wire up matchMedia listeners (no-op when unavailable or already on). */
  private attach(): void {
    if (this.attached) return;
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return;
    }
    this.attached = true;
    this.phoneMq = window.matchMedia(`(max-width: ${PHONE_MAX}px)`);
    this.tabletMq = window.matchMedia(`(max-width: ${TABLET_MAX}px)`);
    const recompute = (): void => {
      this.mode = computeMode();
    };
    this.phoneMq.addEventListener('change', recompute);
    this.tabletMq.addEventListener('change', recompute);
    this.shortMq = window.matchMedia(SHORT_QUERY);
    this.shortMq.addEventListener('change', () => {
      this.short = computeShort();
    });
  }

  get isPhone(): boolean {
    return this.mode === 'phone';
  }
  get isTablet(): boolean {
    return this.mode === 'tablet';
  }
  get isDesktop(): boolean {
    return this.mode === 'desktop';
  }
  /** True on phone OR tablet — i.e. any layout that swaps in mobile chrome. */
  get isMobile(): boolean {
    return this.mode !== 'desktop';
  }
  /** Phone landscape: a tablet-width touch screen under 500 px tall. */
  get isShort(): boolean {
    return this.short && this.mode === 'tablet';
  }
  /** The Navigator is a closable left drawer (phone, or phone on its side)
   *  instead of a fixed sidebar; the status bar is hidden there too. */
  get navDrawer(): boolean {
    return this.mode === 'phone' || this.isShort;
  }
}

export const viewport = new ViewportStore();
