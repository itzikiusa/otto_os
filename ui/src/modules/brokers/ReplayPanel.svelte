<script lang="ts">
  // DLQ/Replay panel. Lets an operator re-publish selected messages from a
  // source topic to a target topic, with optional key/header transforms.
  // Evidence is recorded server-side (broker_replays table).

  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { BrokerCluster } from '../../lib/api/types';
  import type { ReplayResp, ReplaySelector } from './types';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  let sourceTopic = $state('');
  let targetTopic = $state('');
  let selectorType = $state<'latest' | 'offset_range' | 'timestamp'>('latest');
  let count = $state(10);
  let partition = $state(0);
  let fromOffset = $state(0);
  let toOffset = $state(0);
  let timestampMs = $state('');
  let tsLimit = $state(50);
  // Optional transform.
  let setKey = $state('');
  let addHeaderKey = $state('');
  let addHeaderVal = $state('');

  let running = $state(false);
  let result = $state<ReplayResp | null>(null);

  function buildSelector(): ReplaySelector {
    if (selectorType === 'offset_range') {
      return { type: 'offset_range', partition, from: fromOffset, to: toOffset };
    }
    if (selectorType === 'timestamp') {
      return {
        type: 'timestamp',
        timestamp_ms: new Date(timestampMs).getTime() || Date.now(),
        limit: tsLimit,
      };
    }
    return { type: 'latest', count };
  }

  async function runReplay() {
    if (!sourceTopic.trim() || !targetTopic.trim()) {
      toasts.error('Replay', 'Source and target topic are required');
      return;
    }
    const ok = await confirmer.ask(
      `Replay messages from "${sourceTopic}" → "${targetTopic}"${guarded ? ' (guarded cluster)' : ''}. This produces to the target topic.`,
      { title: 'Confirm replay', confirmLabel: 'Replay', danger: guarded },
    );
    if (!ok) return;

    const body: Record<string, unknown> = {
      source_topic: sourceTopic.trim(),
      target_topic: targetTopic.trim(),
      selector: buildSelector(),
      confirm: guarded,
    };
    if (setKey.trim() || (addHeaderKey.trim() && addHeaderVal.trim())) {
      const transform: Record<string, unknown> = {};
      if (setKey.trim()) transform['set_key'] = setKey.trim();
      if (addHeaderKey.trim() && addHeaderVal.trim())
        transform['add_header'] = [addHeaderKey.trim(), addHeaderVal.trim()];
      body['transform'] = transform;
    }

    running = true;
    result = null;
    try {
      result = await api.post<ReplayResp>(`/brokers/clusters/${cluster.id}/replay`, body);
      toasts.success(
        `Replay complete`,
        `${result.count} message${result.count === 1 ? '' : 's'} replayed to "${result.target_topic}"`,
      );
    } catch (e) {
      toasts.error('Replay failed', String(e));
    } finally {
      running = false;
    }
  }
</script>

<div class="replay">
  <h5>DLQ / Replay</h5>
  <p class="muted small">
    Re-publish messages from a source topic (e.g. a dead-letter queue) to a target topic.
    An evidence record is saved for auditing.
  </p>

  <div class="form">
    <label>
      Source topic
      <input type="text" bind:value={sourceTopic} placeholder="e.g. orders-dlq" />
    </label>
    <label>
      Target topic
      <input type="text" bind:value={targetTopic} placeholder="e.g. orders" />
    </label>

    <label>
      Selector
      <select bind:value={selectorType}>
        <option value="latest">Last N messages</option>
        <option value="offset_range">Offset range (single partition)</option>
        <option value="timestamp">Since timestamp</option>
      </select>
    </label>

    {#if selectorType === 'latest'}
      <label>
        Count
        <input type="number" bind:value={count} min="1" max="5000" class="narrow" />
      </label>
    {:else if selectorType === 'offset_range'}
      <div class="inline-row">
        <label>
          Partition
          <input type="number" bind:value={partition} min="0" class="narrow" />
        </label>
        <label>
          From offset
          <input type="number" bind:value={fromOffset} min="0" class="narrow" />
        </label>
        <label>
          To offset
          <input type="number" bind:value={toOffset} min="0" class="narrow" />
        </label>
      </div>
    {:else}
      <div class="inline-row">
        <label>
          From (local time)
          <input type="datetime-local" bind:value={timestampMs} />
        </label>
        <label>
          Limit
          <input type="number" bind:value={tsLimit} min="1" max="5000" class="narrow" />
        </label>
      </div>
    {/if}

    <details class="transform-details">
      <summary class="small muted">Transform (optional)</summary>
      <div class="transform-body">
        <label>
          Override key
          <input type="text" bind:value={setKey} placeholder="leave blank to keep original" />
        </label>
        <div class="inline-row">
          <label>
            Add/overwrite header key
            <input type="text" bind:value={addHeaderKey} placeholder="x-replayed-from" />
          </label>
          <label>
            Value
            <input type="text" bind:value={addHeaderVal} placeholder="source-topic-name" />
          </label>
        </div>
      </div>
    </details>

    <button
      class="btn primary"
      onclick={runReplay}
      disabled={running || !sourceTopic.trim() || !targetTopic.trim()}
    >
      {running ? 'Replaying…' : 'Replay'}
    </button>
  </div>

  <!-- Evidence table -->
  {#if result}
    <div class="evidence">
      <h5>Evidence — replay {result.replay_id.slice(0, 8)}…</h5>
      <p class="muted small">
        {result.count} message{result.count === 1 ? '' : 's'} replayed
        from <code>{result.source_topic}</code> → <code>{result.target_topic}</code>
      </p>
      <table>
        <thead>
          <tr>
            <th>Src P</th><th>Src offset</th><th>Key preview</th>
            <th>Dst P</th><th>Dst offset</th>
          </tr>
        </thead>
        <tbody>
          {#each result.evidence as e (e.partition + '-' + e.offset)}
            <tr>
              <td>{e.partition}</td>
              <td class="mono">{e.offset}</td>
              <td class="mono muted">{e.key_preview ?? '—'}</td>
              <td>{e.target_partition}</td>
              <td class="mono">{e.target_offset}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .replay {
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
  .form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 10px;
    max-width: 560px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-dim);
  }
  label input,
  label select {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 5px 7px;
    color: var(--text);
    font-size: 12.5px;
  }
  .narrow {
    width: 100px;
  }
  .inline-row {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }
  .transform-details {
    font-size: 12px;
  }
  .transform-details summary {
    cursor: pointer;
    padding: 4px 0;
  }
  .transform-body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 0 4px;
  }
  .evidence {
    margin-top: 20px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
    margin-top: 6px;
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
    padding: 3px 8px;
    border-top: 1px solid var(--border);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
  code {
    font-family: var(--font-mono);
    background: var(--surface-2);
    padding: 0 4px;
    border-radius: 3px;
    font-size: 11.5px;
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
</style>
