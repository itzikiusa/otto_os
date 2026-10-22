<script lang="ts">
  // Otto School — the 3D stage's "needs you" queue (ClassroomsBox): the
  // school-wide sessions that need you or failed, as real buttons that walk
  // to the kid (or open it); with nothing waiting, the corridor offers a jump
  // into the busiest classroom instead.
  import Icon from '../../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../../lib/components/ProviderIcon.svelte';
  import { roomSummary, type Kid, type Room } from '../../school/model';
  import type { SchoolHandle, SchoolView } from '../../school/scene';

  interface Props {
    queue: Kid[];
    queueLabel: string;
    /** Expanded (kept by the box while the queue comes and goes). */
    queueOpen: boolean;
    view: SchoolView;
    busiest: Room | null;
    handle: SchoolHandle | null;
    open: (k: Kid) => Promise<void>;
    goTo: (k: Kid) => Promise<void>;
  }
  let { queue, queueLabel, queueOpen = $bindable(), view, busiest, handle, open, goTo }: Props = $props();

  const QUEUE_SHOWN = 5;
</script>

{#if queue.length > 0}
  <!-- Corridor: top-end (doors start at the near left); room: top-start
       (the kid card owns top-end). -->
  <section class="queue" class:end={view.kind === 'corridor'} aria-label="Sessions that need you">
    <button class="queue-head" aria-expanded={queueOpen} onclick={() => (queueOpen = !queueOpen)}>
      <Icon name="hand" size={12} />
      <span>{queueLabel}</span>
      <Icon name={queueOpen ? 'chevronUp' : 'chevronDown'} size={12} />
    </button>
    {#if queueOpen}
      <ul>
        {#each queue.slice(0, QUEUE_SHOWN) as k (k.id)}
          <li class="qrow" class:err={k.pose === 'error'}>
            <button class="qmain" onclick={() => void goTo(k)} title="Go to {k.title} in {k.workspaceName}">
              <ProviderIcon provider={k.provider} size={12} />
              <span class="ellipsis qt">{k.title}</span>
              <small class="ellipsis">{k.workspaceName}</small>
            </button>
            <button class="icon-btn" onclick={() => void open(k)} aria-label="Open {k.title}" title="Open {k.title}"><Icon name="external" size={12} /></button>
          </li>
        {/each}
      </ul>
      {#if queue.length > QUEUE_SHOWN}<p class="qmore">+{queue.length - QUEUE_SHOWN} more in the list</p>{/if}
    {/if}
  </section>
{:else if view.kind === 'corridor' && busiest}
  <button class="jump" onclick={() => busiest && void handle?.enterRoom(busiest.id)} title="Walk into the busiest classroom">
    <span class="ellipsis">{busiest.name}</span><small>{roomSummary(busiest)}</small><Icon name="chevronRight" size={12} />
  </button>
{/if}

<style>
  .queue {
    position: absolute;
    inset-inline-start: 8px;
    top: 8px;
    width: min(280px, calc(100% - 16px));
    max-height: calc(100% - 56px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--warning);
    box-shadow: var(--shadow-card);
  }
  .queue.end {
    inset-inline-start: auto;
    inset-inline-end: 8px;
  }
  .queue ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .queue-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
  }
  .queue-head span {
    flex: 1;
    text-align: start;
  }
  .qrow {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .qmain {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .qmain:hover,
  .jump:hover {
    background: var(--surface-2);
  }
  .qmain small {
    flex: none;
    max-width: 40%;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .qrow.err .qt {
    color: var(--danger);
  }
  .qt {
    flex: 1;
    min-width: 0;
  }
  .qmore {
    margin: 2px 6px 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .jump {
    position: absolute;
    inset-inline-start: 8px;
    top: 8px;
    max-width: min(320px, calc(100% - 16px));
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-card);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
  }
  .jump small {
    font-weight: 400;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .queue-head:focus-visible,
  .qmain:focus-visible,
  .jump:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
