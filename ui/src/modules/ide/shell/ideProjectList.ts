// The project switcher's list and the header's project tabs. Pure (no runes)
// so unit tests import it directly.
//
// Scratches is the built-in default project: listed first, never renamed or
// deleted (it is the user's own folder, not an `/ide/projects` row), and where
// the IDE lands when there is no other project.

import type { MenuItem } from '../../../lib/contextmenu.svelte';

export const SCRATCHES_LOCKED = 'Scratches is the default project — it can’t be renamed or deleted';

interface NamedLike {
  id: string;
  name: string;
  primary_root: string;
  folders: string[];
}

export type ProjectListEntry<P> = { kind: 'scratches' } | { kind: 'named'; project: P };

/** Scratches first (while it matches the filter), then the named projects that match. */
export function projectListEntries<P extends NamedLike>(projects: readonly P[], query: string): ProjectListEntry<P>[] {
  const q = query.trim().toLowerCase();
  const out: ProjectListEntry<P>[] = [];
  if (!q || 'scratches'.includes(q) || 'default'.includes(q)) out.push({ kind: 'scratches' });
  for (const p of projects) {
    if (
      !q ||
      p.name.toLowerCase().includes(q) ||
      p.primary_root.toLowerCase().includes(q) ||
      p.folders.some((f) => f.toLowerCase().includes(q))
    ) {
      out.push({ kind: 'named', project: p });
    }
  }
  return out;
}

/** Right-click menu on the Scratches row: the actions a named project has, disabled with the reason. */
export function scratchesMenuItems(): MenuItem[] {
  return [
    { label: 'Rename project…', icon: 'edit', disabled: true, title: SCRATCHES_LOCKED },
    { separator: true },
    { label: 'Delete project…', icon: 'trash', danger: true, disabled: true, title: SCRATCHES_LOCKED },
  ];
}

export interface ProjectTab {
  /** `scratches` or the named project's id. */
  id: string;
  label: string;
  /** Scratches is the default project: no delete, no rename. */
  deletable: boolean;
  active: boolean;
}

/** The header tabs ("Show projects as tabs"): Scratches first, then every named
 *  project in the switcher's order. A plain folder or SSH target is no tab, so
 *  then none is active. */
export function projectTabs(projects: readonly NamedLike[], activeProjectId: string | null, onScratches: boolean): ProjectTab[] {
  return [
    { id: 'scratches', label: 'Scratches', deletable: false, active: onScratches },
    ...projects.map((p) => ({ id: p.id, label: p.name, deletable: true, active: !onScratches && p.id === activeProjectId })),
  ];
}
