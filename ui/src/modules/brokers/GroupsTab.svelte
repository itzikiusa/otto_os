<script lang="ts">
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type { BrokerCluster, GroupDetail, GroupSummary } from '../../lib/api/types';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  let groups = $state<GroupSummary[]>([]);
  let loading = $state(true);
  let selected = $state<string | null>(null);
  let detail = $state<GroupDetail | null>(null);
  let detailLoading = $state(false);

  $effect(() => {
    void cluster.id;
    selected = null;
    detail = null;
    loading = true;
    api
      .get<GroupSummary[]>(`/brokers/clusters/${cluster.id}/groups`)
      .then((g) => (groups = g))
      .catch((e) => toasts.error('Failed to load groups', String(e)))
      .finally(() => (loading = false));
  });

  function open(id: string) {
    selected = id;
    detail = null;
    detailLoading = true;
    api
      .get<GroupDetail>(`/brokers/clusters/${cluster.id}/groups/${encodeURIComponent(id)}`)
      .then((d) => (detail = d))
      .catch((e) => toasts.error('Failed to describe group', String(e)))
      .finally(() => (detailLoading = false));
  }

  function stateClass(s: string): string {
    const v = s.toLowerCase();
    if (v.includes('stable')) return 'ok';
    if (v.includes('empty') || v.includes('dead')) return 'dim';
    return 'warn';
  }
</script>

<div class="groups">
  <div class="list">
    {#if loading}
      <p class="muted pad">Loading…</p>
    {:else if groups.length === 0}
      <p class="muted pad">No consumer groups.</p>
    {:else}
      {#each groups as g (g.group_id)}
        <button class="grow-row" class:sel={selected === g.group_id} onclick={() => open(g.group_id)}>
          <span class="gid">{g.group_id}</span>
          <span class="badges">
            <span class="state {stateClass(g.state)}">{g.state}</span>
            <span class="muted">{g.members} member{g.members === 1 ? '' : 's'}</span>
          </span>
        </button>
      {/each}
    {/if}
  </div>

  <div class="detail">
    {#if detailLoading}
      <p class="muted pad">Loading group…</p>
    {:else if detail}
      <header>
        <span class="gid big">{detail.group_id}</span>
        <span class="state {stateClass(detail.state)}">{detail.state}</span>
        <span class="lag-total" class:has-lag={detail.total_lag > 0}>
          total lag {detail.total_lag.toLocaleString()}
        </span>
      </header>

      {#if detail.members.length > 0}
        <h5>Members</h5>
        <table>
          <thead><tr><th>Member</th><th>Client</th><th>Host</th><th>Assignments</th></tr></thead>
          <tbody>
            {#each detail.members as m (m.member_id)}
              <tr>
                <td class="mono small">{m.member_id}</td>
                <td>{m.client_id}</td>
                <td class="muted">{m.host}</td>
                <td class="muted small">{m.assignments.length} partitions</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}

      <h5>Committed offsets &amp; lag</h5>
      <table>
        <thead><tr><th>Topic</th><th>P</th><th>Current</th><th>End</th><th>Lag</th></tr></thead>
        <tbody>
          {#each detail.offsets as o (o.topic + '-' + o.partition)}
            <tr>
              <td class="mono">{o.topic}</td>
              <td>{o.partition}</td>
              <td class="muted">{o.current_offset.toLocaleString()}</td>
              <td class="muted">{o.high_watermark.toLocaleString()}</td>
              <td class:has-lag={o.lag > 0}>{o.lag.toLocaleString()}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if detail.offsets.length === 0}
        <p class="muted pad">No committed offsets.</p>
      {/if}
    {:else}
      <p class="muted pad">Select a consumer group to see members and lag.</p>
    {/if}
  </div>
</div>

<style>
  .groups {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .list {
    width: 300px;
    border-right: 1px solid var(--border);
    overflow: auto;
  }
  .grow-row {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 9px 12px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    cursor: pointer;
    border-left: 2px solid transparent;
  }
  .grow-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .grow-row.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-left-color: var(--accent);
  }
  .gid {
    font-family: var(--font-mono);
    font-size: 12.5px;
    word-break: break-all;
  }
  .gid.big {
    font-size: 14px;
  }
  .badges {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 11px;
  }
  .state {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 1px 6px;
    border-radius: 4px;
  }
  .state.ok {
    background: color-mix(in srgb, var(--status-working, #28c840) 22%, transparent);
    color: var(--status-working, #28c840);
  }
  .state.warn {
    background: color-mix(in srgb, #f5a623 22%, transparent);
    color: #f5a623;
  }
  .state.dim {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .detail {
    flex: 1;
    overflow: auto;
    padding: 14px;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 10px;
    flex-wrap: wrap;
  }
  .lag-total {
    margin-left: auto;
    font-size: 12px;
    color: var(--text-dim);
  }
  .lag-total.has-lag,
  td.has-lag {
    color: #f5a623;
    font-weight: 600;
  }
  h5 {
    margin: 16px 0 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    text-align: left;
    font-weight: 500;
    color: var(--text-dim);
    font-size: 11px;
    padding: 5px 8px;
  }
  td {
    padding: 4px 8px;
    border-top: 1px solid var(--border);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .small {
    font-size: 11px;
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 12px;
  }
</style>
