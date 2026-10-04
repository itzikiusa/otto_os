<script lang="ts">
  // Schema version history + version diff panel. Shown inside SchemaTab when a
  // subject is selected. Fetches all registered versions and lets the operator
  // compare any two via the shared DiffView component (word-level diff).

  import { SchemaVersionCache } from './schemaVersionCache';
  import { api } from '../../lib/api/client';
  import { toastError } from '../../lib/toastError';
  import DiffView from '../../lib/components/DiffView.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { BrokerCluster } from '../../lib/api/types';
  import type { SchemaVersion, SchemaVersionDetail, CompatCheckResp } from './types';

  interface Props {
    cluster: BrokerCluster;
    subject: string;
  }
  let { cluster, subject }: Props = $props();

  let versions = $state<SchemaVersion[]>([]);
  let loading = $state(true);
  /** Failed load — inline with Retry (it used to toast AND claim "No versions found."). */
  let loadError = $state<string | null>(null);
  // The two versions selected for the diff.
  let diffA = $state<SchemaVersionDetail | null>(null);
  let diffB = $state<SchemaVersionDetail | null>(null);
  let showDiff = $state(false);
  const cache = new SchemaVersionCache();
  let generation = 0;
  let listController: AbortController | null = null;
  const detailControllers: Record<'A' | 'B', AbortController | null> = { A:null, B:null };
  let detailErrors = $state<Record<'A' | 'B', string | null>>({ A:null, B:null });
  const detailError = $derived(detailErrors.A ?? detailErrors.B);
  let selectedA = $state<number | null>(null);
  let selectedB = $state<number | null>(null);
  const versionBase = () => `/brokers/clusters/${cluster.id}/schema-registry/subjects/${encodeURIComponent(subject)}/versions`;
  function cancelLoads() {
    listController?.abort(); detailControllers.A?.abort(); detailControllers.B?.abort();
  }
  async function selectVersion(side: 'A' | 'B', version: number, reveal = true): Promise<void> {
    detailControllers[side]?.abort();
    const controller = new AbortController(); detailControllers[side] = controller;
    const gen = generation, base = versionBase();
    if (side === 'A') { selectedA = version; diffA = null; } else { selectedB = version; diffB = null; }
    detailErrors[side] = null;
    try {
      const detail = cache.get(base, version) ?? await api.get<SchemaVersionDetail>(`${base}/${version}`, controller.signal);
      if (controller.signal.aborted || gen !== generation) return;
      cache.set(base, detail);
      if (side === 'A') diffA = detail; else diffB = detail;
      if (reveal) showDiff = true;
    } catch (e) {
      if (!controller.signal.aborted && gen === generation) detailErrors[side] = loadErrorText(e);
    }
  }
  function retryDetails() {
    if (selectedA !== null) void selectVersion('A', selectedA);
    if (selectedB !== null) void selectVersion('B', selectedB);
  }

  // Compatibility check against latest.
  let compatSchema = $state('');
  let compatLoading = $state(false);
  let compatResult = $state<CompatCheckResp | null>(null);

  $effect(() => {
    void cluster.id;
    void subject;
    loadVersions();
    return cancelLoads;
  });

  function loadVersions(): void {
    cancelLoads();
    const gen = ++generation;
    const controller = new AbortController(); listController = controller;
    loading = true;
    loadError = null;
    detailErrors = { A:null, B:null };
    versions = [];
    diffA = null; diffB = null;
    selectedA = null; selectedB = null;
    showDiff = false;
    compatResult = null;
    api.get<SchemaVersion[]>(versionBase(), controller.signal)
      .then((v) => {
        if (gen !== generation || controller.signal.aborted) return;
        versions = v;
        if (v.length >= 1) void selectVersion('B', v[v.length - 1].version, false);
        if (v.length >= 2) void selectVersion('A', v[v.length - 2].version, false);
      })
      .catch((e) => { if (gen === generation && !controller.signal.aborted) loadError = loadErrorText(e); })
      .finally(() => { if (gen === generation && !controller.signal.aborted) loading = false; });
  }

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
      toastError('Couldn’t check compatibility', e);
    } finally {
      compatLoading = false;
    }
  }
</script>

<div class="svp">
  {#if loading}
    <p class="muted pad">Loading versions…</p>
  {:else if loadError}
    <LoadState what="schema versions" error={loadError} empty onretry={loadVersions} />
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
            <span class="vid muted">{diffA?.version === v.version ? `#${diffA.id}` : diffB?.version === v.version ? `#${diffB.id}` : ''}</span>
            <span class="vtype muted">{diffA?.version === v.version ? diffA.schema_type : diffB?.version === v.version ? diffB.schema_type : ''}</span>
            <div class="vbtns">
              <button
                class="btn small"
                class:active={selectedA === v.version}
                onclick={() => selectVersion('A', v.version)}
                title="Set as 'before' side of diff"
              >A</button>
              <button
                class="btn small"
                class:active={selectedB === v.version}
                onclick={() => selectVersion('B', v.version)}
                title="Set as 'after' side of diff"
              >B</button>
            </div>
          </div>
        {/each}
      </div>
    </section>

    {#if detailError}
      <LoadState what="selected schema versions" error={detailError} empty onretry={retryDetails} />
    {:else if (selectedA !== null && !diffA) || (selectedB !== null && !diffB)}
      <p class="muted pad">Loading selected versions…</p>
    {/if}

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
        aria-label="Candidate schema"
        dir="ltr"
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
              {#each compatResult.messages as m, i (i)}<li>{m}</li>{/each}
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-m);
  }
  .vnum {
    font-family: var(--font-mono);
    min-width: 40px;
    font-weight: 600;
  }
  .vid {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    min-width: 38px;
  }
  .vtype {
    font-size: var(--fs-xs);
    flex: 1;
  }
  .vbtns {
    display: flex;
    gap: 4px;
  }
  .btn.active {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    border-color: var(--accent);
    color: var(--accent-text);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
  }
  .compat-result.ok {
    color: var(--success);
  }
  .compat-result.fail {
    color: var(--danger);
  }
  .compat-msgs {
    margin: 4px 0 0;
    padding-inline-start: 18px;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  .pad {
    padding: 12px;
  }
</style>
