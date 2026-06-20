<script lang="ts">
  import { api } from '../../lib/api/client';
  import { ApiError } from '../../lib/api/client';
  import type { BrokerCluster, SchemaSubject } from '../../lib/api/types';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  let subjects = $state<SchemaSubject[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let selected = $state<SchemaSubject | null>(null);

  function prettySchema(s: SchemaSubject): string {
    try {
      return JSON.stringify(JSON.parse(s.schema), null, 2);
    } catch {
      return s.schema;
    }
  }

  $effect(() => {
    void cluster.id;
    loading = true;
    error = null;
    selected = null;
    api
      .get<SchemaSubject[]>(`/brokers/clusters/${cluster.id}/schema-registry/subjects`)
      .then((s) => {
        subjects = s;
        if (s.length > 0) selected = s[0];
      })
      .catch((e) => {
        error = e instanceof ApiError ? e.message : String(e);
      })
      .finally(() => (loading = false));
  });
</script>

<div class="schema">
  {#if loading}
    <p class="muted pad">Loading subjects…</p>
  {:else if error}
    <div class="empty">
      <p>{error}</p>
      <p class="muted small">
        Configure a Schema Registry URL on the cluster to browse Avro/Protobuf/JSON schemas.
      </p>
    </div>
  {:else}
    <div class="list">
      {#each subjects as s (s.subject)}
        <button class="srow" class:sel={selected?.subject === s.subject} onclick={() => (selected = s)}>
          <span class="sn">{s.subject}</span>
          <span class="muted small">v{s.version} · {s.schema_type} · #{s.id}</span>
        </button>
      {/each}
      {#if subjects.length === 0}<p class="muted pad">No subjects registered.</p>{/if}
    </div>
    <div class="view">
      {#if selected}
        <pre class="payload">{prettySchema(selected)}</pre>
      {/if}
    </div>
  {/if}
</div>

<style>
  .schema {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .list {
    width: 300px;
    border-right: 1px solid var(--border);
    overflow: auto;
  }
  .srow {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    cursor: pointer;
    border-left: 2px solid transparent;
  }
  .srow:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .srow.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-left-color: var(--accent);
  }
  .sn {
    font-family: var(--font-mono);
    font-size: 12.5px;
    word-break: break-all;
  }
  .view {
    flex: 1;
    overflow: auto;
    padding: 14px;
  }
  .payload {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .empty {
    padding: 30px;
    text-align: center;
    color: var(--text-dim);
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
