<script lang="ts">
  // loop: max iterations / until, and a structured sub-step editor — the loop
  // body runs these in order each iteration. Agent sub-steps expose Provider +
  // Model + Prompt directly (no more raw-JSON editing to set a provider).
  import type { WorkflowNode } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
  import JsonLabel from './JsonLabel.svelte';
  import {
    LOOP_AGENT_KINDS, LOOP_STEP_KINDS, jsonValue, loopSteps as loopStepsOf, loopStepParam as stepParam, loopStepParamList as stepParamList,
    loopStepSummarizer as stepSummarizer, paramBool, paramJson, paramNum, paramStr, withLoopStepParam, withLoopStepSummarizer,
    type LoopStep, type SetParam,
  } from './util';

  let { node, set, onzoom }: { node: WorkflowNode; set: SetParam; onzoom: (field: string, label: string) => void } = $props();
  function loopSteps(): LoopStep[] {
    return loopStepsOf(node);
  }
  function setLoopSteps(steps: LoopStep[]): void {
    set('steps', steps.length ? steps : undefined);
  }
  function addLoopStep(): void {
    setLoopSteps([...loopSteps(), { kind: 'agent_prompt', name: '', params: {} }]);
  }
  function removeLoopStep(i: number): void {
    setLoopSteps(loopSteps().filter((_, idx) => idx !== i));
  }
  function updateLoopStep(i: number, patch: Partial<LoopStep>): void {
    setLoopSteps(loopSteps().map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
  }
  const loopStepParam = (i: number, key: string): string => stepParam(loopSteps()[i], key);
  const loopStepParamList = (i: number, key: string): string => stepParamList(loopSteps()[i], key);
  const loopStepSummarizer = (i: number): string => stepSummarizer(loopSteps()[i]);
  function updateLoopStepParam(i: number, key: string, value: unknown): void {
    const step = loopSteps()[i];
    if (step) updateLoopStep(i, { params: withLoopStepParam(step, key, value) });
  }
  function updateLoopStepSummarizer(i: number, provider: string): void {
    const step = loopSteps()[i];
    if (step) updateLoopStep(i, { params: withLoopStepSummarizer(step, provider) });
  }
</script>

<label for="np-maxiter">Max iterations (1–10)</label>
<input
  id="np-maxiter"
  type="number"
  min="1"
  max="10"
  value={paramNum(node, 'max_iterations', 3)}
  oninput={(e) => set('max_iterations', Number(e.currentTarget.value))}
/>
<label for="np-until">Until (expression, optional)</label>
<input dir="auto"
  id="np-until"
  type="text"
  placeholder="e.g. passed == true"
  value={paramStr(node, 'until')}
  oninput={(e) => set('until', e.currentTarget.value)}
/>
<!-- Structured sub-step editor: the loop body runs these in order
     each iteration. Agent sub-steps expose Provider + Model + Prompt
     directly (no more raw-JSON editing to set a provider). -->
<div class="rv-h np-sec">
  <span class="np-label">Steps — run in order each iteration</span>
  <button class="btn small ghost" type="button" onclick={addLoopStep}>
    <Icon name="plus" size={12} /> Add step
  </button>
</div>
{#if loopSteps().length === 0}
  <p class="insp-note">No steps yet — add one (e.g. a <code>review_run</code> then an <code>agent_prompt</code> “fix”).</p>
{/if}
{#each loopSteps() as step, i (i)}
  <div class="rv-row">
    <div class="rv-top">
      <select aria-label="Step kind"
        class="ls-kind"
        value={step.kind}
        onchange={(e) => updateLoopStep(i, { kind: e.currentTarget.value })}
      >
        {#each LOOP_STEP_KINDS as k (k)}<option value={k}>{k}</option>{/each}
        {#if !LOOP_STEP_KINDS.includes(step.kind)}<option value={step.kind}>{step.kind}</option>{/if}
      </select>
      <input dir="auto" aria-label="Step name"
        class="rv-lens"
        type="text"
        placeholder="name (e.g. fix)"
        value={step.name ?? ''}
        oninput={(e) => updateLoopStep(i, { name: e.currentTarget.value || undefined })}
      />
      <button class="rv-del" type="button" title="Remove step" aria-label="Remove step" onclick={() => removeLoopStep(i)}>
        <Icon name="trash" size={12} />
      </button>
    </div>
    {#if LOOP_AGENT_KINDS.includes(step.kind)}
      <textarea dir="auto" aria-label="Step prompt"
        class="rv-instr np-prompt"
        rows="6"
        placeholder="prompt / instructions for this agent step"
        value={loopStepParam(i, 'prompt')}
        oninput={(e) => updateLoopStepParam(i, 'prompt', e.currentTarget.value)}
      ></textarea>
      <div class="ls-pm">
        <select aria-label="Step provider"
          class="ls-prov"
          value={loopStepParam(i, 'provider')}
          onchange={(e) => updateLoopStepParam(i, 'provider', e.currentTarget.value || undefined)}
        >
          <option value="">default ({defaultAgentProvider()})</option>
          {#each agentProviders() as pv (pv)}<option value={pv}>{pv}</option>{/each}
        </select>
        <input dir="ltr" aria-label="Step model"
          class="ls-model"
          type="text"
          placeholder="model (optional)"
          value={loopStepParam(i, 'model')}
          oninput={(e) => updateLoopStepParam(i, 'model', e.currentTarget.value || undefined)}
        />
      </div>
    {:else if step.kind === 'review_run'}
      <input dir="auto" aria-label="Reviewer providers"
        class="rv-lens"
        type="text"
        placeholder="reviewer providers (comma-separated, e.g. claude, codex)"
        value={loopStepParamList(i, 'providers')}
        oninput={(e) => updateLoopStepParam(i, 'providers', e.currentTarget.value.split(',').map((s) => s.trim()).filter(Boolean))}
      />
      <input dir="auto" aria-label="Summarizer provider"
        class="rv-lens"
        type="text"
        placeholder="summarizer provider (optional)"
        value={loopStepSummarizer(i)}
        oninput={(e) => updateLoopStepSummarizer(i, e.currentTarget.value)}
      />
    {:else}
      <p class="insp-note">
        <code>{step.kind}</code> takes no agent — edit its params in Advanced (JSON) below.
      </p>
    {/if}
  </div>
{/each}
<label class="np-chk">
  <input
    type="checkbox"
    checked={paramBool(node, 'continue_on_error')}
    onchange={(e) => set('continue_on_error', e.currentTarget.checked)}
  /> Continue on step error
</label>
<details class="ls-advanced">
  <summary>Advanced — edit steps as raw JSON</summary>
  <JsonLabel forId="np-steps" label="Steps (JSON array)" onzoom={() => onzoom('steps', 'Steps (JSON array)')} />
  <textarea dir="ltr"
    id="np-steps"
    rows="5"
    placeholder={'[ { "kind": "agent_prompt", "params": {} } ]'}
    value={paramJson(node, 'steps')}
    oninput={(e) => set('steps', jsonValue(e.currentTarget.value))}
  ></textarea>
</details>
