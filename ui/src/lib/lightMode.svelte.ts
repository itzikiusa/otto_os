// Mobile & tablet light version: an opt-in, per-device trim of Otto to five
// core modules — Threads, Connections, Git, API and Usage — for the occasions a
// thinner app on a phone or tablet helps. The full app stays the default
// everywhere; the person turns light on from the bottom nav's More sheet,
// Navigator / ShortNavDrawer, Settings ▸ Appearance, or ⌘K, and back the same ways.
//
// While it is on (and on any phone/tablet or coarse-pointer touch device):
// - every navigation surface offers only the five (lib/sidebar.ts
//   `setModuleScope`, read by availableModules: the bottom nav, the
//   Navigator/Rail, ⌘K "Go to");
// - a route to any other module is the route gate ("not in the light
//   version", shell/RouteGate.svelte) and its chunk never loads
//   (shell/pages.svelte.ts pageKeyOf);
// - the bare route lands on Threads (installShell).
// Non-touch desktop windows are untouched: `active` needs a phone/tablet/touch device.

import { lsGet, lsSet } from './storage';
import { viewport } from './stores/viewport.svelte';
import { isEmbedded, isPopout } from './desktop';
import { navIdForModule, setModuleScope } from './sidebar';
import { router } from './router.svelte';

/** The light version's modules, in bottom-nav order (sidebar ids). */
export const LIGHT_MODULES: readonly string[] = ['threads', 'connections', 'git', 'api', 'usage'];
/** Where the light version lands (the bare route, "Go to Threads"). */
export const LIGHT_HOME = 'threads';
/** This device's choice: '1' = light, anything else = the full app. */
export const LS_PHONE_LIGHT = 'otto_phone_light';

const LIGHT_SET = new Set(LIGHT_MODULES);
/** Routes outside the five that the light version still opens: Settings (the
 *  More sheet's) and the guides Settings links to. Not modules, so never in
 *  the bottom nav or ⌘K "Go to". */
const LIGHT_EXTRA_ROUTES = new Set(['settings', 'walkthroughs']);

/** A phone held on its side is wider than the phone breakpoint (it gets the
 *  tablet layout) but no taller than this. */
const PHONE_ON_ITS_SIDE = '(orientation: landscape) and (max-height: 500px)';
/** Touch-primary devices (iPhone, iPad, Android Phone & Tablet in any orientation,
 *  even when tablet landscape exceeds the 1024px desktop width breakpoint). */
const COARSE_POINTER = '(pointer: coarse)';

/** Whether a sidebar module (or a route that highlights one: Database and
 *  Message Brokers belong to Connections) is in the light version. */
export function lightModuleAllowed(id: string): boolean {
  return LIGHT_SET.has(navIdForModule(id));
}

/** Whether the light version opens a route by its first segment. The bare
 *  route is not "allowed": it is redirected to {@link LIGHT_HOME}. */
export function lightRouteAllowed(first: string): boolean {
  return first !== '' && (LIGHT_EXTRA_ROUTES.has(first) || lightModuleAllowed(first));
}

class LightMode {
  /** The person's choice on this device; reactive. */
  chosen = $state(lsGet(LS_PHONE_LIGHT) === '1');
  private onItsSide = $state(false);
  private coarse = $state(false);

  constructor() {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mqSide = window.matchMedia(PHONE_ON_ITS_SIDE);
    this.onItsSide = mqSide.matches;
    mqSide.addEventListener('change', () => {
      this.onItsSide = mqSide.matches;
    });
    const mqCoarse = window.matchMedia(COARSE_POINTER);
    this.coarse = mqCoarse.matches || (typeof navigator !== 'undefined' && navigator.maxTouchPoints > 0);
    mqCoarse.addEventListener('change', () => {
      this.coarse = mqCoarse.matches || (typeof navigator !== 'undefined' && navigator.maxTouchPoints > 0);
    });
  }

  /** Whether Lite Mode is available on this device/viewport: phones, tablets,
   *  and touch devices (including iPad / Android Tablet landscape > 1024px).
   *  Never a pop-out, side pane, or mouse-only desktop window. */
  get available(): boolean {
    if (isPopout || isEmbedded) return false;
    return viewport.isPhone || viewport.isTablet || this.coarse;
  }

  /** Back-compat alias for callers/tests checking phone/tablet light availability. */
  get phoneSized(): boolean {
    return this.available;
  }

  /** True specifically on phone-sized viewports (portrait or short landscape). */
  get isPhoneViewport(): boolean {
    if (isPopout || isEmbedded) return false;
    return viewport.isPhone || (viewport.isTablet && this.onItsSide);
  }

  /** One route the person opened in the full app from the gate (a "Continue
   *  on phone" link to a terminal session): light steps aside while that
   *  route shows and is back once they move on. Never remembered. */
  private fullVisit = $state<string | null>(null);

  /** Light is in effect: chosen on this device AND Lite Mode is available on
   *  this form factor, except on the route of an {@link openFullOnce} visit. */
  get active(): boolean {
    return this.chosen && this.available && (this.fullVisit === null || router.parts.join('/') !== this.fullVisit);
  }

  /** Show `route` (e.g. `agents/<id>`) in the full app for this one visit,
   *  without changing the person's choice — the gate's way into a session
   *  the light version leaves out, so a handoff link is never a dead end. */
  openFullOnce(route: string): void {
    this.fullVisit = route;
  }

  private applyChoice(on: boolean): void {
    this.fullVisit = null;
    this.chosen = on;
    lsSet(LS_PHONE_LIGHT, on ? '1' : '0');
  }

  /** Switch and remember it for this device. Turning light on from a page it
   *  hides goes to Threads instead of leaving the person on the gate. When a
   *  leave-guard (unsaved edit) is active, awaits confirmation BEFORE flipping
   *  the mode so declining never drops the user's unsaved editor into RouteGate. */
  set(on: boolean): void | Promise<boolean> {
    const needsRedirect = on && this.available && !lightRouteAllowed(router.parts[0] ?? '');
    if (needsRedirect && router.hasGuards && typeof router.goChecked === 'function') {
      return router.goChecked(LIGHT_HOME).then((ok) => {
        if (!ok) return false;
        this.applyChoice(on);
        return true;
      });
    }
    this.applyChoice(on);
    if (needsRedirect) router.go(LIGHT_HOME);
  }
}

export const lightMode = new LightMode();

// Every navigation surface reads the scope inside its own derived, so the
// switch re-renders them all.
setModuleScope((id) => !lightMode.active || lightModuleAllowed(id));
