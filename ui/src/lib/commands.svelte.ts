// Command registry for the ⌘K palette. Modules register commands (with an
// owner key so re-registration replaces the old set) and the palette reads
// `registry.all`.

export interface Command {
  id: string;
  title: string;
  /** group header shown in the palette, e.g. "Sessions", "Git" */
  group?: string;
  /** dim secondary text after the title, e.g. a Go-to target's sidebar section */
  detail?: string;
  /** extra fuzzy-match terms */
  keywords?: string;
  /** display-only shortcut hint, e.g. "⌘T" */
  shortcut?: string;
  run: () => unknown;
}

class CommandRegistry {
  // The authoritative map is PLAIN (non-reactive); `sources` only publishes
  // it. Callers register from $effects and unregister from their teardowns,
  // and Svelte deliberately answers a signal read inside a teardown with the
  // value from BEFORE the current flush (`old_values`). Rebuilding the map
  // from `this.sources` there resurrected a stale snapshot: when two
  // registration effects re-ran in one flush ('nav' + 'side-pane-commands'),
  // the second teardown dropped the first one's fresh set — ⌘K "go to vault"
  // lost every Go-to entry (unit/commandRegistry.test.ts). Never read
  // `sources` from register/unregister.
  private map: Record<string, Command[]> = {};
  // Raw: command sets are replaced wholesale (never mutated), so deep-proxying
  // every Command (closures included) bought nothing.
  private sources: Record<string, Command[]> = $state.raw({});
  /** Memoized flat list — recomputed only when a set is (un)registered. */
  private flat = $derived(Object.values(this.sources).flat());

  /**
   * Register a command set under an owner key. Returns an unregister fn.
   * Calling again with the same owner replaces the previous set.
   */
  register(owner: string, commands: Command[]): () => void {
    this.map = { ...this.map, [owner]: commands };
    this.sources = this.map;
    return () => {
      // A later register() under the same owner already replaced this set —
      // its own unregister owns the slot now.
      if (this.map[owner] !== commands) return;
      const { [owner]: _gone, ...rest } = this.map;
      this.map = rest;
      this.sources = rest;
    };
  }

  get all(): Command[] {
    return this.flat;
  }

  /** True once a command surface (⌘K palette, the floating bar) has been
   *  opened in this document. Command sets that cost a request to build
   *  (connections) wait for it instead of fetching at boot (perf F8). */
  wanted = $state(false);

  /** A command surface opened: let on-demand command sets load. */
  want(): void {
    if (!this.wanted) this.wanted = true;
  }
}

export const registry = new CommandRegistry();
