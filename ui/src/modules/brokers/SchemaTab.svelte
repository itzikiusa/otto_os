<script lang="ts">
  import { api, ApiError } from '../../lib/api/client';
  import type { BrokerCluster, SchemaSubject } from '../../lib/api/types';
  import SchemaVersionsPanel from './SchemaVersionsPanel.svelte';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  let subjects = $state<SchemaSubject[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let selected = $state<SchemaSubject | null>(null);
  // Toggle to show version history / compat panel for the selected subject.
  let showVersions = $state(false);

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
    showVersions = false;
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
        <button
          class="srow"
          class:sel={selected?.subject === s.subject}
          onclick={() => { selected = s; showVersions = false; }}
        >
          <span class="sn">{s.subject}</span>
          <span class="muted small">v{s.version} · {s.schema_type} · #{s.id}</span>
        </button>
      {/each}
      {#if subjects.length === 0}<p class="muted pad">No subjects registered.</p>{/if}
    </div>
    <div class="view">
      {#if selected}
        <div class="view-head">
          <span class="sn-big">{selected.subject}</span>
          <div class="view-tabs">
            <button class:on={!showVersions} onclick={() => (showVersions = false)}>Schema</button>
            <button class:on={showVersions} onclick={() => (showVersions = true)}>Versions &amp; Compat</button>
          </div>
        </div>
        {#if showVersions}
          <SchemaVersionsPanel cluster={cluster} subject={selected.subject} />
        {:else}
          <pre class="payload">{prettySchema(selected)}</pre>
        {/if}
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
    border-inline-end: 1px solid var(--border);
    overflow: auto;
  }
  .srow {
    width: 100%;
    text-align: start;
    border: none;
    background: transparent;
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    cursor: pointer;
    border-inline-start: 2px solid transparent;
  }
  .srow:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .srow.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-inline-start-color: var(--accent);
  }
  .sn {
    font-family: var(--font-mono);
    font-size: 12.5px;
    word-break: break-all;
  }
  .view {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .view-head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 14px 0;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .sn-big {
    font-family: var(--font-mono);
    font-size: 13px;
    font-weight: 600;
    word-break: break-all;
  }
  .view-tabs {
    display: flex;
    gap: 2px;
    margin-inline-start: auto;
  }
  .view-tabs button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    padding: 6px 10px;
    cursor: pointer;
    border-bottom: 2px solid transparent;
  }
  .view-tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .payload {
    flex: 1;
    overflow: auto;
    margin: 0;
    padding: 14px;
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

  /* Phone (≤640px): the 300px fixed subject list + viewer can't sit side-by-side
     on a ~375–430px viewport. Stack them and cap the list height so the schema
     viewer stays reachable. */
  @media (max-width: 640px) {
    .schema {
      flex-direction: column;
    }
    .list {
      width: 100%;
      max-height: 35vh;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    .view {
      min-height: 200px;
    }
  }
</style>
