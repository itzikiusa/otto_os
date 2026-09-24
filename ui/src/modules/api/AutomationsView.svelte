<script lang="ts">
  // The automations list (left pane). Selecting one opens its editor in the
  // main area (AutomationEditor) — the sidebar only lists and creates.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { Id } from '../../lib/api/types';

  interface Props {
    selectedId: Id | null;
    onselect: (id: Id) => void;
  }
  let { selectedId, onselect }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');

  async function create(): Promise<void> {
    const n = await confirmer.promptText('Name', { title: 'New automation', confirmLabel: 'Create', initial: '' });
    if (!n) return;
    const saved = await apiClient.saveAutomation({ name: n, steps: [] });
    if (saved) onselect(saved.id);
  }
</script>

<div class="auto-list-wrap">
  {#if apiClient.automations.length === 0}
    <EmptyState
      icon="zap"
      title="No automations yet"
      body="Run saved requests in order, check each response and pass values (like a login token) to the next request. Useful for smoke tests."
      actionLabel={canEdit && apiClient.requests.length > 0 ? 'New automation' : undefined}
      actionIcon="plus"
      onaction={canEdit ? create : undefined}
    >
      {#if apiClient.requests.length === 0}<p class="hint">Save a request first: automations run saved requests.</p>{/if}
    </EmptyState>
  {:else}
    <div class="head">
      <span class="lead">Saved request sequences with checks.</span>
      {#if canEdit}
        <button class="icon-btn" title="New automation" aria-label="New automation" onclick={create}><Icon name="plus" size={14} /></button>
      {/if}
    </div>
    <ul class="auto-list">
      {#each apiClient.automations as a (a.id)}
        <li>
          <button class="auto-pick" class:active={a.id === selectedId} onclick={() => onselect(a.id)} aria-current={a.id === selectedId ? 'true' : undefined}>
            <Icon name="zap" size={14} />
            <span class="aname">{a.name}</span>
            <span class="acount">{a.steps.length} step{a.steps.length === 1 ? '' : 's'}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .auto-list-wrap {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .lead {
    flex: 1;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hint {
    margin: 0;
    font-size: var(--fs-s);
  }
  .auto-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .auto-pick {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 30px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    text-align: start;
    border-radius: var(--radius-s);
    font-size: var(--fs-m);
  }
  .auto-pick:hover {
    background: var(--hover);
  }
  .auto-pick.active {
    background: var(--accent-soft);
  }
  .aname {
    flex: 1;
    min-width: 0;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .acount {
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
</style>
