import { api } from '../../lib/api/client';
import type { Workflow } from '../../lib/api/types';
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

  resetForTests(): void {
    this.prompt = '';
    this.generating = false;
    this.instructionsOpen = false;
    this.triggersOpen = false;
    this.selectedNodeByWorkflow.clear();
  }
}

export const workflowsUi = new WorkflowsUiState();
