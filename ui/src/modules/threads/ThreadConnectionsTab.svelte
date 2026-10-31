<script lang="ts">
  import { api, isAbortError } from '../../lib/api/client';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { plural } from '../../lib/plural';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { sidePane } from '../../lib/stores/sidePane.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import type { Connection } from '../../lib/api/types';
  import { threads } from './threadsStore.svelte';

  interface Props {
    workspaceId: string | null;
    focusedSessionId: string | null;
  }
  let { workspaceId, focusedSessionId }: Props = $props();

  let connections = $state<Connection[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let retryCount = $state(0);
  let scope = $state<'focused' | 'all'>('all');
  let expandedId = $state<string | null>(null);

  $effect(() => {
    const wid = workspaceId;
    void retryCount;
    if (!wid) {
      connections = [];
      return;
    }
    const ctl = new AbortController();
    loading = true;
    error = null;
    void api
      .get<Connection[]>(`/workspaces/${wid}/connections`, ctl.signal)
      .then((rows) => {
        if (!ctl.signal.aborted) connections = rows;
      })
      .catch((e) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        error = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) loading = false;
      });
    return () => ctl.abort();
  });

  const visibleActivities = $derived(
    scope === 'focused' && focusedSessionId
      ? threads.mcpActivities.filter((a) => a.session_id === focusedSessionId)
      : threads.mcpActivities,
  );

  function envTone(env: Connection['environment']): 'warn' | 'info' | 'neutral' {
    if (env === 'prod') return 'warn';
    if (env === 'staging') return 'info';
    return 'neutral';
  }
</script>

