<script lang="ts">
  import { api } from '../../lib/api/client';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
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

  function onViewKeydown(event: KeyboardEvent) {
    const tabs = Array.from(event.currentTarget instanceof HTMLElement ? event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]') : []);
    const current = tabs.indexOf(event.target as HTMLButtonElement);
    if (current < 0) return;
    let next: number;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = tabs.length - 1;
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      const rtl = getComputedStyle(event.currentTarget as HTMLElement).direction === 'rtl';
      const step = (event.key === 'ArrowRight' ? 1 : -1) * (rtl ? -1 : 1);
      next = (current + step + tabs.length) % tabs.length;
    } else return;
    event.preventDefault();
    showVersions = next === 1;
    tabs[next]?.focus();
  }

  function prettySchema(s: SchemaSubject): string {
    try {
      return JSON.stringify(JSON.parse(s.schema), null, 2);
    } catch {
      return s.schema;
    }
  }

  function load(): void {
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
        error = loadErrorText(e);
      })
      .finally(() => (loading = false));
  }

  $effect(() => {
    void cluster.id;
    load();
  });
</script>

<div class="schema-host">
<div class="schema">
  {#if loading}
    <p class="muted pad">Loading subjects…</p>
  {:else if error}
    <div class="empty">
      {#if cluster.schema_registry_url}
        <!-- A registry IS configured: this is a real failure — say so, with Retry. -->
        <LoadState what="schema subjects" {loading} {error} empty onretry={load} />
      {:else}
        <p>{error}</p>
        <p class="muted small">
          Configure a Schema Registry URL on the cluster to browse Avro/Protobuf/JSON schemas.
        </p>
      {/if}
    </div>
  {:else}
    <div class="list">
      {#each subjects as s (s.subject)}
        <button
          class="srow"
          class:sel={selected?.subject === s.subject}
          onclick={() => { selected = s; showVersions = false; }}
        >
          <span class="sn" title={s.subject}>{s.subject}</span>
          <span class="muted small">v{s.version} · {s.schema_type} · #{s.id}</span>
        </button>
      {/each}
      {#if subjects.length === 0}<p class="muted pad">No subjects registered.</p>{/if}
    </div>
    <div class="view">
      {#if selected}
        <div class="view-head">
          <span class="sn-big">{selected.subject}</span>
          <div class="view-tabs" role="tablist" aria-label="Subject views" tabindex="-1" onkeydown={onViewKeydown}>
            <button class:on={!showVersions} role="tab" aria-selected={!showVersions} tabindex={showVersions ? -1 : 0} onclick={() => (showVersions = false)}>Schema</button>
            <button class:on={showVersions} role="tab" aria-selected={showVersions} tabindex={showVersions ? 0 : -1} onclick={() => (showVersions = true)}>Versions &amp; Compat</button>
          </div>
        </div>
        {#if showVersions}
          <SchemaVersionsPanel cluster={cluster} subject={selected.subject} />
        {:else}
          <pre class="payload" dir="ltr">{prettySchema(selected)}</pre>
        {/if}
      {/if}
    </div>
  {/if}
</div>

</div>
<style>
  .schema-host { container-type: inline-size; height: 100%; min-height: 0; min-width: 0; }
  .schema {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .list {
    width: 300px;
    max-width: 40%;
    flex-shrink: 0;
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
    font-size: var(--fs-m);
    word-break: break-all;
  }
  .view {
    min-width: 0;
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
    font-size: var(--fs-m);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
  }
  .pad {
    padding: 12px;
  }

  /* Stack by available pane width, including tablets and embedded panels. */
  @container (max-width: 760px) {
    .schema {
      flex-direction: column;
    }
    .list {
      width: 100%;
      max-width: none;
      max-height: 25vh;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    .view {
      min-height: 200px;
    }
  }
</style>
