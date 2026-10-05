<script lang="ts">
  // Everything sent from this workspace, newest first: method + path, host,
  // status and when. Opening a row loads its request into a tab (it is NOT
  // re-sent; masked credentials are never sent back — see loadHistoryIntoDraft).
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import AgentChip from '../../lib/components/AgentChip.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import MethodTag, { methodWord } from './MethodTag.svelte';
  import StatusChip from './StatusChip.svelte';
  import RetentionDialog from './RetentionDialog.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { splitUrl } from '../../lib/api/apiVars';
  import type { ApiHistorySummary } from '../../lib/api/types';
  import { RETENTION_PRESETS, formatBytes, suggestRetention, type RetentionPreset } from './storageGauge';
  import { toasts } from '../../lib/toast.svelte';

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
  // The list holds only the newest HISTORY_PAGE summaries. Once it is full, a
  // search also asks the daemon (debounced, aborted on the next keystroke) so
  // older entries are findable; results merge with the local rows and go
  // through the same all-tokens filter below.
  const HISTORY_PAGE = 100;
  let serverRows = $state.raw<ApiHistorySummary[] | null>(null);
  $effect(() => {
    const t = tokens;
    const full = apiClient.history.length >= HISTORY_PAGE;
    serverRows = null;
    if (!t.length || !full) return;
    // The daemon matches ONE literal against method/URL (or an exact status):
    // send the most selective token; the rest narrow client-side.
    const text = t.filter((x) => !/^\d{3}$/.test(x)).sort((a, b) => b.length - a.length)[0];
    const params = text ? { q: text } : { status: Number(t[0]) };
    const ctl = new AbortController();
    const timer = setTimeout(() => {
      apiClient.searchHistory(params, ctl.signal).then(
        (rows) => { if (!ctl.signal.aborted) serverRows = rows; },
        () => { /* aborted / offline: the local rows still filter */ },
      );
    }, 200);
    return () => { clearTimeout(timer); ctl.abort(); };
  });
  const searchPool = $derived.by(() => {
    if (!serverRows) return apiClient.history;
    const seen = new Set(apiClient.history.map((h) => h.id));
    const extra = serverRows.filter((h) => !seen.has(h.id));
    if (!extra.length) return apiClient.history;
    return [...apiClient.history, ...extra].sort((a, b) =>
      a.executed_at === b.executed_at ? (a.id < b.id ? 1 : -1) : a.executed_at < b.executed_at ? 1 : -1,
    );
  });
  const filtered = $derived(
    searchPool.filter(
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
      'Delete all request history in this workspace, including the stored responses? Saved requests are not affected.',
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

  // Storage gauge (perf2 N2): how big history + runs are, and — past a size
  // with no limit set — an inline offer of retention presets. Opt-in only:
  // nothing is deleted until someone picks a preset and confirms.
  $effect(() => {
    if (ws.currentId) void apiClient.loadStorage();
  });
  const gauge = $derived(apiClient.storage);
  const dismissKey = $derived(`otto_api_storage_banner_dismissed:${ws.currentId ?? ''}`);
  let dismissed = $state(false);
  $effect(() => {
    try { dismissed = localStorage.getItem(dismissKey) === '1'; } catch { dismissed = false; }
  });
  const showBanner = $derived(
    !dismissed && suggestRetention(gauge, { rows: retention.rows, days: retention.days, runsKeep: ws.apiRunsKeep }),
  );
  function dismissBanner(): void {
    dismissed = true;
    try { localStorage.setItem(dismissKey, '1'); } catch { /* this session only */ }
  }
  let applying = $state(false);
  async function applyPreset(p: RetentionPreset): Promise<void> {
    if (!canEdit || applying) return;
    const what = [
      p.rows ? `keeps the newest ${p.rows.toLocaleString()} requests` : '',
      p.days ? `deletes requests older than ${p.days} days` : '',
      p.runsKeep ? `keeps the newest ${p.runsKeep} runs of each automation` : '',
    ].filter(Boolean).join(', ');
    if (!(await confirmer.ask(
      `This workspace’s history ${what}. Older entries, with their stored responses, are deleted now and after every request — for everyone in the workspace. Saved requests are not affected.`,
      { title: `Retention: ${p.label}`, confirmLabel: 'Apply retention' },
    ))) return;
    applying = true;
    try {
      await ws.setApiHistoryRetention(p.rows, p.days);
      if (p.runsKeep !== null && !ws.apiRunsKeep) await ws.setApiRunsKeep(p.runsKeep);
      if (await apiClient.applyRetention()) toasts.success('Retention applied', p.label);
    } catch (e) {
      toastError('Couldn’t save retention', e);
    } finally {
      applying = false;
    }
  }

  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Retention…', icon: 'clock', action: () => (retentionOpen = true), disabled: !canEdit },
      { separator: true },
      { label: 'Clear history…', icon: 'trash', danger: true, disabled: !canEdit || apiClient.history.length === 0, action: () => void clear() },
    ]);
  }

  /** ⌘F text of a history row — its cells in DOM order (see the VirtualList). */
  function histFindText(h: ApiHistorySummary): string {
    const u = splitUrl(h.url);
    const agent = apiClient.historySource(h)?.kind === 'agent' ? 'Agent' : '';
    return [methodWord(h.method), u.path, u.host || '—', agent, h.status ?? 'No response', rel(h.executed_at)].join('\n');
  }
