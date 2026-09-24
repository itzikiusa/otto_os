<script lang="ts">
  // Everything sent from this workspace, newest first: method + path, host,
  // status and when. Opening a row loads its request into a tab (it is NOT
  // re-sent; masked credentials are never sent back — see loadHistoryIntoDraft).
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import MethodTag from './MethodTag.svelte';
  import StatusChip from './StatusChip.svelte';
  import RetentionDialog from './RetentionDialog.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { splitUrl } from '../../lib/api/apiVars';
  import type { ApiHistorySummary } from '../../lib/api/types';

  interface Props { onopen?: () => void }
  let { onopen }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  let retentionOpen = $state(false);

  // Search: every whitespace-separated token must match method/url/status —
  // the url covers domain, path and query string, so "api.foo 404 get" works.
  let search = $state('');
  const tokens = $derived(search.trim().toLowerCase().split(/\s+/).filter(Boolean));
  function entryMatches(h: ApiHistorySummary): boolean {
    const hay = `${h.method} ${h.url} ${h.status ?? ''}`.toLowerCase();
    return tokens.every((t) => hay.includes(t));
  }
  const filtered = $derived(
    apiClient.history.filter(
      (h) =>
        (!apiClient.historyAgentOnly || apiClient.historySource(h)?.kind === 'agent') &&
        (!tokens.length || entryMatches(h)),
    ),
  );

  function open(h: ApiHistorySummary): void {
    void apiClient.selectHistory(h.id);
    onopen?.();
  }

  async function clear(): Promise<void> {
    if (!(await confirmer.ask(
      `Delete all ${apiClient.history.length} history entries in this workspace, including their stored responses? Saved requests are not affected.`,
      { title: 'Clear history', confirmLabel: 'Clear history' },
    ))) return;
    await apiClient.clearHistory();
  }

  const retention = $derived(ws.apiHistoryRetention);
  const retentionText = $derived(
    retention.rows || retention.days
      ? `Keeping ${retention.rows ? `the newest ${retention.rows}` : 'all'}${retention.days ? ` for up to ${retention.days} days` : ''}`
      : 'Keeping every request',
  );

  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Retention…', icon: 'clock', action: () => (retentionOpen = true), disabled: !canEdit },
      { separator: true },
      { label: 'Clear history…', icon: 'trash', danger: true, disabled: !canEdit || apiClient.history.length === 0, action: () => void clear() },
    ]);
  }
</script>

<div class="hist-wrap">
  <div class="tools">
    <label class="search">
      <Icon name="search" size={12} />
      <input placeholder="Search URL, method or status" bind:value={search} aria-label="Search request history" />
      {#if search}
        <button class="icon-btn clear" onclick={() => (search = '')} aria-label="Clear search" title="Clear search"><Icon name="x" size={12} /></button>
      {/if}
    </label>
    <button class="icon-btn" onclick={menu} aria-label="History options" title="History options"><Icon name="more" size={14} /></button>
  </div>
  <div class="filters">
    <button
      class="pill-toggle small"
      class:on={apiClient.historyAgentOnly}
      aria-pressed={apiClient.historyAgentOnly}
      title="Show only requests that agents sent through Otto’s tools"
      onclick={() => (apiClient.historyAgentOnly = !apiClient.historyAgentOnly)}
    >Agent runs only</button>
    <span class="ret" title="History retention (workspace setting)">{retentionText}</span>
  </div>

  {#if apiClient.historyLoadingId}<div class="state" role="status">Loading request…</div>{/if}

  {#if apiClient.history.length === 0}
    <EmptyState icon="clock" title="Nothing sent yet" body="Every request you send appears here, so you can open it again later." />
  {:else if filtered.length === 0}
    <div class="state">
      {apiClient.historyAgentOnly && !tokens.length ? 'No agent runs yet.' : `No history matches “${search.trim()}”.`}
    </div>
  {:else}
    <VirtualList items={filtered} estimateHeight={44} class="hist-vlist">
      {#snippet row(h: ApiHistorySummary)}
        {@const src = apiClient.historySource(h)}
        {@const u = splitUrl(h.url)}
        <button class="hist-row" onclick={() => open(h)} title="{h.method} {h.url}&#10;{new Date(h.executed_at).toLocaleString()}">
          <span class="l1">
            <MethodTag method={h.method} fixed />
            <span class="path mono" dir="ltr">{u.path}</span>
          </span>
          <span class="l2">
            <span class="host" dir="ltr">{u.host || '—'}</span>
            {#if src?.kind === 'agent'}<span class="chip agent" title="Sent by an agent session">Agent</span>{/if}
            <StatusChip status={h.status} small />
            <span class="when">{rel(h.executed_at)}</span>
          </span>
        </button>
      {/snippet}
    </VirtualList>
  {/if}
</div>

{#if retentionOpen}<RetentionDialog onclose={() => (retentionOpen = false)} />{/if}

<style>
  .hist-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    flex: 1;
    gap: 8px;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .search {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 4px 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    outline: none;
  }
  .search input::placeholder {
    color: var(--text-dim);
  }
  .clear {
    width: 20px;
    height: 20px;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .pill-toggle.small {
    height: 22px;
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
  .ret {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    font-size: var(--fs-s);
    color: var(--text-dim);
    padding: 4px;
  }
  :global(.hist-vlist) {
    flex: 1;
    min-height: 0;
  }
  .hist-row {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    width: 100%;
    height: 44px;
    padding: 0 6px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    border-radius: var(--radius-s);
  }
  .hist-row:hover {
    background: var(--hover);
  }
  .l1,
  .l2 {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .l2 {
    padding-inline-start: 42px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .host {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .chip.agent {
    height: 18px;
  }
</style>
