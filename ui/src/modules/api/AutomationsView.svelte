<script lang="ts">
  import { plural } from '../../lib/plural';
  // The automations list (left pane). Selecting one opens its editor in the
  // main area (AutomationEditor) — the sidebar only lists and creates.
  import type { Snippet } from 'svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideRow from '../../lib/components/side/SideRow.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { Id } from '../../lib/api/types';

  interface Props {
    /** The pane's views (Collections | Automations | History), its header. */
    views?: Snippet;
    selectedId: Id | null;
    onselect: (id: Id) => void;
  }
  let { views, selectedId, onselect }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');

  async function create(): Promise<void> {
    const n = await confirmer.promptText('Name', { title: 'New automation', confirmLabel: 'Create', initial: '' });
    if (!n) return;
    const saved = await apiClient.saveAutomation({ name: n, steps: [] });
    if (saved) onselect(saved.id);
  }
</script>

<!-- A failed / in-flight load is not "No automations yet". -->
<ListPane
  {views}
  menu={canEdit ? () => [{ label: 'New automation…', icon: 'plus', action: () => void create() }] : undefined}
  loading={apiClient.automationsLoading && apiClient.automations.length === 0}
  error={apiClient.automations.length === 0 ? apiClient.automationsError : null}
  onretry={() => void apiClient.loadAutomations({ force: true })}
>
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
    {#each apiClient.automations as a (a.id)}
      <SideRow icon="zap" title={a.name} selected={a.id === selectedId} testid="api-automation-row" onclick={() => onselect(a.id)}>
        {#snippet trailing()}{plural(a.steps.length, 'step')}{/snippet}
      </SideRow>
    {/each}
  {/if}
</ListPane>

<style>
  .hint {
    margin: 0;
    font-size: var(--fs-s);
  }
</style>
