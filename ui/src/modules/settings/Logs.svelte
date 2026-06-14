<script lang="ts">
  import { tick } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { DaemonLogs } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toasts } from '../../lib/toast.svelte';

  type LogMode = 'all' | 'tail';

  const ALL_FILES = '__all__';

  let loading = $state(true);
  let refreshing = $state(false);
  let autoRefresh = $state(true);
  let follow = $state(true);
  let mode: LogMode = $state('all');
  let tailLines = $state(2000);
  let selected = $state('');
  let filter = $state('');
  let content = $state('');
  let nextOffset = $state(0);
  let payload: DaemonLogs | null = $state(null);
  let logEl: HTMLPreElement | null = $state(null);

  const visibleContent = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return content;
    return content
      .split('\n')
      .filter((line) => line.toLowerCase().includes(q))
      .join('\n');
  });

  const status = $derived.by(() => {
    if (!payload) return '';
    const fileCount = payload.files.length;
    const bytes = new Intl.NumberFormat().format(content.length);
    return `${fileCount} file${fileCount === 1 ? '' : 's'} · ${bytes} chars · offset ${payload.next_offset}`;
  });

  $effect(() => {
    void loadInitial();
  });

  $effect(() => {
    if (!autoRefresh) return;
    const id = window.setInterval(() => {
      void refreshIncremental();
    }, 1500);
    return () => window.clearInterval(id);
  });

  async function loadInitial(): Promise<void> {
    loading = true;
    try {
      const data = await fetchLogs(mode);
      payload = data;
      selected = data.selected;
      content = data.content;
      nextOffset = data.next_offset;
      await maybeFollow();
    } catch (e) {
      toasts.error('Could not load daemon logs', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function refreshFull(): Promise<void> {
    refreshing = true;
    try {
      const data = await fetchLogs(mode);
      payload = data;
      selected = data.selected;
      content = data.content;
      nextOffset = data.next_offset;
      await maybeFollow();
    } catch (e) {
      toasts.error('Refresh failed', e instanceof Error ? e.message : String(e));
    } finally {
      refreshing = false;
    }
  }

  async function refreshIncremental(): Promise<void> {
    if (refreshing || loading) return;
    if (!selected || selected === ALL_FILES || mode === 'tail') {
      await refreshFull();
      return;
    }
    refreshing = true;
    try {
      const data = await fetchLogs('since', nextOffset);
      payload = { ...data, mode: 'all' };
      selected = data.selected;
      if (data.content) {
        content += data.content;
      }
      nextOffset = data.next_offset;
      await maybeFollow();
    } catch (e) {
      toasts.error('Log update failed', e instanceof Error ? e.message : String(e));
    } finally {
      refreshing = false;
    }
  }

  async function fetchLogs(fetchMode: LogMode | 'since', offset?: number): Promise<DaemonLogs> {
    const params = new URLSearchParams();
    if (selected) params.set('file', selected);
    params.set('mode', fetchMode);
    if (fetchMode === 'tail') params.set('lines', String(Math.max(1, tailLines)));
    if (fetchMode === 'since') params.set('offset', String(offset ?? 0));
    return api.get<DaemonLogs>(`/logs/daemon?${params.toString()}`);
  }

  async function onFileChange(): Promise<void> {
    await refreshFull();
  }

  async function onModeChange(): Promise<void> {
    await refreshFull();
  }

  async function maybeFollow(): Promise<void> {
    if (!follow) return;
    await tick();
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  }
</script>

<div class="page logs-page">
  <div class="page-header">
    <div>
      <h1>Logs</h1>
      <div class="sub">{payload?.log_dir ?? '~/Library/Logs/Otto'}</div>
    </div>
    <div class="header-actions">
      <button class="btn" disabled={refreshing || loading} onclick={refreshFull}>
        <Icon name="refresh" size={13} />
        {refreshing ? 'Refreshing…' : 'Refresh'}
      </button>
    </div>
  </div>

  {#if loading}
    <Skeleton rows={8} height={34} />
  {:else}
    <div class="toolbar">
      <label class="field compact">
        <span>File</span>
        <select class="input mono" bind:value={selected} onchange={onFileChange}>
          <option value={ALL_FILES}>All log files</option>
          {#each payload?.files ?? [] as file (file.name)}
            <option value={file.name}>{file.name}</option>
          {/each}
        </select>
      </label>

      <label class="field compact">
        <span>Read</span>
        <select class="input" bind:value={mode} onchange={onModeChange}>
          <option value="all">Full file</option>
          <option value="tail">Tail</option>
        </select>
      </label>

      {#if mode === 'tail'}
        <label class="field lines">
          <span>Lines</span>
          <input
            class="input mono"
            type="number"
            min="1"
            max="50000"
            bind:value={tailLines}
            onkeydown={(e) => e.key === 'Enter' && refreshFull()}
          />
        </label>
      {/if}

      <label class="field search-field">
        <span>Filter</span>
        <input class="input" placeholder="slack, telegram, bridge…" bind:value={filter} />
      </label>

      <label class="check-control" title="Poll for new log lines">
        <input type="checkbox" bind:checked={autoRefresh} />
        Live
      </label>

      <label class="check-control" title="Keep the log view scrolled to the bottom">
        <input type="checkbox" bind:checked={follow} />
        Follow
      </label>
    </div>

    <div class="log-meta">
      <span>{status}</span>
      {#if refreshing}
        <span>updating…</span>
      {/if}
    </div>

    <pre class="log-view" bind:this={logEl}>{visibleContent}</pre>
  {/if}
</div>

<style>
  .logs-page {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .header-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 10px;
    padding: 0 24px 12px;
    border-bottom: 1px solid var(--border);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field span {
    font-size: 11px;
    color: var(--text-dim);
  }
  .field.compact {
    width: 210px;
  }
  .field.lines {
    width: 96px;
  }
  .search-field {
    flex: 1;
    min-width: 180px;
  }
  .check-control {
    height: 30px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text);
    user-select: none;
  }
  .log-meta {
    min-height: 28px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 0 24px;
    color: var(--text-dim);
    font-size: 11.5px;
    border-bottom: 1px solid var(--border);
  }
  .log-view {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 14px 18px 32px;
    overflow: auto;
    background: #111312;
    color: #d7ded8;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace;
    font-size: 11.5px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    border: 0;
  }
</style>
