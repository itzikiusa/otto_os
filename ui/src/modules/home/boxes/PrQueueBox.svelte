<script lang="ts">
  // PR review queue box: open pull requests and active/recent AI reviews across
  // workspace repos. Uses existing `/workspaces/{id}/repos` (or `/git/repos`),
  // `/git/accounts`, `/repos/{id}/prs?state=open`, `/repos/{id}/prs/{n}/review`,
  // and `/repos/{id}/local-reviews` endpoints on the background lane via
  // `livePoll`, respecting `active` and header `tick` refresh.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { api } from '../../../lib/api/client';
  import { mapLimit } from '../../../lib/poll';
  import { loadErrorText } from '../../../lib/loadError';
  import { plural } from '../../../lib/plural';
  import { router } from '../../../lib/router.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import type { GitAccount, PrListResp, Repo, Review } from '../../../lib/api/types';
  import type { HomeBox } from '../home.svelte';
  import { freshness, livePoll, type Poller } from './poll';
  import {
    DEFAULT_BOX_ROW_LIMIT,
    prCandidateRepos,
    summarizePrQueue,
    type LocalReviewRawItem,
    type PrQueueRawItem,
  } from './homeBoxesData';

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
    active?: boolean;
  }
  let { box: _box, zoomed, tick, active = true }: Props = $props();

  const POLL_MS = 60_000;
  const MAX_REVIEW_PROBES = 6;

  let repos = $state<Repo[]>([]);
  let prs = $state<PrQueueRawItem[]>([]);
  let localReviews = $state<LocalReviewRawItem[]>([]);
  let loading = $state(true);
  let loaded = $state(false);
  let error = $state('');

  let seq = 0;

  async function load(signal?: AbortSignal): Promise<boolean> {
    const mine = ++seq;
    const wsId = ws.currentId;
    try {
      const repoUrl = wsId ? `/workspaces/${encodeURIComponent(wsId)}/repos` : '/git/repos';
      const [fetchedRepos, accounts] = await Promise.all([
        api.bg.get<Repo[]>(repoUrl, signal),
        api.bg.get<GitAccount[]>('/git/accounts', signal).catch(() => null),
      ]);
      if (mine !== seq || signal?.aborted) return true;

      const candidates = prCandidateRepos(fetchedRepos, accounts);
      const prSettled = await mapLimit(candidates, 2, async (repo): Promise<PromiseSettledResult<PrQueueRawItem[]>> => {
        if (mine !== seq || signal?.aborted) return { status: 'fulfilled', value: [] };
        try {
          const page = await api.bg.get<PrListResp>(
            `/repos/${encodeURIComponent(repo.id)}/prs?state=open&page=1&per_page=10`,
            signal,
          );
          return { status: 'fulfilled', value: page.items.map((pr) => ({ repo, pr })) };
        } catch (reason) {
          return { status: 'rejected', reason };
        }
      });
      if (mine !== seq || signal?.aborted) return true;

      const nextPrs: PrQueueRawItem[] = [];
      let failedRepos = 0;
      let firstErr: unknown = null;
      for (const s of prSettled) {
        if (s.status === 'fulfilled') nextPrs.push(...s.value);
        else {
          failedRepos += 1;
          if (!firstErr) firstErr = s.reason;
        }
      }

      // Probe AI reviews for the freshest open PRs (404 = no review run yet).
      nextPrs.sort((a, b) => (b.pr.updated_at ?? '').localeCompare(a.pr.updated_at ?? ''));
      const toProbe = nextPrs.slice(0, MAX_REVIEW_PROBES);
      const reviewResults = await mapLimit(toProbe, 2, async (item): Promise<Review | null> => {
        if (mine !== seq || signal?.aborted) return null;
        return api.bg
          .get<Review>(`/repos/${encodeURIComponent(item.repo.id)}/prs/${item.pr.number}/review`, signal)
          .catch(() => null);
      });
      if (mine !== seq || signal?.aborted) return true;
      for (let i = 0; i < toProbe.length; i++) {
        toProbe[i].review = reviewResults[i] ?? null;
      }

      // Probe latest local AI review per repo (bounded to avoid fan-out bursts).
      const localSettled = await mapLimit(fetchedRepos.slice(0, MAX_REVIEW_PROBES), 2, async (repo): Promise<LocalReviewRawItem | null> => {
        if (mine !== seq || signal?.aborted) return null;
        const list = await api.bg
          .get<Review[]>(`/repos/${encodeURIComponent(repo.id)}/local-reviews`, signal)
          .catch(() => [] as Review[]);
        const latest = list.find((r) => r.status !== 'cancelled');
        return latest ? { repo, review: latest } : null;
      });
      if (mine !== seq || signal?.aborted) return true;

      const nextLocal = localSettled.filter((x): x is LocalReviewRawItem => x !== null);
      repos = fetchedRepos;
      prs = nextPrs;
      localReviews = nextLocal;
      loaded = true;

      if (candidates.length > 0 && failedRepos === candidates.length && nextLocal.length === 0) {
        error = loadErrorText(firstErr);
        return false;
      }
      error = failedRepos > 0 ? `${plural(failedRepos, 'repository')} couldn’t be reached.` : '';
      return failedRepos === 0;
    } catch (e) {
      if (mine !== seq || signal?.aborted) return true;
      error = loadErrorText(e);
      return false;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  let poller: Poller | null = null;
  const fresh = freshness(load);
  let lastWs: string | null | undefined;
  $effect(() => {
    const wsKey = ws.currentId ?? '';
    if (lastWs !== undefined && lastWs !== wsKey) {
      untrack(() => {
        seq += 1;
        repos = [];
        prs = [];
        localReviews = [];
        loaded = false;
        loading = true;
        error = '';
      });
    }
    lastWs = wsKey;
    if (!active) return;
    poller?.stop();
    poller = livePoll(fresh.run, POLL_MS, ['review_changed', 'repo_status_changed'], {
      immediate: fresh.start(wsKey, POLL_MS),
    });
    return () => poller?.stop();
  });

  // Header refresh button only — avoid queuing a duplicate load on mount.
  let seenTick = untrack(() => tick);
  $effect(() => {
    const t = tick;
    if (t === seenTick) return;
    seenTick = t;
    untrack(() => poller?.now());
  });

  const summary = $derived(
    summarizePrQueue({
      repos,
      prs,
      localReviews,
      limit: zoomed ? 30 : DEFAULT_BOX_ROW_LIMIT,
    }),
  );

  const hasData = $derived(loaded || summary.rows.length > 0);
  const unknown = $derived(!loaded && (loading || !!error));
  const statText = (n: number): string => (unknown ? '—' : String(n));
</script>

<div class="prq">
  <div class="stats">
    <div class="stat">
      <span class="n" class:warn={!unknown && summary.stats.openPrs > 0}>{statText(summary.stats.openPrs)}</span>
      <span class="l">open PRs</span>
    </div>
    <div class="stat">
      <span class="n" class:info={!unknown && summary.stats.reviewing > 0}>{statText(summary.stats.reviewing)}</span>
      <span class="l">reviewing</span>
    </div>
    <div class="stat">
      <span class="n">{statText(summary.stats.repos)}</span>
      <span class="l">repos</span>
    </div>
  </div>

  <LoadState
    what="PR review queue"
    variant="compact"
    loading={loading && !hasData}
    error={error || null}
    empty={summary.rows.length === 0}
    onretry={() => poller?.now()}
  >
    {#snippet emptyView()}
      <EmptyState
        icon="branch"
        title="No open PRs or reviews"
        body={repos.length === 0 ? 'Add a repository in Git to track pull requests and AI reviews here.' : 'Nothing waiting for review across your workspace repositories.'}
      >
        <button class="btn small" onclick={() => router.go('git')}>Open Git</button>
      </EmptyState>
    {/snippet}

    <ul class="rows">
      {#each summary.rows as row (row.id)}
        <li>
          <button class="prow" onclick={() => router.go(row.route)} title={row.title}>
            <span class="ricon"><Icon name={row.kind === 'local-review' ? 'sparkle' : 'pr'} size={12} /></span>
            <span class="meta">
              <span class="title ellipsis">{row.title}</span>
              <span class="sub ellipsis">{row.detail}</span>
            </span>
            <Badge tone={row.badgeTone} label={row.badgeLabel} live={row.live} />
            {#if row.updatedAt}
              <span class="ago" title={new Date(row.updatedAt).toLocaleString()}>{rel(row.updatedAt, { compact: true })}</span>
            {/if}
            <span class="go" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
          </button>
        </li>
      {/each}
    </ul>

    <div class="foot-bar">
      <button class="foot" onclick={() => router.go('git')}>
        Open Git<Icon name="chevronRight" size={12} />
      </button>
      {#if summary.truncatedCount > 0}
        <span class="more">+{summary.truncatedCount} more</span>
      {/if}
    </div>
  </LoadState>
</div>

<style>
  .prq {
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100%;
    min-height: 0;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0;
  }
  .stat {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 0;
    padding: 2px 12px;
    border-inline-start: 1px solid var(--separator);
  }
  .stat:first-child {
    padding-inline-start: 2px;
    border-inline-start: none;
  }
  .n {
    font-size: var(--fs-xl);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }
  .n.info {
    color: var(--info);
  }
  .n.warn {
    color: var(--warning);
  }
  .l {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
  }
  .prow {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    font: inherit;
    font-size: var(--fs-s);
  }
  .prow:hover {
    background: var(--hover);
  }
  .ricon {
    display: inline-flex;
    color: var(--text-dim);
    flex: none;
  }
  .meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .title {
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ago {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    flex: none;
  }
  .go {
    display: inline-flex;
    color: var(--text-dim);
    opacity: 0;
    transition: opacity var(--dur-fast) ease-out;
    flex: none;
  }
  .prow:hover .go,
  .prow:focus-visible .go {
    opacity: 1;
  }
  .foot-bar {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .foot {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .foot:hover {
    text-decoration: underline;
  }
  .more {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
