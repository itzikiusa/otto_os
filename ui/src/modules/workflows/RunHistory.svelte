<script lang="ts">
  // The sidebar's run history: every run of the workspace (or only the open
  // workflow), newest first, one click from the page, with one open-text
  // search over workflow, trigger message, prompt/goals, repo, branch, PR,
  // step names and errors (server-side: GET /workspaces/{wid}/workflow-runs).
  // Live: run events refresh the first page (rate-capped).
  import Icon from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { api } from '../../lib/api/client';
  import { liveQuery, type LiveEvent } from '../../lib/live';
  import { loadErrorText } from '../../lib/loadError';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { runStatus } from '../../lib/status';
  import { fmtStepMs } from './runProgress';
  import type { RunListItem, RunListPage } from '../../lib/api/types';

  interface Props {
    /** The workflow open in the page (enables the "This workflow" filter). */
    workflowId: string | null;
    workflowName?: string;
    activeRunId: string | null;
    onopen: (workflowId: string, runId: string) => void;
  }
  let { workflowId, workflowName = '', activeRunId, onopen }: Props = $props();

  const OPEN_KEY = 'otto.wf.runHistory.open';
  function readOpen(): boolean {
    try { return localStorage.getItem(OPEN_KEY) !== '0'; } catch { return true; }
  }
  let open = $state(readOpen());
  function toggle(): void {
    open = !open;
    try { localStorage.setItem(OPEN_KEY, open ? '1' : '0'); } catch { /* per-viewer nicety only */ }
  }

  let query = $state('');
  let q = $state('');
  let onlyThis = $state(false);
  let onlyFailed = $state(false);
  let items = $state<RunListItem[]>([]);
  let next = $state<string | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let loadingMore = $state(false);
  let reload = $state(0);

  // Debounce typing; the request itself is bounded and paginated server-side.
  $effect(() => {
    const v = query.trim();
    const t = setTimeout(() => (q = v), 250);
    return () => clearTimeout(t);
  });

  function params(cursor?: string): string {
    const p = new URLSearchParams({ limit: '30' });
    if (q) p.set('q', q);
    if (onlyThis && workflowId) p.set('workflow_id', workflowId);
    if (onlyFailed) p.set('status', 'error');
    if (cursor) p.set('cursor', cursor);
    return p.toString();
  }

  // One live query per (workspace, filters); a newer one supersedes the old.
  $effect(() => {
    const wid = ws.currentId;
    const key = params();
    void reload;
    if (!wid || !open) return;
    loading = true;
    error = null;
    const poller = liveQuery({
      on: ['workflow_run_updated'],
      match: (ev: LiveEvent) => ev.workspace_id === wid,
      fallbackMs: 30_000,
      safetyMs: 120_000,
      minIntervalMs: 5_000,
      run: async (signal) => {
        try {
          const page = await api.bg.get<RunListPage>(`/workspaces/${wid}/workflow-runs?${key}`, signal);
          if (signal.aborted) return;
          items = page.items;
          next = page.next_cursor ?? null;
          error = null;
        } catch (e) {
          if (!signal.aborted) error = loadErrorText(e);
          throw e;
        } finally {
          if (!signal.aborted) loading = false;
        }
      },
    });
    return () => poller.stop();
  });

  async function more(): Promise<void> {
    const wid = ws.currentId;
    if (!wid || !next || loadingMore) return;
    loadingMore = true;
    const key = params(next);
    try {
      const page = await api.get<RunListPage>(`/workspaces/${wid}/workflow-runs?${key}`);
      items = [...items, ...page.items.filter((i) => !items.some((x) => x.id === i.id))];
      next = page.next_cursor ?? null;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loadingMore = false;
    }
  }

  const TRIGGER: Record<string, string> = { manual: 'Manual', chat: 'Slack', schedule: 'Schedule', event: 'Event', webhook: 'Webhook' };
  function triggerLabel(t: string): string {
    return TRIGGER[t] ?? t.charAt(0).toUpperCase() + t.slice(1);
  }
  function meta(r: RunListItem): string {
    const parts = [
      r.repo,
      r.branch,
      r.pr ? (/^\d+$/.test(r.pr) ? `PR #${r.pr}` : `PR ${r.pr}`) : null,
      r.status === 'running' && r.current_step ? r.current_step : r.duration_ms != null ? fmtStepMs(r.duration_ms) : null,
    ];
    return parts.filter(Boolean).join(' · ');
  }
  function rowTitle(r: RunListItem): string {
    return [r.workflow_name, r.message, r.error ? `Error: ${r.error}` : null].filter(Boolean).join('\n');
  }
