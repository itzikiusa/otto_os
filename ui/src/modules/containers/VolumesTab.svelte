<script lang="ts">
  // Volumes tab: list local Docker volumes, inspect mountpoint/labels, delete
  // individual volumes, or prune unused volumes. All destructive actions gate
  // through confirmer.ask({ danger: true }).
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { containersApi } from '../../lib/api/containers';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { copyWithToast } from '../../lib/clipboard';
  import { formatBytes } from '../../lib/metric-format';
  import { plural } from '../../lib/plural';
  import { rel } from '../../lib/stores/now.svelte';
  import type { VolumeSummary } from '../../lib/api/types/containers';
  import { filterVolumes } from './containers-util';

  interface Props {
    volumes: readonly VolumeSummary[];
    loading: boolean;
    error: string | null;
    onrefresh: () => void;
  }

  let { volumes, loading, error, onrefresh }: Props = $props();

  let query = $state('');
  let onlyUnused = $state(false);
  let busyName = $state<string | null>(null);
  let pruning = $state(false);
  let inspected = $state<VolumeSummary | null>(null);

  const visible = $derived(filterVolumes(volumes, query, onlyUnused));

  async function removeVolume(vol: VolumeSummary): Promise<void> {
    const ok = await confirmer.ask(
      `Delete volume “${vol.name}”? All data stored in this volume will be permanently lost.`,
      {
        title: 'Delete volume',
        confirmLabel: 'Delete volume',
        danger: true,
      },
    );
    if (!ok) return;
    busyName = vol.name;
    try {
      await containersApi.removeVolume(vol.name);
      toasts.success('Volume deleted', vol.name);
      onrefresh();
    } catch (e) {
      toastError('Couldn’t delete volume', e);
    } finally {
      busyName = null;
    }
  }

  async function pruneVolumes(): Promise<void> {
    const ok = await confirmer.ask(
      'Remove all local volumes not currently referenced by any container? Stored data in unused volumes cannot be recovered.',
      {
        title: 'Prune unused volumes',
        confirmLabel: 'Prune volumes',
        danger: true,
      },
    );
    if (!ok) return;
    pruning = true;
    try {
      const res = await containersApi.pruneVolumes();
      const count = res.deleted.length;
      toasts.success(
        count > 0 ? `Pruned ${plural(count, 'volume')}` : 'No unused volumes to prune',
        res.space_reclaimed_bytes > 0 ? `Reclaimed ${formatBytes(res.space_reclaimed_bytes)}` : undefined,
      );
      onrefresh();
    } catch (e) {
      toastError('Couldn’t prune volumes', e);
    } finally {
      pruning = false;
    }
  }
</script>

