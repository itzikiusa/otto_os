<script lang="ts">
  // A dashboard tile: runs /db/widgets/{id}/run on mount (and on a refresh
  // interval set per-dashboard) and renders the result via Chart per its viz.
  import Icon from '../../lib/components/Icon.svelte';
  import Chart from './Chart.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { api, isAbortError } from '../../lib/api/client';
  import { pollWhileVisible } from '../../lib/poll';
  import { widgetGate } from './widgetGate';
  import type { DbWidget, QueryResult } from '../../lib/api/types';

  interface Props {
    widget: DbWidget;
    /** Refresh interval in seconds (0/undefined = manual only). */
    refreshSecs?: number | null;
    /** Open the edit dialog for this widget (owned by the dashboard view). */
    onedit?: (w: DbWidget) => void;
    /** False while the card is off screen (an inactive Home space, review 06
     *  F3): no runs and no auto-refresh; the last result stays. A manual-only
     *  card (refreshSecs 0) runs ONCE per widget — never again on its own,
     *  however often it is shown again. */
    active?: boolean;
  }
  let { widget, refreshSecs = null, onedit, active = true }: Props = $props();
  /** The widget id + query whose result is on screen (manual-only cards run
   *  once per that key), and when it last succeeded (auto-refresh resumes
   *  without an immediate re-run while that result is within its cadence). */
  let ranKey = '';
  let okAt = 0;

  let result = $state<QueryResult | null>(null);
  let loading = $state(false);
  // Failure message rendered IN the card — a banner over the last good chart
  // when one exists (the chart is never wiped by a failed refresh).
  let error = $state<string | null>(null);
  async function run(manual = false, signal?: AbortSignal): Promise<boolean> {
    loading = true;
    try {
      // The store's runWidget toasts on failure — right for an explicit click,
      // wrong for a background tick (20 tiles on a broken connection would storm
      // the toaster every refresh), so auto-refresh posts directly and surfaces
      // the failure inline instead. Auto runs share one dashboard-wide gate
      // (widgetGate: ≤ 2 queries at once across every tile) and ride the
      // background lane; unmount aborts the in-flight one.
      const r = manual
        ? await database.runWidget(widget.id)
        : await widgetGate(() => api.bg.post<QueryResult>(`/db/widgets/${widget.id}/run`, {}, signal), signal);
      if (r) {
        result = r; // only a SUCCESS replaces the data
        error = null;
        return true;
      }
      error = 'Query failed';
      return false;
    } catch (e) {
      if (isAbortError(e) || signal?.aborted) return true;
      error = e instanceof Error ? e.message : String(e);
      return false;
    } finally {
      if (!signal?.aborted) loading = false;
    }
  }

  // Initial run + auto-refresh, recreated whenever the widget id or cadence
  // changes and stopped on unmount. `pollWhileVisible` supplies the rules the
  // old hand-rolled setTimeout chain lacked (r3-04-03): no ticks while the
  // window is hidden (one catch-up run on return), the in-flight query is
  // ABORTED on unmount, ±15 % jitter so 20 tiles don't fire together, and a
  // ×2-per-failure backoff (capped ×16) so a dead connection isn't hammered.
  $effect(() => {
    const key = `${widget.id}\u0000${widget.connection_id}\u0000${widget.statement}`; // a swapped/edited widget re-runs
    const secs = refreshSecs ?? 0;
    if (!active) return;
    const fresh = key === ranKey && okAt > 0;
    if (secs <= 0) {
      if (fresh) return; // manual only: shown again ≠ asked again
      const ctl = new AbortController();
      void run(false, ctl.signal).then((ok) => {
        if (ok) {
          ranKey = key;
          okAt = Date.now();
        }
      });
      return () => ctl.abort();
    }
    const poller = pollWhileVisible(
      async (signal) => {
        const ok = await run(false, signal);
        if (ok) {
          ranKey = key;
          okAt = Date.now();
        }
        return ok;
      },
      {
        ms: secs * 1000,
        floorMs: 5000,
        jitter: 0.15,
        maxBackoff: 16,
        lane: 'bg',
        immediate: !(fresh && Date.now() - okAt < secs * 1000),
      },
    );
    return () => poller.stop();
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
    padding-block: 8px; padding-inline: 12px 8px;
    border-bottom: 1px solid var(--border);
  }
  .wc-title {
    flex: 1;
    font-size: var(--fs-m);
    font-weight: 600;
    min-width: 0;
  }
  .wc-conn {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    max-width: 40%;
    padding: 1px 7px;
    font-size: var(--fs-xs);
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
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .wc-stale {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    margin-bottom: 4px;
    padding: 1px 7px;
    font-size: var(--fs-xs);
    color: var(--warning);
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
    animation: otto-spin 0.8s linear infinite;
  }
  
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
