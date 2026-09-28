<script lang="ts">
  // Keep only a source identity. Open/parked query tabs are already live in the
  // Explorer store: no extra document, query, copied rows or editing callbacks.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import ResultsGrid from './ResultsGrid.svelte';

  let { children, enabled = true }: { children: Snippet; enabled?: boolean } = $props();
  let pinned = $state<{ connId: string; tabId: number; scope: string } | null>(null);
  const scope = $derived(JSON.stringify([ws.currentId, auth.me?.id, database.accessRevision]));
  const tabs = $derived(database.openQueryTabs());
  const source = $derived(pinned?.scope === scope
    ? tabs.find(({ connId, tab }) => connId === pinned?.connId && tab.id === pinned?.tabId) ?? null
    : null);
  const connection = $derived(source ? database.connections.find((c) => c.id === source.connId) : null);
  const submitted = $derived(source?.tab.error ? source.tab.err_statement : source?.tab.ran_statement);
  const active = $derived(tabs.find(({ connId, tab }) => connId === database.selectedConnId && tab.id === database.tab?.id));

  // Closure or access/identity changes remove both the reference and any modal
  // the grid had opened. Reopening an identically named tab never re-pins it.
  $effect(() => { if (pinned && !source) pinned = null; });

  function pinActive(): void {
    if (active) {
      database.setMainTab('query');
      pinned = { connId: active.connId, tabId: active.tab.id, scope };
    }
  }
  function selectSource(value: string): void {
    const next = tabs.find(({ connId, tab }) => `${connId}:${tab.id}` === value);
    pinned = next ? { connId: next.connId, tabId: next.tab.id, scope } : null;
  }
  async function focusSource(): Promise<void> {
    if (!source) return;
    try { await database.focusTab(source.tab.id); }
    catch { toasts.error('Couldn’t focus the source', 'Open its connection tab and try again.'); }
  }
  function sourceLabel(connId: string, name: string): string {
    const connectionName = database.connections.find((c) => c.id === connId)?.name ?? 'Connection';
    return `${connectionName} · ${name || 'Query'}`;
  }
  $effect(() => registry.register('database-comparison', [
    { id: 'db.compare-results', title: source ? 'Clear comparison' : 'Compare results', group: 'Database', keywords: 'compare connection query results side by side', run: () => { if (source) pinned = null; else pinActive(); } },
    ...(source ? [{ id: 'db.comparison-focus', title: 'Focus comparison source', group: 'Database', keywords: 'compare connection query edit', run: focusSource }] : []),
  ]));
</script>

<div class="comparison-workbench" class:comparing={enabled && source !== null}>
  {#if enabled}
  <div class="comparison-tools">
    <button class="btn small ghost" aria-pressed={source !== null} disabled={!active} onclick={() => source ? (pinned = null) : pinActive()} title="Keep loaded results beside the current query">
      <Icon name="columns" size={13} />Compare results
    </button>
    {#if source}<span class="comparison-note">Loaded results · read-only comparison</span>{/if}
  </div>
  {/if}
  <div class="comparison-content">
    <div class="comparison-editor">{@render children()}</div>
    {#if enabled && source && connection}
      <aside class="comparison-reference" aria-label="Comparison results" data-testid="connection-comparison">
        <div class="reference-head">
          <span class="reference-title">Comparison</span>
          <EnvBadge env={connection.environment} readOnly={connection.read_only} />
          <span class="reference-grow"></span>
          <button class="btn small ghost" onclick={() => void focusSource()} title="Open this source tab for editing">Focus source</button>
          <button class="icon-btn" onclick={() => (pinned = null)} aria-label="Clear comparison" title="Clear comparison"><Icon name="x" size={13} /></button>
        </div>
        <label class="source-label">
          <span>Source</span>
          <select aria-label="Comparison source" value={`${source.connId}:${source.tab.id}`} onchange={(event) => selectSource(event.currentTarget.value)}>
            {#each tabs as entry (`${entry.connId}:${entry.tab.id}`)}
              <option value={`${entry.connId}:${entry.tab.id}`}>{sourceLabel(entry.connId, entry.tab.name)}</option>
            {/each}
          </select>
        </label>
        <div class="reference-query">
          <span class="query-label">Submitted query</span>
          <code data-testid="comparison-statement" title={submitted ?? ''}>{submitted ?? 'This tab has not run a query yet.'}</code>
        </div>
        <div class="reference-results">
          {#key `${scope}:${source.connId}:${source.tab.id}`}
            {#if !source.tab.result && !source.tab.error && !source.tab.running}
              <EmptyState icon="grid" title="No results in this tab" body="Focus the source to run a query. Selecting a comparison never runs it." />
            {:else}
              <ResultsGrid result={source.tab.result} error={source.tab.error} running={source.tab.running} />
            {/if}
          {/key}
        </div>
      </aside>
    {/if}
  </div>
</div>

<style>
  .comparison-workbench {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    container: comparison / inline-size;
  }
  .comparison-tools {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    padding-block-end: 8px;
  }
  .comparison-note { color: var(--text-dim); font-size: var(--fs-xs); }
  .comparison-content { display: flex; flex: 1; gap: 12px; min-width: 0; min-height: 0; }
  .comparison-editor { display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; }
  .comparison-reference {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden;
  }
  .reference-head { display: flex; align-items: center; gap: 8px; padding: 8px; flex-shrink: 0; }
  .reference-title { font-size: var(--fs-m); font-weight: 600; }
  .reference-grow { flex: 1; }
  .source-label { display: flex; align-items: center; gap: 8px; padding: 0 8px 8px; font-size: var(--fs-s); }
  .source-label select { flex: 1; width: 0; min-width: 0; font-size: var(--fs-s); }
  .reference-query { display: flex; flex-direction: column; gap: 4px; padding: 8px; border-block: 1px solid var(--border); }
  .query-label { color: var(--text-dim); font-size: var(--fs-xs); }
  .reference-query code { font-size: var(--fs-xs); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 64px; overflow: auto; }
  .reference-results { display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; padding: 8px; }
  @container comparison (max-width: 1024px) {
    .comparing .comparison-content { flex-direction: column; overflow: auto; }
    .comparing .comparison-editor { flex: 0 0 420px; }
    .comparison-reference { flex: 0 0 360px; }
    .comparison-note { display: none; }
  }
  /* QueryEditor uses a natural-height accordion on phones (including a 340px
     results block). Let it size this stack instead of overflowing a desktop
     split height and painting over the comparison source controls. */
  @media (max-width: 640px) {
    .comparing .comparison-editor { flex: 0 0 auto; min-height: 420px; }
  }
</style>
