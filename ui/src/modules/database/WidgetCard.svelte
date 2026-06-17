<script lang="ts">
  // A dashboard tile: runs /db/widgets/{id}/run on mount (and on a refresh
  // interval set per-dashboard) and renders the result via Chart per its viz.
  import Icon from '../../lib/components/Icon.svelte';
  import Chart from './Chart.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { DbWidget, QueryResult } from '../../lib/api/types';

  interface Props {
    widget: DbWidget;
    /** Refresh interval in seconds (0/undefined = manual only). */
    refreshSecs?: number | null;
  }
  let { widget, refreshSecs = null }: Props = $props();

  let result = $state<QueryResult | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  async function run(): Promise<void> {
    loading = true;
    error = null;
    const r = await database.runWidget(widget.id);
    if (r) result = r;
    else error = 'Query failed';
    loading = false;
  }

  // Initial run + auto-refresh. The interval is recreated whenever the widget id
  // or refresh cadence changes, and cleared on unmount (Svelte $effect cleanup).
  $effect(() => {
    const id = widget.id;
    const secs = refreshSecs ?? 0;
    void id; // track id so a swapped widget re-runs
    void run();
    if (secs > 0) {
      const handle = setInterval(() => void run(), secs * 1000);
      return () => clearInterval(handle);
    }
  });

  const canEdit = $derived(ws.myRole !== 'viewer');

  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Refresh', icon: 'refresh', action: () => void run() },
      ...(canEdit
        ? [
            { separator: true },
            { label: 'Delete widget', icon: 'trash', danger: true as const, action: () => void database.deleteWidget(widget.id) },
          ]
        : []),
    ]);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="widget-card" oncontextmenu={menu}>
  <div class="wc-head">
    <span class="wc-title ellipsis" title={widget.title}>{widget.title}</span>
    <button class="icon-btn" onclick={run} title="Refresh" aria-label="Refresh widget">
      <span class:spin={loading}><Icon name="refresh" size={12} /></span>
    </button>
    {#if canEdit}
      <button class="icon-btn" onclick={() => database.deleteWidget(widget.id)} title="Delete" aria-label="Delete widget">
        <Icon name="trash" size={12} />
      </button>
    {/if}
  </div>
  <div class="wc-body">
    {#if error}
      <div class="wc-error">{error}</div>
    {:else if loading && !result}
      <div class="wc-loading"><Icon name="refresh" size={14} /></div>
    {:else}
      <Chart {result} viz={widget.viz} mapping={widget.mapping} />
    {/if}
  </div>
</div>

<style>
  .widget-card {
    display: flex;
    flex-direction: column;
    min-height: 200px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .wc-head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 8px 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .wc-title {
    flex: 1;
    font-size: 12.5px;
    font-weight: 600;
    min-width: 0;
  }
  .wc-body {
    flex: 1;
    min-height: 0;
    padding: 8px 10px 10px;
    display: flex;
    flex-direction: column;
  }
  .wc-error {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--status-exited);
    font-size: 11.5px;
  }
  .wc-loading {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--text-dim);
  }
  .spin {
    display: inline-grid;
    place-items: center;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
