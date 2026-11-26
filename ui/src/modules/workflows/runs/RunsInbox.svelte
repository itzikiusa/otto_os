<script lang="ts">
  // The Runs inbox (ux.md §3), the Workflows module's landing tab: Needs you,
  // Running and Recent in one reading column. Needs you and Running come from
  // the active-runs store the page hands in (live from `workflow_run_updated`,
  // held runs included, DR-101); Recent is the server's run history with
  // search, a Failed chip and a workflow filter, refetched only when a run
  // ENDS (debounced 5 s) — never on every event.
  import { onDestroy } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import InboxFold from '../../../lib/components/InboxFold.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { api } from '../../../lib/api/client';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { liveQuery, type LiveEvent } from '../../../lib/live';
  import { loadErrorText } from '../../../lib/loadError';
  import { now as clockNow } from '../../../lib/stores/now.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import type { ActiveWorkflowRun, RunListPage } from '../../../lib/api/types';
  import RunRow from './RunRow.svelte';
  import { fromActive, fromHistory, inboxFold, inboxFolds, type InboxRun } from './runWords';
  import { markInboxSeen, seenRuns } from './seenRuns';

  interface Props {
    workspaceId: string;
    /** The active runs (lib/stores/workflowRuns.svelte.ts). */
    active: readonly ActiveWorkflowRun[];
    /** The workspace's workflows, for the filter (empty: no workflows yet). */
    workflows: readonly { id: string; name: string }[];
    onopen: (workflowId: string, runId: string) => void;
    /** The row menu's actions; each is left out when absent. */
    menu?: (run: InboxRun) => MenuItem[];
    /** The empty states' primaries. */
    onrun?: () => void;
    onnewworkflow?: () => void;
    /** Only this workflow's runs (the page's ⋯ sets it on a phone). */
    workflowFilter?: string | null;
  }
  let { workspaceId, active, workflows, onopen, menu, onrun, onnewworkflow, workflowFilter = $bindable(null) }: Props = $props();

  const unread = seenRuns();
  onDestroy(markInboxSeen);
  const isUnread = (r: InboxRun) => unread(r.id, r.finishedAt);
  const openRow = (r: InboxRun) => onopen(r.workflowId, r.id);

  let query = $state('');
  let q = $state('');
  let onlyFailed = $state(false);
  let recent = $state<InboxRun[]>([]);
  let next = $state<string | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let loadingMore = $state(false);
  let reload = $state(0);

  $effect(() => {
    const v = query.trim();
    const t = setTimeout(() => (q = v), 250);
    return () => clearTimeout(t);
  });

  function params(cursor?: string): string {
    const p = new URLSearchParams({ limit: '30' });
    if (q) p.set('q', q);
    if (workflowFilter) p.set('workflow_id', workflowFilter);
    if (onlyFailed) p.set('status', 'error');
    if (cursor) p.set('cursor', cursor);
    return p.toString();
  }

  const TERMINAL = new Set(['success', 'error', 'canceled']);
  $effect(() => {
    const wid = workspaceId;
    const key = params();
    void reload;
    loading = true;
    error = null;
    const poller = liveQuery({
      on: ['workflow_run_updated'],
      // Only a run that ends changes Recent.
      match: (ev: LiveEvent) => ev.workspace_id === wid && typeof ev.status === 'string' && TERMINAL.has(ev.status),
      fallbackMs: 60_000,
      safetyMs: 120_000,
      minIntervalMs: 5_000,
      run: async (signal) => {
        try {
          const page = await api.bg.get<RunListPage>(`/workspaces/${wid}/workflow-runs?${key}`, signal);
          if (signal.aborted) return;
          recent = page.items.map(fromHistory);
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
    if (!next || loadingMore) return;
    loadingMore = true;
    try {
      const page = await api.get<RunListPage>(`/workspaces/${workspaceId}/workflow-runs?${params(next)}`);
      recent = [...recent, ...page.items.map(fromHistory).filter((i) => !recent.some((x) => x.id === i.id))];
      next = page.next_cursor ?? null;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loadingMore = false;
    }
  }

  const live = $derived(active.map(fromActive).filter((r) => !workflowFilter || r.workflowId === workflowFilter));
  const folds = $derived.by(() => {
    const ids = new Set(live.map((r) => r.id));
    return inboxFolds([...live, ...recent.filter((r) => !ids.has(r.id) && inboxFold(r.state) === 'recent')], isUnread);
  });
  const filtered = $derived(!!q || onlyFailed || !!workflowFilter);
  const filterName = $derived(workflows.find((w) => w.id === workflowFilter)?.name ?? 'All workflows');
  const nothing = $derived(!loading && !error && !filtered && folds.needs.length + folds.running.length + folds.recent.length === 0);
  const now = $derived(clockNow());

  function pickWorkflow(e: MouseEvent): void {
    const el = e.currentTarget as HTMLElement;
    ctxMenu.showAt(el, [
      { label: 'All workflows', checked: !workflowFilter, pinned: true, action: () => (workflowFilter = null) },
      { separator: true, pinned: true },
      ...workflows.map((w) => ({ label: w.name, checked: w.id === workflowFilter, action: () => (workflowFilter = w.id) })),
    ], { filter: workflows.length > 8, filterPlaceholder: 'Find a workflow' });
  }
</script>

<!-- A calm heading shows its count only beside actions: these folds have none. -->
{#snippet nothingMore()}{/snippet}

{#snippet tools()}
    <div class="tools">
      <span class="search">
        <Icon name="search" size={13} />
        <input type="search" dir="auto" bind:value={query} placeholder="Search runs…" aria-label="Search runs" data-testid="wf-runs-search" />
      </span>
      <button type="button" class="chip wfchip" aria-pressed={onlyFailed} data-testid="wf-runs-filter-failed" onclick={() => (onlyFailed = !onlyFailed)}>Failed</button>
      {#if !viewport.isPhone || workflowFilter}
        <button type="button" class="chip wfchip" aria-pressed={!!workflowFilter} data-testid="wf-runs-filter-workflow" onclick={pickWorkflow}>
          <span class="ellipsis">{filterName}</span><Icon name="chevronDown" size={12} />
        </button>
      {/if}
    </div>
{/snippet}

<div class="read-col runs-inbox">
  {#if nothing}
    {#if workflows.length === 0}
      <EmptyState variant="page" icon="workbench" title="No workflows yet" body="A workflow chains agent steps, checks and messages into one run." actionLabel={onnewworkflow ? 'New workflow' : undefined} onaction={onnewworkflow} headingLevel={2} />
    {:else}
      <EmptyState variant="page" icon="play" title="No runs yet" body="Runs started from Slack, a schedule or Run… appear here." actionLabel={onrun ? 'Run…' : undefined} onaction={onrun} headingLevel={2} />
    {/if}
  {:else}
    {#if folds.needs.length}
      <div data-testid="wf-runs-needs">
        <InboxFold label="Needs you" count={folds.needs.length} actions={nothingMore}>
          {#each folds.needs as run (run.id)}<RunRow {run} unread={false} {now} onopen={openRow} {menu} />{/each}
        </InboxFold>
      </div>
    {/if}
    {#if folds.running.length}
      <div data-testid="wf-runs-running">
        <InboxFold label="Running" count={folds.running.length} actions={nothingMore}>
          {#each folds.running as run (run.id)}<RunRow {run} unread={false} {now} onopen={openRow} {menu} />{/each}
        </InboxFold>
      </div>
    {/if}
    <div data-testid="wf-runs-recent">
      <InboxFold label="Recent" actions={viewport.isPhone ? undefined : tools}>
        {#if viewport.isPhone}{@render tools()}{/if}
        <LoadState what="runs" {loading} {error} empty={folds.recent.length === 0} onretry={() => reload++} rows={4}>
          {#snippet emptyView()}
            <p class="none">{q ? `No runs match “${q}”.` : filtered ? 'No runs match these filters.' : 'No finished runs yet.'}</p>
          {/snippet}
          {#each folds.recent as run (run.id)}<RunRow {run} unread={isUnread(run)} {now} onopen={openRow} {menu} />{/each}
          {#if next}
            <div class="more"><button type="button" class="btn" disabled={loadingMore} onclick={more}>{loadingMore ? 'Loading more…' : 'Show more'}</button></div>
          {/if}
        </LoadState>
      </InboxFold>
    </div>
  {/if}
</div>

<style>
  .runs-inbox {
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
    padding-block: var(--sp-6);
  }
  .tools {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--sp-3);
    min-inline-size: 0;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    inline-size: 220px;
    max-inline-size: 100%;
    block-size: 28px;
    padding-inline: var(--sp-4);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent-line);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .search input {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    outline: none;
  }
  .wfchip {
    max-inline-size: 180px;
  }
  .none {
    margin: 0;
    padding: var(--sp-5);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .more {
    display: flex;
    justify-content: center;
    padding-block: var(--sp-4);
  }
  @media (max-width: 640px) {
    .runs-inbox {
      padding-inline: var(--sp-4);
    }
    .tools {
      justify-content: flex-start;
      flex-wrap: nowrap;
      padding-block-end: var(--sp-4);
    }
    .search {
      flex: 1;
      inline-size: auto;
      block-size: 40px;
    }
  }
</style>
