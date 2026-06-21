<script lang="ts">
  // Lag alert configuration UI. Lists configured alerts for a cluster,
  // showing breach status when the last metrics sweep detected a threshold
  // crossing. Allows creating and deleting alerts.

  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { BrokerCluster } from '../../lib/api/types';
  import type { LagAlert } from './types';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  let alerts = $state<LagAlert[]>([]);
  let loading = $state(true);

  // New alert form fields.
  let newTopic = $state('');
  let newGroup = $state('');
  let newThreshold = $state(1000);
  let creating = $state(false);

  $effect(() => {
    void cluster.id;
    loading = true;
    alerts = [];
    api
      .get<LagAlert[]>(`/brokers/clusters/${cluster.id}/lag-alerts`)
      .then((a) => (alerts = a))
      .catch((e) => toasts.error('Failed to load lag alerts', String(e)))
      .finally(() => (loading = false));
  });

  async function createAlert() {
    if (!newTopic.trim() || !newGroup.trim()) return;
    if (newThreshold <= 0) {
      toasts.error('Threshold must be > 0');
      return;
    }
    creating = true;
    try {
      const a = await api.post<LagAlert>(`/brokers/clusters/${cluster.id}/lag-alerts`, {
        topic: newTopic.trim(),
        group_name: newGroup.trim(),
        threshold: newThreshold,
      });
      alerts = [...alerts, a];
      newTopic = '';
      newGroup = '';
      newThreshold = 1000;
      toasts.success('Alert created');
    } catch (e) {
      toasts.error('Create alert failed', String(e));
    } finally {
      creating = false;
    }
  }

  async function deleteAlert(alert: LagAlert) {
    const ok = await confirmer.ask(
      `Delete lag alert for "${alert.topic}" / "${alert.group_name}"?`,
      { title: 'Delete alert' },
    );
    if (!ok) return;
    try {
      await api.del(`/brokers/clusters/${cluster.id}/lag-alerts/${alert.id}`);
      alerts = alerts.filter((a) => a.id !== alert.id);
      toasts.success('Alert deleted');
    } catch (e) {
      toasts.error('Delete alert failed', String(e));
    }
  }
</script>

<div class="lag-alerts">
  <h5>Lag Alerts</h5>
  <p class="muted small">
    Alerts fire when the group's lag for a topic exceeds the threshold.
    Breach status is evaluated each time metrics are refreshed and shown here.
  </p>

  {#if loading}
    <p class="muted pad">Loading…</p>
  {:else}
    {#if alerts.length > 0}
      <table>
        <thead>
          <tr>
            <th>Topic</th><th>Group</th><th>Threshold</th>
            <th>Status</th><th></th>
          </tr>
        </thead>
        <tbody>
          {#each alerts as a (a.id)}
            <tr>
              <td class="mono">{a.topic}</td>
              <td class="mono">{a.group_name}</td>
              <td>{a.threshold.toLocaleString()}</td>
              <td>
                {#if a.breach_lag !== undefined}
                  <span class="badge breach">
                    Breached ({a.breach_lag.toLocaleString()})
                  </span>
                {:else if a.enabled}
                  <span class="badge ok">Active</span>
                {:else}
                  <span class="badge dim">Disabled</span>
                {/if}
              </td>
              <td>
                <button
                  class="btn small danger"
                  onclick={() => deleteAlert(a)}
                  title="Delete alert"
                >Delete</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="muted pad">No alerts configured.</p>
    {/if}

    <h5 class="create-head">Add alert</h5>
    <div class="create-row">
      <input type="text" bind:value={newTopic} placeholder="topic" class="field" />
      <input type="text" bind:value={newGroup} placeholder="consumer group" class="field wide" />
      <label class="thresh-label">
        Threshold
        <input type="number" bind:value={newThreshold} min="1" class="narrow" />
      </label>
      <button
        class="btn small primary"
        onclick={createAlert}
        disabled={creating || !newTopic.trim() || !newGroup.trim()}
      >
        {creating ? 'Creating…' : 'Add'}
      </button>
    </div>
  {/if}
</div>

<style>
  .lag-alerts {
    padding: 14px;
    overflow: auto;
    height: 100%;
  }
  h5 {
    margin: 0 0 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  h5.create-head {
    margin-top: 20px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
    margin-top: 8px;
  }
  th {
    text-align: start;
    font-weight: 500;
    color: var(--text-dim);
    font-size: 11px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }
  td {
    padding: 4px 8px;
    border-top: 1px solid var(--border);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
  .badge {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 4px;
  }
  .badge.breach {
    background: color-mix(in srgb, var(--status-exited, #ff5f57) 18%, transparent);
    color: var(--status-exited, #ff5f57);
    font-weight: 600;
  }
  .badge.ok {
    background: color-mix(in srgb, var(--status-working, #28c840) 18%, transparent);
    color: var(--status-working, #28c840);
  }
  .badge.dim {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .create-row {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    flex-wrap: wrap;
    margin-top: 6px;
  }
  .field {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 5px 7px;
    color: var(--text);
    font-size: 12.5px;
    min-width: 120px;
  }
  .field.wide {
    min-width: 180px;
  }
  .narrow {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 5px 7px;
    color: var(--text);
    font-size: 12.5px;
    width: 90px;
  }
  .thresh-label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
  .pad {
    padding: 8px 0;
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
  }
</style>
