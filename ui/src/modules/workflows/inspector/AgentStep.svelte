<script lang="ts">
  // An agent step (ux.md §9.3, mockup 06): the prompt; the agent as provider ·
  // model · effort (Default = the provider's own setting); skills; a new
  // thread or the thread of an earlier step; and, folded, the time limit,
  // retries and keeping the thread open.
  import type { WorkflowGraph, WorkflowNode } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import RetryFields from './RetryFields.svelte';
  import SkillsField from './SkillsField.svelte';
  import StepModel from './StepModel.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { paramBool, paramNum, paramStr, type SetParam } from './util';
  import { providerName } from '../editor/nodeSummary';
  import { continueChoices, stepProvider } from '../editor/stepRules';

  interface Props {
    node: WorkflowNode;
    graph: WorkflowGraph;
    set: SetParam;
    onedit: (mutate: (n: WorkflowNode) => void) => void;
    pickers: InspectorOptions;
    defaultProvider: string;
    onzoom: (field: string, label: string) => void;
  }
  let { node, graph, set, onedit, pickers, defaultProvider, onzoom }: Props = $props();
  const ids = $props.id();

  const provider = $derived(stepProvider(node, defaultProvider));
  const from = $derived(paramStr(node, 'continue_from'));
  const choices = $derived(continueChoices(graph, node, defaultProvider));
  const eligible = $derived(choices.filter((c) => !c.reason));
  const source = $derived(graph.nodes.find((n) => n.id === from));
  const skills = $derived(Array.isArray((node.params as { skills?: unknown } | null)?.skills) ? ((node.params as { skills: unknown[] }).skills.filter((s) => typeof s === 'string') as string[]) : []);
  const minutes = $derived(paramNum(node, 'timeout_s', 0) / 60);

  function setModel(v: { provider: string; model: string; effort: string }): void {
    set('provider', v.provider || undefined);
    set('model', v.model || undefined);
    set('effort', v.effort || undefined);
    // A thread runs on one provider: a continue that no longer matches goes.
    if (from && stepProvider(source ?? node, defaultProvider) !== (v.provider || defaultProvider)) set('continue_from', undefined);
  }
</script>

<div class="np-jsonlabel">
  <label for="{ids}-prompt">Prompt</label>
  <button type="button" class="np-zoom" aria-label="Open the prompt in a larger editor" title="Open the prompt in a larger editor" onclick={() => onzoom('prompt', 'Prompt')}>
    <Icon name="maximize" size={12} />
  </button>
</div>
<textarea dir="auto" id="{ids}-prompt" class="np-prompt np-prompt-lg" rows="10" value={paramStr(node, 'prompt')} oninput={(e) => set('prompt', e.currentTarget.value)}></textarea>
<p class="node-hint">Insert run data with {'{input.…}'} and {'{steps.…}'}.</p>

<span class="np-label np-sec">Agent</span>
<StepModel
  value={{ provider: paramStr(node, 'provider'), model: paramStr(node, 'model'), effort: paramStr(node, 'effort') }}
  {defaultProvider}
  sameAs={source ? source.name || source.id : null}
  onchange={setModel}
/>
<p class="node-hint">Default follows the provider’s own setting, as in Threads.</p>

<span class="np-label np-sec">Skills</span>
<SkillsField {skills} {provider} {pickers} onchange={(s) => set('skills', s)} />

<span class="np-label np-sec" id="{ids}-conv">Conversation</span>
<div class="as-conv" role="radiogroup" aria-labelledby="{ids}-conv" data-testid="wf-continue-from">
  <label class="as-radio">
    <input type="radio" name="{ids}-conv" checked={!from} onchange={() => set('continue_from', undefined)} />
    <span><span>New thread</span><span class="node-hint">Starts clean, like every step today.</span></span>
  </label>
  <label class="as-radio" class:off={eligible.length === 0}>
    <input type="radio" name="{ids}-conv" checked={!!from} disabled={eligible.length === 0} onchange={() => set('continue_from', eligible[0]?.node.id)} />
    <span>
      <span>Continue the thread of an earlier step</span>
      {#if eligible.length === 0}<span class="node-hint">No earlier {providerName(provider)} agent step to continue.</span>{/if}
    </span>
  </label>
  {#if from}
    <select aria-label="The step whose thread this step continues" value={from} onchange={(e) => set('continue_from', e.currentTarget.value)}>
      {#each choices as c (c.node.id)}
        <option value={c.node.id} disabled={!!c.reason} title={c.reason ?? ''}>{c.node.name || c.node.id}{c.reason ? ` — ${providerName(stepProvider(c.node, defaultProvider))}` : ''}</option>
      {/each}
    </select>
    {#each choices.filter((c) => c.reason) as c (c.node.id)}<p class="node-hint">{c.reason}</p>{/each}
  {/if}
</div>

<details class="as-adv np-sec">
  <summary>Advanced · time limit, retries, thread</summary>
  <div class="as-adv-body">
    <label for="{ids}-limit">Fail the step if it hasn’t finished after (minutes)</label>
    <input id="{ids}-limit" type="number" min="1" max="600" placeholder="No limit" value={minutes || ''}
      oninput={(e) => { const m = Number(e.currentTarget.value); set('timeout_s', m > 0 ? Math.round(Math.min(600, Math.max(1, m)) * 60) : undefined); }} />
    <p class="node-hint">Counts time waiting on you too. A step past its limit fails as Timed out and is not retried.</p>
    <RetryFields {node} {onedit} />
    <label class="np-chk">
      <input type="checkbox" checked={paramBool(node, 'keep_session')} onchange={(e) => set('keep_session', e.currentTarget.checked || undefined)} />
      Keep this step’s thread open after it finishes
    </label>
  </div>
</details>

<style>
  .as-conv {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-inline-size: 0;
  }
  .as-conv .as-radio {
    margin: 0;
    font-weight: 400;
  }
  .as-radio {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .as-radio > span {
    display: flex;
    flex-direction: column;
  }
  .as-radio input {
    margin-block-start: 2px;
  }
  .as-radio.off > span > span:first-child {
    color: var(--text-dim);
  }
  .as-adv summary {
    cursor: pointer;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .as-adv-body {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block-start: var(--sp-2);
  }
</style>
