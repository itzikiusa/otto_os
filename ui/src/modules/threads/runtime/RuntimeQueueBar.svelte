<script lang="ts">
  // Above a runtime thread's composer: background work the agent started that
  // is still running, the messages waiting in the thread's (durable,
  // server-side) queue — Send now (into the running turn, where the agent
  // takes it; it never stops the turn), Edit, Remove — and, on a closed thread, the
  // "Revive" line for a closed or dormant agent.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { PromptDelivery, QueuedPrompt } from '../../../lib/api/types';
  import { backgroundStripText, type AgentTask } from './agentTasks';
  import { toastError } from '../../../lib/toastError';
  import { runtimeThreads } from './runtimeThreads.svelte';

  interface Props {
    sessionId: string;
    /** Running background work (agentTasks.ts), oldest first. */
    background: AgentTask[];
    /** Show every task (Threads: Thread details' Agents & tasks). */
    onshowtasks?: () => void;
    queued: QueuedPrompt[];
    running: boolean;
    canSteer: boolean;
    closed: boolean;
    dormant: boolean;
    reopening: boolean;
    onreopen: () => void;
    /** Put a queued message back in the composer. */
    onedit: (text: string) => void;
  }
  let { sessionId, background, onshowtasks, queued, running, canSteer, closed, dormant, reopening, onreopen, onedit }: Props = $props();
  const bgText = $derived(backgroundStripText(background));
  const bgTitle = $derived(background.map((b) => b.label).join('\n'));

  async function deliver(q: QueuedPrompt, delivery: PromptDelivery): Promise<void> {
    try {
      await runtimeThreads.deliverQueued(sessionId, q.id, delivery);
    } catch (e) {
      toastError(delivery === 'steer' ? 'Couldn’t send it into the turn' : 'Couldn’t send it now', e);
    }
  }
  async function edit(q: QueuedPrompt): Promise<void> {
    try {
      await runtimeThreads.unqueue(sessionId, q.id);
      onedit(q.text);
    } catch (e) {
      toastError('Couldn’t edit the queued message', e);
    }
  }
  async function remove(q: QueuedPrompt): Promise<void> {
    try {
      await runtimeThreads.unqueue(sessionId, q.id);
    } catch (e) {
      toastError('Couldn’t remove the queued message', e);
    }
  }
</script>

{#if background.length > 0 && !closed}
  {#if onshowtasks}
    <button type="button" class="qb qb-btn" data-testid="runtime-background" data-count={background.length} title={bgTitle} aria-label="{bgText} — show agents and tasks" onclick={onshowtasks}>
      <span class="spinner" aria-hidden="true"></span>
      <span class="ellipsis" dir="auto">{bgText}</span>
      <Icon name="chevronRight" size={12} />
    </button>
  {:else}
    <div class="qb" data-testid="runtime-background" data-count={background.length} title={bgTitle}>
      <span class="spinner" aria-hidden="true"></span>
      <span class="ellipsis" dir="auto">{bgText}</span>
    </div>
  {/if}
{/if}
{#each queued as q (q.id)}
  <div class="qb" data-testid="runtime-queued">
    <span class="ellipsis" dir="auto">Queued: {q.text}{q.attachments ? ` (+${q.attachments})` : ''}</span>
    <!-- Send now steers into the running turn; it never stops it (Stop does that). -->
    {#if running && canSteer}
      <button type="button" class="btn small" title="Send it into the running turn — the agent reads it at its next step" onclick={() => void deliver(q, 'steer')}>Send now</button>
    {/if}
    <button type="button" class="btn small" onclick={() => void edit(q)}>Edit</button>
    <button type="button" class="icon-btn" aria-label="Remove queued message" title="Remove" onclick={() => void remove(q)}>
      <Icon name="x" size={12} />
    </button>
  </div>
{/each}
{#if closed || dormant}
  <div class="qb" data-testid="runtime-closed">
    <span class="ellipsis">{closed ? 'Closed — in History.' : 'Agent stopped.'} Revive to continue this conversation.</span>
    <button type="button" class="btn small" disabled={reopening} onclick={onreopen} data-testid="runtime-reopen">{reopening ? 'Reviving…' : 'Revive'}</button>
  </div>
{/if}

<style>
  .qb {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding-block: 4px;
    padding-inline: 10px 6px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .qb-btn {
    inline-size: 100%;
    border: none;
    text-align: start;
    font: inherit;
    cursor: pointer;
  }
  .qb-btn:hover {
    color: var(--text);
    background: var(--hover);
  }
  .qb-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .qb .ellipsis {
    flex: 1;
    min-width: 0;
  }
  .qb .spinner {
    --spinner-size: 11px;
  }
</style>
