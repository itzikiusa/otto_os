<script lang="ts">
  // T3-Code-style Pending Approval / Question Dock anchored onto the top edge
  // of the composer box whenever the session is waiting on tool approval or an
  // interactive terminal prompt (`status === 'approval'` or `ws.needsYou`).
  import Icon from '../../../lib/components/Icon.svelte';
  import { toastError } from '../../../lib/toastError';
  import { toasts } from '../../../lib/toast.svelte';
  import { sendRawInput, submitPrompt } from './api';
  import { askQuestions, toolLine, type ToolCallBlock } from './format';

  interface Props {
    sessionId: string;
    agentName: string;
    pendingCall?: ToolCallBlock | null;
    oncustomreply: () => void;
  }
  let { sessionId, agentName, pendingCall = null, oncustomreply }: Props = $props();

  let busyAction = $state<string | null>(null);

  const line = $derived(pendingCall ? toolLine(pendingCall) : null);
  const questions = $derived(pendingCall ? askQuestions(pendingCall.input) : []);
  const firstQuestion = $derived(questions[0] ?? null);
  const summaryTarget = $derived.by(() => {
    if (firstQuestion) return firstQuestion.question;
    if (!line) return 'Waiting for your confirmation in the session prompt';
    return line.target ? `${line.base} ${line.target}` : `wants to ${line.base}`;
  });

  async function sendDecision(key: string, raw: string, label: string): Promise<void> {
    if (busyAction) return;
    busyAction = key;
    try {
      await sendRawInput(sessionId, raw);
      toasts.info(label, `Sent to ${agentName}`);
    } catch (e) {
      toastError('Couldn’t send the response', e);
    } finally {
      busyAction = null;
    }
  }

  async function pickOption(option: string): Promise<void> {
    if (busyAction) return;
    busyAction = option;
    try {
      await submitPrompt(sessionId, option);
      toasts.info('Reply sent', option);
    } catch (e) {
      toastError('Couldn’t send the reply', e);
    } finally {
      busyAction = null;
    }
  }
</script>

<div class="approval-dock" data-approval-dock role="region" aria-label="Pending approval for {agentName}">
  <div class="dock-main">
    <span class="dock-icon" aria-hidden="true">
      <Icon name={firstQuestion ? 'comment' : 'warning'} size={14} />
    </span>
    <div class="dock-copy">
      <div class="dock-title">
        <strong>{firstQuestion ? `${agentName} has a question` : `${agentName} needs approval`}</strong>
        {#if line?.hint}<span class="dock-hint">{line.hint}</span>{/if}
      </div>
      <div class="dock-target" class:mono={!firstQuestion && !!line?.mono} dir={!firstQuestion && line?.mono ? 'ltr' : 'auto'}>
        {summaryTarget}
      </div>
    </div>
  </div>

  {#if firstQuestion && firstQuestion.options.length > 0}
    <div class="dock-actions">
      {#each firstQuestion.options as opt (opt)}
        <button
          type="button"
          class="btn small"
          disabled={busyAction !== null}
          onclick={() => void pickOption(opt)}
        >
          {opt}
        </button>
      {/each}
      <button type="button" class="btn small ghost" onclick={oncustomreply}>
        Custom reply…
      </button>
    </div>
  {:else}
    <div class="dock-actions">
      <button
        type="button"
        class="btn small primary"
        disabled={busyAction !== null}
        onclick={() => void sendDecision('yes', 'y\r', 'Approved')}
        title="Send y + Enter to approve this step"
      >
        <Icon name="check" size={12} /> Approve (y)
      </button>
      <button
        type="button"
        class="btn small"
        disabled={busyAction !== null}
        onclick={() => void sendDecision('always', 'a\r', 'Always allow sent')}
        title="Send a + Enter to allow for this session"
      >
        Always allow
      </button>
      <button
        type="button"
        class="btn small"
        disabled={busyAction !== null}
        onclick={() => void sendDecision('no', 'n\r', 'Declined')}
        title="Send n + Enter to decline"
      >
        <Icon name="x" size={12} /> No (n)
      </button>
      <button
        type="button"
        class="btn small ghost"
        disabled={busyAction !== null}
        onclick={oncustomreply}
        title="Type a custom answer in the composer below"
      >
        Custom reply…
      </button>
    </div>
  {/if}
</div>

<style>
  .approval-dock {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 12px;
    margin-block: -8px 6px;
    margin-inline: -12px -6px;
    padding: 8px 12px;
    background: var(--warning-soft);
    border-bottom: 1px solid color-mix(in srgb, var(--warning) 38%, var(--border));
    min-width: 0;
  }
  .dock-main {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    flex: 1 1 220px;
    min-width: 0;
  }
  .dock-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-block-start: 2px;
    color: var(--warning);
    flex-shrink: 0;
  }
  .dock-copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .dock-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text);
    flex-wrap: wrap;
  }
  .dock-title strong {
    font-weight: 600;
  }
  .dock-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .dock-target {
    font-size: var(--fs-xs);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .dock-target.mono {
    unicode-bidi: isolate;
    text-align: start;
  }
  .dock-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    flex-shrink: 0;
  }
</style>
