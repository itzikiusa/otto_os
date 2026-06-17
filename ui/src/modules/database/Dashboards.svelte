<script lang="ts">
  // Superset-style dashboards: pick/create/rename/delete a dashboard, set an
  // auto-refresh cadence, and render its widgets in a responsive grid. New
  // widgets are added from a query result via the "Add widget" sheet.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import WidgetCard from './WidgetCard.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { DbViz } from '../../lib/api/types';

  const canEdit = $derived(ws.myRole !== 'viewer');
  const dashboard = $derived(database.selectedDashboard);
  const widgets = $derived(database.widgetsForSelectedDashboard());

  const REFRESH_OPTS: { label: string; secs: number | null }[] = [
    { label: 'Off', secs: null },
    { label: '10s', secs: 10 },
    { label: '30s', secs: 30 },
    { label: '1m', secs: 60 },
    { label: '5m', secs: 300 },
  ];

  async function newDashboard(): Promise<void> {
    const name = await confirmer.promptText('Dashboard name', {
      title: 'New dashboard',
      confirmLabel: 'Create',
    });
    if (name) await database.createDashboard(name);
  }
  async function rename(): Promise<void> {
    if (!dashboard) return;
    const name = await confirmer.promptText('Rename dashboard', {
      title: 'Rename dashboard',
      confirmLabel: 'Rename',
      initial: dashboard.name,
    });
    if (name && name !== dashboard.name) await database.renameDashboard(dashboard.id, name);
  }
  async function remove(): Promise<void> {
    if (!dashboard) return;
    if (await confirmer.ask(`Delete dashboard “${dashboard.name}” and its widgets?`, { title: 'Delete dashboard' })) {
      await database.deleteDashboard(dashboard.id);
    }
  }

  // ── Add widget sheet ────────────────────────────────────────────────────
  let adding = $state(false);
  let wTitle = $state('');
  let wStatement = $state('');
  let wViz = $state<DbViz>('table');
  let wX = $state('');
  let wY = $state('');
  let saving = $state(false);

  function openAdd(): void {
    // Seed from the active query tab so "save this result as a widget" is one step.
    const t = database.tab;
    wStatement = t.statement;
    wTitle = t.name && t.name !== 'Query' ? t.name : '';
    wX = t.result?.columns[0]?.name ?? '';
    wY = t.result?.columns[1]?.name ?? '';
    wViz = 'table';
    adding = true;
  }

  const VIZ: DbViz[] = ['table', 'number', 'line', 'bar', 'area', 'pie'];
  const needsAxes = $derived(wViz === 'line' || wViz === 'bar' || wViz === 'area');
  const resultCols = $derived(database.tab.result?.columns ?? []);

  async function saveWidget(): Promise<void> {
    if (!dashboard || !wTitle.trim() || !wStatement.trim()) return;
    saving = true;
    const w = await database.createWidget({
      title: wTitle.trim(),
      statement: wStatement.trim(),
      viz: wViz,
      mapping: {
        x: wX || undefined,
        y: wY ? [wY] : undefined,
        category: wViz === 'pie' ? wX || undefined : undefined,
        value: wViz === 'number' ? wY || undefined : undefined,
      },
      dashboard_id: dashboard.id,
    });
    saving = false;
    if (w) adding = false;
  }
</script>

