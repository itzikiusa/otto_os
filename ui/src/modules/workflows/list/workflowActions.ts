// What a person does to a whole workflow — create a blank one, duplicate,
// rename, delete — shared by the Workflows tab's row menu, the editor's ⋯
// and ⌘K. Each asks and reports for itself (confirm, toast) and resolves what
// changed, so the caller only updates its list and route.
import { api } from '../../../lib/api/client';
import { patchWorkflow } from '../../../lib/api/workflows';
import type { RunListItem, Workflow } from '../../../lib/api/types';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';

export async function createBlankWorkflow(wsId: string): Promise<Workflow | null> {
  try {
    return await api.post<Workflow>(`/workspaces/${wsId}/workflows`, {
      name: 'Untitled workflow',
      graph: { nodes: [{ id: 'trigger', kind: 'manual_trigger', name: 'Start', x: 60, y: 80, params: null }], edges: [] },
    });
  } catch (e) {
    toastError('Couldn’t create the workflow', e);
    return null;
  }
}

/** A full copy of the current graph (fetched fresh, not the list row's). */
export async function duplicateWorkflow(wsId: string, wf: Pick<Workflow, 'id'>): Promise<Workflow | null> {
  try {
    const src = await api.get<Workflow>(`/workflows/${wf.id}`);
    const copy = await api.post<Workflow>(`/workspaces/${wsId}/workflows`, { name: `${src.name} (copy)`, description: src.description, graph: src.graph });
    toasts.success(`Duplicated → “${copy.name}”`, 'Rename it to trigger it independently.');
    return copy;
  } catch (e) {
    toastError('Couldn’t duplicate the workflow', e);
    return null;
  }
}

/** Chat triggers match workflows by name, so each copy needs its own. */
export async function renameWorkflow(wf: Pick<Workflow, 'id' | 'name'>): Promise<Workflow | null> {
  const name = (await confirmer.promptText('Name', { title: 'Rename workflow', initial: wf.name, confirmLabel: 'Rename' }))?.trim();
  if (!name || name === wf.name) return null;
  try {
    return await patchWorkflow(wf.id, { name });
  } catch (e) {
    toastError('Couldn’t rename the workflow', e);
    return null;
  }
}

const isLive = (r: { status: string }) => r.status === 'pending' || r.status === 'running';
const liveRuns = async (id: string) => (await api.get<Pick<RunListItem, 'id' | 'status'>[]>(`/workflows/${id}/runs?summary=true`)).filter(isLive).map((r) => r.id);

/** The server refuses (409) to delete a workflow with live runs, so with runs
 *  going the confirm offers "Cancel N runs and delete": it cancels each, waits
 *  (bounded) for them to settle, then deletes. True once it is gone. */
export async function deleteWorkflow(wf: Pick<Workflow, 'id' | 'name'>): Promise<boolean> {
  let live: string[] | null = null;
  try {
    live = await liveRuns(wf.id);
  } catch {
    live = null; // unknown — the confirm says so rather than claiming none
  }
  const n = live?.length ?? 0;
  const note = live === null
    ? ' Otto couldn’t check for running runs — if one is still going, the delete is refused until it’s canceled.'
    : n > 0 ? ` ${n === 1 ? '1 run is' : `${n} runs are`} still going — ${n === 1 ? 'it is' : 'they are'} canceled first.` : '';
  const ok = await confirmer.ask(`Delete “${wf.name}”?${note} Its run history and triggers are deleted with it. This can’t be undone.`, {
    title: 'Delete workflow',
    confirmLabel: n > 0 ? `Cancel ${n === 1 ? 'run' : `${n} runs`} and delete` : 'Delete workflow',
    danger: true,
  });
  if (!ok) return false;
  if (live?.length) {
    try {
      await Promise.all(live.map((id) => api.post(`/workflow-runs/${encodeURIComponent(id)}/cancel`, {})));
    } catch (e) {
      toastError('Couldn’t cancel the running runs — the workflow was not deleted', e);
      return false;
    }
    let settled = false;
    for (let i = 0; i < 20 && !settled; i++) {
      await new Promise((r) => setTimeout(r, 750));
      settled = await liveRuns(wf.id).then((ids) => ids.length === 0, () => false);
    }
    if (!settled) {
      toasts.error('Runs are still stopping', 'The workflow was not deleted — try again in a moment.');
      return false;
    }
  }
  try {
    await api.del(`/workflows/${wf.id}`);
    toasts.success('Workflow deleted', wf.name);
    return true;
  } catch (e) {
    toastError('Couldn’t delete the workflow', e);
    return false;
  }
}
