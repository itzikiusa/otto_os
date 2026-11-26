<script lang="ts">
  // The editor's step inspector (ux.md §9.3): the step's kind and the file it
  // writes, its name, and its settings — the editor's own forms for agent
  // steps, the review step and the new deterministic steps; every other kind
  // keeps today's form (NodeParams) without a provider on steps that run no
  // agent. Desktop: the right column; phone: the step sheet.
  import type { NodeTypeSpec, WorkflowGraph, WorkflowNode } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import AgentStep from './AgentStep.svelte';
  import Check from './Check.svelte';
  import CheckoutPr from './CheckoutPr.svelte';
  import DataCondition from './DataCondition.svelte';
  import GitPr from './GitPr.svelte';
  import NodeParams from './NodeParams.svelte';
  import PostReviewComments from './PostReviewComments.svelte';
  import RetryFields from './RetryFields.svelte';
  import ReviewRun from './ReviewRun.svelte';
  import RunScript from './RunScript.svelte';
  import StepModel from './StepModel.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { paramStr, withParam } from './util';
  import { AGENT_KINDS, nodeIcon } from '../editor/nodeSummary';
  import { stepFile } from '../editor/stepRules';
  import { startFields } from '../runs/startForm';

  interface Props {
    node: WorkflowNode;
    graph: WorkflowGraph;
    types: readonly NodeTypeSpec[];
    wsId: string;
    pickers: InspectorOptions;
    defaultProvider: string;
    workflowName: string | undefined;
    /** Change the node (the editor marks the workflow unsaved). */
    onedit: (mutate: (n: WorkflowNode) => void) => void;
    onzoom: (field: string, label: string) => void;
  }
  let { node, graph, types, wsId, pickers, defaultProvider, workflowName, onedit, onzoom }: Props = $props();
  const ids = $props.id();

  const set = (field: string, value: unknown): void => onedit((n) => (n.params = withParam(n.params, field, value)));
  const KIND_WORDS: Record<string, string> = {
    manual_trigger: 'Start', agent_prompt: 'Agent step', review_run: 'Review step', git_checkout: 'Check out PR', script: 'Run script',
    post_review_comments: 'Post review comments', check: 'Check criteria', condition: 'Condition',
  };
  const kindWord = $derived(KIND_WORDS[node.kind] ?? types.find((t) => t.kind === node.kind)?.label ?? node.kind);
  const file = $derived(node.kind === 'manual_trigger' ? null : stepFile(graph, node.id));
  const inputKeys = $derived(startFields(graph.nodes.find((n) => n.kind === 'manual_trigger')?.params).map((f) => f.key));
  const params = $derived((node.params ?? {}) as { ok_exit_codes?: number[]; idempotent?: boolean });
  const OWN = new Set(['agent_prompt', 'git_pr', 'git_checkout', 'script', 'post_review_comments', 'condition', 'check', 'review_run']);
</script>

<div class="si" data-testid="wf-inspector" data-node-id={node.id}>
  <p class="si-kind"><Icon name={nodeIcon(node.kind)} size={12} />{kindWord}{#if file && AGENT_KINDS.includes(node.kind)}{' · writes '}{file}{/if}</p>
  <label class="si-name-l" for="{ids}-name">Name</label>
  <input dir="auto" id="{ids}-name" class="si-name" type="text" value={node.name} oninput={(e) => { const v = e.currentTarget.value; onedit((n) => (n.name = v)); }} />

  <NodeParams {node} {set} setParams={(p) => onedit((n) => (n.params = p))} {wsId} {pickers} {workflowName} {onzoom}
    body={OWN.has(node.kind) ? editorForm : undefined} />

  {#if node.kind !== 'manual_trigger' && node.kind !== 'agent_prompt'}
    <details class="si-adv">
      <summary>Advanced · retries{node.kind === 'script' ? ', exit codes' : ''}</summary>
      <div class="si-adv-body">
        <RetryFields {node} {onedit} />
        {#if node.kind === 'script'}
          <label for="{ids}-codes">Exit codes that count as success</label>
          <input dir="ltr" id="{ids}-codes" type="text" placeholder="0" value={Array.isArray(params.ok_exit_codes) ? params.ok_exit_codes.join(', ') : ''}
            oninput={(e) => { const v = e.currentTarget.value.split(',').map((s) => Number(s.trim())).filter((x) => Number.isInteger(x)); set('ok_exit_codes', v.length ? v : undefined); }} />
          <label class="si-chk"><input type="checkbox" checked={params.idempotent === true} onchange={(e) => set('idempotent', e.currentTarget.checked || undefined)} /> Safe to run again after a restart</label>
        {/if}
      </div>
    </details>
  {/if}
</div>

{#snippet editorForm()}
  {#if node.kind === 'agent_prompt'}
    <AgentStep {node} {graph} {set} {onedit} {pickers} {defaultProvider} {onzoom} />
  {:else if node.kind === 'git_pr'}
    <GitPr {node} {set} {wsId} {pickers} />
    <span class="np-label np-sec">Agent (drafts the PR message)</span>
    <StepModel value={{ provider: paramStr(node, 'provider'), model: paramStr(node, 'model'), effort: paramStr(node, 'effort') }} {defaultProvider}
      onchange={(v) => { set('provider', v.provider || undefined); set('model', v.model || undefined); set('effort', v.effort || undefined); }} />
  {:else if node.kind === 'git_checkout'}
    <CheckoutPr {node} {set} />
  {:else if node.kind === 'script'}
    <RunScript {node} {set} />
  {:else if node.kind === 'post_review_comments'}
    <PostReviewComments {node} {graph} {set} />
  {:else if node.kind === 'condition'}
    <DataCondition {node} {set} {inputKeys} />
  {:else if node.kind === 'check'}
    <Check {node} {set} {defaultProvider} />
  {:else if node.kind === 'review_run'}
    <ReviewRun {node} {set} {wsId} {pickers} pickModels />
  {/if}
{/snippet}

<style>
  .si {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-inline-size: 0;
  }
  .si-kind {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .si-name-l {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .si-name {
    font: inherit;
    font-size: var(--fs-l);
    font-weight: 600;
    padding: 4px 6px;
    margin-inline-start: -6px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
  }
  .si-name:hover {
    border-color: var(--border);
  }
  .si-name:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .si-adv {
    margin-block-start: var(--sp-3);
    padding-block-start: var(--sp-3);
    border-block-start: 1px solid var(--border);
  }
  .si :global(details > summary) {
    cursor: pointer;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .si-adv-body {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block-start: var(--sp-2);
    font-size: var(--fs-s);
  }
  .si-adv-body :global(input) {
    font: inherit;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
  }
  .si-adv-body :global(.node-hint) {
    color: var(--text-dim);
  }
  .si-chk {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .si-adv-body :global(label) {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
</style>
