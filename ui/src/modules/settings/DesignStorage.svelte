<script lang="ts">
  import { plural } from '../../lib/plural';
  // Settings → Backup & restore → "Design Hall storage" (root only).
  // Every Design Hall content save keeps a full copy, so the blob store only
  // grows. Auto-tidy squashes OLD autosave versions (older than a week, one
  // kept per 10-minute editing window; never the head, approved, pinned,
  // published or signal-cited versions) — it deletes user history, so it is
  // OPT-IN: off by default, and this card shows what it would reclaim right
  // now so the person decides with the number in front of them
  // (GET /design/admin/storage, PUT /design/admin/auto-tidy).
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { DesignStorageReport, DesignAutoTidyReq } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { formatBytes } from '../../lib/metric-format';
  import SettingToggle from './SettingToggle.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { toastError } from '../../lib/toastError';

  let report = $state<DesignStorageReport | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);

  async function load() {
    loading = true;
    loadError = '';
    try {
      report = await api.get<DesignStorageReport>('/design/admin/storage');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(load);

  const days = $derived(report ? Math.round(report.min_age_secs / 86_400) : 7);
  const forcedOff = $derived(report?.auto_tidy_forced === false);
  const forcedOn = $derived(report?.auto_tidy_forced === true);

  async function toggle(on: boolean): Promise<void> {
    if (!report) return;
    if (on) {
      const r = report.reclaimable;
      const ok = await confirmer.ask(
        `Once a day, Otto will delete Design Hall autosave versions older than ${days} days, keeping the last one ` +
          'of each 10-minute editing window. Named, agent, approved, pinned and published versions and the current ' +
          `version are never touched. Right now this would remove ${plural(r.versions, 'version')} ` +
          `and free about ${formatBytes(r.bytes)}. Deleted autosaves can’t be restored.`,
        { title: 'Turn on auto-tidy', confirmLabel: 'Turn on', danger: true },
      );
      if (!ok) return;
    }
    saving = true;
    try {
      report = await api.put<DesignStorageReport>('/design/admin/auto-tidy', { enabled: on } satisfies DesignAutoTidyReq);
      toasts.success(on ? 'Design auto-tidy is on' : 'Design auto-tidy is off');
    } catch (e) {
      toastError('Couldn’t change auto-tidy', e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="ds-card" aria-label="Design Hall storage" data-testid="design-storage">
  <h2 class="card-title">Design Hall storage</h2>
  <p>Every save of a design keeps a full copy, so Design Hall storage only grows. Auto-tidy can thin out old autosaves for
    you. It’s off unless you turn it on.</p>
  {#if !report}
    <LoadState what="Design Hall storage" {loading} error={loadError} empty={true} onretry={load} />
  {:else}
    <dl class="facts">
      <div><dt>On disk</dt><dd data-testid="design-storage-bytes">{formatBytes(report.blob_bytes)}</dd></div>
      <div><dt>Versions</dt><dd>{report.version_count.toLocaleString()}</dd></div>
      <div>
        <dt>Auto-tidy would free</dt>
        <dd data-testid="design-storage-reclaim">
          {formatBytes(report.reclaimable.bytes)}
          <span class="dim">({report.reclaimable.versions.toLocaleString()} old autosave{report.reclaimable.versions === 1 ? '' : 's'})</span>
        </dd>
      </div>
    </dl>
    <SettingToggle
      label="Auto-tidy old autosaves"
      checked={report.auto_prune}
      disabled={saving || report.auto_tidy_forced !== null}
      title={forcedOff
        ? 'Turned off by OTTO_DESIGN_AUTO_PRUNE=0 on the daemon'
        : forcedOn
          ? 'Forced on by OTTO_DESIGN_AUTO_PRUNE=1 on the daemon'
          : undefined}
      hint={forcedOff
        ? 'Disabled on this daemon (OTTO_DESIGN_AUTO_PRUNE=0).'
        : forcedOn
          ? 'Forced on by the daemon (OTTO_DESIGN_AUTO_PRUNE=1).'
          : `Once a day, delete autosaves older than ${days} days, keeping one per 10-minute editing window.`}
      onchange={toggle}
      testid="design-auto-tidy"
    />
    {#if report.last_prune}
      <p class="dim" role="status">
        Last tidy {new Date(report.last_prune.at).toLocaleString()}: removed {plural(report.last_prune.versions_removed, 'version')}.
      </p>
    {/if}
  {/if}
</section>

<style>
  .ds-card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-m); box-shadow: var(--shadow-card); padding: 16px 18px; margin: 0 0 16px; max-width: var(--settings-col); }
  .card-title { margin: 0 0 6px; font-size: var(--fs-m); font-weight: 600; }
  p { margin: 0 0 8px; font-size: var(--fs-s); line-height: 1.5; }
  .dim { color: var(--text-dim); }
  .facts { display: flex; flex-wrap: wrap; gap: 8px 24px; margin: 10px 0 8px; font-size: var(--fs-s); }
  .facts dt { color: var(--text-dim); }
  .facts dd { margin: 2px 0 0; font-weight: 500; font-variant-numeric: tabular-nums; }
</style>
