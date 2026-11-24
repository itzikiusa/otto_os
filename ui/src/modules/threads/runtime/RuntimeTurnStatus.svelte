<script lang="ts">
  // The status line under a runtime thread's latest turn: live activity +
  // elapsed timer while running ("Working · Running npm test · 14s" or
  // "Background wakeup · Thinking… · 4s"), a calm waiting line when idle with
  // subagents or background work ("Waiting on 2 subagents and 1 command ·
  // <titles> · Stop" — the one live mention of subagents near the composer), or outcome
  // ("Stopped", "Stopped · no progress", "Partly done"). Each names the model
  // (and effort) the turn actually ran on when the daemon says, and a
  // finished turn the tokens it used. A failed turn is ONE calm block: the
  // cause in words (turnFailure.ts), the provider's text behind Details, and
  // its actions once — Edit · Send again, Revive only when the agent process
  // is stopped. Screen readers hear when a turn ends (not while it streams).
  import { announce } from '../../../lib/announce.svelte';
  import type { MessageAttachment } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import type { AgentTask } from './agentTasks';
  import { deriveLiveFooter } from './liveStatus';
  import { toastError } from '../../../lib/toastError';
  import { stopFamily } from './taskStops.svelte';
  import type { RuntimeProjection } from './projection';
  import { formatEffortShort } from '../../../lib/providerModels';
  import { latestTurn, workLabel, type TurnOutcome } from './timeline';
  import type { FailureView } from './turnFailure';
  import { turnUsageText } from './turnUsage';

  interface Props {
    projection?: RuntimeProjection | null;
    /** Formatted elapsed string while the turn runs, else null. */
    workingFor: string | null;
    /** The running turn is the agent's own (`origin === 'agent'`). */
    ownTurn: boolean;
    background?: readonly AgentTask[];
    /** The thread: the waiting line's Stop stops its background work. */
    sessionId?: string;
    outcome: TurnOutcome | null;
    /** The failed turn's cause in words (outcome `failed`). */
    failure?: FailureView | null;
    /** "Claude" / "Codex", for what screen readers hear. */
    agentName?: string;
    /** The agent process is stopped: the failure block offers Revive. */
    revive?: () => void;
    disabled?: boolean;
    onresend: (text: string, attachments?: readonly MessageAttachment[]) => void;
    onedit?: (text: string, attachments?: readonly MessageAttachment[]) => void;
  }
  let {
    projection = null,
    workingFor,
    ownTurn,
    background = [],
    sessionId = '',
    outcome,
    failure = null,
    agentName = 'The agent',
    revive,
    disabled = false,
    onresend,
    onedit,
  }: Props = $props();
  let showFailed = $state(false);
  let showDetail = $state(false);
  const turn = $derived(projection ? latestTurn(projection) : null);
  /** "claude-sonnet-4-5 · High": what the latest turn really ran on (absent on older daemons). */
  const ranOn = $derived([turn?.model, turn?.effort ? formatEffortShort(turn.effort) : null].filter(Boolean).join(' · '));
  const live = $derived(deriveLiveFooter(projection, workingFor, ownTurn, background));
  let stoppingAll = $state(false);
  /** Stop everything the line waits on: one family stop with the thread's own background work. */
  async function stopWaiting(): Promise<void> {
    if (stoppingAll || !sessionId) return;
    stoppingAll = true;
    try {
      await stopFamily(sessionId, true);
    } catch (e) {
      toastError('Couldn’t stop everything', e);
    } finally {
      stoppingAll = false;
    }
  }
  const retryText = $derived(outcome?.retryText ?? '');
  const retryAttachments = $derived(outcome?.retryAttachments);
  const canRetry = $derived(Boolean(retryText || (retryAttachments && retryAttachments.length > 0)));
  const tokens = $derived(turnUsageText(turn));

  // A turn this view saw running is announced once when it ends.
  let watching: string | null = null;
  $effect(() => {
    const t = turn;
    if (!t) return;
    if (t.status === 'running' || t.status === 'pending') {
      watching = t.id;
      return;
    }
    if (watching !== t.id) return;
    watching = null;
    if (t.status === 'completed') announce(`${agentName} finished`);
    else if (t.status === 'failed') announce(`Turn failed. ${failure?.title ?? ''}`, { assertive: true });
    else announce('Turn stopped');
  });
