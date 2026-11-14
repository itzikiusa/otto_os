// Shared reactive preferences for the IDE Bottom Dock, Debugger layout,
// Console log formatting, and Project Tree Git status indicators.
// Persists into `'otto.ide.preferences.v1'` by merging keys so sibling
// editor/settings tracks never overwrite each other.

export const IDE_PREFS_STORAGE_KEY = 'otto.ide.preferences.v1';
export const IDE_PREFS_EVENT = 'otto:ide-prefs-changed';

export type DebuggerLayoutMode = 'tabbed' | 'split';

interface RawIdePrefs extends Record<string, unknown> {
  debuggerLayout?: string;
  consolePrettyJson?: boolean;
  prettyJsonLogs?: boolean;
  debuggerPrettyJson?: boolean;
  consoleWrapLines?: boolean;
  wrapConsoleLines?: boolean;
  consoleLineWrap?: boolean;
  gitStatusInTree?: boolean;
  showGitStatusInTree?: boolean;
  dbgShowFrames?: boolean;
  dbgShowVariables?: boolean;
  dbgShowWatches?: boolean;
  dbgShowConsole?: boolean;
  hiddenDockTabs?: string[];
}

function readRawPrefs(): RawIdePrefs {
  if (typeof localStorage === 'undefined') return {};
  try {
    const raw = localStorage.getItem(IDE_PREFS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as RawIdePrefs) : {};
  } catch {
    return {};
  }
}

function boolOr(val: unknown, fallback: boolean): boolean {
  return typeof val === 'boolean' ? val : fallback;
}

class IdeDockPreferences {
  debuggerLayout = $state<DebuggerLayoutMode>('tabbed');
  consolePrettyJson = $state(true);
  consoleWrapLines = $state(true);
  gitStatusInTree = $state(true);
  dbgShowFrames = $state(true);
  dbgShowVariables = $state(true);
  dbgShowWatches = $state(true);
  dbgShowConsole = $state(true);
  hiddenDockTabs = $state<string[]>([]);

  private broadcasting = false;

  constructor() {
    this.syncFromStorage();
    if (typeof window !== 'undefined') {
      window.addEventListener(IDE_PREFS_EVENT, (ev: Event) => {
        if (this.broadcasting) return;
        const detail = (ev as CustomEvent<RawIdePrefs | undefined>).detail;
        this.applyRaw(detail && typeof detail === 'object' ? detail : readRawPrefs());
      });
      window.addEventListener('storage', (ev: StorageEvent) => {
        if (ev.key === IDE_PREFS_STORAGE_KEY) this.syncFromStorage();
      });
    }
  }

  syncFromStorage(): void {
    this.applyRaw(readRawPrefs());
  }

  private applyRaw(p: RawIdePrefs): void {
    this.debuggerLayout = p.debuggerLayout === 'split' ? 'split' : 'tabbed';
    this.consolePrettyJson = boolOr(
      p.consolePrettyJson ?? p.prettyJsonLogs ?? p.debuggerPrettyJson,
      true,
    );
    this.consoleWrapLines = boolOr(
      p.consoleWrapLines ?? p.wrapConsoleLines ?? p.consoleLineWrap,
      true,
    );
    this.gitStatusInTree = boolOr(p.gitStatusInTree ?? p.showGitStatusInTree, true);
    this.dbgShowFrames = boolOr(p.dbgShowFrames, true);
    this.dbgShowVariables = boolOr(p.dbgShowVariables, true);
    this.dbgShowWatches = boolOr(p.dbgShowWatches, true);
    this.dbgShowConsole = boolOr(p.dbgShowConsole, true);
    this.hiddenDockTabs = Array.isArray(p.hiddenDockTabs)
      ? p.hiddenDockTabs.filter((x): x is string => typeof x === 'string')
      : [];
  }

  private patchStorage(patch: RawIdePrefs): void {
    if (typeof localStorage === 'undefined') return;
    try {
      const merged: RawIdePrefs = { ...readRawPrefs(), ...patch };
      localStorage.setItem(IDE_PREFS_STORAGE_KEY, JSON.stringify(merged));
      if (typeof window !== 'undefined') {
        this.broadcasting = true;
        try {
          window.dispatchEvent(new CustomEvent(IDE_PREFS_EVENT, { detail: merged }));
        } finally {
          this.broadcasting = false;
        }
      }
    } catch {
      /* ignore quota / private mode */
    }
  }

  setDebuggerLayout(next: DebuggerLayoutMode): void {
    this.debuggerLayout = next;
    this.patchStorage({ debuggerLayout: next });
  }

  toggleDebuggerLayout(): void {
    this.setDebuggerLayout(this.debuggerLayout === 'tabbed' ? 'split' : 'tabbed');
  }

  setConsolePrettyJson(next: boolean): void {
    this.consolePrettyJson = next;
    this.patchStorage({ consolePrettyJson: next, prettyJsonLogs: next });
  }

  setConsoleWrapLines(next: boolean): void {
    this.consoleWrapLines = next;
    this.patchStorage({ consoleWrapLines: next, wrapConsoleLines: next });
  }

  toggleFrames(): void {
    // In split mode, prevent hiding the very last visible column.
    if (
      this.debuggerLayout === 'split' &&
      this.dbgShowFrames &&
      !this.dbgShowVariables &&
      !this.dbgShowConsole
    ) {
      return;
    }
    this.dbgShowFrames = !this.dbgShowFrames;
    this.patchStorage({ dbgShowFrames: this.dbgShowFrames });
  }

  toggleVariables(): void {
    if (
      this.debuggerLayout === 'split' &&
      this.dbgShowVariables &&
      !this.dbgShowFrames &&
      !this.dbgShowConsole
    ) {
      return;
    }
    this.dbgShowVariables = !this.dbgShowVariables;
    this.patchStorage({ dbgShowVariables: this.dbgShowVariables });
  }

  toggleWatches(): void {
    this.dbgShowWatches = !this.dbgShowWatches;
    this.patchStorage({ dbgShowWatches: this.dbgShowWatches });
  }

  toggleConsolePane(): void {
    if (
      this.debuggerLayout === 'split' &&
      this.dbgShowConsole &&
      !this.dbgShowFrames &&
      !this.dbgShowVariables
    ) {
      return;
    }
    this.dbgShowConsole = !this.dbgShowConsole;
    this.patchStorage({ dbgShowConsole: this.dbgShowConsole });
  }

  isDockTabHidden(tabId: string): boolean {
    return this.hiddenDockTabs.includes(tabId);
  }

  toggleDockTabHidden(tabId: string, allIds: readonly string[]): void {
    const currentlyHidden = this.hiddenDockTabs.includes(tabId);
    if (!currentlyHidden) {
      const remaining = allIds.filter((id) => id !== tabId && !this.hiddenDockTabs.includes(id));
      if (remaining.length === 0) return;
      this.hiddenDockTabs = [...this.hiddenDockTabs, tabId];
    } else {
      this.hiddenDockTabs = this.hiddenDockTabs.filter((id) => id !== tabId);
    }
    this.patchStorage({ hiddenDockTabs: this.hiddenDockTabs });
  }

  ensureDockTabVisible(tabId: string): void {
    if (!this.hiddenDockTabs.includes(tabId)) return;
    this.hiddenDockTabs = this.hiddenDockTabs.filter((id) => id !== tabId);
    this.patchStorage({ hiddenDockTabs: this.hiddenDockTabs });
  }

  resetDockTabs(): void {
    this.hiddenDockTabs = [];
    this.patchStorage({ hiddenDockTabs: [] });
  }
}

export const ideDockPrefs = new IdeDockPreferences();
