// The phone's light version: an opt-in, per-device trim of Otto to five
// modules — Threads, Connections, Git, API and Usage — for the occasions a
// thinner app on a phone helps. The full app stays the default everywhere;
// the person turns light on from the bottom nav's More sheet (one tap), from
// Settings ▸ Appearance or ⌘K, and back the same ways.
//
// While it is on (and only on a phone-sized screen):
// - every navigation surface offers only the five (lib/sidebar.ts
//   `setModuleScope`, read by availableModules: the bottom nav, the
//   Navigator a phone on its side shows, ⌘K "Go to");
// - a route to any other module is the route gate ("not in the light
//   version", shell/RouteGate.svelte) and its chunk never loads
//   (shell/pages.svelte.ts pageKeyOf);
// - the bare route lands on Threads (installShell).
// Tablet and desktop are untouched: `active` needs a phone-sized screen.

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

  constructor() {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia(PHONE_ON_ITS_SIDE);
    this.onItsSide = mq.matches;
    mq.addEventListener('change', () => {
      this.onItsSide = mq.matches;
    });
  }

  /** A phone-sized main window: the phone layout, or a phone on its side
   *  (the tablet layout on a short screen). Never a pop-out or side pane. */
  get phoneSized(): boolean {
    if (isPopout || isEmbedded) return false;
    return viewport.isPhone || (viewport.isTablet && this.onItsSide);
  }

  /** Light is in effect: chosen on this device AND the screen is a phone's. */
  get active(): boolean {
    return this.chosen && this.phoneSized;
  }

  /** Switch and remember it for this device. Turning light on from a page it
   *  hides goes to Threads instead of leaving the person on the gate. */
  set(on: boolean): void {
    this.chosen = on;
    lsSet(LS_PHONE_LIGHT, on ? '1' : '0');
    if (on && this.phoneSized && !lightRouteAllowed(router.parts[0] ?? '')) router.go(LIGHT_HOME);
  }
}

export const lightMode = new LightMode();

// Every navigation surface reads the scope inside its own derived, so the
// switch re-renders them all.
setModuleScope((id) => !lightMode.active || lightModuleAllowed(id));
