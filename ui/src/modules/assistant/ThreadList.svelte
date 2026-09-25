<script lang="ts">
  // The thread list: the four pinned Spaces (01–04) and Recent threads, each
  // with its needs-you count. ↑/↓ move between rows, Enter opens.
  import Icon from '../../lib/components/Icon.svelte';
  import { assistant } from '../../lib/stores/assistant.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { groupThreads, spaceLabel, SPACE_SLOTS } from './model';
  import type { AssistantThread } from '../../lib/api/types';

  interface Props {
    threads: AssistantThread[];
    selected: string | null;
    onopen: (id: string) => void;
    onnew: () => void;
    onnewspace: (slot: 1 | 2 | 3 | 4) => void;
    creating?: boolean;
  }
  let { threads, selected, onopen, onnew, onnewspace, creating = false }: Props = $props();

  const groups = $derived(groupThreads(threads));
  const needs = $derived(assistant.needsByThread);

  let listEl = $state<HTMLElement | null>(null);
  function onKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const rows = Array.from(listEl?.querySelectorAll<HTMLButtonElement>('button.th-row') ?? []);
    const at = rows.indexOf(document.activeElement as HTMLButtonElement);
    if (at < 0) return;
    e.preventDefault();
    rows[Math.max(0, Math.min(rows.length - 1, at + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus();
  }
</script>

<nav class="list" aria-label="Assistant threads" bind:this={listEl}>
  <div class="head">
    <span class="section-title">Spaces</span>
  </div>
  <ul class="rows">
    {#each SPACE_SLOTS as slot (slot)}
      {@const t = groups.spaces[slot - 1]}
      <li>
        {#if t}
          <button class="th-row" onkeydown={onKey} class:on={selected === t.id} aria-current={selected === t.id ? 'page' : undefined} onclick={() => onopen(t.id)} title={t.title}>
            <span class="num mono">{spaceLabel(slot)}</span>
            <span class="title">{t.title}</span>
            {#if t.incognito}<Icon name="eyeOff" size={12} />{/if}
            {#if needs[t.id]}<span class="needs" title={`${needs[t.id]} waiting on you`}>{needs[t.id]}</span>{/if}
          </button>
        {:else}
          <button class="th-row empty-slot" onkeydown={onKey} onclick={() => onnewspace(slot)} disabled={creating} title={`Start space ${spaceLabel(slot)}`}>
            <span class="num mono">{spaceLabel(slot)}</span>
            <span class="title">Empty space</span>
            <Icon name="plus" size={12} />
          </button>
        {/if}
      </li>
    {/each}
  </ul>
  <div class="head">
    <span class="section-title">Recent</span>
    <button class="icon-btn" onclick={onnew} disabled={creating} aria-label="New thread" title="New thread">
      <Icon name="plus" size={14} />
    </button>
  </div>
  {#if groups.recent.length}
    <ul class="rows">
      {#each groups.recent as t (t.id)}
        <li>
          <button class="th-row" onkeydown={onKey} class:on={selected === t.id} aria-current={selected === t.id ? 'page' : undefined} onclick={() => onopen(t.id)} title={t.title}>
            <span class="title">{t.title}</span>
            {#if t.incognito}<Icon name="eyeOff" size={12} />{/if}
            {#if needs[t.id]}
              <span class="needs" title={`${needs[t.id]} waiting on you`}>{needs[t.id]}</span>
            {:else}
              <time class="when" datetime={t.updated_at} title={new Date(t.updated_at).toLocaleString()}>{rel(t.updated_at)}</time>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="none">No other threads yet.</p>
  {/if}
</nav>

<style>
  .list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 8px 16px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 28px;
    padding: 8px 8px 2px;
  }
  .head .section-title {
    margin: 0;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .th-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 30px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .th-row:hover:not(:disabled) {
    background: var(--hover);
  }
  .th-row.on {
    background: var(--accent-soft);
    font-weight: 600;
  }
  .th-row :global(svg) {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .num {
    width: 18px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .th-row.on .num {
    color: var(--text);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty-slot .title {
    color: var(--text-dim);
  }
  .needs {
    min-width: 18px;
    height: 18px;
    padding: 0 6px;
    border-radius: 999px;
    display: inline-grid;
    place-items: center;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--warning);
    background: var(--warning-soft);
  }
  .when {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 400;
  }
  .none {
    margin: 4px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .th-row {
      min-height: 40px;
    }
  }
</style>
