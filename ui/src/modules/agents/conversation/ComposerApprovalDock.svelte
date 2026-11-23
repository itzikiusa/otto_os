<script lang="ts">
  // Pending Approval / Question Dock anchored onto the top edge of the
  // composer box whenever a legacy PTY session is waiting on tool approval or
  // an interactive terminal prompt (`status === 'approval'` or `ws.needsYou`).
  // Never sends blind keystrokes (`y`/`a`/`n`) to providers with interactive
  // TUI menus (Claude Code) or generic prompts where the expected input isn't
  // a y/n confirmation; shows the full command/arguments before acting.
  import Icon from '../../../lib/components/Icon.svelte';
  import { toastError } from '../../../lib/toastError';
  import { toasts } from '../../../lib/toast.svelte';
  import { sendRawInput, submitPrompt } from './api';
  import { askQuestions, toolLine, type ToolCallBlock } from './format';

  interface Props {
    sessionId: string;
    agentName: string;
    provider?: string;
    pendingCall?: ToolCallBlock | null;
    oncustomreply: () => void;
    onopenterminal?: () => void;
  }
  let {
    sessionId,
    agentName,
    provider = '',
    pendingCall = null,
    oncustomreply,
    onopenterminal,
  }: Props = $props();

  let busyAction = $state<string | null>(null);
  let showDetails = $state(false);

  const line = $derived(pendingCall ? toolLine(pendingCall) : null);
  const questions = $derived(pendingCall ? askQuestions(pendingCall.input) : []);
  const firstQuestion = $derived(questions[0] ?? null);
  const summaryTarget = $derived.by(() => {
    if (firstQuestion) return firstQuestion.question;
    if (!line) return 'Waiting for your response in the session prompt';
    return line.target ? `${line.base} ${line.target}` : `wants to ${line.base}`;
  });
  const detailsText = $derived.by(() => {
    if (firstQuestion || !pendingCall?.input || typeof pendingCall.input !== 'object') return '';
    try {
      const raw = JSON.stringify(pendingCall.input, null, 2);
      return raw && raw !== '{}' ? raw : '';
    } catch {
      return '';
    }
  });
  // Only providers with plain y/n PTY prompts (and a known pending tool call)
  // can safely use raw `y\r`/`n\r`; Claude Code's TUI uses numbered/arrow menus
  // and generic `ws.needsYou` prompts may expect text or passwords.
  const canSendYesNo = $derived(
    !!pendingCall && !firstQuestion && provider !== '' && !provider.toLowerCase().startsWith('claude'),
  );

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
        <strong>{firstQuestion ? `${agentName} has a question` : pendingCall ? `${agentName} needs approval` : `${agentName} is waiting for input`}</strong>
        {#if line?.hint}<span class="dock-hint">{line.hint}</span>{/if}
        {#if detailsText}
          <button
            type="button"
            class="dock-details-btn"
            aria-expanded={showDetails}
            onclick={() => (showDetails = !showDetails)}
          >
            {showDetails ? 'Hide details' : 'Details'}
          </button>
        {/if}
      </div>
      <div class="dock-target" class:mono={!firstQuestion && !!line?.mono} dir={!firstQuestion && line?.mono ? 'ltr' : 'auto'}>
        {summaryTarget}
      </div>
      {#if showDetails && detailsText}
        <pre class="dock-details mono" dir="ltr">{detailsText}</pre>
      {/if}
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
  {:else if canSendYesNo}
    <div class="dock-actions">
      <button
        type="button"
        class="btn small"
        disabled={busyAction !== null}
        onclick={() => void sendDecision('no', 'n\r', 'Denied')}
        title="Send n + Enter to deny"
      >
        <Icon name="x" size={12} /> Deny (n)
      </button>
      <button
        type="button"
        class="btn small primary"
        disabled={busyAction !== null}
        onclick={() => void sendDecision('yes', 'y\r', 'Approved')}
        title="Send y + Enter to approve this step"
      >
        <Icon name="check" size={12} /> Approve (y)
      </button>
      {#if onopenterminal}
        <button
          type="button"
          class="btn small ghost"
          disabled={busyAction !== null}
          onclick={onopenterminal}
          title="Open the terminal to see all prompt options"
        >
          <Icon name="terminal" size={12} /> Terminal
        </button>
      {/if}
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
  {:else}
    <div class="dock-actions">
      {#if onopenterminal}
        <button
          type="button"
          class="btn small primary"
          onclick={onopenterminal}
          title="Open the terminal to answer {agentName}’s interactive prompt"
        >
          <Icon name="terminal" size={12} /> Open terminal
        </button>
      {/if}
      <button
        type="button"
        class="btn small"
        class:primary={!onopenterminal}
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
    align-items: flex-start;
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
    gap: 4px;
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
  .dock-details-btn {
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .dock-details-btn:hover {
    text-decoration: underline;
  }
  .dock-target {
    font-size: var(--fs-xs);
    line-height: 1.45;
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-block-size: 6em;
    overflow-y: auto;
    min-width: 0;
  }
  .dock-target.mono {
    unicode-bidi: isolate;
    text-align: start;
  }
  .dock-details {
    margin: 2px 0 0;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    line-height: 1.4;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-block-size: 140px;
    overflow-y: auto;
  }
  .dock-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    flex-shrink: 0;
  }
</style>
