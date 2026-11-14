<script lang="ts">
  // The status line under a runtime thread's latest turn: live activity +
  // elapsed timer while running ("Working · Running npm test · 14s" or
  // "Background wakeup · Thinking… · 4s"), a calm waiting line when idle with
  // scheduled/background tasks ("Waiting on 1 background task: …"), or outcome
  // ("Stopped", "Stopped · no progress", "Partly done").
  import Icon from '../../../lib/components/Icon.svelte';
  import type { AgentTask } from './agentTasks';
  import { deriveLiveFooter } from './liveStatus';
  import type { RuntimeProjection } from './projection';
  import { workLabel, type TurnOutcome } from './timeline';

  interface Props {
    projection?: RuntimeProjection | null;
    /** Formatted elapsed string while the turn runs, else null. */
    workingFor: string | null;
    /** The running turn is the agent's own (`origin === 'agent'`). */
    ownTurn: boolean;
    background?: readonly AgentTask[];
    outcome: TurnOutcome | null;
    disabled?: boolean;
    onresend: (text: string) => void;
  }
  let {
    projection = null,
    workingFor,
    ownTurn,
    background = [],
    outcome,
    disabled = false,
    onresend,
  }: Props = $props();
  let showFailed = $state(false);
  const live = $derived(deriveLiveFooter(projection, workingFor, ownTurn, background));
</script>

{#if live?.mode === 'working'}
  <p class="ts" data-testid="runtime-working">
    <span class="spinner" aria-hidden="true"></span>
    <span>{live.prefix} · {live.activity} · {live.elapsed}</span>
  </p>
{:else if live?.mode === 'waiting_bg'}
  <p class="ts ts-bg" data-testid="runtime-bg-waiting">
    <span class="pulse-dot" aria-hidden="true"></span>
    <span>{live.text}</span>
  </p>
{:else if outcome?.kind === 'stopped'}
  <p class="ts">Stopped</p>
{:else if outcome?.kind === 'stalled'}
  <p class="ts" title="No output for a long while, so Otto stopped the turn">Stopped · no progress</p>
{:else if outcome?.kind === 'partial'}
  <div class="ts" data-testid="runtime-partial">
    <Icon name="info" size={12} />Partly done
    <button type="button" class="btn small" aria-expanded={showFailed} onclick={() => (showFailed = !showFailed)}>Details</button>
    {#if outcome.retryText}
      {@const again = outcome.retryText}
      <button type="button" class="btn small" {disabled} title="Send the same message again" onclick={() => onresend(again)}>Send again</button>
    {/if}
  </div>
  {#if showFailed}
    <ul class="ts-failed" aria-label="Steps that failed">
      {#each outcome.failed as f (f.id)}<li dir="auto">{workLabel(f)}</li>{/each}
    </ul>
  {/if}
{/if}

<style>
  .ts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .ts .spinner {
    --spinner-size: 11px;
  }
  .ts-bg {
    padding-block: 4px;
  }
  .pulse-dot {
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--accent-text);
    animation: otto-pulse 2s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse-dot {
      animation: none;
    }
  }
  .ts-failed {
    margin: 2px 0 0;
    padding-inline-start: 22px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
</style>
