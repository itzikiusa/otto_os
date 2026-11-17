// The built-in Scratches project: the signed-in user's own scratch folder
// (`GET /scratches` → `root`, see docs/contracts/api.md "Scratches"). It is the
// same folder in every workspace and opens like any local project, so the
// tree, editor, LSP, run configs, Local History, search and agents all apply.
// It is the default project: the IDE falls back to it when there is no other.

import { api } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { lsGet, lsSet } from '../../../lib/storage';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import { toastError } from '../../../lib/toastError';
import { ide } from '../ideStore.svelte';
import { normalizeRootPath } from './ideProjectHelpers';
import { pickReturnProject, type RealProject } from './ideProjectList';

/** The folder's absolute path once known. */
export const scratches = $state({ root: null as string | null });

/** The IDE shows the Scratches project right now (reactive). */
export function onScratches(): boolean {
  return ide.target.kind === 'local' && !!scratches.root && normalizeRootPath(ide.target.root) === normalizeRootPath(scratches.root);
}

let rootP: Promise<string> | null = null;

/** Fetch the folder once (the daemon also copies any old Workbench docs into it). */
export function loadScratchesRoot(): Promise<string> {
  rootP ??= api
    .get<{ root: string }>('/scratches')
    .then((r) => (scratches.root = r.root))
    .catch((e: unknown) => {
      rootP = null;
      throw e;
    });
  return rootP;
}

const LAST_REAL_KEY = 'otto.ide.lastRealProject';

function readLastReal(): RealProject | null {
  try {
    const v = JSON.parse(lsGet(LAST_REAL_KEY) || 'null') as RealProject | null;
    return v && typeof v.root === 'string' ? v : null;
  } catch {
    return null;
  }
}

export async function openScratches(): Promise<boolean> {
  try {
    const root = await loadScratchesRoot();
    // Remember the project we leave, for the Project tab.
    const t = ide.target;
    if (t.root && !onScratches()) {
      const last: RealProject = t.kind === 'ssh' && t.connection_id
        ? { root: t.root, workspaceId: null, projectId: null, ssh: { connectionId: t.connection_id, connectionName: t.connection_name ?? null } }
        : { root: t.root, workspaceId: t.workspace_id ?? null, projectId: ide.activeProjectId };
      lsSet(LAST_REAL_KEY, JSON.stringify(last));
    }
    return await ide.setLocalTarget(root, null);
  } catch (e) {
    toastError('Couldn’t open Scratches', e);
    return false;
  }
}

/** The Project tab: back to the project open before Scratches (or the first one there is). */
export function returnProject(): RealProject | null {
  const workspaces = ws.workspaces.filter((w) => w.id !== SCRATCH_WORKSPACE_ID);
  return pickReturnProject(readLastReal(), ide.namedProjects, workspaces);
}

export async function openReturnProject(): Promise<boolean> {
  const p = returnProject();
  if (!p) return false;
  if (p.ssh) return ide.setSshTarget(p.ssh.connectionId, p.root, p.ssh.connectionName);
  if (p.projectId) {
    await ide.activateProject(p.projectId);
    return ide.activeProjectId === p.projectId;
  }
  return ide.setLocalTarget(p.root, p.workspaceId);
}

/** Ask for a name (its extension picks the language), create the file at the
 *  top of Scratches and open it. Opens the Scratches project first if needed. */
export async function newScratchFile(): Promise<void> {
  if (!onScratches() && !(await openScratches())) return;
  const taken = new Set(ide.treeRootEntries.map((e) => e.name.toLowerCase()));
  let initial = 'scratch.md';
  for (let n = 2; taken.has(initial); n++) initial = `scratch-${n}.md`;
  const name = (
    await confirmer.promptText('The extension picks the language: .sql, .json, .md, .py, .mmd, .http, …', {
      title: 'New scratch file',
      confirmLabel: 'Create',
      initial,
    })
  )?.trim();
  if (!name) return;
  await ide.mutateFs('create_file', `${ide.target.root.replace(/\/+$/, '')}/${name.replace(/^\/+/, '')}`); // opens it
}
