<script lang="ts">
  import { api, ApiError } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { BrokerCluster, GroupDetail, GroupOffset, GroupSummary } from '../../lib/api/types';
  import type { DryRunResp } from './types';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  let groups = $state<GroupSummary[]>([]);
  let loading = $state(true);
  // True when the broker's ACLs deny consumer-group access (probed once, cached
  // server-side). We show a clear banner instead of erroring, and skip re-probes.
  let accessDenied = $state(false);
  let accessMsg = $state('');
  let selected = $state<string | null>(null);
  let detail = $state<GroupDetail | null>(null);
  let detailLoading = $state(false);
  // Sort offsets table by lag descending; toggle to sort by topic+partition.
  let sortByLag = $state(true);
  // Offset reset state.
  let resetting = $state(false);
  let resetMode = $state<'earliest' | 'latest' | 'offset' | 'timestamp'>('latest');
  let resetOffset = $state(0);
  let resetTs = $state('');
  let resetTopic = $state('');
  // Dry-run preview state.
  let dryRunLoading = $state(false);
  let dryRunResult = $state<DryRunResp | null>(null);

  $effect(() => {
    void cluster.id;
    selected = null;
    detail = null;
    loading = true;
    accessDenied = false;
    api
      .get<GroupSummary[]>(`/brokers/clusters/${cluster.id}/groups`)
      .then((g) => {
        groups = g;
        accessDenied = false;
      })
      .catch((e) => {
        if (e instanceof ApiError && e.status === 403 && /consumer-group access/i.test(e.message)) {
          // Broker ACLs deny group access — show the banner, don't toast/retry.
          accessDenied = true;
          accessMsg = e.message;
          groups = [];
        } else {
          toasts.error('Failed to load groups', String(e));
        }
      })
      .finally(() => (loading = false));
  });

  function open(id: string) {
    selected = id;
    detail = null;
    detailLoading = true;
    resetTopic = '';
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

  // Per-topic subtotals: sum lag across partitions.
  const topicSubtotals = $derived.by(() => {
    if (!detail) return new Map<string, number>();
    const m = new Map<string, number>();
    for (const o of detail.offsets) {
      m.set(o.topic, (m.get(o.topic) ?? 0) + o.lag);
    }
    return m;
  });

  // Sorted offsets view.
  const sortedOffsets = $derived.by(() => {
    if (!detail) return [] as GroupOffset[];
    const offsets = [...detail.offsets];
    if (sortByLag) {
      offsets.sort((a, b) => b.lag - a.lag);
    }
    // else keep the default (topic+partition order from the server)
    return offsets;
  });

  // Max lag across all partitions — used to scale the per-row bars.
  const maxLag = $derived(Math.max(1, ...sortedOffsets.map((o) => o.lag)));

  // Unique topics in the current group (for the topic filter dropdown).
  const groupTopics = $derived(
    [...new Set(detail?.offsets.map((o) => o.topic) ?? [])].sort(),
  );

  function buildResetBody(confirm: boolean): Record<string, unknown> {
    let body: Record<string, unknown>;
    if (resetMode === 'offset') {
      body = { mode: 'offset', offset: Number(resetOffset), confirm };
    } else if (resetMode === 'timestamp') {
      body = {
        mode: 'timestamp',
        timestamp_ms: new Date(resetTs).getTime() || Date.now(),
        confirm,
      };
    } else {
      body = { mode: resetMode, confirm };
    }
    if (resetTopic) body['topic'] = resetTopic;
    return body;
  }

  async function previewReset() {
    if (!selected) return;
    dryRunLoading = true;
    dryRunResult = null;
    try {
      dryRunResult = await api.post<DryRunResp>(
        `/brokers/clusters/${cluster.id}/groups/${encodeURIComponent(selected)}/reset?dry_run=true`,
        buildResetBody(false),
      );
    } catch (e) {
      toasts.error('Dry-run failed', String(e));
    } finally {
      dryRunLoading = false;
    }
  }

  async function applyReset() {
    if (!selected) return;
    // Clear preview and proceed to confirmation.
    dryRunResult = null;
    await resetOffsets();
  }

  async function resetOffsets() {
    if (!selected) return;
    const typed = await confirmer.promptText(
      `Type the group name to confirm offset reset.`,
      { title: `Reset offsets for "${selected}"`, confirmLabel: 'Reset', placeholder: selected },
    );
    if (typed !== selected) return;

    resetting = true;
    try {
      const updated = await api.post<GroupDetail>(
        `/brokers/clusters/${cluster.id}/groups/${encodeURIComponent(selected)}/reset`,
        buildResetBody(guarded),
      );
      detail = updated;
      toasts.success(`Offsets reset for "${selected}"`);
    } catch (e) {
      toasts.error('Offset reset failed', String(e));
    } finally {
      resetting = false;
    }
  }
</script>

<div class="groups">
  <div class="list">
    {#if loading}
      <p class="muted pad">Loading…</p>
    {:else if accessDenied}
      <div class="acl-denied pad">
        <p class="acl-title">Consumer-group access not granted</p>
        <p class="muted">{accessMsg}</p>
        <p class="muted">
          Lag and connected consumers need <code>DescribeGroup</code> permission on the broker.
          Otto probed once and won't keep retrying (so it stops hitting the broker with denied
          requests); grant the ACL and re-test the cluster, and this tab will populate.
        </p>
      </div>
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

      <div class="offsets-header">
        <h5>Committed offsets &amp; lag</h5>
        <label class="sort-toggle">
          <input type="checkbox" bind:checked={sortByLag} />
          Sort by lag
        </label>
      </div>
      <table>
        <thead>
          <tr>
            <th>Topic</th><th>P</th><th>Current</th><th>End</th>
            <th class="lag-col">Lag</th><th class="bar-col"></th>
          </tr>
        </thead>
        <tbody>
          {#each sortedOffsets as o (o.topic + '-' + o.partition)}
            <tr>
              <td class="mono">{o.topic}</td>
              <td>{o.partition}</td>
              <td class="muted">{o.current_offset.toLocaleString()}</td>
              <td class="muted">{o.high_watermark.toLocaleString()}</td>
              <td class:has-lag={o.lag > 0}>{o.lag.toLocaleString()}</td>
              <td class="bar-col">
                {#if o.lag > 0}
                  <div class="lag-bar">
                    <div class="lag-fill" style="width: {Math.round((o.lag / maxLag) * 100)}%"></div>
                  </div>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if topicSubtotals.size > 1}
        <h5>Per-topic totals</h5>
        <table>
          <thead><tr><th>Topic</th><th>Total lag</th></tr></thead>
          <tbody>
            {#each [...topicSubtotals.entries()].sort((a, b) => b[1] - a[1]) as [t, lag] (t)}
              <tr>
                <td class="mono">{t}</td>
                <td class:has-lag={lag > 0}>{lag.toLocaleString()}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
      {#if detail.offsets.length === 0}
        <p class="muted pad">No committed offsets.</p>
      {/if}

      <!-- Offset reset panel (Editor only; dry-run preview then typed confirm) -->
      <h5>Reset offsets</h5>
      <div class="reset-bar">
        <select bind:value={resetMode} onchange={() => (dryRunResult = null)}>
          <option value="earliest">Earliest</option>
          <option value="latest">Latest</option>
          <option value="offset">Specific offset</option>
          <option value="timestamp">From timestamp</option>
        </select>
        {#if resetMode === 'offset'}
          <input type="number" class="sm-input" bind:value={resetOffset} placeholder="offset"
            oninput={() => (dryRunResult = null)} />
        {/if}
        {#if resetMode === 'timestamp'}
          <input type="datetime-local" class="sm-input wide" bind:value={resetTs}
            oninput={() => (dryRunResult = null)} />
        {/if}
        {#if groupTopics.length > 1}
          <select bind:value={resetTopic} title="Scope to one topic (blank = all)"
            onchange={() => (dryRunResult = null)}>
            <option value="">All topics</option>
            {#each groupTopics as t (t)}<option value={t}>{t}</option>{/each}
          </select>
        {/if}
        <button
          class="btn small"
          onclick={previewReset}
          disabled={dryRunLoading || resetting}
          title="Preview what this reset would do without committing"
        >
          {dryRunLoading ? 'Previewing…' : 'Preview'}
        </button>
        <button
          class="btn small danger"
          onclick={applyReset}
          disabled={resetting || dryRunLoading}
          title={guarded ? 'Cluster is guarded — requires confirmation' : 'Reset committed offsets'}
        >
          {resetting ? 'Resetting…' : 'Reset'}
        </button>
      </div>

      <!-- Dry-run preview table -->
      {#if dryRunResult}
        <div class="dryrun-preview">
          <div class="dryrun-summary">
            <span>Preview: lag <strong>{dryRunResult.total_lag_before.toLocaleString()}</strong>
            → <strong class:ok={dryRunResult.total_lag_after < dryRunResult.total_lag_before}
                       class:warn={dryRunResult.total_lag_after > dryRunResult.total_lag_before}>
              {dryRunResult.total_lag_after.toLocaleString()}</strong>
            ({dryRunResult.partitions.length} partition{dryRunResult.partitions.length === 1 ? '' : 's'} affected)
            </span>
            <button class="close-dry" onclick={() => (dryRunResult = null)} title="Close preview">✕</button>
          </div>
          <table class="dryrun-table">
            <thead><tr><th>Topic</th><th>P</th><th>Current</th><th>Target</th><th>Lag Δ</th></tr></thead>
            <tbody>
              {#each dryRunResult.partitions as p (p.topic + '-' + p.partition)}
                <tr>
                  <td class="mono">{p.topic}</td>
                  <td>{p.partition}</td>
                  <td class="muted">{p.current_offset.toLocaleString()}</td>
                  <td class="muted">{p.target_offset.toLocaleString()}</td>
                  <td class:ok={p.lag_delta > 0} class:warn={p.lag_delta < 0}>
                    {p.lag_delta >= 0 ? '-' : '+'}{Math.abs(p.lag_delta).toLocaleString()}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
          <p class="muted small">Click <strong>Reset</strong> above to apply after reviewing.</p>
        </div>
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
    border-inline-end: 1px solid var(--border);
    overflow: auto;
  }
  .grow-row {
    width: 100%;
    text-align: start;
    border: none;
    background: transparent;
    padding: 9px 12px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    cursor: pointer;
    border-inline-start: 2px solid transparent;
  }
  .grow-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .grow-row.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-inline-start-color: var(--accent);
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
    margin-inline-start: auto;
    font-size: 12px;
    color: var(--text-dim);
  }
  .lag-total.has-lag,
  td.has-lag {
    color: #f5a623;
    font-weight: 600;
  }
  .offsets-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .sort-toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
    margin-inline-start: auto;
  }
  .lag-col {
    min-width: 70px;
  }
  .bar-col {
    width: 80px;
    padding: 4px 8px;
  }
  .lag-bar {
    height: 6px;
    border-radius: 3px;
    background: color-mix(in srgb, #f5a623 15%, transparent);
    overflow: hidden;
  }
  .lag-fill {
    height: 100%;
    background: #f5a623;
    border-radius: 3px;
  }
  .reset-bar {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
    padding: 8px 0 4px;
  }
  .reset-bar select,
  .reset-bar input {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .sm-input {
    width: 120px;
  }
  .sm-input.wide {
    width: 190px;
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
    text-align: start;
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
  .acl-denied {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 8px;
    border: 1px solid color-mix(in srgb, var(--status-degraded, #e3b341) 45%, var(--border));
    border-inline-start: 3px solid var(--status-degraded, #e3b341);
    border-radius: var(--radius-s, 4px);
    background: color-mix(in srgb, var(--status-degraded, #e3b341) 8%, transparent);
  }
  .acl-title {
    font-weight: 600;
    color: var(--text);
  }
  .acl-denied code {
    font-family: var(--mono, monospace);
    background: var(--surface-2);
    padding: 0 4px;
    border-radius: 3px;
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
  }
  /* Dry-run preview */
  .dryrun-preview {
    margin-top: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    overflow: hidden;
  }
  .dryrun-summary {
    display: flex;
    align-items: center;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    font-size: 12px;
    gap: 8px;
  }
  .dryrun-summary .ok {
    color: var(--status-working, #28c840);
  }
  .dryrun-summary .warn {
    color: #f5a623;
  }
  .close-dry {
    margin-inline-start: auto;
    border: none;
    background: transparent;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 12px;
  }
  .dryrun-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .dryrun-table th {
    text-align: start;
    padding: 4px 8px;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 500;
    border-bottom: 1px solid var(--border);
  }
  .dryrun-table td {
    padding: 3px 8px;
    border-top: 1px solid var(--border);
  }
  .dryrun-table td.ok {
    color: var(--status-working, #28c840);
  }
  .dryrun-table td.warn {
    color: #f5a623;
  }
  .dryrun-preview p {
    padding: 5px 10px;
    margin: 0;
  }
</style>
