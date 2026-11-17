<script lang="ts">
  // Above a runtime thread's composer: background work the agent started that
  // is still running, the messages waiting in the thread's (durable,
  // server-side) queue — Send now (into the running turn, where the agent
  // takes it; it never stops the turn), Edit, Remove — and, on a closed thread, the
  // "Revive" line for a closed or dormant agent. The background line has
  // "Stop all" (confirmed, naming how many): the composer's Stop ends only a turn.
  import { announceShown } from '../../../lib/announce.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { plural } from '../../../lib/plural';
  import { queuedLabel, queuedRow } from './queueRows';
  import { stopSummary } from './taskStop';
  import { taskStops } from './taskStops.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import type { PromptDelivery, QueuedPrompt } from '../../../lib/api/types';
  import { backgroundStripText, type AgentTask } from './agentTasks';
  import { toastError } from '../../../lib/toastError';
  import { runtimeThreads } from './runtimeThreads.svelte';

  import type { ComposerFile } from './runtimeComposer';

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
    /** Put a queued message (and any attached screenshots/files) back in the composer. */
    onedit: (text: string, attachments: ComposerFile[], targetSessionId: string) => void;
  }
  let { sessionId, background, onshowtasks, queued, running, canSteer, closed, dormant, reopening, onreopen, onedit }: Props = $props();
  const bgText = $derived(backgroundStripText(background));
  const bgTitle = $derived(background.map((b) => b.label).join('\n'));
  /** The tasks the daemon can stop one by one (not a swarm member or a turn's own subagent). */
  const stoppable = $derived(background.filter((b) => b.stopId));
  let stoppingAll = $state(false);
  // A phone (or a short window) folds several queued messages into one line.
  let small = $state(false);
  let showQueued = $state(false);
  $effect(() => {
    if (typeof matchMedia !== 'function') return;
    const mq = matchMedia('(max-width: 640px), (max-height: 500px)');
    small = mq.matches;
    const on = (): void => void (small = mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  });
  const foldQueued = $derived(small && !showQueued && queued.length > 1);

  async function stopAll(): Promise<void> {
    const ids = stoppable.map((b) => b.stopId!);
    if (ids.length === 0 || stoppingAll) return;
    const what = ids.length === 1 ? `“${stoppable[0].label}”` : plural(ids.length, 'background task');
    const ok = await confirmer.ask(
      `Stop ${what}? ${ids.length === 1 ? 'It ends' : 'They end'} now; the agent itself and its conversation keep going.`,
      { title: 'Stop background work', confirmLabel: ids.length === 1 ? 'Stop it' : `Stop ${ids.length}`, danger: true },
    );
    if (!ok) return;
    stoppingAll = true;
    try {
      const tally = await taskStops.stopAll(sessionId, ids);
      if (!tally) return;
      const s = stopSummary(tally);
      if (s.ok) toasts.info(s.title, s.body);
      else toasts.error(s.title, s.body);
    } finally {
      stoppingAll = false;
    }
  }

  async function deliver(q: QueuedPrompt, delivery: PromptDelivery): Promise<void> {
    const targetSessionId = sessionId;
    try {
      await runtimeThreads.deliverQueued(targetSessionId, q.id, delivery);
    } catch (e) {
      toastError(delivery === 'steer' ? 'Couldn’t send it into the turn' : delivery === 'queue' ? 'Couldn’t retry the queued message' : 'Couldn’t send it now', e);
    }
  }
  async function edit(q: QueuedPrompt): Promise<void> {
    const targetSessionId = sessionId;
    try {
      const popped = await runtimeThreads.unqueue(targetSessionId, q.id);
      onedit(popped.text ?? q.text, popped.attachments, targetSessionId);
    } catch (e) {
      toastError('Couldn’t edit the queued message', e);
    }
  }
  async function remove(q: QueuedPrompt): Promise<void> {
    const targetSessionId = sessionId;
    try {
      await runtimeThreads.unqueue(targetSessionId, q.id);
    } catch (e) {
      toastError('Couldn’t remove the queued message', e);
    }
  }
</script>

{#if background.length > 0 && !closed}
  <div class="qb qb-bg" data-testid="runtime-background" data-count={background.length}>
    {#if onshowtasks}
      <button type="button" class="qb-btn" data-testid="runtime-background-show" title={bgTitle} aria-label="{bgText} — show agents and tasks" onclick={onshowtasks}>
        <span class="spinner" aria-hidden="true"></span>
        <span class="ellipsis" dir="auto">{bgText}</span>
        <Icon name="chevronRight" size={12} />
      </button>
    {:else}
      <span class="qb-btn" title={bgTitle}>
        <span class="spinner" aria-hidden="true"></span>
        <span class="ellipsis" dir="auto">{bgText}</span>
      </span>
    {/if}
    {#if stoppable.length > 0 && !taskStops.unsupported}
      <button
        type="button"
        class="btn small qb-stop"
        disabled={stoppingAll}
        aria-label={stoppable.length === 1 ? `Stop ${stoppable[0].label}` : `Stop all ${stoppable.length} background tasks`}
        title={stoppable.length === 1 ? `Stop ${stoppable[0].label}` : `Stop all ${stoppable.length} background tasks`}
        data-testid="runtime-background-stop-all"
        onclick={() => void stopAll()}
      >
        <Icon name="stop" size={12} />{stoppingAll ? 'Stopping…' : stoppable.length === 1 ? 'Stop' : 'Stop all'}
      </button>
    {/if}
  </div>
{/if}
{#if foldQueued}
  <button type="button" class="qb qb-fold" aria-expanded="false" data-testid="runtime-queued-fold" onclick={() => (showQueued = true)}>
    <span class="ellipsis">{queued.length} queued messages{queued.some((q) => q.error) ? ' · one couldn’t send' : ''}</span>
    <Icon name="chevronDown" size={12} />
  </button>
{/if}
{#each foldQueued ? [] : queued as q (q.id)}
  {@const v = queuedRow(q, { running, canSteer })}
  <div class="qb" class:qb-failed={v.error} data-testid="runtime-queued" data-failed={v.error ? 'true' : undefined}>
    <span class="qb-text">
      <span class="ellipsis" dir="auto" title={q.text.slice(0, 500)}>{queuedLabel(q.text)}{q.attachments ? ` (+${q.attachments})` : ''}</span>
      {#if v.error}<span class="qb-error" dir="auto" use:announceShown={{ text: v.error, assertive: true }} data-testid="runtime-queued-error">{v.error}</span>{/if}
    </span>
    <!-- Send now steers into the running turn; it never stops it (Stop does that). -->
    {#if v.actions.includes('steer')}
      <button type="button" class="btn small qb-act" title="Send it into the running turn — the agent reads it at its next step" onclick={() => void deliver(q, 'steer')}>Send now</button>
    {/if}
    {#if v.actions.includes('retry')}
      <button type="button" class="btn small qb-act" title="Put it back in line — it is sent after the messages ahead of it" data-testid="runtime-queued-retry" onclick={() => void deliver(q, 'queue')}>Send again</button>
    {/if}
    <button type="button" class="btn small qb-act" onclick={() => void edit(q)}>Edit</button>
    <button type="button" class="icon-btn qb-act" aria-label="Remove queued message" title="Remove" onclick={() => void remove(q)}>
      <Icon name="x" size={12} />
    </button>
  </div>
{/each}
{#if closed || dormant}
  <div class="qb" data-testid="runtime-closed">
    <span class="ellipsis">{closed ? 'Closed — in History. Revive to continue this conversation.' : 'Agent stopped.'}</span>
    <button type="button" class="btn small qb-act" title={closed ? undefined : 'Start the agent again (your next message does this too)'} disabled={reopening} onclick={onreopen} data-testid="runtime-reopen">{reopening ? 'Reviving…' : 'Revive'}</button>
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
  .qb-bg {
    padding-inline-end: 4px;
  }
  .qb-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-inline-size: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    text-align: start;
    font: inherit;
  }
  button.qb-btn {
    cursor: pointer;
  }
  button.qb-btn:hover {
    color: var(--text);
  }
  .qb-stop {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .qb-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .qb .ellipsis {
    flex: 1;
    min-width: 0;
  }
  .qb-text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-inline-size: 0;
  }
  .qb-text .ellipsis {
    max-inline-size: 100%;
  }
  .qb-error {
    color: var(--danger);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .qb .spinner {
    --spinner-size: 11px;
  }
  .qb-fold {
    inline-size: 100%;
    border: 0;
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .qb-fold:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  @media (pointer: coarse) {
    .qb-act,
    .qb-stop,
    .qb-fold {
      min-block-size: var(--hit-min);
      min-inline-size: var(--hit-min);
    }
  }
</style>
