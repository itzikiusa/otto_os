// Command registry for the ⌘K palette. Modules register commands (with an
// owner key so re-registration replaces the old set) and the palette reads
// `registry.all`.

import { untrack } from 'svelte';

export interface Command {
  id: string;
  title: string;
  /** group header shown in the palette, e.g. "Sessions", "Git" */
  group?: string;
  /** extra fuzzy-match terms */
  keywords?: string;
  /** display-only shortcut hint, e.g. "⌘T" */
  shortcut?: string;
  run: () => unknown;
}

class CommandRegistry {
  private sources: Record<string, Command[]> = $state({});

  /**
   * Register a command set under an owner key. Returns an unregister fn.
   * Calling again with the same owner replaces the previous set.
   */
  register(owner: string, commands: Command[]): () => void {
    // untrack: callers register from $effects — reading `sources` there must
    // not make the effect depend on it (read+write loop).
    this.sources = { ...untrack(() => this.sources), [owner]: commands };
    return () => {
      const { [owner]: _gone, ...rest } = untrack(() => this.sources);
      this.sources = rest;
    };
  }

  get all(): Command[] {
    return Object.values(this.sources).flat();
  }
}

export const registry = new CommandRegistry();
