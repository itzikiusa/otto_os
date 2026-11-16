// Reactive Svelte 5 singleton wrapping `cm-ide-prefs.ts` so all IDE surfaces
// (`ideSettingsStore`, `ideDockPrefs`, `ideEditorStoreBridge`, `ideShellCommands`,
// `CodeEditor`) share one schema, one set of defaults, and live event updates.

import {
  cloneIdePreferences,
  IDE_PREFS_EVENT,
  IDE_PREFS_STORAGE_KEY,
  readIdePreferences,
  writeIdePreferences,
  type IdePreferences,
} from './components/cm-ide-prefs';

export * from './components/cm-ide-prefs';

class IdePreferencesStore {
  current = $state<IdePreferences>(readIdePreferences());

  constructor() {
    if (typeof window !== 'undefined') {
      const sync = (e?: Event): void => {
        const detail = (e as CustomEvent<IdePreferences> | undefined)?.detail;
        this.current = detail ? cloneIdePreferences(detail) : readIdePreferences();
      };
      window.addEventListener(IDE_PREFS_EVENT, sync);
      window.addEventListener('storage', (ev) => {
        if (
          !ev.key ||
          ev.key === IDE_PREFS_STORAGE_KEY ||
          ev.key === 'otto.ide.keymapPreset' ||
          ev.key === 'otto.ide.keymap' ||
          ev.key === 'otto.ide.formatOnSave'
        ) {
          sync();
        }
      });
    }
  }

  reload(): IdePreferences {
    const next = readIdePreferences();
    this.current = next;
    return next;
  }

  update(patch: Partial<IdePreferences>): IdePreferences {
    const next = writeIdePreferences(patch);
    this.current = cloneIdePreferences(next);
    return next;
  }
}

export const idePreferences = new IdePreferencesStore();
