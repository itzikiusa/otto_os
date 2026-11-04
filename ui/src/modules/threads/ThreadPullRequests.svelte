<script lang="ts">
  // The stage's "Pull requests" view (T3's footer git icon): the project's PRs
  // from Otto's PR list API with search ("label:bug" too), Sort, Filters,
  // All / Mine and refresh. A row opens the PR in Git; ↗ opens it on the forge.
  import { untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import { gitApi } from '../../lib/api/git';
  import type { GitAccount, PrState, PrSummary, Repo } from '../../lib/api/types';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { openExternal } from '../../lib/external';
  import { loadErrorText } from '../../lib/loadError';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { forgeLabel } from './threadsHelpers';

  interface Props {
    repo: Repo | null;
  }
  let { repo }: Props = $props();

  type Sort = 'updated' | 'oldest' | 'number';
  let items = $state<PrSummary[]>([]);
  let hasMore = $state(false);
  let page = $state(1);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let retry = $state(0);
  let query = $state('');
  let sort = $state<Sort>('updated');
  let prState = $state<PrState | 'all'>('open');
  let mine = $state(false);
  let handles = $state<Set<string>>(new Set());
  let sortEl = $state<HTMLButtonElement | null>(null);
  let filterEl = $state<HTMLButtonElement | null>(null);

  const forge = $derived(forgeLabel(repo));

  // Forge usernames power "Mine" (best-effort, as in Git's Focus view).
  $effect(() => {
    void api
      .get<GitAccount[]>('/git/accounts')
      .then((accs) => (handles = new Set(accs.map((a) => a.username.toLowerCase()).filter(Boolean))))
      .catch(() => {});
  });

  async function load(reset: boolean): Promise<void> {
    if (!repo || !forge) return;
    const next = reset ? 1 : page + 1;
    loading = true;
    error = null;
    try {
      const r = await gitApi.prs.list(repo.id, prState, next, 50);
      items = reset ? r.items : [...items, ...r.items];
      hasMore = r.has_more;
      page = next;
    } catch (e) {
      error = loadErrorText(e);
      if (reset) items = [];
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void repo?.id;
    void prState;
    void retry;
    untrack(() => void load(true));
  });

  const shown = $derived.by(() => {
    const raw = query.trim().toLowerCase();
    const labels = [...raw.matchAll(/label:(\S+)/g)].map((m) => m[1]);
    const text = raw.replace(/label:\S+/g, '').trim();
    let list = items.filter((p) => {
      if (mine && handles.size > 0 && !handles.has(p.author.toLowerCase())) return false;
      if (labels.length && !labels.every((l) => (p.labels ?? []).some((x) => x.toLowerCase().includes(l)))) return false;
      if (!text) return true;
      return `#${p.number} ${p.title} ${p.author} ${p.source_branch} ${p.target_branch}`.toLowerCase().includes(text);
    });
    list = [...list].sort((a, b) =>
      sort === 'number' ? b.number - a.number : sort === 'oldest' ? a.updated_at.localeCompare(b.updated_at) : b.updated_at.localeCompare(a.updated_at),
    );
    return list;
  });

  function openSort(): void {
    if (!sortEl) return;
    const opt = (id: Sort, label: string) => ({ label, checked: sort === id, action: () => (sort = id) });
    ctxMenu.showAt(sortEl, [opt('updated', 'Recently updated'), opt('oldest', 'Least recently updated'), opt('number', 'Newest number')], { align: 'end' });
  }

  function openFilters(): void {
    if (!filterEl) return;
    const opt = (id: PrState | 'all', label: string) => ({ label, checked: prState === id, action: () => (prState = id) });
    ctxMenu.showAt(filterEl, [opt('open', 'Open'), opt('merged', 'Merged'), opt('declined', 'Declined'), opt('all', 'All states')], { align: 'end' });
  }
</script>

<section class="tpr" aria-label="Pull requests" data-testid="threads-prs-view">
  <header class="tpr-bar">
    <h2 class="tpr-title">Pull requests{repo ? ` · ${repo.name}` : ''}</h2>
    <div class="tpr-search">
      <Icon name="search" size={13} />
      <input
        type="search"
        dir="auto"
        bind:value={query}
        placeholder="Search pull requests, or label:bug"
        aria-label="Search pull requests"
        data-testid="threads-prs-search"
      />
    </div>
    <button bind:this={sortEl} type="button" class="btn small ghost" aria-haspopup="menu" onclick={openSort}>
      <Icon name="arrowDown" size={12} /> Sort
    </button>
    <button bind:this={filterEl} type="button" class="btn small ghost" aria-haspopup="menu" onclick={openFilters}>
      <Icon name="filter" size={12} /> Filters
    </button>
    <div class="segmented" role="group" aria-label="Whose pull requests">
      <button type="button" class:active={!mine} aria-pressed={!mine} onclick={() => (mine = false)}>All</button>
      <button
        type="button"
        class:active={mine}
        aria-pressed={mine}
        title={handles.size ? 'Opened by your git accounts' : 'Add a git account to filter by you'}
        disabled={handles.size === 0}
        onclick={() => (mine = true)}
      >
        Mine
      </button>
    </div>
    <button
      type="button"
      class="icon-btn"
      aria-label="Refresh pull requests"
      title="Refresh"
      disabled={loading}
      onclick={() => retry++}
    >
      <Icon name="refresh" size={13} />
    </button>
  </header>

  <div class="tpr-body">
    {#if !repo}
      <EmptyState variant="panel" icon="pr" title="No repository here" body="Open a thread in a git repository to see its pull requests." />
    {:else if !forge}
      <EmptyState variant="panel" icon="pr" title="No supported remote" body="{repo.name} has no GitHub, Bitbucket or GitLab remote." />
    {:else}
      <LoadState
        what="pull requests"
        variant="panel"
        rows={6}
        loading={loading && items.length === 0}
        {error}
        empty={shown.length === 0}
        onretry={() => retry++}
      >
        {#snippet emptyView()}
          <EmptyState
            variant="panel"
            icon="pr"
            title={query.trim() || mine ? 'No pull requests match' : 'No pull requests'}
            body={query.trim() || mine ? 'Clear the search or switch to All.' : `Nothing ${prState === 'all' ? '' : `${prState} `}on ${forge}.`}
          />
        {/snippet}
        <ul class="tpr-list">
          {#each shown as p (p.number)}
            <li class="tpr-row reveal-host">
              <button
                type="button"
                class="tpr-main"
                title="Open #{p.number} in Git"
                onclick={() => router.go(`git/${encodeURIComponent(repo!.id)}/pr/${p.number}`)}
              >
                <Icon name="pr" size={14} />
                <span class="tpr-name">
                  <span class="tpr-line">
                    <span class="tpr-num mono">#{p.number}</span>
                    <span class="ellipsis" dir="auto">{p.title}</span>
                    {#if p.draft}<span class="tpr-tag">Draft</span>{/if}
                  </span>
                  <span class="tpr-sub">
                    <span>{p.author}</span>
                    <span class="mono ellipsis" dir="ltr">{p.source_branch} → {p.target_branch}</span>
                    {#each p.labels ?? [] as l (l)}<span class="tpr-tag">{l}</span>{/each}
                  </span>
                </span>
                <span class="tpr-time">{rel(p.updated_at)}</span>
              </button>
              <button
                type="button"
                class="icon-btn reveal-on-hover"
                aria-label="Open #{p.number} on {forge}"
                title="Open on {forge}"
                onclick={() => void openExternal(p.url)}
              >
                <Icon name="external" size={12} />
              </button>
            </li>
          {/each}
        </ul>
        {#if hasMore}
          <button type="button" class="btn small ghost tpr-more" disabled={loading} onclick={() => void load(false)}>
            {loading ? 'Loading more…' : 'Load more'}
          </button>
        {/if}
      </LoadState>
    {/if}
  </div>
</section>

<style>
  .tpr {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .tpr-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 12px 16px;
    border-block-end: 1px solid var(--border);
  }
  .tpr-title {
    margin: 0;
    margin-inline-end: 8px;
    font-size: var(--fs-l);
    font-weight: 500;
  }
  .tpr-search {
    flex: 1;
    min-width: 180px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    color: var(--text-dim);
  }
  .tpr-search:focus-within {
    border-color: var(--accent-line-strong);
  }
  .tpr-search input {
    flex: 1;
    min-width: 0;
    block-size: 28px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    outline: none;
  }
  .tpr-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 12px;
  }
  .tpr-list {
    list-style: none;
    margin: 0 auto;
    padding: 0;
    max-width: 960px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tpr-row {
    display: flex;
    align-items: center;
    border-radius: var(--radius-m);
  }
  .tpr-row:hover {
    background: var(--hover);
  }
  .tpr-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .tpr-main :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .tpr-name {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tpr-line,
  .tpr-sub {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .tpr-sub {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tpr-num {
    color: var(--text-dim);
  }
  .tpr-tag {
    padding-inline: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .tpr-time {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tpr-more {
    display: block;
    margin: 8px auto;
  }
</style>
