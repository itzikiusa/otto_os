// Shared reactive preferences for the IDE Bottom Dock, Debugger layout,
// Console log formatting, and Project Tree Git status indicators.
// Delegates persistence and canonical field names to `ui/src/lib/idePreferences.svelte.ts`.

import {
  IDE_PREFS_EVENT,
  IDE_PREFS_STORAGE_KEY,
  idePreferences,
} from '../../../lib/idePreferences.svelte';

export { IDE_PREFS_EVENT, IDE_PREFS_STORAGE_KEY };

export type DebuggerLayoutMode = 'tabbed' | 'split';

class IdeDockPreferences {
  get debuggerLayout(): DebuggerLayoutMode {
    return idePreferences.current.debuggerLayout;
  }

  get consolePrettyJson(): boolean {
    return idePreferences.current.prettyJsonLogs;
  }

  get consoleWrapLines(): boolean {
    return idePreferences.current.consoleLineWrap;
  }

  get gitStatusInTree(): boolean {
    return idePreferences.current.gitTreeBadges;
  }

  get nestRelatedFiles(): boolean {
    return idePreferences.current.nestRelatedFiles;
  }

  get dbgShowFrames(): boolean {
    return idePreferences.current.dbgShowFrames;
  }

  get dbgShowVariables(): boolean {
    return idePreferences.current.dbgShowVariables;
  }

  get dbgShowWatches(): boolean {
    return idePreferences.current.dbgShowWatches;
  }

  get dbgShowConsole(): boolean {
    return idePreferences.current.dbgShowConsole;
  }

  get hiddenDockTabs(): string[] {
    return idePreferences.current.hiddenDockTabs;
  }

  syncFromStorage(): void {
    idePreferences.reload();
  }

  setDebuggerLayout(next: DebuggerLayoutMode): void {
    idePreferences.update({ debuggerLayout: next });
  }

  toggleDebuggerLayout(): void {
    this.setDebuggerLayout(this.debuggerLayout === 'tabbed' ? 'split' : 'tabbed');
  }

  setConsolePrettyJson(next: boolean): void {
    idePreferences.update({ prettyJsonLogs: next });
  }

  setConsoleWrapLines(next: boolean): void {
    idePreferences.update({ consoleLineWrap: next });
  }

  toggleFrames(): void {
    if (
      this.debuggerLayout === 'split' &&
      this.dbgShowFrames &&
      !this.dbgShowVariables &&
      !this.dbgShowConsole
    ) {
      return;
    }
    idePreferences.update({ dbgShowFrames: !this.dbgShowFrames });
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
    idePreferences.update({ dbgShowVariables: !this.dbgShowVariables });
  }

  toggleWatches(): void {
    idePreferences.update({ dbgShowWatches: !this.dbgShowWatches });
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
    idePreferences.update({ dbgShowConsole: !this.dbgShowConsole });
  }

  isDockTabHidden(tabId: string): boolean {
    return this.hiddenDockTabs.includes(tabId);
  }

  toggleDockTabHidden(tabId: string, allIds: readonly string[]): void {
    const currentlyHidden = this.hiddenDockTabs.includes(tabId);
    let next: string[];
    if (!currentlyHidden) {
      const remaining = allIds.filter((id) => id !== tabId && !this.hiddenDockTabs.includes(id));
      if (remaining.length === 0) return;
      next = [...this.hiddenDockTabs, tabId];
    } else {
      next = this.hiddenDockTabs.filter((id) => id !== tabId);
    }
    idePreferences.update({ hiddenDockTabs: next });
  }

  ensureDockTabVisible(tabId: string): void {
    if (!this.hiddenDockTabs.includes(tabId)) return;
    idePreferences.update({ hiddenDockTabs: this.hiddenDockTabs.filter((id) => id !== tabId) });
  }

  resetDockTabs(): void {
    idePreferences.update({ hiddenDockTabs: [] });
  }
}

export const ideDockPrefs = new IdeDockPreferences();