<div class="tco-wrap" data-testid="inspector-connections-tab">
  <div class="tco-launch-bar">
    <button
      type="button"
      class="btn small"
      data-testid="connections-open-beside"
      onclick={() => sidePane.open('database')}
    >
      <Icon name="columns" size={12} />
      Open side-by-side
    </button>
    <button
      type="button"
      class="btn small ghost"
      data-testid="connections-open-tab"
      onclick={() => router.go('database')}
    >
      <Icon name="external" size={12} />
      Open Connections tab
    </button>
  </div>

  <section class="tco-section" aria-label="Workspace connections">
    <div class="tco-sec-head">
      <span class="tco-sec-title">Workspace Connections</span>
      <span class="tco-count">{connections.length}</span>
    </div>
    <LoadState
      what="connections"
      variant="compact"
      {loading}
      {error}
      empty={connections.length === 0}
      onretry={() => retryCount++}
    >
      {#snippet emptyView()}
        <p class="tco-empty-note">No saved DB or SSH connections in this workspace yet.</p>
      {/snippet}
      <div class="tco-conn-list">
        {#each connections as conn (conn.id)}
          <div class="tco-conn-row">
            <div class="tco-conn-info">
              <Icon name={conn.kind === 'ssh' ? 'terminal' : 'db'} size={13} />
              <span class="tco-conn-name ellipsis" dir="auto">{conn.name}</span>
              <Badge tone="neutral" variant="outline" label={conn.kind} />
              {#if conn.environment !== 'dev'}
                <Badge tone={envTone(conn.environment)} label={conn.environment} />
              {/if}
            </div>
            <button
              type="button"
              class="icon-btn"
              aria-label="Open {conn.name} side-by-side"
              title="Open in Database Explorer beside Threads"
              onclick={() => sidePane.open(`database/${conn.id}`)}
            >
              <Icon name="columns" size={12} />
            </button>
          </div>
        {/each}
      </div>
    </LoadState>
  </section>

  <section class="tco-section tco-stream" aria-label="Live MCP query stream">
    <div class="tco-sec-head">
      <div class="tco-stream-title">
        <span class="tco-sec-title">Live MCP Query Stream</span>
        <Badge tone="info" variant="outline" label={`${visibleActivities.length}`} />
      </div>
      <div class="segmented" role="group" aria-label="MCP activity filter">
        <button
          type="button"
          class:active={scope === 'focused'}
          aria-pressed={scope === 'focused'}
          onkeydown={onTabKey}
          onclick={() => (scope = 'focused')}
        >
          Focused
        </button>
        <button
          type="button"
          class:active={scope === 'all'}
          aria-pressed={scope === 'all'}
          onkeydown={onTabKey}
          onclick={() => (scope = 'all')}
        >
          All
        </button>
      </div>
    </div>

    {#if visibleActivities.length === 0}
      <EmptyState
        variant="panel"
        icon="db"
        title="Listening for MCP database queries"
        body="When an agent calls otto.db_query or inspects schemas via MCP, live SQL, row counts, durations and result previews stream here."
      />
    {:else}
      <div class="tco-act-list" data-testid="mcp-activity-list">
        {#each visibleActivities as act (act.activity_id)}
          {@const open = expandedId === act.activity_id}
          <article class="tco-act-card">
            <div class="tco-act-top">
              <Badge
                tone={act.status === 'error' ? 'bad' : act.status === 'running' ? 'warn' : 'ok'}
                live={act.status === 'running'}
                label={act.tool}
              />
              {#if act.session_title}
                <span class="tco-act-who ellipsis" dir="auto">{act.session_title}</span>
              {/if}
              <span class="tco-act-meta mono">
                {#if act.row_count != null}{plural(act.row_count, 'row')}{/if}
                {#if act.duration_ms != null} · {act.duration_ms}ms{/if}
                · {rel(act.ts)}
              </span>
            </div>

            {#if act.sql}
              <pre class="tco-sql mono" dir="ltr">{act.sql}</pre>
            {/if}

            {#if act.error}
              <p class="tco-act-err">{act.error}</p>
            {/if}

            {#if act.columns && act.columns.length > 0 && act.rows_preview && act.rows_preview.length > 0}
              <button
                type="button"
                class="btn small ghost tco-preview-toggle"
                aria-expanded={open}
                onclick={() => (expandedId = open ? null : act.activity_id)}
              >
                <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
                {open ? 'Hide preview' : `Preview ${plural(act.rows_preview.length, 'row')}`}
              </button>
              {#if open}
                <div class="tco-table-wrap">
                  <table class="tco-table mono" dir="ltr">
                    <thead>
                      <tr>
                        {#each act.columns as col (col)}
                          <th>{col}</th>
                        {/each}
                      </tr>
                    </thead>
                    <tbody>
                      {#each act.rows_preview.slice(0, 10) as row, rIdx (rIdx)}
                        <tr>
                          {#each act.columns as _, cIdx (cIdx)}
                            <td>{row[cIdx] == null ? 'NULL' : String(row[cIdx])}</td>
                          {/each}
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              {/if}
            {/if}
          </article>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .tco-wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
  }
  .tco-launch-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 10px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .tco-section {
    display: flex;
    flex-direction: column;
    padding: 8px 10px;
    gap: 6px;
    border-block-end: 1px solid var(--border);
  }
  .tco-stream {
    flex: 1;
    border-block-end: 0;
  }
  .tco-sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .tco-stream-title {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .tco-sec-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .tco-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .tco-empty-note {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tco-conn-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tco-conn-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .tco-conn-info {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1;
  }
  .tco-conn-name {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text);
  }
  .tco-act-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tco-act-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .tco-act-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .tco-act-who {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text);
    min-width: 0;
  }
  .tco-act-meta {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .tco-sql {
    margin: 0;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .tco-act-err {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .tco-preview-toggle {
    align-self: flex-start;
  }
  .tco-table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .tco-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
  }
  .tco-table th,
  .tco-table td {
    padding: 4px 8px;
    border-block-end: 1px solid var(--border);
    text-align: start;
    white-space: nowrap;
  }
  .tco-table th {
    background: var(--surface-2);
    font-weight: 600;
  }
</style>