<div class="dash">
  <div class="dash-bar">
    <select
      class="input dash-pick"
      value={dashboard?.id ?? ''}
      onchange={(e) => (database.selectedDashboardId = (e.currentTarget as HTMLSelectElement).value || null)}
      disabled={database.dashboards.length === 0}
    >
      {#if database.dashboards.length === 0}
        <option value="">No dashboards</option>
      {/if}
      {#each database.dashboards as d (d.id)}
        <option value={d.id}>{d.name}</option>
      {/each}
    </select>

    {#if canEdit}
      <button class="btn small" onclick={newDashboard}><Icon name="plus" size={11} />New</button>
    {/if}

    {#if dashboard}
      {#if canEdit}
        <button class="icon-btn" onclick={rename} title="Rename" aria-label="Rename dashboard"><Icon name="edit" size={13} /></button>
        <button class="icon-btn" onclick={remove} title="Delete" aria-label="Delete dashboard"><Icon name="trash" size={13} /></button>
      {/if}
      <span class="grow"></span>
      <div class="refresh-pick">
        <Icon name="refresh" size={12} />
        <select
          class="input"
          value={String(dashboard.refresh_secs ?? '')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            void database.setDashboardRefresh(dashboard.id, v === '' ? null : Number(v));
          }}
        >
          {#each REFRESH_OPTS as o (o.label)}
            <option value={o.secs == null ? '' : String(o.secs)}>{o.label}</option>
          {/each}
        </select>
      </div>
      {#if canEdit}
        <button class="btn small primary" onclick={openAdd}><Icon name="plus" size={11} />Add widget</button>
      {/if}
    {/if}
  </div>

  {#if database.dashboards.length === 0}
    <EmptyState
      icon="grid"
      title="No dashboards yet"
      body="Create a dashboard, then add widgets from your query results to build live charts."
      actionLabel={canEdit ? 'New dashboard' : undefined}
      onaction={canEdit ? newDashboard : undefined}
    />
  {:else if dashboard && widgets.length === 0}
    <EmptyState
      icon="box"
      title="No widgets"
      body="Run a query in the Query tab, then click “Add widget” to chart it here."
      actionLabel={canEdit ? 'Add widget' : undefined}
      onaction={canEdit ? openAdd : undefined}
    />
  {:else if dashboard}
    <div class="widget-grid">
      {#each widgets as w (w.id)}
        <WidgetCard widget={w} refreshSecs={dashboard.refresh_secs} />
      {/each}
    </div>
  {/if}
</div>

{#if adding}
  <Modal title="Add widget" width={520} onclose={() => (adding = false)}>
    <div class="field">
      <label for="w-title">Title</label>
      <input id="w-title" class="input" bind:value={wTitle} placeholder="Daily signups" />
    </div>
    <div class="field">
      <label for="w-stmt">Statement</label>
      <textarea id="w-stmt" class="input mono" rows="4" bind:value={wStatement} spellcheck="false"></textarea>
    </div>
    <div class="field">
      <label for="w-viz">Visualization</label>
      <div class="viz-row" id="w-viz">
        {#each VIZ as v (v)}
          <button class="viz-chip" class:selected={wViz === v} onclick={() => (wViz = v)}>{v}</button>
        {/each}
      </div>
    </div>
    {#if needsAxes || wViz === 'pie' || wViz === 'number'}
      <div class="field-row">
        {#if needsAxes || wViz === 'pie'}
          <div class="field grow">
            <label for="w-x">{wViz === 'pie' ? 'Category' : 'X axis'}</label>
            <select id="w-x" class="input" bind:value={wX}>
              <option value="">(auto)</option>
              {#each resultCols as c (c.name)}<option value={c.name}>{c.name}</option>{/each}
            </select>
          </div>
        {/if}
        <div class="field grow">
          <label for="w-y">{wViz === 'number' ? 'Value' : wViz === 'pie' ? 'Value' : 'Y series'}</label>
          <select id="w-y" class="input" bind:value={wY}>
            <option value="">(auto)</option>
            {#each resultCols as c (c.name)}<option value={c.name}>{c.name}</option>{/each}
          </select>
        </div>
      </div>
      {#if resultCols.length === 0}
        <p class="axes-hint">Run the query once in the Query tab to map specific columns; otherwise columns are auto-detected.</p>
      {/if}
    {/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (adding = false)}>Cancel</button>
      <button class="btn primary" disabled={saving || !wTitle.trim() || !wStatement.trim()} onclick={saveWidget}>
        {saving ? 'Saving…' : 'Add widget'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .dash {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .dash-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 0 12px;
    flex-wrap: wrap;
  }
  .dash-pick {
    min-width: 180px;
    max-width: 260px;
  }
  .refresh-pick {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-dim);
  }
  .refresh-pick .input {
    width: 72px;
  }
  .widget-grid {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 14px;
    align-content: start;
    padding-bottom: 16px;
  }
  .viz-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .viz-chip {
    height: 24px;
    padding: 0 11px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    text-transform: capitalize;
  }
  .viz-chip.selected {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
    font-weight: 500;
  }
  .field-row {
    display: flex;
    gap: 12px;
  }
  .axes-hint {
    font-size: 11px;
    color: var(--text-dim);
    margin: 0 0 8px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
</style>
