// Per-folder load failures for the project tree. A folder whose listing failed shows an
// inline "Couldn’t load" row with Retry under it (not a toast that leaves it expanded and
// silently empty). Cleared when the folder loads, and on every root (re)load.

class IdeTreeDirErrors {
  byPath = $state<Record<string, string>>({});

  set(path: string, message: string): void {
    this.byPath = { ...this.byPath, [path]: message };
  }

  clear(path: string): void {
    if (!(path in this.byPath)) return;
    const next = { ...this.byPath };
    delete next[path];
    this.byPath = next;
  }

  reset(): void {
    if (Object.keys(this.byPath).length > 0) this.byPath = {};
  }
}

export const ideTreeDirErrors = new IdeTreeDirErrors();
