// The Workspaces section's admin actions (rename, change folder, remove) —
// kept out of Navigator.svelte, which renders the menu that calls them.

import { api } from '../lib/api/client';
import type { BulkSessionResult, BulkSessionsReq, Session, WorkspaceWithRole } from '../lib/api/types';
import { confirmer } from '../lib/confirm.svelte';
import { plural } from '../lib/plural';
import { toasts } from '../lib/toast.svelte';
import { toastError } from '../lib/toastError';
import { ws } from '../lib/stores/workspace.svelte';
import { ui } from '../lib/stores/ui.svelte';
import { router } from '../lib/router.svelte';
import { copyText } from '../lib/clipboard';
import type { MenuItem } from '../lib/contextmenu.svelte';

/** `POST /sessions/bulk` takes at most this many ids per call. */
const BULK_MAX = 200;

export async function renameWorkspace(w: WorkspaceWithRole): Promise<void> {
  const name = await confirmer.promptText('New workspace name', {
    title: `Rename “${w.name}”`,
    confirmLabel: 'Rename',
    initial: w.name,
  });
  if (!name || name === w.name) return;
  try {
    await ws.updateWorkspace(w.id, { name });
  } catch (e) {
    toastError('Couldn’t rename', e);
  }
}

/** The daemon expands `~` and refuses a folder that doesn't exist (400), so
 *  the toast names the folder it actually stored. */
export async function changeWorkspaceDir(w: WorkspaceWithRole): Promise<void> {
  const root = await confirmer.promptText('Working folder (an existing folder: absolute path, ~ ok)', {
    title: `Change folder of “${w.name}”`,
    browseFolder: true,
    confirmLabel: 'Change',
    initial: w.root_path,
    placeholder: '~/projects/my-repo',
  });
  if (!root || root === w.root_path) return;
  try {
    await ws.updateWorkspace(w.id, { root_path: root.trim() });
    const stored = ws.workspaces.find((x) => x.id === w.id)?.root_path ?? root;
    toasts.success('Folder changed', `${w.name} → ${stored}`);
  } catch (e) {
    toastError('Couldn’t change the folder', e);
  }
}

/** Sessions the workspace still has (not archived), or null when the list
 *  couldn't be read — a failed read proves nothing either way. */
async function liveSessions(id: string): Promise<Session[] | null> {
  try {
    return await api.get<Session[]>(`/workspaces/${id}/sessions?archived=false`);
  } catch {
    return null;
  }
}

/** Archive (stop) `ids` through the bulk API; resolves the number that failed. */
export async function archiveInBulk(ids: string[]): Promise<number> {
  let failed = 0;
  for (let i = 0; i < ids.length; i += BULK_MAX) {
    const body: BulkSessionsReq = { action: 'archive', ids: ids.slice(i, i + BULK_MAX) };
    try {
      const res = await api.post<BulkSessionResult[]>('/sessions/bulk', body);
      failed += res.filter((r) => !r.ok).length;
    } catch {
      failed += body.ids.length;
    }
  }
  return failed;
}

/**
 * "Remove", not "Delete": the workspace is archived (off the sidebar) and its
 * folder and files are never touched (patterns.md §7). A removed workspace's
 * sessions drop out of every list, so when it still has some the person picks:
 * stop and archive them, or leave them running (restorable later from
 * "Removed workspaces…").
 */
export async function removeWorkspace(w: WorkspaceWithRole): Promise<void> {
  const live = (await liveSessions(w.id)) ?? [];
  let stop = false;
  if (live.length > 0) {
    const running = live.filter((s) => s.status !== 'exited').length;
    const picked = await confirmer.choose(
      `“${w.name}” still has ${plural(live.length, 'session')}${running ? ` (${running} running)` : ''}. Once it is removed they no longer show anywhere in Otto. Stop and archive them (their history is kept), or leave them running — restore the workspace from “Removed workspaces…” to see them again. The folder and its files on disk are not touched.`,
      {
        title: 'Remove workspace',
        options: [
          { label: 'Stop and archive its sessions', value: 'stop', kind: 'danger' },
          { label: 'Leave them running', value: 'leave', kind: 'normal' },
        ],
      },
    );
    if (picked.value !== 'stop' && picked.value !== 'leave') return;
    stop = picked.value === 'stop';
  } else {
    const ok = await confirmer.ask(
      `Remove “${w.name}” from Otto? It leaves the sidebar; the folder and its files on disk are not touched. Restore it from “Removed workspaces…”.`,
      { title: 'Remove workspace', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
  }
  try {
    if (!(await ws.archiveWorkspace(w.id))) return;
  } catch (e) {
    toastError('Couldn’t remove the workspace', e);
    return;
  }
  if (!stop) {
    toasts.info('Workspace removed', w.name);
    return;
  }
  const failed = await archiveInBulk(live.map((s) => s.id));
  if (failed === 0) toasts.info('Workspace removed', `${w.name} · ${plural(live.length, 'session')} archived`);
  else toasts.error('Workspace removed', `${plural(failed, 'session')} of ${w.name} couldn’t be archived. Restore the workspace to stop them.`);
}

/** Open the New Session sheet in workspace `id` (switching there first —
 *  the sheet always targets the current workspace). */
export async function newSessionIn(id: string): Promise<void> {
  if (ws.currentId !== id && !(await ws.select(id))) return;
  ui.newSessionScratch = false;
  ui.newSessionOpen = true;
}

/** Open the New Session sheet pre-set to "No workspace". */
export function newScratchSession(): void {
  ui.newSessionScratch = true;
  ui.newSessionOpen = true;
}

/** Right-click on a workspace folder (the Agents inbox's workspace chip, a
 *  Workspaces row): start work there first, then manage it. `onremoved`
 *  opens the Removed workspaces sheet. */
export function workspaceMenuItems(w: WorkspaceWithRole, onremoved: () => void): MenuItem[] {
  const items: MenuItem[] = [
    { label: `New session in “${w.name}”…`, icon: 'plus', action: () => void newSessionIn(w.id) },
    { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
    { separator: true },
  ];
  if (ws.currentId !== w.id) {
    items.push({ label: 'Switch to this workspace', icon: 'check', action: () => void ws.select(w.id) });
  }
  items.push(
    {
      label: 'Copy folder path',
      icon: 'copy',
      title: w.root_path,
      action: async () => {
        if (await copyText(w.root_path)) toasts.success('Copied', w.root_path);
        else toasts.error('Couldn’t copy the path', `The clipboard isn’t available here. Select the path and copy it manually: ${w.root_path}`);
      },
    },
    { label: 'Workspace context', icon: 'note', action: async () => { if (await ws.select(w.id)) router.go('settings/context-soul'); } },
  );
  if (w.my_role === 'admin') {
    items.push(
      { separator: true },
      { label: 'Rename…', icon: 'edit', action: () => void renameWorkspace(w) },
      { label: 'Change folder…', icon: 'folder', action: () => void changeWorkspaceDir(w) },
      { label: 'Remove workspace…', icon: 'trash', danger: true, action: () => void removeWorkspace(w) },
    );
  }
  items.push({ separator: true }, { label: 'Add workspace…', icon: 'plus', action: () => (ui.newWorkspaceOpen = true) }, { label: 'Removed workspaces…', icon: 'archive', action: onremoved });
  return items;
}
