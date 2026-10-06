<script lang="ts">
  import { plural } from '../../lib/plural';
  // The governed-call audit ledger — every invoke (UI tester, gateway, inward
  // read-only server, outward otto.* tools) writes one redacted row here. The
  // table is filterable by server, tool, and decision; rows show the decision,
  // ok/error, latency, bytes, and time.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { sentenceCase } from '../../lib/labels';
  import { loadErrorText } from '../../lib/loadError';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { McpCallLogRow, McpServerDetail } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';
  import StatsTab from './StatsTab.svelte';

  interface Props {
    servers: McpServerDetail[];
  }
  let { servers }: Props = $props();

  let rows = $state<McpCallLogRow[]>([]);
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let fServer = $state('');
  let fTool = $state('');
  let fDecision = $state('');
  let view = $state<'log' | 'stats'>('log');
  let filtersOpen = $state(false);

  /** Rows per request. The server pages the ledger BEFORE its per-row
   *  visibility check, so `offset` counts ledger rows, not shown ones. */
  const PAGE = 200;
  let nextOffset = 0;
  let hasMore = $state(false);
  let loadingMore = $state(false);
  let moreError = $state<string | null>(null);
  /** Bumped per (re)load so a stale "load more" never appends to new filters. */
  let generation = 0;

  function query(offset: number) {
    return {
      server_id: fServer || undefined,
      tool: fTool.trim() || undefined,
      decision: fDecision || undefined,
      limit: PAGE,
      offset,
    };
  }

  async function load(): Promise<void> {
    const gen = ++generation;
    loading = true;
    moreError = null;
    try {
      const page = await mcpCpApi.cpAudit(query(0));
      if (gen !== generation) return;
      rows = page;
      nextOffset = PAGE;
      // The server reads PAGE rows and THEN drops the ones this caller can't
      // see, so a short page doesn't mean the end (non-admins lost "Load more"
      // and older rows). Only an empty page proves there's nothing older.
      hasMore = page.length > 0;
      loadError = null;
    } catch (e) {
      if (gen === generation) loadError = loadErrorText(e);
    } finally {
      if (gen === generation) loading = false;
    }
  }

  /** The next page, appended (rows already shown — e.g. shifted by new calls
   *  landing on top — are skipped by id). */
  async function loadMore(): Promise<void> {
    if (loadingMore) return;
    const gen = generation;
    loadingMore = true;
    try {
      const page = await mcpCpApi.cpAudit(query(nextOffset));
      if (gen !== generation) return;
      const seen = new Set(rows.map((r) => r.id));
      rows = [...rows, ...page.filter((r) => !seen.has(r.id))];
      nextOffset += PAGE;
      hasMore = page.length > 0;
      moreError = null;
    } catch (e) {
      if (gen === generation) moreError = loadErrorText(e);
    } finally {
      if (gen === generation) loadingMore = false;
    }
  }

  $effect(() => {
    // Server + decision selects apply immediately; the tool box applies on submit.
    void fServer;
    void fDecision;
    void load();
  });

  function fmtBytes(b: number | null): string {
    if (b == null) return '—';
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / 1024 / 1024).toFixed(1)} MB`;
  }
  /** Tool tooltip naming the agent session behind the call, when known. */
  function callTitle(r: McpCallLogRow): string {
    const sid = r.caller_session_id;
    if (!sid) return r.tool;
    const title = ws.getSession(sid)?.title;
    return `${r.tool} — from session ${title ?? `${sid.slice(0, 8)}…`}`;
  }
</script>

<div class="audit" data-testid="mcp-audit">
  <div class="head">
    <h2>Audit</h2>
    <span class="grow"></span>
    {#if view === 'log'}
      <span class="count">{hasMore ? `${rows.length}+ rows` : plural(rows.length, 'row')}</span>
    {/if}
    <div class="views" role="group" aria-label="Audit view">
      <button
        class:on={view === 'log'}
        aria-pressed={view === 'log'}
        onclick={() => (view = 'log')}
      >
        Log
      </button>
      <button
        class:on={view === 'stats'}
        data-testid="mcp-audit-bytool"
        aria-pressed={view === 'stats'}
        onclick={() => (view = 'stats')}
      >
        By tool
      </button>
    </div>
  </div>

  {#if view === 'stats'}
    <StatsTab />
  {:else}
    <div class="filters">
      <button
        class="filter-toggle"
        aria-expanded={filtersOpen}
        onclick={() => (filtersOpen = !filtersOpen)}
      >
        Filters
        <Icon name={filtersOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
      {#if filtersOpen}
        <div class="bar">
          <select aria-label="Server" bind:value={fServer}>
            <option value="">All servers</option>
            {#each servers as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
          </select>
          <input dir="ltr" aria-label="Filter by tool"
            bind:value={fTool}
            placeholder="Filter tool…"
            class="mono"
            onkeydown={(e) => e.key === 'Enter' && void load()}
          />
          <select aria-label="Decision" bind:value={fDecision}>
            <option value="">All decisions</option>
            <option value="allowed">{sentenceCase('allowed')}</option>
            <option value="approved">{sentenceCase('approved')}</option>
            <option value="auto_approved">{sentenceCase('auto_approved')}</option>
            <option value="denied">{sentenceCase('denied')}</option>
            <option value="dry_run">{sentenceCase('dry_run')}</option>
            <option value="pending_approval">{sentenceCase('pending_approval')}</option>
            <option value="error">{sentenceCase('error')}</option>
          </select>
          <button class="btn small" onclick={() => void load()}>Apply</button>
        </div>
      {/if}
    </div>
    <LoadState what="the audit log" {loading} error={loadError} empty={rows.length === 0} onretry={() => void load()}>
      {#snippet emptyView()}
        <EmptyState icon="note" title="No calls yet" body="Calls made by external MCP clients and by sessions through the gateway appear here." />
      {/snippet}
      <div class="grid">
        <div class="thead">
          <span>Time</span>
          <span>Server</span>
          <span>Tool</span>
          <span>Decision</span>
          <span>Direction</span>
          <span class="num">Result</span>
          <span class="num">Latency</span>
          <span class="num">Bytes</span>
        </div>
        {#each rows as r (r.id)}
          <!-- Each cell carries its column name (`.cl`): visually hidden while the
               header row shows, inline once rows stack on a phone. -->
          <div class="arow">
            <span class="cell when"><span class="cl">Time</span>{new Date(r.created_at).toLocaleString()}</span>
            <span class="cell"><span class="cl">Server</span><span class="trunc" title={r.server_name ?? undefined}>{r.server_name ?? '—'}</span></span>
            <span class="cell mono"><span class="cl">Tool</span><span class="trunc" title={callTitle(r)}>{r.tool}</span>{#if r.dry_run}<span class="dry">dry run</span>{/if}</span>
            <span class="cell" title={r.decision_reason ?? undefined}><span class="cl">Decision</span><McpPill kind="decision" value={r.decision} small /></span>
            <span class="cell"><span class="cl">Direction</span><McpPill kind="direction" value={r.direction} small /></span>
            <span class="cell num">
              <span class="cl">Result</span>
              <!-- Words, not just a glyph: the icon is decorative. -->
              {#if r.ok}<span class="ok" title="Succeeded"><Icon name="check" size={13} /><span class="cl">OK</span></span>{:else}<span class="bad" title={r.error ?? 'Failed'}><Icon name="x" size={13} /><span class="cl">Failed</span></span>{/if}
            </span>
            <span class="cell num"><span class="cl">Latency</span>{r.latency_ms != null ? `${r.latency_ms}ms` : '—'}</span>
            <span class="cell num"><span class="cl">Bytes</span>{fmtBytes(r.bytes)}</span>
            <details class="call-details">
              <summary>Call details</summary>
              <dl>
                <dt>Server</dt><dd>{r.server_name ?? '—'}</dd>
                <dt>Tool</dt><dd>{r.tool}</dd>
                {#if r.decision_reason}<dt>Decision reason</dt><dd>{r.decision_reason}</dd>{/if}
                {#if !r.ok}<dt>Error</dt><dd>{r.error ?? 'Failed'}</dd>{/if}
              </dl>
            </details>
          </div>
        {/each}
      </div>
      {#if hasMore || moreError}
        <div class="more">
          {#if moreError}<span class="bad-text" role="alert">{moreError}</span>{/if}
          <button
            class="btn small"
            data-testid="mcp-audit-more"
            disabled={loadingMore}
            onclick={() => void loadMore()}
          >
            {loadingMore ? 'Loading more rows…' : moreError ? 'Retry' : 'Load more'}
          </button>
        </div>
      {/if}
    </LoadState>
  {/if}
</div>

<style>
  .audit {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .views {
    display: inline-flex;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
  }
  .views button {
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    padding: 4px 8px;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .views button.on {
    background: var(--surface);
    color: var(--text);
  }
  .filters {
    border-bottom: 1px solid var(--border);
  }
  .filter-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    padding: 8px 14px;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  .count {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  select,
  input {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 6px 8px;
    font-size: var(--fs-m);
  }
  .grid {
    overflow: auto;
  }
  .thead,
  .arow {
    display: grid;
    grid-template-columns: 170px minmax(110px, 1fr) minmax(140px, 1.4fr) 130px 80px 64px 80px 80px;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
  }
  .thead {
    position: sticky;
    top: 0;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    z-index: 1;
  }
  .arow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    font-size: var(--fs-m);
  }
  .arow:hover {
    background: var(--hover);
  }
  .call-details {
    grid-column: 1 / -1;
    min-inline-size: 0;
  }
  .call-details summary {
    cursor: pointer;
    min-block-size: 32px;
    align-content: center;
    inline-size: fit-content;
  }
  .call-details dl {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 4px;
    margin: 8px 0;
  }
  .call-details dt { font-weight: 500; }
  .call-details dd {
    margin: 0 0 8px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    min-inline-size: 0;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* `.cell` is a flex box, so its own text-overflow never reaches a bare text
     node — long server/tool names need a real block child to get an ellipsis. */
  .trunc {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    justify-content: flex-end;
    text-align: end;
  }
  .when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .dry {
    margin-inline-start: 4px;
    flex: none;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    color: var(--info);
  }
  .bad {
    color: var(--danger);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .ok {
    color: var(--success);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .cl {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .more {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 12px 14px;
  }
  .bad-text {
    color: var(--danger);
    font-size: var(--fs-s);
  }

  @media (max-width: 640px) {
    .thead {
      display: none;
    }
    .arow {
      grid-template-columns: 1fr 1fr;
      gap: 4px 8px;
    }
    .cell.num {
      justify-content: flex-start;
    }
    .cl {
      position: static;
      inline-size: auto;
      block-size: auto;
      overflow: visible;
      clip-path: none;
      flex: none;
      font-size: var(--fs-xs);
      color: var(--text-dim);
    }
  }
</style>
