<script lang="ts">
  import { api } from '../../lib/api/client';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { loadErrorText } from '../../lib/loadError';
  import { TableWindow } from '../../lib/tableWindow.svelte';
  import type { BrokerCluster, SchemaSubject } from '../../lib/api/types';
  import SchemaVersionsPanel from './SchemaVersionsPanel.svelte';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  // Raw: a registry can hold thousands of subjects, each carrying its schema.
  let subjects = $state.raw<SchemaSubject[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let selected = $state.raw<SchemaSubject | null>(null);
  // Toggle to show version history / compat panel for the selected subject.
  let showVersions = $state(false);

  // Window the subject list (SC-03): thousands of subjects mount only the
  // visible slice between two spacers.
  const tw = new TableWindow();
  const win = $derived(tw.range(subjects.length));
  let listEl = $state<HTMLDivElement>();
  $effect(() => {
    void win;
    tw.measure(listEl, '.srow');
  });
  // ⌘F over every subject, not just the mounted slice.
  $effect(() =>
    tw.findRows(() => listEl, () => subjects, (s) => `${s.subject}\nv${s.version} · ${s.schema_type} · #${s.id}`, '.srow'),
  );

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
        tw.reset(listEl);
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
    <div class="pad"><Skeleton rows={6} label="schema subjects" /></div>
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
    <div
      class="list"
      class:windowed={tw.active(subjects.length)}
      bind:this={listEl}
      bind:clientHeight={tw.viewH}
      onscroll={tw.onscroll}
    >
      {#if win.top}<div class="tw-spacer" aria-hidden="true" style="height:{win.top}px"></div>{/if}
      {#each subjects.slice(win.start, win.end) as s (s.subject)}
        <button
          class="srow"
          class:sel={selected?.subject === s.subject}
          onclick={() => { selected = s; showVersions = false; }}
        >
          <span class="sn" title={s.subject}>{s.subject}</span>
          <span class="muted small">v{s.version} · {s.schema_type} · #{s.id}</span>
        </button>
      {/each}
      {#if win.bottom}<div class="tw-spacer" aria-hidden="true" style="height:{win.bottom}px"></div>{/if}
      {#if subjects.length === 0}<p class="muted pad">No subjects in this registry.</p>{/if}
    </div>
    <div class="view">
      {#if selected}
        <div class="view-head">
          <span class="sn-big">{selected.subject}</span>
          <div class="view-tabs" role="tablist" aria-label="Subject views" tabindex="-1" onkeydown={onTabKey}>
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
    background: var(--hover);
  }
  .srow.sel {
    background: var(--accent-soft);
    border-inline-start-color: var(--accent);
  }
  .sn {
    font-family: var(--font-mono);
    font-size: var(--fs-m);
    word-break: break-all;
  }
  /* Windowed rows must be uniform: one-line names (full name in the title
     and the detail header). */
  .list.windowed .sn {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    word-break: normal;
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
