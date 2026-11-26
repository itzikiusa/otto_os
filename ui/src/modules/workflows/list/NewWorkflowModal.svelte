<script lang="ts">
  // "New workflow" (ux.md §8; a lazy chunk): describe the flow and Generate,
  // start from a template, or start blank. Each lands in the editor of the
  // new workflow (`oncreated`). The describe-it text survives closing (it is
  // workflowsUi.prompt), so a half-written description is never lost.
  import { untrack } from 'svelte';
  import { focusOnMount } from '../../../lib/focusOnMount';
  import Icon, { asIcon } from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { api } from '../../../lib/api/client';
  import type { Workflow, WorkflowTemplate } from '../../../lib/api/types';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { workflowsUi } from '../workflowsUiState.svelte';
  import { createBlankWorkflow } from './workflowActions';

  interface Props {
    workspaceId: string;
    oncreated: (wf: Workflow) => void;
    onclose: () => void;
  }
  let { workspaceId, oncreated, onclose }: Props = $props();

  let templates = $state<WorkflowTemplate[]>([]);
  let busy = $state(false);
  $effect(() => {
    untrack(() => void api.get<WorkflowTemplate[]>('/workflows/templates').then((t) => (templates = t), () => {}));
  });

  async function done(p: Promise<Workflow | null>): Promise<void> {
    busy = true;
    try {
      const wf = await p;
      if (wf) oncreated(wf);
    } finally {
      busy = false;
    }
  }
  const generate = () => void done(workflowsUi.generate(workspaceId));
  const blank = () => void done(createBlankWorkflow(workspaceId));
  const fromTemplate = (t: WorkflowTemplate) => void done(
    api.post<Workflow>(`/workspaces/${workspaceId}/workflows/from-template`, { template_id: t.id }).then(
      (wf) => { toasts.success(`Created “${wf.name}”`, 'Ready to run.'); return wf; },
      (e) => { toastError('Couldn’t create the workflow from the template', e); return null; },
    ),
  );
</script>

<Modal title="New workflow" width={640} {onclose} dismissable={!workflowsUi.generating}>
  <div class="nw" data-testid="wf-new-workflow">
    <label for="wf-prompt">Describe the flow</label>
    <textarea dir="auto" id="wf-prompt" rows="4" use:focusOnMount bind:value={workflowsUi.prompt}
      placeholder="e.g. Ask an agent to summarize the repo, then POST the summary to our webhook."
      onkeydown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) generate(); }}></textarea>
    {#if templates.length}
      <h3>Or start from a template</h3>
      <div class="tpls">
        {#each templates as t (t.id)}
          <button type="button" class="tpl" disabled={busy} title={t.description} onclick={() => fromTemplate(t)}>
            <span class="ic"><Icon name={asIcon(t.icon, 'box')} size={14} /></span>
            <span class="tb"><span class="tn">{t.name}</span>{#if t.description}<span class="ts">{t.description}</span>{/if}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn ghost" disabled={busy} onclick={blank}><Icon name="plus" size={13} /> Start blank</button>
    <span class="grow"></span>
    <button type="button" class="btn" onclick={onclose}>Cancel</button>
    <button type="button" class="btn primary" disabled={busy || workflowsUi.generating || !workflowsUi.prompt.trim()}
      title={workflowsUi.prompt.trim() ? 'Generate the workflow (⌘↵)' : 'Describe the flow first'} onclick={generate}>
      {#if workflowsUi.generating}<span class="spinner" style="--spinner-size: 11px" aria-hidden="true"></span> Building…{:else}<Icon name="zap" size={13} /> Generate{/if}
    </button>
  {/snippet}
</Modal>

<style>
  .nw {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  label,
  h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  h3 {
    margin-block-start: var(--sp-5);
  }
  textarea {
    inline-size: 100%;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    resize: vertical;
  }
  .tpls {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: var(--sp-3);
  }
  .tpl {
    display: flex;
    gap: var(--sp-4);
    align-items: flex-start;
    min-block-size: 44px;
    padding: var(--sp-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    text-align: start;
    cursor: pointer;
  }
  .tpl:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--surface-2);
  }
  .ic {
    flex: none;
    color: var(--text-dim);
  }
  .tb {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }
  .tn {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .ts {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
</style>
