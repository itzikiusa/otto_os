// Threads' page chrome beyond the thread store (kept apart so neither file
// outgrows its line cap): the right panel's opened surfaces, the sheets over
// the page, the stage's alternate view, the bottom terminal drawer and the
// floating project card. Per-viewer prefs in localStorage; sheets and the
// stage view are never persisted.
import type { ThreadsInspectorTab } from './threadsStore.svelte';

const KEY = 'otto_threads_chrome_v1';

interface Persisted {
  surfaces?: ThreadsInspectorTab[];
  cardVisible?: boolean;
  terminalOpen?: boolean;
  terminalIds?: string[];
  terminalSplit?: boolean;
  lastProjectByWs?: Record<string, string>;
  subagentsIn?: 'panel' | 'main';
}

const SURFACES: readonly ThreadsInspectorTab[] = ['files', 'changes', 'pr', 'side', 'connections', 'browser'];

function read(): Persisted {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}');
    return v && typeof v === 'object' ? (v as Persisted) : {};
  } catch {
    return {};
  }
}

class ThreadsChrome {
  /** Surfaces open as tabs in the right panel; empty = the launcher. */
  surfaces = $state<ThreadsInspectorTab[]>([]);
  /** The right panel takes the whole content area. */
  maximized = $state(false);
  /** A sheet over the page (Settings / Usage). */
  sheet = $state<'settings' | 'usage' | null>(null);
  /** The stage shows this instead of the thread / new-thread view. */
  stageView = $state<'prs' | null>(null);
  /** Floating project card on the stage (top-bar toggle, remembered). */
  cardVisible = $state(true);
  /** Bottom terminal drawer under the stage. */
  terminalOpen = $state(false);
  terminalIds = $state<string[]>([]);
  terminalActive = $state<string | null>(null);
  terminalSplit = $state(false);
  /** Folder of the last thread started per workspace (Scratch included): the
   *  new-thread stage opens there until the person picks another. */
  lastProjectByWs = $state<Record<string, string>>({});
  /** The subagent open as the right panel's tab (not remembered). */
  subagentId = $state<string | null>(null);
  /** Where a click opens a subagent on this device (⌘-click: the other). */
  subagentsIn = $state<'panel' | 'main'>('panel');

  constructor() {
    const p = read();
    if (Array.isArray(p.surfaces)) this.surfaces = p.surfaces.filter((s) => SURFACES.includes(s));
    if (typeof p.cardVisible === 'boolean') this.cardVisible = p.cardVisible;
    if (typeof p.terminalOpen === 'boolean') this.terminalOpen = p.terminalOpen;
    if (Array.isArray(p.terminalIds)) this.terminalIds = p.terminalIds.filter((x) => typeof x === 'string');
    if (typeof p.terminalSplit === 'boolean') this.terminalSplit = p.terminalSplit;
    if (p.lastProjectByWs && typeof p.lastProjectByWs === 'object') this.lastProjectByWs = p.lastProjectByWs;
    this.terminalActive = this.terminalIds[0] ?? null;
    if (p.subagentsIn === 'main') this.subagentsIn = 'main';
  }

  private save(): void {
    try {
      const v: Persisted = {
        surfaces: this.surfaces,
        cardVisible: this.cardVisible,
        terminalOpen: this.terminalOpen,
        terminalIds: this.terminalIds,
        terminalSplit: this.terminalSplit,
        lastProjectByWs: this.lastProjectByWs,
        subagentsIn: this.subagentsIn,
      };
      localStorage.setItem(KEY, JSON.stringify(v));
    } catch {
      /* private mode */
    }
  }

  rememberProject(wsKey: string, path: string | null): void {
    const p = (path ?? '').trim().replace(/\/+$/, '');
    if (!p || this.lastProjectByWs[wsKey] === p) return;
    this.lastProjectByWs = { ...this.lastProjectByWs, [wsKey]: p };
    this.save();
  }

  addSurface(tab: ThreadsInspectorTab): void {
    if (!this.surfaces.includes(tab)) {
      this.surfaces = [...this.surfaces, tab];
      this.save();
    }
  }

  /** Close a surface tab; returns the surface to show next (or null = launcher). */
  closeSurface(tab: ThreadsInspectorTab): ThreadsInspectorTab | null {
    const i = this.surfaces.indexOf(tab);
    this.surfaces = this.surfaces.filter((s) => s !== tab);
    this.save();
    return this.surfaces[Math.min(Math.max(i, 0), this.surfaces.length - 1)] ?? null;
  }

  setSubagentsIn(where: 'panel' | 'main'): void {
    this.subagentsIn = where;
    this.save();
  }

  toggleCard(): void {
    this.cardVisible = !this.cardVisible;
    this.save();
  }

  setTerminal(open: boolean): void {
    this.terminalOpen = open;
    this.save();
  }

  addTerminal(id: string): void {
    this.terminalIds = [...this.terminalIds.filter((x) => x !== id), id];
    this.terminalActive = id;
    this.save();
  }

  dropTerminal(id: string): void {
    const i = this.terminalIds.indexOf(id);
    this.terminalIds = this.terminalIds.filter((x) => x !== id);
    if (this.terminalActive === id) this.terminalActive = this.terminalIds[Math.max(0, i - 1)] ?? null;
    if (this.terminalIds.length < 2) this.terminalSplit = false;
    this.save();
  }

  /** Keep only terminals whose sessions still exist. */
  pruneTerminals(live: ReadonlySet<string>): void {
    const kept = this.terminalIds.filter((id) => live.has(id));
    if (kept.length === this.terminalIds.length) return;
    this.terminalIds = kept;
    if (!this.terminalActive || !kept.includes(this.terminalActive)) this.terminalActive = kept[0] ?? null;
    this.save();
  }

  toggleSplit(): void {
    this.terminalSplit = !this.terminalSplit && this.terminalIds.length > 1;
    this.save();
  }
}

export const threadsChrome = new ThreadsChrome();
