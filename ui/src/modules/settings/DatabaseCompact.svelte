<script lang="ts">
  // Settings → Backup & restore → "Database storage" (root only).
  // Shows otto.db's size + reclaimable free space and runs the ONE-TIME
  // compaction (POST /admin/db/compact): auto_vacuum=INCREMENTAL + VACUUM.
  // It rewrites the whole file and blocks every database write meanwhile, so
  // it only runs after an explicit confirm. Afterwards the daemon's hourly
  // maintenance gives free pages back on its own.
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { DbStatsResp, DbCompactReport } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { formatBytes } from '../../lib/metric-format';
  import Icon from '../../lib/components/Icon.svelte';

  let stats = $state<DbStatsResp | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let compacting = $state(false);
  let last = $state<DbCompactReport | null>(null);
  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  async function load() {
    loading = true;
    loadError = '';
    try {
      stats = await api.get<DbStatsResp>('/admin/db/stats');
    } catch (e) {
      loadError = `Couldn’t read the database size. ${msg(e)}`;
    } finally {
      loading = false;
    }
  }
  onMount(load);

  const compacted = $derived(stats?.auto_vacuum === 2);

  async function compact() {
    if (!stats) return;
    const ok = await confirmer.ask(
      `Rewrite otto.db (${formatBytes(stats.size_bytes)}) to give back about ${formatBytes(stats.free_bytes)} of free space? ` +
        'While it runs — usually under a minute, longer for a large database — every database write waits, so sessions, ' +
        'workflows and the UI may stall or show errors. Nothing is deleted. Run it when no agents are busy.',
      { title: 'Compact database', confirmLabel: 'Compact now', danger: true },
    );
    if (!ok) return;
    compacting = true;
    try {
      last = await api.post<DbCompactReport>('/admin/db/compact', { confirm: true });
      toasts.success('Database compacted', `Freed ${formatBytes(last.freed_bytes)} in ${(last.duration_ms / 1000).toFixed(1)} s.`);
      await load();
    } catch (e) {
      toasts.error('Couldn’t compact the database', msg(e));
    } finally {
      compacting = false;
    }
  }
</script>

<section class="db-card" aria-label="Database storage">
  <h2 class="card-title">Database storage</h2>
  <p>Old run history, review retry data and expired sign-ins are pruned automatically, and the database is tidied every
    hour. Space freed by pruning is reused but the file only shrinks after a one-time compaction.</p>
  {#if loading && !stats}
    <p class="dim" role="status">Reading database size…</p>
  {:else if loadError}
    <p class="error" role="alert">{loadError}</p>
    <div class="controls"><button class="btn" onclick={load}>Retry</button></div>
  {:else if stats}
    <dl class="facts">
      <div><dt>Size</dt><dd>{formatBytes(stats.size_bytes)}</dd></div>
      <div><dt>Reclaimable</dt><dd>{formatBytes(stats.free_bytes)}</dd></div>
      <div><dt>Compaction</dt><dd>{compacted ? 'Done — free space is returned hourly' : 'Not yet'}</dd></div>
    </dl>
    <div class="controls">
      <button class="btn" disabled={compacting || stats.compacting} onclick={compact}>
        <Icon name="db" size={13} />
        {compacting || stats.compacting ? 'Compacting…' : compacted ? 'Compact again…' : 'Compact database…'}
      </button>
    </div>
    {#if last}
      <p class="notice" role="status">{formatBytes(last.before_bytes)} → {formatBytes(last.after_bytes)} in {(last.duration_ms / 1000).toFixed(1)} s.</p>
    {/if}
  {/if}
</section>

<style>
  .db-card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-m); box-shadow: var(--shadow-card); padding: 16px 18px; margin: 0 0 16px; max-width: var(--settings-col); }
  .card-title { margin: 0 0 6px; font-size: var(--fs-m); font-weight: 600; }
  p { margin: 0 0 8px; font-size: var(--fs-s); line-height: 1.5; }
  .dim { color: var(--text-dim); }
  .facts { display: flex; flex-wrap: wrap; gap: 8px 24px; margin: 10px 0 0; font-size: var(--fs-s); }
  .facts dt { color: var(--text-dim); }
  .facts dd { margin: 2px 0 0; font-weight: 500; font-variant-numeric: tabular-nums; }
  .controls { display: flex; flex-wrap: wrap; gap: 8px; margin: 12px 0 0; }
  .notice { margin: 10px 0 0; color: var(--success); }
  .error { margin: 10px 0 0; color: var(--danger); overflow-wrap: anywhere; }
</style>
