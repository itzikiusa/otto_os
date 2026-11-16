// The project switcher's list and the Project | Scratches tabs' "back to the
// project" rule. Pure (no runes) so unit tests import it directly.
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

/** Where the IDE was before Scratches (a named project or a plain folder). */
export interface RealProject {
  root: string;
  workspaceId: string | null;
  projectId: string | null;
  /** Set when that project was a folder on an SSH host. */
  ssh?: { connectionId: string; connectionName: string | null };
}

/** What the Project tab returns to: the last real project if it still exists, else the
 *  first named project, else the first workspace folder; null when there is none. */
export function pickReturnProject(
  last: RealProject | null,
  projects: readonly NamedLike[],
  workspaces: readonly { id: string; root_path: string }[],
): RealProject | null {
  if (last && (!last.projectId || projects.some((p) => p.id === last.projectId))) return last;
  const p = projects[0];
  if (p) return { root: p.primary_root, workspaceId: null, projectId: p.id };
  const w = workspaces.find((x) => x.root_path);
  return w ? { root: w.root_path, workspaceId: w.id, projectId: null } : null;
}
