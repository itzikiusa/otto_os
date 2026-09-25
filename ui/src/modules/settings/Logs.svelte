<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
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
      failed('Refresh failed', e);
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
      failed('Log update failed', e);
    } finally {
      refreshing = false;
    }
  }

  /** A failed read. While Live is on this ran every 1.5 s and stacked a toast
   *  per tick; pause Live instead and say so once. */
  function failed(title: string, e: unknown): void {
    const msg = e instanceof Error ? e.message : String(e);
    if (autoRefresh) {
      autoRefresh = false;
      toasts.error(`${title} — Live paused`, `${msg} Turn Live back on to retry.`);
    } else {
      toasts.error(title, msg);
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
    if (selected === ALL_FILES) mode = 'all';
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

<div class="settings-section logs-section">
  <PageHeader title={sectionLabel('logs')} subtitle={payload?.log_dir ?? '~/Library/Logs/Otto'}>
    {#snippet actions()}
      <button class="btn" disabled={refreshing || loading} onclick={refreshFull}>
        <Icon name="refresh" size={13} />
        {refreshing ? 'Refreshing…' : 'Refresh'}
      </button>
    {/snippet}
  </PageHeader>
  <PageBody padded={false} fill>

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
        <!-- The daemon reads every file in full for "All log files" whatever
             the mode, so Tail is only offered for a single file. -->
        <select
          class="input"
          bind:value={mode}
          onchange={onModeChange}
          disabled={selected === ALL_FILES}
          title={selected === ALL_FILES ? 'Pick one file to tail it — all files are always read in full' : undefined}
        >
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
            title="Up to 50,000 lines — press Enter or Refresh to apply"
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
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 10px;
    padding: 12px 20px;
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
    padding: 0 20px;
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
