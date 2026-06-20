<script lang="ts">
  // Schema version history + version diff panel. Shown inside SchemaTab when a
  // subject is selected. Fetches all registered versions and lets the operator
  // compare any two via the shared DiffView component (word-level diff).

  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import DiffView from '../../lib/components/DiffView.svelte';
  import type { BrokerCluster } from '../../lib/api/types';
  import type { SchemaVersion, CompatCheckResp } from './types';

  interface Props {
    cluster: BrokerCluster;
    subject: string;
  }
  let { cluster, subject }: Props = $props();

  let versions = $state<SchemaVersion[]>([]);
  let loading = $state(true);
  // The two versions selected for the diff.
  let diffA = $state<SchemaVersion | null>(null);
  let diffB = $state<SchemaVersion | null>(null);
  let showDiff = $state(false);

  // Compatibility check against latest.
  let compatSchema = $state('');
  let compatLoading = $state(false);
  let compatResult = $state<CompatCheckResp | null>(null);

  $effect(() => {
    void cluster.id;
    void subject;
    loading = true;
    versions = [];
    diffA = null;
    diffB = null;
    showDiff = false;
    compatResult = null;
    api
      .get<SchemaVersion[]>(
        `/brokers/clusters/${cluster.id}/schema-registry/subjects/${encodeURIComponent(subject)}/versions`,
      )
      .then((v) => {
        versions = v;
        if (v.length >= 1) diffB = v[v.length - 1];
        if (v.length >= 2) diffA = v[v.length - 2];
      })
      .catch((e) => toasts.error('Failed to load versions', String(e)))
      .finally(() => (loading = false));
  });

  function pretty(schema: string): string {
    try {
      return JSON.stringify(JSON.parse(schema), null, 2);
    } catch {
      return schema;
    }
  }

  async function checkCompat() {
    if (!compatSchema.trim()) return;
    compatLoading = true;
    compatResult = null;
    try {
      compatResult = await api.post<CompatCheckResp>(
        `/brokers/clusters/${cluster.id}/schema-registry/subjects/${encodeURIComponent(subject)}/compatibility`,
        { schema: compatSchema },
      );
    } catch (e) {
      toasts.error('Compatibility check failed', String(e));
    } finally {
      compatLoading = false;
    }
  }
</script>

<div class="svp">
  {#if loading}
    <p class="muted pad">Loading versions…</p>
  {:else if versions.length === 0}
    <p class="muted pad">No versions found.</p>
  {:else}
    <!-- Version list -->
    <section class="version-list">
      <h5>Versions ({versions.length})</h5>
      <div class="vtable">
        {#each versions as v (v.version)}
          <div class="vrow">
            <span class="vnum">v{v.version}</span>
            <span class="vid muted">#{v.id}</span>
            <span class="vtype muted">{v.schema_type}</span>
            <div class="vbtns">
              <button
                class="btn small"
                class:active={diffA?.version === v.version}
                onclick={() => { diffA = v; showDiff = !!(diffA && diffB); }}
                title="Set as 'before' side of diff"
              >A</button>
              <button
                class="btn small"
                class:active={diffB?.version === v.version}
                onclick={() => { diffB = v; showDiff = !!(diffA && diffB); }}
                title="Set as 'after' side of diff"
              >B</button>
            </div>
          </div>
        {/each}
      </div>
    </section>

    <!-- Version diff -->
    {#if diffA && diffB && showDiff}
      <section class="diff-section">
        <h5>Diff v{diffA.version} → v{diffB.version}</h5>
        <DiffView before={pretty(diffA.schema)} after={pretty(diffB.schema)} mode="word" contextLines={3} />
      </section>
    {:else if diffA && diffB}
      <section>
        <button class="btn small" onclick={() => (showDiff = true)}>
          Show diff v{diffA.version} → v{diffB.version}
        </button>
      </section>
    {/if}

    <!-- Compatibility check panel -->
    <section class="compat-section">
      <h5>Check compatibility against latest</h5>
      <p class="muted small">Paste a candidate schema to check it against the latest registered version.</p>
      <textarea
        class="compat-input"
        bind:value={compatSchema}
        placeholder={'{"type":"record","name":"...","fields":[...]}'}
        rows="5"
      ></textarea>
      <div class="compat-row">
        <button class="btn small" onclick={checkCompat} disabled={compatLoading || !compatSchema.trim()}>
          {compatLoading ? 'Checking…' : 'Check'}
        </button>
        {#if compatResult}
          <span class="compat-result" class:ok={compatResult.compatible} class:fail={!compatResult.compatible}>
            {compatResult.compatible ? 'Compatible' : 'Incompatible'}
          </span>
          {#if compatResult.messages.length > 0}
            <ul class="compat-msgs">
              {#each compatResult.messages as m (m)}<li>{m}</li>{/each}
            </ul>
          {/if}
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .svp {
    padding: 12px 14px;
    overflow: auto;
    height: 100%;
  }
  h5 {
    margin: 14px 0 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  .vtable {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .vrow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
    font-size: 12.5px;
  }
  .vnum {
    font-family: var(--font-mono);
    min-width: 40px;
    font-weight: 600;
  }
  .vid {
    font-family: var(--font-mono);
    font-size: 11px;
    min-width: 38px;
  }
  .vtype {
    font-size: 11px;
    flex: 1;
  }
  .vbtns {
    display: flex;
    gap: 4px;
  }
  .btn.active {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    border-color: var(--accent);
    color: var(--accent);
  }
  .diff-section {
    margin-top: 10px;
  }
  .compat-section {
    margin-top: 18px;
  }
  .compat-input {
    width: 100%;
    box-sizing: border-box;
    font-family: var(--font-mono);
    font-size: 11.5px;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    resize: vertical;
    margin-top: 6px;
  }
  .compat-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
  .compat-result {
    font-weight: 600;
    font-size: 12.5px;
  }
  .compat-result.ok {
    color: var(--status-working, #28c840);
  }
  .compat-result.fail {
    color: var(--status-exited, #ff5f57);
  }
  .compat-msgs {
    margin: 4px 0 0;
    padding-left: 18px;
    font-size: 11.5px;
    color: var(--status-exited, #ff5f57);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
  .pad {
    padding: 12px;
  }
</style>
