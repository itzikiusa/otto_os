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

class ViewportStore {
  /** Current responsive mode; reactive. */
  mode: ViewportMode = $state(computeMode());

  // Hold references so the listeners can be (re)attached idempotently. Two
  // queries cover the three buckets: phone (≤640) and tablet (≤1024). When
  // neither matches we're on desktop.
  private phoneMq: MediaQueryList | null = null;
  private tabletMq: MediaQueryList | null = null;
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
}

export const viewport = new ViewportStore();
