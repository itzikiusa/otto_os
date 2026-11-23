// Which tool windows the IDE's bottom dock offers (I1-14). The everyday four — Terminal,
// Problems, Git, Run — always show. Debug, Usages and DB Watch are "hidden until used": they
// appear while they hold something (a debug session, a usages result, an armed watch), once
// they were opened this session (⌘K, ⋯, a shortcut, starting a debug run), or when the person
// chose to show them from the dock's ⋯. An explicit hide (`ideDockPrefs.hiddenDockTabs`) wins
// over everything except being the open tab.

export const IDE_DOCK_ORDER = ['terminal', 'problems', 'git', 'run', 'devices', 'debug', 'usages', 'dbwatch'] as const;
export const IDE_DOCK_ON_DEMAND: readonly string[] = ['devices', 'debug', 'usages', 'dbwatch'];

export interface IdeDockTabFacts {
  /** Explicitly hidden from the dock's ⋯ (the persisted preference). */
  hidden: readonly string[];
  /** Explicitly shown from the dock's ⋯ (an on-demand tab kept on the strip). */
  shown: readonly string[];
  /** Opened at least once this session. */
  used: readonly string[];
  /** The tab holds something worth seeing right now. */
  hasContent: boolean;
  active: boolean;
}

export function ideDockTabVisible(id: string, f: IdeDockTabFacts): boolean {
  if (f.active) return true;
  if (f.hidden.includes(id)) return false;
  if (!IDE_DOCK_ON_DEMAND.includes(id)) return true;
  return f.hasContent || f.shown.includes(id) || f.used.includes(id);
}

/** Desktop order: the edit loop's tools first; the stacked phone pane reorders on its own. */
export function ideDockOrder<T extends { id: string }>(tabs: readonly T[]): T[] {
  const rank = (id: string): number => {
    const i = (IDE_DOCK_ORDER as readonly string[]).indexOf(id);
    return i < 0 ? IDE_DOCK_ORDER.length : i;
  };
  return [...tabs].sort((a, b) => rank(a.id) - rank(b.id));
}

const LS_SHOWN = 'otto_ide_dock_shown';

function readShown(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(LS_SHOWN) ?? '[]');
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

class IdeDockTabMemory {
  shown = $state<string[]>(readShown());
  used = $state<string[]>([]);

  markUsed(id: string): void {
    if (!this.used.includes(id)) this.used = [...this.used, id];
  }

  setShown(id: string, on: boolean): void {
    this.shown = on ? [...this.shown.filter((x) => x !== id), id] : this.shown.filter((x) => x !== id);
    try {
      localStorage.setItem(LS_SHOWN, JSON.stringify(this.shown));
    } catch {
      /* private window: the choice lasts this session */
    }
  }

  reset(): void {
    this.used = [];
    this.shown = [];
    try {
      localStorage.removeItem(LS_SHOWN);
    } catch {
      /* nothing stored */
    }
  }
}

export const ideDockTabMemory = new IdeDockTabMemory();