</script>

{#snippet retryActions(primary = false)}
  {#if canRetry}
    {#if onedit}
      <button
        type="button"
        class="btn small ts-btn"
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
      class="btn small ts-btn"
      class:primary
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
    <span>{live.prefix} · {live.activity} · {live.elapsed}{ranOn ? ` · ${ranOn}` : ''}</span>
  </p>
{:else if live?.mode === 'waiting_bg'}
  <div class="ts ts-bg" data-testid="runtime-bg-waiting">
    <span class="pulse-dot" aria-hidden="true"></span>
    <span class="ts-wait"><span class="ts-wait-h">{live.text}</span>{#if live.detail}<span class="ts-wait-d ellipsis" dir="auto" title={live.detail}>{live.detail}</span>{/if}</span>
    <button type="button" class="btn small ghost ts-btn" disabled={stoppingAll} data-testid="runtime-bg-stop" onclick={() => void stopWaiting()}>{stoppingAll ? 'Stopping…' : 'Stop'}</button>
  </div>
{:else if outcome?.kind === 'stopped'}
  <div class="ts" data-testid="runtime-stopped">
    <span>Stopped{ranOn ? ` · ${ranOn}` : ''}</span>
    {@render retryActions()}
  </div>
{:else if outcome?.kind === 'stalled'}
  <div class="ts" data-testid="runtime-stalled" title="No output for a long while, so Otto stopped the turn">
    <span>Stopped · no progress</span>
    {@render retryActions()}
  </div>
{:else if outcome?.kind === 'failed'}
  <div class="tf" role="group" aria-label="Turn failed" data-testid="runtime-failed" data-cause={failure?.cause}>
    <p class="tf-title" dir="auto"><Icon name="warning" size={13} /><span>{failure?.title ?? 'Turn failed'}</span></p>
    {#if failure?.hint}<p class="tf-hint" dir="auto">{failure.hint}</p>{/if}
    <div class="tf-actions">
      {@render retryActions(true)}
      {#if revive}
        <button type="button" class="btn small ts-btn" {disabled} title="Start the agent again; the conversation is kept" data-testid="runtime-turn-revive" onclick={revive}>Revive</button>
      {/if}
      {#if failure?.detail}
        <button type="button" class="btn small ghost ts-btn" aria-expanded={showDetail} data-testid="runtime-failed-details" onclick={() => (showDetail = !showDetail)}>Details</button>
      {/if}
    </div>
    {#if showDetail && failure?.detail}<pre class="tf-detail" dir="auto">{failure.detail}{ranOn ? `\n${ranOn}` : ''}</pre>{/if}
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
{:else if ranOn && turn?.status === 'completed'}
  <p class="ts" data-testid="runtime-ran-on">Ran on {ranOn}{tokens ? ` · ${tokens}` : ''}</p>
{:else if tokens && turn?.status === 'completed'}
  <p class="ts" data-testid="runtime-ran-on">{tokens}</p>
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
    flex-wrap: nowrap;
    padding-block: 4px;
  }
  .ts-wait {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex: 1;
    min-inline-size: 0;
  }
  .ts-wait-h {
    flex: none;
    color: var(--text);
  }
  .ts-wait-d {
    min-inline-size: 0;
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
  .tf {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 6px 0 0;
    padding: 8px 10px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, var(--border));
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    font-size: var(--fs-s);
  }
  .tf-title {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: var(--danger);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .tf-hint {
    margin: 0;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .tf-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .tf-detail {
    margin: 0;
    max-block-size: 160px;
    overflow: auto;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  /* Touch: the per-turn "Ran on … · N tokens" line stays for screen readers
     only — the model is on the chip, the context meter and details hold the rest. */
  @media (hover: none) {
    p.ts[data-testid='runtime-ran-on'] {
      position: absolute;
      inline-size: 1px;
      block-size: 1px;
      overflow: hidden;
      clip-path: inset(50%);
      white-space: nowrap;
    }
  }
  @media (pointer: coarse) {
    .ts-btn {
      min-block-size: calc(var(--hit-min) + 8px);
      padding-inline: 14px;
    }
  }
  .ts-failed {
    margin: 2px 0 0;
    padding-inline-start: 22px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
</style>
