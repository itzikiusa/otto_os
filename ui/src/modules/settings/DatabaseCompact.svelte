<script lang="ts">
  // Settings → Backup & restore → "Database storage" (root only).
  // Shows otto.db's size + reclaimable free space and runs the ONE-TIME
  // compaction (POST /admin/db/compact): auto_vacuum=INCREMENTAL + VACUUM.
  // It rewrites the whole file and blocks every database write meanwhile, so
  // it only runs after an explicit confirm. Afterwards the daemon's hourly
  // maintenance gives free pages back on its own.
  // Also hosts the opt-in "Run history" window (`data_retention.
  // run_history_days`, default 0 = keep forever): turning it on deletes old
  // finished runs, so it asks first and says exactly what goes.
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { DbStatsResp, DbCompactReport } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { formatBytes } from '../../lib/metric-format';
  import Icon from '../../lib/components/Icon.svelte';
  import { RUN_HISTORY_CHOICES, runHistoryDays, runHistoryLabel, withRunHistoryDays } from './runHistory';

  let stats = $state<DbStatsResp | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let compacting = $state(false);
  let last = $state<DbCompactReport | null>(null);
  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  // Run-history retention (opt-in). `settings` is the stored map, so a change
  // writes the whole `data_retention` object back with one field changed.
  let settings = $state<Record<string, unknown> | null>(null);
  let historyError = $state('');
  let savingHistory = $state(false);
  const historyDays = $derived(runHistoryDays(settings));
  // A stored custom window (e.g. 45) stays selectable.
  const historyChoices = $derived(
    RUN_HISTORY_CHOICES.includes(historyDays) ? RUN_HISTORY_CHOICES : [...RUN_HISTORY_CHOICES, historyDays].sort((a, b) => a - b),
  );

  async function loadSettings() {
    historyError = '';
    try {
      settings = await api.get<Record<string, unknown>>('/settings');
    } catch (e) {
      historyError = `Couldn’t read the retention setting. ${msg(e)}`;
    }
  }

  async function load() {
    loading = true;
    loadError = '';
    void loadSettings();
    try {
      stats = await api.get<DbStatsResp>('/admin/db/stats');
    } catch (e) {
      loadError = `Couldn’t read the database size. ${msg(e)}`;
    } finally {
      loading = false;
    }
  }
  onMount(load);

  async function setHistory(ev: Event) {
    const select = ev.currentTarget as HTMLSelectElement;
    const days = Number(select.value);
    if (days === historyDays) return;
    if (days > 0) {
      const ok = await confirmer.ask(
        `Delete finished runs older than ${days} days, every hour from now on? This removes Run with Otto runs and their ` +
          'events (unless a Proof Pack is attached), finished swarm runs and swarm messages, and all but the last iteration ' +
          'of finished goal loops. Running work is never touched, and swarm budgets keep counting what was removed. ' +
          'Deleted history can’t be recovered.',
        { title: 'Prune old run history', confirmLabel: `Keep ${days} days`, danger: true },
      );
      if (!ok) {
        select.value = String(historyDays);
        return;
      }
    }
    savingHistory = true;
    try {
      settings = await api.put<Record<string, unknown>>('/settings', {
        data_retention: withRunHistoryDays(settings, days),
      });
      toasts.success(
        days > 0 ? 'Run history window set' : 'Run history kept forever',
        days > 0 ? `Finished runs older than ${days} days are pruned hourly.` : 'Nothing is pruned from run history.',
      );
    } catch (e) {
      select.value = String(historyDays);
      toasts.error('Couldn’t change run history retention', msg(e));
    } finally {
      savingHistory = false;
    }
  }

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
  <p>Old audit and event logs, review retry data and expired sign-ins are pruned automatically, and the database is
    tidied every hour. Run history is kept unless you choose a window below. Space freed by pruning is reused but the
    file only shrinks after a one-time compaction.</p>
  <div class="history">
    <label for="run-history-days">Run history</label>
    {#if settings}
      <select
        id="run-history-days"
        class="input"
        value={String(historyDays)}
        disabled={savingHistory}
        aria-describedby="run-history-hint"
        onchange={setHistory}
      >
        {#each historyChoices as d (d)}
          <option value={String(d)}>{runHistoryLabel(d)}</option>
        {/each}
      </select>
    {:else if historyError}
      <span class="error inline" role="alert">{historyError}</span>
      <button class="btn" onclick={loadSettings}>Retry</button>
    {:else}
      <span class="dim" role="status">Reading…</span>
    {/if}
  </div>
  <p id="run-history-hint" class="dim">
    Finished Run with Otto runs, swarm runs and messages, and old goal-loop iterations. Off by default; at least 14 days.
  </p>
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
  .error.inline { margin: 0; }
  .history { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; margin: 10px 0 4px; font-size: var(--fs-s); }
  .history label { font-weight: 500; }
  .history select { min-inline-size: 12rem; max-inline-size: 100%; }
</style>
