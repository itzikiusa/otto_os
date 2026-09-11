<script lang="ts">
  // DB dashboard box: embeds one Database Explorer dashboard (its widgets, each
  // running its own query on the dashboard's refresh cadence) via the same
  // WidgetCard the Dashboards tab uses. The dashboard is chosen once (stored in
  // the box config); an unset box shows an inline picker.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import WidgetCard from '../../database/WidgetCard.svelte';
  import { database } from '../../../lib/stores/database.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { router } from '../../../lib/router.svelte';
  import { home, type HomeBox } from '../home.svelte';

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
  }
  let { box, viewId, zoomed, tick }: Props = $props();

  let loading = $state(false);
  let loadedFor = '';

  // Dashboards + widgets are workspace-scoped; load them once per workspace
  // (the Connections page does the same on mount). `connections` feeds the
  // per-widget connection chip.
  async function ensureLoaded(force = false): Promise<void> {
    const id = ws.currentId;
    if (!id || (loadedFor === id && !force)) return;
    loadedFor = id;
    loading = true;
    try {
      if (database.connections.length === 0) await database.loadConnections();
      await database.loadDashboards();
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void ws.currentId;
    untrack(() => void ensureLoaded());
  });
  $effect(() => {
    void tick;
    untrack(() => void ensureLoaded(true));
  });

  const dashboardId = $derived(typeof box.config.dashboardId === 'string' ? box.config.dashboardId : '');
  const dashboard = $derived(database.dashboards.find((d) => d.id === dashboardId) ?? null);
  const widgets = $derived(dashboardId ? database.widgets.filter((w) => w.dashboard_id === dashboardId) : []);

  function pick(id: string): void {
    home.updateBoxConfig(viewId, box.id, { dashboardId: id });
  }
</script>

<div class="dbd" class:zoomed>
  {#if loading && database.dashboards.length === 0}
    <Skeleton rows={3} />
  {:else if database.dashboards.length === 0}
    <EmptyState
      icon="db"
      title="No DB dashboards yet"
      body="Create one in Connections → a database → Dashboards, then pick it here."
      actionLabel="Open Connections"
      onaction={() => router.go('connections')}
    />
  {:else if !dashboard}
    <div class="picker">
      <p>Choose the dashboard this box shows:</p>
      <select class="input" value="" onchange={(e) => pick((e.currentTarget as HTMLSelectElement).value)}>
        <option value="" disabled>Pick a dashboard…</option>
        {#each database.dashboards as d (d.id)}
          <option value={d.id}>{d.name}</option>
        {/each}
      </select>
    </div>
  {:else if widgets.length === 0}
    <EmptyState icon="db" title="“{dashboard.name}” has no widgets" body="Add widgets from a query result in the Database Explorer." />
  {:else}
    <div class="grid">
      {#each widgets as w (w.id)}
        <WidgetCard widget={w} refreshSecs={dashboard.refresh_secs ?? null} />
      {/each}
    </div>
  {/if}
</div>

<style>
  .dbd {
    height: 100%;
    min-height: 0;
    overflow-y: auto;
  }
  .picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .picker p {
    margin: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 10px;
  }
  .zoomed .grid {
    grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
  }
</style>
