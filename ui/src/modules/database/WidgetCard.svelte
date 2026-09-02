<script lang="ts">
  // A dashboard tile: runs /db/widgets/{id}/run on mount (and on a refresh
  // interval set per-dashboard) and renders the result via Chart per its viz.
  import Icon from '../../lib/components/Icon.svelte';
  import Chart from './Chart.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { api } from '../../lib/api/client';
  import type { DbWidget, QueryResult } from '../../lib/api/types';

  interface Props {
    widget: DbWidget;
    /** Refresh interval in seconds (0/undefined = manual only). */
    refreshSecs?: number | null;
    /** Open the edit dialog for this widget (owned by the dashboard view). */
    onedit?: (w: DbWidget) => void;
  }
  let { widget, refreshSecs = null, onedit }: Props = $props();

  let result = $state<QueryResult | null>(null);
  let loading = $state(false);
  // Failure message rendered IN the card — a banner over the last good chart
  // when one exists (the chart is never wiped by a failed refresh).
  let error = $state<string | null>(null);
  // Consecutive failure count — stretches the auto-refresh cadence (read at
  // schedule time only, so plain, not $state).
  let failures = 0;

  async function run(manual = false): Promise<void> {
    loading = true;
    try {
      // The store's runWidget toasts on failure — right for an explicit click,
      // wrong for a background tick (20 tiles on a broken connection would storm
      // the toaster every refresh), so auto-refresh posts directly and surfaces
      // the failure inline instead.
      const r = manual
        ? await database.runWidget(widget.id)
        : await api.post<QueryResult>(`/db/widgets/${widget.id}/run`, {});
      if (r) {
        result = r; // only a SUCCESS replaces the data
        error = null;
        failures = 0;
      } else {
        error = 'Query failed';
        failures += 1;
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      failures += 1;
    } finally {
      loading = false;
    }
  }

  // Initial run + auto-refresh. The schedule is recreated whenever the widget id
  // or refresh cadence changes, and cleared on unmount (Svelte $effect cleanup).
  //
  // Each tick is jittered by up to ±15 % of the refresh period so that a
  // dashboard with 20 tiles doesn't issue 20 parallel queries in the same
  // scheduler tick. A setTimeout CHAIN (not setInterval) lets consecutive
  // failures back the cadence off — ×2 per failure, capped at ×16 — so a dead
  // connection isn't hammered at full rate; the next success restores it.
  $effect(() => {
    const id = widget.id;
    const secs = refreshSecs ?? 0;
    void id; // track id so a swapped widget re-runs
    failures = 0;
    void run();
    if (secs <= 0) return;
    let stopped = false;
    let handle = 0;
    const schedule = (): void => {
      const jitterMs = (Math.random() * 0.3 - 0.15) * secs * 1000; // ±15%
      const baseMs = Math.max(secs * 1000 + jitterMs, 5000); // floor 5s
      const backoff = Math.min(2 ** failures, 16);
      handle = window.setTimeout(() => {
        void run().finally(() => {
          if (!stopped) schedule();
        });
      }, baseMs * backoff);
    };
    schedule();
    return () => {
      stopped = true;
      clearTimeout(handle);
    };
  });

  const canEdit = $derived(ws.myRole !== 'viewer');

  // The connection this widget's query runs on — widgets bind to whichever
  // connection was focused at creation, so surface it on the card.
  const connName = $derived(
    database.connections.find((c) => c.id === widget.connection_id)?.name ?? widget.connection_id,
  );

  async function confirmDelete(): Promise<void> {
    if (await confirmer.ask(`Delete widget “${widget.title}”?`, { title: 'Delete widget' })) {
      await database.deleteWidget(widget.id);
    }
  }

  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Refresh', icon: 'refresh', action: () => void run(true) },
      ...(canEdit && onedit ? [{ label: 'Edit…', icon: 'edit', action: () => onedit?.(widget) }] : []),
      ...(canEdit
        ? [
            { separator: true },
            { label: 'Delete widget', icon: 'trash', danger: true as const, action: () => void confirmDelete() },
          ]
        : []),
    ]);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="widget-card" oncontextmenu={menu}>
  <div class="wc-head">
    <span class="wc-title ellipsis" title={widget.title}>{widget.title}</span>
    <span class="wc-conn ellipsis" title="Connection: {connName}"><Icon name="db" size={9} />{connName}</span>
    <button class="icon-btn" onclick={() => void run(true)} title="Refresh" aria-label="Refresh widget">
      <span class:spin={loading}><Icon name="refresh" size={12} /></span>
    </button>
    {#if canEdit}
      {#if onedit}
        <button class="icon-btn" onclick={() => onedit?.(widget)} title="Edit" aria-label="Edit widget">
          <Icon name="edit" size={12} />
        </button>
      {/if}
      <button class="icon-btn" onclick={() => void confirmDelete()} title="Delete" aria-label="Delete widget">
        <Icon name="trash" size={12} />
      </button>
    {/if}
  </div>
  <div class="wc-body">
    {#if error && !result}
      <div class="wc-error">{error}</div>
    {:else if loading && !result}
      <div class="wc-loading"><Icon name="refresh" size={14} /></div>
    {:else}
      {#if error}
        <div class="wc-stale" title={error}>
          <Icon name="zap" size={10} />refresh failed — showing last data
        </div>
      {/if}
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
  .wc-conn {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    max-width: 40%;
    padding: 1px 7px;
    font-size: 10px;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 999px;
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
  .wc-stale {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    margin-bottom: 4px;
    padding: 1px 7px;
    font-size: 10px;
    color: var(--status-warn);
    background: var(--status-warn-soft);
    border-radius: 999px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
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
