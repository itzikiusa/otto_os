// The built-in Scratches project: the signed-in user's own scratch folder
// (`GET /scratches` → `root`, see docs/contracts/api.md "Scratches"). It is the
// same folder in every workspace and opens like any local project, so the
// tree, editor, LSP, run configs, Local History, search and agents all apply.
// It is the default project: the IDE falls back to it when there is no other.

import { api } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { toastError } from '../../../lib/toastError';
import { ide } from '../ideStore.svelte';
import { normalizeRootPath } from './ideProjectHelpers';

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

export async function openScratches(): Promise<boolean> {
  try {
    return await ide.setLocalTarget(await loadScratchesRoot(), null);
  } catch (e) {
    toastError('Couldn’t open Scratches', e);
    return false;
  }
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
