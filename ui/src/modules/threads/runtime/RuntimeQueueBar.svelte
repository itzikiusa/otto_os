<script lang="ts">
  // Above a runtime thread's composer: background work the agent started that
  // is still running, the messages waiting in the thread's (durable,
  // server-side) queue — Steer (into the running turn; Claude), Send now
  // (stop the turn and send it), Edit, Remove — and, on a closed thread, the
  // "Closed · Reopen" line.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { BackgroundTask, PromptDelivery, QueuedPrompt } from '../../../lib/api/types';
  import { toastError } from '../../../lib/toastError';
  import { runtimeThreads } from './runtimeThreads.svelte';

  interface Props {
    sessionId: string;
    background: BackgroundTask[];
    queued: QueuedPrompt[];
    running: boolean;
    canSteer: boolean;
    closed: boolean;
    reopening: boolean;
    onreopen: () => void;
    /** Put a queued message back in the composer. */
    onedit: (text: string) => void;
  }
  let { sessionId, background, queued, running, canSteer, closed, reopening, onreopen, onedit }: Props = $props();

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
  <div class="qb" data-testid="runtime-background">
    <span class="spinner" aria-hidden="true"></span>
    <span class="ellipsis" dir="auto">
      {background.length === 1 ? 'Running in the background' : `${background.length} tasks running in the background`}:
      {background.map((b) => b.label).join(' · ')}
    </span>
  </div>
{/if}
{#each queued as q (q.id)}
  <div class="qb" data-testid="runtime-queued">
    <span class="ellipsis" dir="auto">Queued: {q.text}{q.attachments ? ` (+${q.attachments})` : ''}</span>
    {#if running && canSteer}
      <button type="button" class="btn small" title="Send it into the running turn — the agent reads it at its next step" onclick={() => void deliver(q, 'steer')}>Steer</button>
    {/if}
    {#if running}
      <button type="button" class="btn small" title="Stop the running turn and send this now" onclick={() => void deliver(q, 'interrupt')}>Send now</button>
    {/if}
    <button type="button" class="btn small" onclick={() => void edit(q)}>Edit</button>
    <button type="button" class="icon-btn" aria-label="Remove queued message" title="Remove" onclick={() => void remove(q)}>
      <Icon name="x" size={12} />
    </button>
  </div>
{/each}
{#if closed}
  <div class="qb" data-testid="runtime-closed">
    <span class="ellipsis">Closed — in History. Send a message to pick it up again.</span>
    <button type="button" class="btn small" disabled={reopening} onclick={onreopen} data-testid="runtime-reopen">Reopen</button>
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
  .qb .ellipsis {
    flex: 1;
    min-width: 0;
  }
  .qb .spinner {
    --spinner-size: 11px;
  }
</style>
