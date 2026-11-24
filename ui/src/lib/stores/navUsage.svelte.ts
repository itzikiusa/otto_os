// The sidebar's "used lately" list and its More row (lib/sidebarMore.ts).
// The used list is read ONCE per app load (root only — the telemetry routes
// are) and cached per device, so the first paint already has it; the More
// row's open state is remembered per device.
import { api } from '../api/client';
import type { NavigationCount } from '../api/types';
import { navBadge } from '../navBadge';
import { navIdForModule, type SidebarModule, type SidebarSection } from '../sidebar';
import { shownModules, splitMore, usedModules } from '../sidebarMore';
import { lsGet, lsSet } from '../storage';
import { ui } from './ui.svelte';

const USED_KEY = 'otto_sidebar_used';
const MORE_KEY = 'otto_sidebar_more_open';
/** The only modules that carry a badge today (lib/navBadge.ts). */
const BADGED = ['agents', 'threads', 'workflows'];

function cachedUsed(): string[] | null {
  try {
    const v: unknown = JSON.parse(lsGet(USED_KEY) ?? 'null');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : null;
  } catch {
    return null;
  }
}

class NavUsage {
  used = $state<string[] | null>(cachedUsed());
  moreOpen = $state(lsGet(MORE_KEY) === '1');
  #loading = false;

  /** Refresh the used list (once per load; a failure keeps the cache). */
  async load(): Promise<void> {
    if (this.#loading) return;
    this.#loading = true;
    try {
      const ids = usedModules(await api.get<NavigationCount[]>('/telemetry/navigation?days=30'), navIdForModule);
      if (ids.length === 0) return;
      this.used = ids;
      lsSet(USED_KEY, JSON.stringify(ids));
    } catch {
      /* no telemetry here: the cache or the defaults stand */
    }
  }

  toggleMore(): void {
    this.moreOpen = !this.moreOpen;
    lsSet(MORE_KEY, this.moreOpen ? '1' : '0');
  }

  /** `sections` without the modules that fold under More, and those modules.
   *  Reads reactive state — call it from a `$derived`. */
  split(sections: readonly SidebarSection[], active: string): { sections: SidebarSection[]; more: SidebarModule[] } {
    const badged = BADGED.filter((id) => navBadge(id) !== null);
    return splitMore(sections, shownModules({ favorites: ui.sidebarFavorites, used: this.used, badged, active }));
  }
}

export const navUsage = new NavUsage();
