<script lang="ts">
  import { api } from '../../lib/api/client';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
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
  // Phone: the subject list is the page until one is tapped (pushed, "‹ Subjects").
  let phoneOpen = $state(false);
  function pick(s: SchemaSubject): void {
    selected = s;
    showVersions = false;
    phoneOpen = true;
  }

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

<div class="schema-host schema">
  {#if error && !loading}
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
    <ListDetail module="brokers" pane="subjects" name="Subjects" open={phoneOpen && !!selected} onback={() => (phoneOpen = false)} empty={!loading && subjects.length === 0}>
      {#snippet list()}
        <ListPane count={subjects.length} {loading} menu={() => [{ label: 'Refresh', icon: 'refresh', action: load }]} testid="brokers-subjects-list">
          <div
            class="list"
            class:windowed={tw.active(subjects.length)}
            bind:this={listEl}
            bind:clientHeight={tw.viewH}
            onscroll={tw.onscroll}
          >
            {#if win.top}<div class="tw-spacer" aria-hidden="true" style="height:{win.top}px"></div>{/if}
            {#each subjects.slice(win.start, win.end) as s (s.subject)}
              {@const sel = selected?.subject === s.subject}
              <div class="side-row two-line" class:selected={sel}>
                <button type="button" class="side-row-main srow" data-side-row aria-current={sel ? 'true' : undefined} onclick={() => pick(s)}>
                  <span class="side-row-text">
                    <span class="side-row-title sn" title={s.subject}>{s.subject}</span>
                    <span class="side-row-meta">v{s.version} · {s.schema_type} · #{s.id}</span>
                  </span>
                </button>
              </div>
            {/each}
            {#if win.bottom}<div class="tw-spacer" aria-hidden="true" style="height:{win.bottom}px"></div>{/if}
          </div>
        </ListPane>
      {/snippet}
      <div class="view">
        {#if selected}
          <div class="view-head">
            <span class="sn-big">{selected.subject}</span>
            <div class="view-tabs">
              <Tabs
                label="Subject views"
                size="s"
                tabs={[{ id: 'schema', label: 'Schema' }, { id: 'versions', label: 'Versions & Compat' }]}
                value={showVersions ? 'versions' : 'schema'}
                onchange={(id) => (showVersions = id === 'versions')}
              />
            </div>
          </div>
          {#if showVersions}
            <SchemaVersionsPanel cluster={cluster} subject={selected.subject} />
          {:else}
            <pre class="payload" dir="ltr">{prettySchema(selected)}</pre>
          {/if}
        {:else if !loading && subjects.length === 0}
          <EmptyState icon="file" title="No subjects in this registry" body="Subjects appear here once producers register a schema." />
        {/if}
      </div>
    </ListDetail>
  {/if}
</div>

<style>
  .schema-host { display: flex; height: 100%; min-height: 0; min-width: 0; }
  /* The windowed scroller fills the list pane's body. */
  .list {
    block-size: 100%;
    overflow-y: auto;
  }
  .sn {
    font-family: var(--font-mono);
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
    margin-inline-start: auto;
  }
  .view-tabs :global(.otabs) {
    border-block-end: 0;
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
    flex: 1;
    padding: 28px;
    text-align: center;
    color: var(--text-dim);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
</style>
