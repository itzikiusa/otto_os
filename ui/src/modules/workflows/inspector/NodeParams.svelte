<script lang="ts">
  // The selected node's per-kind param form (split out of WorkflowsPage.svelte).
  // Each kind exposes only its meaningful params; unrecognised kinds fall
  // through to a raw JSON editor. Every edit goes through the page's `set`
  // (the selected node's `onParam`), which marks the canvas dirty.
  import type { WorkflowNode } from '../../../lib/api/types';
  import AgentPrompt from './AgentPrompt.svelte';
  import BrokerPeek from './BrokerPeek.svelte';
  import Canvas from './Canvas.svelte';
  import DbQuery from './DbQuery.svelte';
  import GitPr from './GitPr.svelte';
  import HttpRequest from './HttpRequest.svelte';
  import Loop from './Loop.svelte';
  import ManualTrigger from './ManualTrigger.svelte';
  import PrepareContext from './PrepareContext.svelte';
  import ProductPublish from './ProductPublish.svelte';
  import ProductStep from './ProductStep.svelte';
  import ProviderModel from './ProviderModel.svelte';
  import RawParams from './RawParams.svelte';
  import ReviewRun from './ReviewRun.svelte';
  import SelfImprove from './SelfImprove.svelte';
  import SwarmTask from './SwarmTask.svelte';
  import Utility from './Utility.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { KINDS_WITH_OWN_PROVIDER, type SetParam } from './util';

  interface Props {
    node: WorkflowNode;
    set: SetParam;
    /** Replace the node's params wholesale (the raw-JSON fallback). */
    setParams: (params: unknown) => void;
    /** The workflow's workspace — what the pickers list. */
    wsId: string;
    pickers: InspectorOptions;
    workflowName: string | undefined;
    /** Open the page's big JSON editor for `field`. */
    onzoom: (field: string, label: string) => void;
  }
  let { node, set, setParams, wsId, pickers, workflowName, onzoom }: Props = $props();
  const UTILITY_KINDS = ['delay', 'transform', 'game_engine', 'channel_notify', 'budget_gate', 'human_approval', 'condition'];
  const PRODUCT_STEP_KINDS = ['product_analyze', 'product_rewrite', 'product_plan'];
</script>

