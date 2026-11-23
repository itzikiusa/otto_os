<script lang="ts">
  // Networks tab: list Docker networks with subnet, driver, and attached
  // containers, inspect details, delete custom networks, or prune unused ones.
  // All destructive actions confirm via confirmer.ask({ danger: true }).
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
  import { plural } from '../../lib/plural';
  import type { NetworkSummary } from '../../lib/api/types/containers';
  import { filterNetworks } from './containers-util';

  interface Props {
    networks: readonly NetworkSummary[];
    loading: boolean;
    error: string | null;
    onrefresh: () => void;
  }

  let { networks, loading, error, onrefresh }: Props = $props();

  let query = $state('');
  let busyId = $state<string | null>(null);
  let pruning = $state(false);
  let inspected = $state<NetworkSummary | null>(null);

  const visible = $derived(filterNetworks(networks, query));

  async function removeNetwork(net: NetworkSummary): Promise<void> {
    const ok = await confirmer.ask(
      `Remove network “${net.name}”? Containers relying on this network won’t be able to reconnect to it until recreated.`,
      {
        title: 'Remove network',
        confirmLabel: 'Remove network',
        danger: true,
      },
    );
    if (!ok) return;
    busyId = net.id;
    try {
      await containersApi.removeNetwork(net.id);
      toasts.success('Network removed', net.name);
      onrefresh();
    } catch (e) {
      toastError('Couldn’t remove network', e);
    } finally {
      busyId = null;
    }
  }

  async function pruneNetworks(): Promise<void> {
    const ok = await confirmer.ask(
      'Remove all custom Docker networks not used by at least one container? Built-in networks (bridge, host, none) are kept.',
      {
        title: 'Prune unused networks',
        confirmLabel: 'Prune networks',
        danger: true,
      },
    );
    if (!ok) return;
    pruning = true;
    try {
      const res = await containersApi.pruneNetworks();
      const count = res.deleted.length;
      toasts.success(
        count > 0 ? `Pruned ${plural(count, 'network')}` : 'No unused networks to prune',
      );
      onrefresh();
    } catch (e) {
      toastError('Couldn’t prune networks', e);
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
        aria-label="Filter networks"
        placeholder="Filter networks by name, driver, subnet, or container…"
        bind:value={query}
      />
      {#if query}
        <button
          type="button"
          class="icon-btn"
          aria-label="Clear network filter"
          title="Clear network filter"
          onclick={() => (query = '')}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>

    <div class="ct-res-actions">
      <button
        type="button"
        class="btn small danger"
        disabled={pruning || networks.length === 0}
        onclick={() => void pruneNetworks()}
      >
        <Icon name="trash" size={12} />
        {pruning ? 'Pruning…' : 'Prune…'}
      </button>
    </div>
  </div>

  <LoadState
    what="networks"
    {loading}
    {error}
    empty={networks.length === 0}
    onretry={onrefresh}
  >
    {#snippet emptyView()}
      <EmptyState
        icon="globe"
        title="No networks found"
        body="Docker networks reported by the active daemon will appear here."
      />
    {/snippet}

    {#if visible.length === 0}
      <EmptyState
        icon="search"
        title="No matching networks"
        body="Clear the search filter to see all networks."
        actionLabel="Clear filter"
        actionKind="secondary"
        onaction={() => (query = '')}
      />
    {:else}
      <div class="ct-table-wrap">
        <table class="ct-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Driver</th>
              <th>Subnet</th>
              <th class="ct-num">Containers</th>
              <th>Kind</th>
              <th class="ct-col-act"><span class="sr-only">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {#each visible as net (net.id)}
              <tr>
                <td class="ct-mono ct-strong" dir="ltr" title={net.name}>{net.name}</td>
                <td>{net.driver}</td>
                <td class="ct-mono ct-dim" dir="ltr">{net.subnet || '—'}</td>
                <td class="ct-num">{net.containers.length}</td>
                <td>
                  {#if net.built_in}
                    <Badge tone="neutral" label="Built-in" />
                  {:else if net.internal}
                    <Badge tone="info" label="Internal" />
                  {:else}
                    <Badge tone="ok" variant="outline" label="Custom" />
                  {/if}
                </td>
                <td class="ct-col-act">
                  <div class="ct-row-acts">
                    <button
                      type="button"
                      class="icon-btn"
                      aria-label={`Inspect network ${net.name}`}
                      title={`Inspect network ${net.name}`}
                      onclick={() => (inspected = net)}
                    >
                      <Icon name="info" size={13} />
                    </button>
                    <button
                      type="button"
                      class="icon-btn ct-danger-icon"
                      disabled={busyId === net.id || net.built_in || net.containers.length > 0}
                      aria-label={`Remove network ${net.name}`}
                      title={net.built_in
                        ? 'Built-in networks cannot be removed'
                        : net.containers.length > 0
                          ? 'Disconnect attached containers first'
                          : `Remove network ${net.name}`}
                      onclick={() => void removeNetwork(net)}
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
        <span>{plural(visible.length, 'network')}</span>
      </div>
    {/if}
  </LoadState>
</div>

{#if inspected}
  {@const net = inspected}
  <Modal title="Network details" width={540} onclose={() => (inspected = null)}>
    <dl class="ct-inspect-dl">
      <dt>Name</dt>
      <dd class="ct-mono" dir="ltr">{net.name}</dd>
      <dt>ID</dt>
      <dd class="ct-mono" dir="ltr">{net.id}</dd>
      <dt>Driver</dt>
      <dd>{net.driver} ({net.scope})</dd>
      <dt>Subnet</dt>
      <dd class="ct-mono" dir="ltr">{net.subnet || '—'}</dd>
      <dt>Gateway</dt>
      <dd class="ct-mono" dir="ltr">{net.gateway || '—'}</dd>
      <dt>Containers</dt>
      <dd class="ct-mono" dir="ltr">
        {#if net.containers.length > 0}
          {net.containers.map((c) => `${c.name}${c.ipv4_address ? ` (${c.ipv4_address})` : ''}`).join('\n')}
        {:else}
          None attached
        {/if}
      </dd>
    </dl>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => void copyWithToast(net.id, 'network ID')}>
        <Icon name="copy" size={12} /> Copy ID
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
