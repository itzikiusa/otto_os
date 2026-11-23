<script lang="ts">
  // The Run… panel (W2): WHERE you provide the run input the trigger emits
  // (repo_id, story_id, goals, msg, jira_ticket, …), the run's prompt and the
  // review-mode override. Its state is RunInputForm (runInputForm.svelte.ts);
  // the page starts the run.
  import { announceShown } from '../../lib/announce.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { WorkflowGraph } from '../../lib/api/types';
  import type { InspectorOptions } from './inspector/options.svelte';
  import type { RunInputForm } from './runInputForm.svelte';
  import { graphHasDbQuery, graphSideEffects } from './sideEffects';

  interface Props {
    form: RunInputForm;
    graph?: WorkflowGraph;
    wsId?: string;
    pickers?: InspectorOptions;
    /** The run POST is in flight. */
    running: boolean;
    onrun: () => void;
  }
  let { form, graph, wsId = '', pickers, running, onrun }: Props = $props();

  $effect(() => {
    if (wsId && pickers && graphHasDbQuery(graph)) pickers.dbConnections.ensure(wsId);
  });
  const touches = $derived(graph ? graphSideEffects(graph, pickers?.dbConnections.rows(wsId) ?? []) : []);
</script>

<div class="run-input">
  <div class="ri-head">
    <strong>Prompt</strong>
    <span class="ri-hint">What you want done this run — merged into the JSON below as `prompt` (optional).</span>
  </div>
  <textarea dir="auto" aria-label="Run prompt"
    class="ri-prompt"
    rows="3"
    bind:value={form.prompt}
    placeholder="What you want done — instructions for the agents."
  ></textarea>
  <div class="ri-head">
    <strong>Run input</strong>
    <span class="ri-hint">JSON the Start trigger emits to the graph — fill in repo_id / story_id / goals as needed. Leave empty to run with no input.</span>
    <button class="btn small ghost" onclick={() => form.suggest()} title="Insert the keys this workflow needs, as placeholders to replace">Suggest</button>
  </div>
  <textarea dir="ltr"
    class="ri-text mono"
    rows="8"
    bind:value={form.text}
    spellcheck="false"
    aria-label="Run input (JSON)"
    aria-invalid={form.error ? 'true' : undefined}
    aria-describedby={form.error ? 'wf-run-input-err' : undefined}
    placeholder={form.placeholder}
  ></textarea>
  {#if form.error}<p class="ri-err" id="wf-run-input-err" use:announceShown={{ text: form.error, assertive: true }}>{form.error}</p>{/if}
  {#if form.hasReview}
  <div class="ri-head">
    <strong>Review mode</strong>
    <span class="ri-hint">Overrides the execution mode of every review step in this run, including steps that set their own.</span>
  </div>
  <select
    class="ri-select"
    data-testid="run-review-mode"
    bind:value={form.reviewMode}
    title="Overrides the execution mode of every review step in this run, including steps that set their own."
  >
    <option value="">Workflow default</option>
    <option value="fan_out">Fan-out</option>
    <option value="orchestrator">Orchestrator</option>
  </select>
  {/if}
  {#if form.destinations.length}
    <p class="ri-dest" data-testid="run-result-destination">
      <Icon name="send" size={12} /> Results will be posted to {form.destinations.join(' and ')}.
    </p>
  {/if}
  {#if touches.length > 0}
    <div class="ri-touches" data-testid="run-preflight-touches">
      <div class="ri-touches-head">
        <Icon name="shield" size={12} />
        <strong>What this run will touch</strong>
        <span class="ri-hint">{touches.length} outward or mutating {touches.length === 1 ? 'step' : 'steps'}</span>
      </div>
      <ul class="ri-touch-list">
        {#each touches as item, i (`${item.nodeId}:${item.kind}:${i}`)}
          <li class="ri-touch-item">
            <span class="ri-se-pill" class:danger={item.danger}>{item.badge}</span>
            <span class="ri-se-name">{item.nodeName}</span>
            <span class="ri-se-desc">{item.summary}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
  <div class="ri-actions">
    <button class="btn small" onclick={() => (form.open = false)}>Cancel</button>
    <button class="btn primary small" disabled={running} onclick={onrun}>
      <Icon name="play" size={12} /> {running ? 'Starting…' : 'Run'}
    </button>
  </div>
</div>

<style>
  .ri-dest {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-s);
    color: var(--warning);
  }
  .ri-touches {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
  }
  .ri-touches-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .ri-touch-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ri-touch-item {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-xs);
  }
  .ri-se-pill {
    padding: 1px 6px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--warning);
    font-weight: 600;
  }
  .ri-se-pill.danger {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .ri-se-name {
    font-weight: 600;
    color: var(--text);
  }
  .ri-se-desc {
    color: var(--text-dim);
  }
  .ri-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  /* Human-approval banner */
  .run-input {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    max-height: min(50vh, 420px);
    overflow-y: auto;
  }
  .ri-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .ri-hint {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  /* `.ri-text` is the JSON run-input box; the prompt box above it is
     `.ri-prompt` — same tokens, its own hook (a single `.ri-text` is what
     identifies the run input, in the UI and in e2e). */
  .ri-text,
  .ri-prompt {
    width: 100%;
    min-height: 120px;
    resize: vertical;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  /* Same tokens as .ri-text, but sized to its content — the popover is a
     column flexbox, so an auto width needs the cross-axis opt-out too. */
  .ri-select {
    width: auto;
    align-self: flex-start;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .ri-actions {
    display: flex;
    gap: 8px;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