<!-- `display: contents`: the fields stay flex items of the inspector column. -->
<div class="np-form">
  {#if node.kind === 'manual_trigger'}
    <ManualTrigger {node} {set} {wsId} {pickers} {workflowName} />
  {:else if node.kind === 'agent_prompt'}
    <AgentPrompt {node} {set} />
  {:else if node.kind === 'prepare_context'}
    <PrepareContext {node} {set} {pickers} />
  {:else if node.kind === 'http_request' || node.kind === 'api_run'}
    <HttpRequest {node} {set} {onzoom} />
  {:else if UTILITY_KINDS.includes(node.kind)}
    <Utility {node} {set} {onzoom} />
  {:else if node.kind === 'db_query'}
    <DbQuery {node} {set} {wsId} {pickers} />
  {:else if node.kind === 'broker_peek'}
    <BrokerPeek {node} {set} {wsId} {pickers} />
  {:else if node.kind === 'swarm_task'}
    <SwarmTask {node} {set} {wsId} {pickers} />
  {:else if node.kind === 'loop'}
    <Loop {node} {set} {onzoom} />
  {:else if node.kind === 'review_run'}
    <ReviewRun {node} {set} {wsId} {pickers} />
  {:else if PRODUCT_STEP_KINDS.includes(node.kind)}
    <ProductStep {node} {set} {wsId} {pickers} />
  {:else if node.kind === 'product_publish'}
    <ProductPublish {node} {set} {wsId} {pickers} />
  {:else if node.kind === 'canvas'}
    <Canvas {node} {set} />
  {:else if node.kind === 'git_pr'}
    <GitPr {node} {set} {wsId} {pickers} />
  {:else if node.kind === 'self_improve'}
    <SelfImprove {node} {set} />
  {:else if node.kind !== 'log' && node.kind !== 'verifier'}
    <RawParams {node} {setParams} />
  {/if}

  <!-- Universal Provider + Model: EVERY node that doesn't render its own
       provider control gets one here, so no node is ever missing it. For
       agent-running kinds (git_pr drafts the PR message, swarm_task, …) it
       drives the agent; for pure utility nodes it's simply present and unused.
       Kinds with their own control are excluded to avoid a duplicate. -->
  {#if !KINDS_WITH_OWN_PROVIDER.includes(node.kind) && node.kind !== 'manual_trigger'}
    <ProviderModel {node} {set} />
  {/if}
</div>

<style>
  /* The forms' styles, moved with them from WorkflowsPage. Rules the page had
     scoped under `.inspector` carry a doubled `.np-form.np-form`, plain ones a
     single `.np-form`, so the two keep the cascade order they had there.
     `:where(:not(.np-mp *))` keeps ModelPicker's own label/select untouched,
     as the page's scoped rules never reached them. */
  .np-form,
  .np-form :global(.np-mp) {
    display: contents;
  }
  .np-form :global(textarea) {
    width: 100%;
    resize: vertical;
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.45;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
  }
  /* EVERY textarea in the inspector holds a prompt, an instruction list or a
     JSON blob: a readable floor, capped against the viewport, the CONTENT
     scrolls inside the box. Node-kind agnostic on purpose. */
  .np-form.np-form :global(textarea) {
    min-height: 110px;
    max-height: min(45vh, 460px);
    overflow-y: auto;
  }
  .np-form :global(textarea:focus) {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  /* Prompt-sized textareas (reviewer/summarizer instructions, goals, checks). */
  .np-form.np-form :global(textarea.np-prompt) {
    min-height: 120px;
    max-height: min(40vh, 380px);
    overflow-y: auto;
    resize: vertical;
  }
  .np-form.np-form :global(textarea.np-prompt-lg) {
    min-height: 260px;
    max-height: min(60vh, 620px);
  }
  .np-form :global(.insp-note) {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .np-form :global(.insp-slack) {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .np-form :global(.is-head) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .np-form :global(.is-snip) {
    margin: 0;
    padding: 8px 10px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.5;
    white-space: pre-wrap;
    color: var(--text);
    overflow-x: auto;
  }
  /* Each label opens a field group — so a node with a dozen params doesn't
     read as one wall of controls. */
  .np-form.np-form :global(label:where(:not(.np-mp *))) {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    margin-top: 6px;
  }
  /* …except a first label inside a group (not the form's own first field,
     which follows the inspector header) and the compact-row labels. */
  .np-form.np-form :global(:not(.np-form) > label:first-child:where(:not(.np-mp *))),
  .np-form.np-form :global(.np-chk),
  .np-form.np-form :global(.rv-sc),
  .np-form.np-form :global(.rv-chip) {
    margin-top: 0;
  }
  /* Opens a major section inside a node's params (Reviewers, Summarizer,
     Scoring, Goals, Checks, loop Steps …). */
  .np-form.np-form :global(.np-sec) {
    margin-top: 16px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .np-form :global(.np-label) {
    display: block;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .np-form.np-form :global(input[type='text']),
  .np-form.np-form :global(input[type='url']),
  .np-form.np-form :global(input[type='number']) {
    width: 100%;
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
  }
  .np-form.np-form :global(input[type='text']:focus),
  .np-form.np-form :global(input[type='url']:focus),
  .np-form.np-form :global(input[type='number']:focus),
  .np-form.np-form :global(select:where(:not(.np-mp *)):focus) {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .np-form.np-form :global(select:where(:not(.np-mp *))) {
    width: 100%;
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  /* Node-form JSON field: label + zoom button; the editor lives in a modal. (R10) */
  .np-form :global(.np-jsonlabel) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .np-form :global(.np-zoom) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 2px 4px;
    border-radius: var(--radius-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .np-form :global(.np-zoom:hover) {
    color: var(--text);
    background: var(--accent-soft);
  }
  .np-form :global(.node-hint) {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin: 2px 0 0;
  }
  .np-form :global(.np-chk) {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
    margin-top: 2px;
  }
  .np-form :global(.np-chk input) {
    width: auto;
  }
  /* review_run reviewer editor (per-lens agents + summarizer + scoring) */
  .np-form :global(.rv-h) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 4px;
  }
  /* A reviewer list is never capped or scrolled on its own: EVERY agent
     renders at full size and the inspector panel scrolls. */
  .np-form :global(.rv-list) {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .np-form :global(.rv-row) {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px;
    /* A tint so each agent reads as its own card in a long list. */
    background: color-mix(in srgb, var(--surface-2) 55%, transparent);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .np-form :global(.rv-top) {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .np-form :global(.rv-lens) {
    flex: 1;
  }
  /* loop sub-step editor */
  .np-form :global(.ls-kind) {
    flex: 0 0 auto;
    max-width: 150px;
  }
  .np-form :global(.ls-pm) {
    display: flex;
    gap: 6px;
  }
  .np-form :global(.ls-prov),
  .np-form :global(.ls-model) {
    flex: 1;
    min-width: 0;
  }
  .np-form :global(.ls-advanced) {
    margin-top: 2px;
  }
  .np-form :global(.ls-advanced summary) {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }
  .np-form :global(.rv-del) {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px;
  }
  .np-form :global(.rv-del:hover) {
    color: var(--danger);
  }
  .np-form :global(.rv-provs) {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .np-form :global(.rv-chip) {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
    cursor: pointer;
    color: var(--text-dim);
  }
  .np-form :global(.rv-chip.on) {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--text);
  }
  .np-form :global(.rv-chip input) {
    width: auto;
    margin: 0;
  }
  .np-form :global(.rv-instr) {
    width: 100%;
    font-size: var(--fs-s);
  }
  .np-form :global(.rv-score) {
    display: flex;
    gap: 8px;
  }
  .np-form :global(.rv-sc) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex: 1;
  }
  .np-form :global(.rv-sc input) {
    width: 100%;
  }
</style>
