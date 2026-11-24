<script lang="ts">
  // Storage and retention (root): how long usage history is kept, how often
  // system load is sampled, which ClickHouse binary runs the engine — a sheet
  // opened from the Usage details card or ⋯.
  import Modal from '../../lib/components/Modal.svelte';
  import PathField from '../../lib/components/PathField.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { formatBytes, formatCount } from '../../lib/metric-format';
  import { pluralNoun } from '../../lib/plural';
  import { usage, type UsageStatus } from '../../lib/api/usage.svelte';

  let { status, onclose }: { status: UsageStatus; onclose: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let retention = $state(status.retention_days);
  // svelte-ignore state_referenced_locally
  let interval = $state(status.metrics_interval_secs);
  // svelte-ignore state_referenced_locally
  let chPath = $state(status.binary ?? '');
  const storageError = $derived(
    !Number.isInteger(retention) || retention < 1 || retention > 3650
      ? 'Keep history must be a whole number of days from 1 to 3650.'
      : !Number.isInteger(interval) || interval < 5 || interval > 3600
        ? 'Sample interval must be a whole number of seconds from 5 to 3600.'
        : '',
  );

  async function save(): Promise<void> {
    await usage.saveConfig({ enabled: true, retention_days: retention, metrics_interval_secs: interval, clickhouse_path: chPath.trim() });
    onclose();
  }
</script>

<Modal title="Storage and retention" width={560} {onclose}>
  <div class="cfg-grid">
    <label for="cfg-retention">Keep history for (days)</label>
    <input id="cfg-retention" class="input" type="number" min="1" max="3650" bind:value={retention} />
    <label for="cfg-interval">Sample system load every (seconds)</label>
    <input id="cfg-interval" class="input" type="number" min="5" max="3600" bind:value={interval} />
    <label for="cfg-path">ClickHouse binary</label>
    <PathField bind:value={chPath} files><input dir="ltr" id="cfg-path" class="input mono" bind:value={chPath} spellcheck="false" /></PathField>
  </div>
  {#if storageError}<p class="budget-error" use:announceShown={{ text: storageError, assertive: true }}>{storageError}</p>{/if}
  <div class="cfg-actions">
    <button class="btn" disabled={usage.installing} onclick={() => usage.install()}>{usage.installing ? 'Updating…' : 'Update ClickHouse'}</button>
    <button class="btn primary" disabled={usage.saving || !!storageError} title={storageError || undefined} onclick={() => void save()}>{usage.saving ? 'Saving…' : 'Save settings'}</button>
  </div>
  <div class="engine-meta">
    <span>ClickHouse <span class="mono">{status.version ?? '—'}</span></span>
    <span class="ellip-any" title={status.data_dir}>Data: <span class="mono">{status.data_dir}</span></span>
    <span title="{status.usage_rows.toLocaleString()} usage rows · {status.metric_rows.toLocaleString()} metric rows">
      On disk: {formatBytes(status.disk_bytes)} ({formatCount(status.usage_rows)} {pluralNoun(status.usage_rows, 'row', 'rows')})
    </span>
    <span>Kept for {status.retention_days} days</span>
  </div>
</Modal>

<style>
  .cfg-grid {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
    gap: 10px 12px;
    align-items: center;
    max-width: 640px;
  }
  .cfg-grid label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .cfg-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    max-width: 640px;
    margin-top: 14px;
  }
  .engine-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    margin-top: 14px;
    padding-top: 12px;
    border-top: 1px solid var(--separator);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .engine-meta > span {
    max-width: 100%;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .ellip-any {
    display: block;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .budget-error {
    color: var(--danger);
    font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .cfg-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
