import { api, ApiError } from '../../lib/api/client';
import type { Workflow, WorkflowGraph } from '../../lib/api/types';
import { patchWorkflow } from '../../lib/api/workflows';
import { confirmer } from '../../lib/confirm.svelte';
import { rememberSelection } from '../../lib/lastSelection';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';

class WorkflowsUiState {
  prompt = $state('');
  generating = $state(false);
  instructionsOpen = $state(false);
  triggersOpen = $state(false);
  selectedNodeByWorkflow = new Map<string, string | null>();

  async generate(wsId: string | null | undefined): Promise<Workflow | null> {
    const desc = this.prompt.trim();
    if (!wsId || this.generating || !desc) return null;
    this.generating = true;
    try {
      const wf = await api.post<Workflow>(`/workspaces/${wsId}/workflows/generate`, {
        description: desc,
      });
      this.prompt = '';
      rememberSelection('workflows', wf.id);
      toasts.success('Workflow generated', 'Tweak it on the canvas, then run.');
      return wf;
    } catch (e) {
      toastError('Couldn’t generate the workflow', e);
      return null;
    } finally {
      this.generating = false;
    }
  }

  /** Save `graph` over `version` (the saved workflow goes to `onsaved`, and
   *  resolves true), never over a newer save made elsewhere (A1): the
   *  daemon's 409 asks the person to reload theirs (to `onreload`) or
   *  overwrite with this one. */
  async saveGraph(id: string, graph: WorkflowGraph, version: number | undefined, onsaved: (wf: Workflow) => void, onreload: (wf: Workflow) => void): Promise<boolean> {
    const save = async (body: { graph: WorkflowGraph; expected_version?: number }): Promise<true> => {
      onsaved(await patchWorkflow(id, body));
      return true;
    };
    try {
      return await save({ graph, expected_version: version });
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 409)) throw e;
      const { value } = await confirmer.choose('This workflow was saved elsewhere after you opened it.', {
        title: 'Changed elsewhere',
        options: [
          { label: 'Reload theirs', value: 'reload', kind: 'primary' },
          { label: 'Overwrite with mine', value: 'overwrite', kind: 'danger' },
        ],
      });
      if (value === 'reload') onreload(await api.get<Workflow>(`/workflows/${encodeURIComponent(id)}`));
      return value === 'overwrite' ? save({ graph }) : false;
    }
  }

  resetForTests(): void {
    this.prompt = '';
    this.generating = false;
    this.instructionsOpen = false;
    this.triggersOpen = false;
    this.selectedNodeByWorkflow.clear();
  }
}

export const workflowsUi = new WorkflowsUiState();