</script>

<div class="hist-wrap">
  <div class="tools">
    <label class="search">
      <Icon name="search" size={12} />
      <input placeholder="Filter by URL, method or status…" bind:value={search} aria-label="Search request history" />
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
      aria-label="Show only agent runs"
      title="Show only requests that agents sent through Otto’s tools"
      onclick={() => (apiClient.historyAgentOnly = !apiClient.historyAgentOnly)}
    >Agent runs only</button>
    <span class="ret" title="History retention (workspace setting)">{retentionText}{#if gauge} · {formatBytes(gauge.history_bytes)}{/if}</span>
  </div>
  {#if showBanner && gauge}
    <div class="size-banner" role="status">
      <div class="sb-text">
        History is using <strong>{formatBytes(gauge.history_bytes + gauge.run_bytes)}</strong>
        ({gauge.history_rows.toLocaleString()} requests, {gauge.run_rows.toLocaleString()} automation runs). Everything is kept until you set a limit.
      </div>
      <div class="sb-actions">
        {#if canEdit}
          {#each RETENTION_PRESETS as p (p.label)}
            <button class="btn small" disabled={applying} onclick={() => void applyPreset(p)}>{p.label}</button>
          {/each}
          <button class="btn small ghost" onclick={() => (retentionOpen = true)}>Custom…</button>
        {/if}
        <button class="icon-btn sb-dismiss" onclick={dismissBanner} aria-label="Dismiss storage notice" title="Dismiss"><Icon name="x" size={12} /></button>
      </div>
    </div>
  {/if}

  {#if apiClient.historyLoadingId}<div class="state" role="status"><span class="spinner" style="--spinner-size: 11px" aria-hidden="true"></span> Loading request…</div>{/if}

  {#if apiClient.history.length === 0 && (apiClient.historyLoadError || !apiClient.historyLoaded)}
    <LoadState
      what="history"
      variant="compact"
      loading={!apiClient.historyLoaded && !apiClient.historyLoadError}
      error={apiClient.historyLoadError}
      empty
      onretry={() => apiClient.retryHistory()}
    />
  {:else if apiClient.history.length === 0}
    <EmptyState icon="clock" title="Nothing sent yet" body="Every request you send appears here, so you can open it again later." />
  {:else if filtered.length === 0}
    <div class="state">
      {apiClient.historyAgentOnly && !tokens.length ? 'No agent runs yet.' : `No history matches “${search.trim()}”.`}
    </div>
  {:else}
    <!-- findText: the row's text in DOM order (MethodTag's short word, path,
         host, source chip, status, age), so ⌘F reaches unmounted rows. -->
    <VirtualList items={filtered} estimateHeight={44} class="hist-vlist" findText={histFindText}>
      {#snippet row(h: ApiHistorySummary)}
        {@const src = apiClient.historySource(h)}
        {@const u = splitUrl(h.url)}
        <button class="hist-row" data-url={h.url} onclick={() => open(h)} title="{h.method} {h.url}&#10;{new Date(h.executed_at).toLocaleString()}">
          <span class="l1">
            <MethodTag method={h.method} fixed />
            <span class="path mono" dir="ltr">{u.path}</span>
          </span>
          <span class="l2">
            <span class="host" dir="ltr">{u.host || '—'}</span>
            {#if src?.kind === 'agent'}<AgentChip title="Sent by an agent session" />{/if}
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
  .size-banner {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-block: 4px 6px;
    margin-inline: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--info-soft);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .sb-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .sb-dismiss {
    margin-inline-start: auto;
  }
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
    padding-block: 0; padding-inline: 8px 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
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
    padding-inline-start: 40px;
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
</style>