<div class="ct-res-view">
  <div class="ct-res-bar">
    <div class="ct-res-search">
      <Icon name="search" size={13} />
      <input
        class="ct-res-input"
        type="search"
        dir="auto"
        aria-label="Filter volumes"
        placeholder="Filter volumes by name, driver, or path…"
        bind:value={query}
      />
      {#if query}
        <button
          type="button"
          class="icon-btn"
          aria-label="Clear volume filter"
          title="Clear volume filter"
          onclick={() => (query = '')}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>

    <div class="segmented" role="group" aria-label="Volume filter">
      <button
        type="button"
        class:active={!onlyUnused}
        aria-pressed={!onlyUnused}
        onclick={() => (onlyUnused = false)}
      >
        All
      </button>
      <button
        type="button"
        class:active={onlyUnused}
        aria-pressed={onlyUnused}
        onclick={() => (onlyUnused = true)}
      >
        Unused
      </button>
    </div>

    <div class="ct-res-actions">
      <button
        type="button"
        class="btn small danger"
        disabled={pruning || volumes.length === 0}
        onclick={() => void pruneVolumes()}
      >
        <Icon name="trash" size={12} />
        {pruning ? 'Pruning…' : 'Prune…'}
      </button>
    </div>
  </div>

  <LoadState
    what="volumes"
    {loading}
    {error}
    empty={volumes.length === 0}
    onretry={onrefresh}
  >
    {#snippet emptyView()}
      <EmptyState
        icon="db"
        title="No volumes yet"
        body="Named and anonymous Docker volumes created by containers will appear here."
      />
    {/snippet}

    {#if visible.length === 0}
      <EmptyState
        icon="search"
        title="No matching volumes"
        body="Clear the search filter to see all volumes."
        actionLabel="Clear filter"
        actionKind="secondary"
        onaction={() => {
          query = '';
          onlyUnused = false;
        }}
      />
    {:else}
      <div class="ct-table-wrap">
        <table class="ct-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Driver</th>
              <th>Status</th>
              <th>Created</th>
              <th class="ct-num">Size</th>
              <th class="ct-col-act"><span class="sr-only">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {#each visible as vol (vol.name)}
              <tr>
                <td class="ct-mono ct-strong" dir="ltr" title={vol.name}>{vol.name}</td>
                <td>{vol.driver}</td>
                <td>
                  {#if vol.in_use}
                    <Badge tone="ok" label="In use" />
                  {:else}
                    <Badge tone="neutral" label="Unused" />
                  {/if}
                </td>
                <td class="ct-dim">{vol.created_at ? rel(vol.created_at) : '—'}</td>
                <td class="ct-num">
                  {vol.size_bytes != null && vol.size_bytes >= 0 ? formatBytes(vol.size_bytes) : '—'}
                </td>
                <td class="ct-col-act">
                  <div class="ct-row-acts">
                    <button
                      type="button"
                      class="icon-btn"
                      aria-label={`Inspect volume ${vol.name}`}
                      title={`Inspect volume ${vol.name}`}
                      onclick={() => (inspected = vol)}
                    >
                      <Icon name="info" size={13} />
                    </button>
                    <button
                      type="button"
                      class="icon-btn ct-danger-icon"
                      disabled={busyName === vol.name || vol.in_use}
                      aria-label={`Delete volume ${vol.name}`}
                      title={vol.in_use ? 'In use by a container' : `Delete volume ${vol.name}`}
                      onclick={() => void removeVolume(vol)}
                    >
                      <Icon name="trash" size={13} />
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="ct-res-foot">
        <span>{plural(visible.length, 'volume')}</span>
      </div>
    {/if}
  </LoadState>
</div>

{#if inspected}
  {@const vol = inspected}
  {@const labels = Object.entries(vol.labels ?? {})}
  <Modal title="Volume details" width={540} onclose={() => (inspected = null)}>
    <dl class="ct-inspect-dl">
      <dt>Name</dt>
      <dd class="ct-mono" dir="ltr">{vol.name}</dd>
      <dt>Driver</dt>
      <dd>{vol.driver} ({vol.scope})</dd>
      <dt>Mountpoint</dt>
      <dd class="ct-mono" dir="ltr">{vol.mountpoint || '—'}</dd>
      {#if vol.created_at}
        <dt>Created</dt>
        <dd>{rel(vol.created_at)}</dd>
      {/if}
      {#if labels.length > 0}
        <dt>Labels</dt>
        <dd class="ct-mono" dir="ltr">{labels.map(([k, v]) => `${k}=${v}`).join('\n')}</dd>
      {/if}
    </dl>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => void copyWithToast(vol.mountpoint || vol.name, 'volume path')}>
        <Icon name="copy" size={12} /> Copy path
      </button>
      <button type="button" class="btn primary" onclick={() => (inspected = null)}>Close</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ct-res-view {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-inline-size: 0;
  }
  .ct-res-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .ct-res-search {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 220px;
    min-inline-size: 160px;
    max-inline-size: 360px;
    block-size: 28px;
    padding-inline: 8px 4px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .ct-res-search:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .ct-res-input {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .ct-res-input::placeholder {
    color: var(--text-dim);
  }
  .ct-res-actions {
    margin-inline-start: auto;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ct-table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .ct-table {
    inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .ct-table th {
    text-align: start;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    padding: 8px 10px;
    border-block-end: 1px solid var(--border);
    white-space: nowrap;
  }
  .ct-table td {
    padding: 6px 10px;
    border-block-end: 1px solid var(--border);
    vertical-align: middle;
    max-inline-size: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-table tbody tr:last-child td {
    border-block-end: 0;
  }
  .ct-table tbody tr:hover {
    background: var(--hover);
  }
  .ct-num {
    text-align: end;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .ct-col-act {
    inline-size: 64px;
    text-align: end;
  }
  .ct-row-acts {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .ct-mono {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .ct-strong {
    font-weight: 500;
  }
  .ct-dim {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .ct-danger-icon:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .ct-res-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ct-inspect-dl {
    margin: 0;
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    gap: 8px 12px;
    font-size: var(--fs-s);
  }
  .ct-inspect-dl dt {
    color: var(--text-dim);
    font-weight: 500;
  }
  .ct-inspect-dl dd {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .sr-only {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
  }
</style>