</script>

<section class="hist" data-testid="run-history">
  <button class="hist-h" onclick={toggle} aria-expanded={open}>
    <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
    Runs
  </button>
  {#if open}
    <div class="tools">
      <label class="search">
        <Icon name="search" size={12} />
        <input
          dir="auto"
          type="search"
          bind:value={query}
          placeholder="Search runs"
          aria-label="Search runs by workflow, message, repo, branch, PR, step or error"
          title="Matches workflow, trigger message, prompt and goals, repo, branch, PR, step names and errors"
          data-testid="run-history-search"
        />
      </label>
      <div class="chips">
        {#if workflowId}
          <button class="hist-filter" class:on={onlyThis} aria-pressed={onlyThis} onclick={() => (onlyThis = !onlyThis)} title={workflowName ? `Only runs of ${workflowName}` : 'Only runs of this workflow'}>This workflow</button>
        {/if}
        <button class="hist-filter" class:on={onlyFailed} aria-pressed={onlyFailed} onclick={() => (onlyFailed = !onlyFailed)}>Failed</button>
      </div>
    </div>
    <div class="rows">
      <LoadState what="runs" variant="compact" {loading} {error} empty={items.length === 0} onretry={() => reload++}>
        {#snippet emptyView()}<div class="empty">{q ? `No runs match “${q}”` : 'No runs yet'}</div>{/snippet}
        {#each items as r (r.id)}
          <button class="hist-row" class:active={activeRunId === r.id} data-testid="run-history-item" title={rowTitle(r)} onclick={() => onopen(r.workflow_id, r.id)}>
            <span class="dot {runStatus(r.status).key}" aria-hidden="true"></span>
            <span class="sr-only">{runStatus(r.status).label}:</span>
            <span class="body">
              <span class="line">
                <span class="name">{r.workflow_name}</span>
                <span class="trig">{triggerLabel(r.trigger)}</span>
                <span class="when"><RelTime iso={r.started_at} fallback="" /></span>
              </span>
              {#if meta(r) || r.message}
                <span class="sub">{meta(r) || r.message}</span>
              {/if}
            </span>
          </button>
        {/each}
        {#if next}
          <button class="more" disabled={loadingMore} onclick={() => void more()}>{loadingMore ? 'Loading more runs…' : 'Show more'}</button>
        {/if}
      </LoadState>
    </div>
  {/if}
</section>

<style>
  .hist {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    max-height: 45%;
    min-height: 0;
    padding: 4px 8px 8px;
    border-top: 1px solid var(--border);
  }
  .hist-h {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    text-align: start;
  }
  .tools {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 2px 6px 4px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    padding: 4px 0;
    outline: none;
  }
  .search:focus-within {
    border-color: var(--accent-text);
  }
  .chips {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .hist-filter {
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: none;
    color: var(--text-dim);
    cursor: pointer;
  }
  .hist-filter.on {
    background: var(--accent-soft);
    border-color: transparent;
    color: var(--accent-text);
  }
  .rows {
    overflow-y: auto;
    min-height: 0;
  }
  .hist-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    min-width: 0;
    padding: 6px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    color: var(--text);
  }
  .hist-row:hover {
    background: var(--hover);
  }
  .hist-row.active {
    background: var(--accent-soft);
  }
  .body {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .name {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trig,
  .when,
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .sub {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .empty {
    padding: 6px 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .more {
    width: 100%;
    padding: 4px;
    background: none;
    border: none;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .dot {
    width: 8px;
    height: 8px;
    margin-top: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .dot.succeeded {
    background: var(--status-working);
  }
  .dot.failed {
    background: var(--status-exited);
  }
  .dot.running {
    background: var(--info);
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  .dot.waiting {
    background: var(--status-warn);
  }
  @container wf-side (max-width: 280px) {
    .trig {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot.running {
      animation: none;
    }
  }
</style>
