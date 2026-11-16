<script lang="ts">
  // The status line under a runtime thread's latest turn: live activity +
  // elapsed timer while running ("Working · Running npm test · 14s" or
  // "Background wakeup · Thinking… · 4s"), a calm waiting line when idle with
  // scheduled/background tasks ("Waiting on 1 background task: …"), or outcome
  // ("Stopped", "Stopped · no progress", "Failed", "Partly done").
  import type { MessageAttachment } from '../../../lib/api/types';
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
    onresend: (text: string, attachments?: readonly MessageAttachment[]) => void;
    onedit?: (text: string, attachments?: readonly MessageAttachment[]) => void;
  }
  let {
    projection = null,
    workingFor,
    ownTurn,
    background = [],
    outcome,
    disabled = false,
    onresend,
    onedit,
  }: Props = $props();
  let showFailed = $state(false);
  const live = $derived(deriveLiveFooter(projection, workingFor, ownTurn, background));
  const retryText = $derived(outcome?.retryText ?? '');
  const retryAttachments = $derived(outcome?.retryAttachments);
  const canRetry = $derived(Boolean(retryText || (retryAttachments && retryAttachments.length > 0)));
</script>

{#snippet retryActions()}
  {#if canRetry}
    {#if onedit}
      <button
        type="button"
        class="btn small"
        {disabled}
        title="Restore this message and its attachments into the composer"
        data-testid="runtime-turn-edit"
        onclick={() => onedit(retryText, retryAttachments)}
      >
        Edit
      </button>
    {/if}
    <button
      type="button"
      class="btn small"
      {disabled}
      title="Send the same message and attachments again"
      data-testid="runtime-turn-resend"
      onclick={() => onresend(retryText, retryAttachments)}
    >
      Send again
    </button>
  {/if}
{/snippet}

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
  <div class="ts" data-testid="runtime-stopped">
    <span>Stopped</span>
    {@render retryActions()}
  </div>
{:else if outcome?.kind === 'stalled'}
  <div class="ts" data-testid="runtime-stalled" title="No output for a long while, so Otto stopped the turn">
    <span>Stopped · no progress</span>
    {@render retryActions()}
  </div>
{:else if outcome?.kind === 'failed'}
  <div class="ts" data-testid="runtime-failed">
    <Icon name="warning" size={12} />
    <span>Turn failed</span>
    {@render retryActions()}
  </div>
{:else if outcome?.kind === 'partial'}
  <div class="ts" data-testid="runtime-partial">
    <Icon name="info" size={12} />Partly done
    <button type="button" class="btn small" aria-expanded={showFailed} onclick={() => (showFailed = !showFailed)}>Details</button>
    {@render retryActions()}
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
    flex-wrap: wrap;
    gap: 6px;
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
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
