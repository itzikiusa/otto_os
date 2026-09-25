// Agent UI control — the command catalog, straight from the ONE source of truth
// `docs/contracts/ui-commands.json` (the daemon include_str!s the same file).
// unit/uiCommands.test.ts asserts every entry has exactly one registered
// handler under its module and vice versa; `index.ts` repeats that check at
// dev time so a mismatch shows up in the console before it reaches the test.

import raw from '../../../../docs/contracts/ui-commands.json';
import type { UiCommandSpec, UiCommandsCatalog } from '../api/types';

export const UI_CATALOG: UiCommandsCatalog = {
  version: raw.version,
  // `$comment` markers are documentation only; the rest is the spec.
  commands: (raw.commands as unknown as (UiCommandSpec & { $comment?: string })[]).map(
    ({ $comment: _c, ...spec }) => spec,
  ),
};

const byName = new Map(UI_CATALOG.commands.map((c) => [c.name, c]));

/** The catalog entry for a command name (`db_run_query`), if any. */
export function uiCommandSpec(name: string): UiCommandSpec | undefined {
  return byName.get(name);
}

/**
 * Registered handlers vs the catalog: names the catalog has but no handler
 * implements (`missing`), handlers the catalog doesn't know (`extra`), and
 * handlers registered under another module than the catalog's (`misplaced`).
 */
export function catalogDrift(registered: { name: string; module: string }[]): {
  missing: string[];
  extra: string[];
  misplaced: string[];
} {
  const have = new Map(registered.map((r) => [r.name, r.module]));
  return {
    missing: UI_CATALOG.commands.filter((c) => !have.has(c.name)).map((c) => c.name),
    extra: registered.filter((r) => !byName.has(r.name)).map((r) => r.name),
    misplaced: registered
      .filter((r) => byName.has(r.name) && byName.get(r.name)!.module !== r.module)
      .map((r) => `${r.name} (${r.module} ≠ ${byName.get(r.name)!.module})`),
  };
}
