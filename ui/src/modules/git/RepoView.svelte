<script lang="ts">
  // One repo: toolbar header + tabs (Graph / Changes / History / Pull Requests).
  import { api } from '../../lib/api/client';
  import type { MergeResult, Repo, RepoStatusResp } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import GitToolbar from './GitToolbar.svelte';
  import ChangesView from './ChangesView.svelte';
  import GraphView from './GraphView.svelte';
  import HistoryView from './HistoryView.svelte';
  import PrList from './PrList.svelte';
  import LocalReviewPanel from './LocalReviewPanel.svelte';
  import MergeApprovalModal from './MergeApprovalModal.svelte';
  import ConflictResolverView from './ConflictResolverView.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repo: Repo;
    tab: string; // graph | changes | history | prs | review
    /** Embedded in the agent-mode right panel: switch tabs via `onTab` (local
     *  state) instead of routing, and hide the "← Repos" back button. */
    embedded?: boolean;
    onTab?: (tab: string) => void;
  }
  let { repo, tab, embedded = false, onTab }: Props = $props();

  let status: RepoStatusResp | null = $state(null);

  // ── Merge / conflict-resolution state ──────────────────────────────────────
  // The pending merge approval (set when a branch is dropped onto another).
  let mergeReq = $state<{ source: string; target: string } | null>(null);
  // True while the conflict resolver view is showing instead of the tabs.
  let resolving = $state(false);
  // Seed data carried from a conflicting merge into the resolver.
  let conflictSeed = $state<{ files: string[]; source: string | null }>({ files: [], source: null });
  // True when a merge is in progress (surfaces the "Resolve conflicts" banner).
  let merging = $state(false);

  $effect(() => {
    const id = repo.id;
    status = null;
    resolving = false;
    merging = false;
    mergeReq = null;
    void api
      .get<RepoStatusResp>(`/repos/${id}/status`)
      .then((s) => (status = s))
      .catch(() => (status = null));
    // Detect an in-progress merge (e.g. left mid-resolution) so we can offer to
    // resume it via a banner.
    void git
      .getMergeStatus(id)
      .then((m) => {
        merging = m.merging;
        if (m.merging) conflictSeed = { files: m.conflicted_files, source: m.source };
      })
      .catch(() => {
        merging = false;
      });
  });

  function setStatus(s: RepoStatusResp): void {
    status = s;
    if (git.primary?.id === repo.id) git.primaryStatus = s;
  }

  function requestMerge(source: string, target: string): void {
    mergeReq = { source, target };
  }

  function onMerged(result: MergeResult): void {
    mergeReq = null;
    setStatus(result.repo_status);
    merging = false;
    // Refresh refs/commits is handled inside GraphView's own effect on status
    // change; force a light status reconcile + PR refresh via the store.
    if (git.primary?.id === repo.id) void git.refreshPrimary();
    // Re-mount the graph so its refs/commits effect re-runs after history moved.
    graphKey++;
  }

  function onConflicts(result: MergeResult): void {
    // Capture the source label before clearing the request.
    const source = mergeReq?.source ?? null;
    mergeReq = null;
    setStatus(result.repo_status);
    merging = true;
    conflictSeed = { files: result.conflicted_files, source };
    resolving = true;
  }

  function openResolver(): void {
    resolving = true;
  }

  function leaveResolver(): void {
    resolving = false;
    merging = false;
    setStatusFromDaemon();
    if (git.primary?.id === repo.id) void git.refreshPrimary();
    graphKey++;
  }

  function setStatusFromDaemon(): void {
    void api
      .get<RepoStatusResp>(`/repos/${repo.id}/status`)
      .then((s) => setStatus(s))
      .catch(() => {});
  }

  // Bumping this key re-mounts GraphView so its refs/commits effect re-runs
  // after a merge changes history.
  let graphKey = $state(0);

  const tabs = [
    { id: 'graph', label: 'Graph' },
    { id: 'changes', label: 'Changes' },
    { id: 'history', label: 'History' },
    { id: 'prs', label: 'Pull Requests' },
    { id: 'review', label: 'Review' },
  ];

  // Repo switcher: jump between the workspace's repositories without going back
  // to the list. Built from the full repo set the git store holds.
  function openRepoSwitcher(e: MouseEvent): void {
    const others = git.repos.filter((r) => r.id !== repo.id);
    ctxMenu.show(e, [
      ...others.map((r) => ({
        label: r.name,
        icon: 'branch',
        action: () => router.go(`git/${r.id}/${tab}`),
      })),
      ...(others.length > 0 ? [{ separator: true }] : []),
      { label: 'Add repository…', icon: 'plus', action: () => router.go('git') },
      { label: 'All repositories…', icon: 'folder', action: () => router.go('git') },
    ]);
  }
</script>

<div class="repoview">
  <header class="rv-head">
    {#if !embedded}
      <button class="btn ghost small" onclick={() => router.go('git')}>← Repos</button>
    {/if}
    <button
      class="rv-name rv-switch"
      title="Switch repository"
      onclick={openRepoSwitcher}
      oncontextmenu={openRepoSwitcher}
    >
      <Icon name="branch" size={13} />
      {repo.name}
      {#if git.repos.length > 1}<span class="rv-count">{git.repos.length}</span>{/if}
      <Icon name="chevronDown" size={11} />
    </button>
    {#if repo.provider}<span class="chip">{repo.provider}</span>{/if}
    <span class="grow"></span>
    {#if status}
      <GitToolbar repoId={repo.id} {status} onstatus={setStatus} />
    {:else}
      <div class="toolbar-skeleton"></div>
    {/if}
  </header>

  <nav class="rv-tabs">
    {#each tabs as t (t.id)}
      <button
        class="rv-tab"
        class:active={tab === t.id && !resolving}
        onclick={() => {
          resolving = false;
          if (embedded) onTab?.(t.id);
          else router.go(`git/${repo.id}/${t.id}`);
        }}
      >
        {t.label}
        {#if t.id === 'changes' && status && status.changes.length > 0}
          <span class="count">{status.changes.length}</span>
        {/if}
      </button>
    {/each}
    {#if merging}
      <button class="rv-tab conflict-tab" class:active={resolving} onclick={openResolver}>
        <Icon name="merge" size={12} />
        Resolve conflicts
        {#if conflictSeed.files.length > 0}
          <span class="count conflict-count">{conflictSeed.files.length}</span>
        {/if}
      </button>
    {/if}
  </nav>

  <!-- In-progress merge banner (shown when not already in the resolver). -->
  {#if merging && !resolving}
    <div class="merge-banner">
      <Icon name="merge" size={13} />
      <span>
        A merge is in progress{#if conflictSeed.source}
          (merging <span class="mono">{conflictSeed.source}</span>){/if}.
        {#if conflictSeed.files.length > 0}
          {conflictSeed.files.length} file{conflictSeed.files.length === 1 ? '' : 's'} need resolution.
        {/if}
      </span>
      <span class="grow"></span>
      <button class="btn small primary" onclick={openResolver}>Resolve conflicts</button>
    </div>
  {/if}

  <div class="rv-body">
    {#if resolving}
      <ConflictResolverView
        repoId={repo.id}
        initialFiles={conflictSeed.files}
        initialSource={conflictSeed.source}
        onleave={leaveResolver}
      />
    {:else if tab === 'graph'}
      {#if status}
        {#key graphKey}
          <GraphView repoId={repo.id} {status} onstatus={setStatus} onmergerequest={requestMerge} />
        {/key}
      {:else}
        <div style="padding: 16px"><Skeleton rows={5} height={36} /></div>
      {/if}
    {:else if tab === 'history'}
      <HistoryView repoId={repo.id} />
    {:else if tab === 'prs'}
      <PrList repoId={repo.id} />
    {:else if tab === 'review'}
      <div class="rv-tab-scroll">
        <LocalReviewPanel repoId={repo.id} />
      </div>
    {:else if status}
      <ChangesView repoId={repo.id} {status} onstatus={setStatus} />
    {:else}
      <div style="padding: 16px"><Skeleton rows={5} height={36} /></div>
    {/if}
  </div>
</div>

{#if mergeReq}
  <MergeApprovalModal
    repoId={repo.id}
    source={mergeReq.source}
    target={mergeReq.target}
    onclose={() => (mergeReq = null)}
    onmerged={onMerged}
    onconflicts={onConflicts}
  />
{/if}

<style>
  .repoview {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .rv-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .rv-name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13.5px;
    font-weight: 600;
  }
  .rv-switch {
    border: 1px solid transparent;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    padding: 3px 7px;
    border-radius: var(--radius-s);
  }
  .rv-switch:hover {
    background: var(--surface-2);
    border-color: var(--border);
  }
  .rv-count {
    font-size: 10px;
    font-weight: 700;
    padding: 0 5px;
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .rv-tabs {
    display: flex;
    gap: 2px;
    padding: 6px 14px 0;
    border-bottom: 1px solid var(--border);
  }
  .rv-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border: none;
    background: transparent;
    border-bottom: 2px solid transparent;
    font-size: 12.5px;
    color: var(--text-dim);
    cursor: pointer;
    transition: color 130ms ease-out, border-color 130ms ease-out;
  }
  .rv-tab:hover {
    color: var(--text);
  }
  .rv-tab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
    font-weight: 500;
  }
  .count {
    font-size: 10px;
    font-weight: 600;
    min-width: 16px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
    display: grid;
    place-items: center;
  }
  .conflict-tab {
    margin-left: auto;
    color: #b8860b;
    gap: 5px;
  }
  .conflict-tab:hover {
    color: #b8860b;
  }
  .conflict-tab.active {
    color: #b8860b;
    border-bottom-color: #febc2e;
  }
  .conflict-count {
    background: color-mix(in srgb, #febc2e 24%, transparent);
    color: #b8860b;
  }
  .merge-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 14px;
    background: color-mix(in srgb, #febc2e 12%, transparent);
    border-bottom: 1px solid var(--border);
    color: #b8860b;
    font-size: 12px;
  }
  .merge-banner .mono {
    font-family: var(--font-mono);
    font-weight: 600;
  }
  .merge-banner .grow {
    flex: 1;
  }
  .rv-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .rv-tab-scroll {
    height: 100%;
    overflow-y: auto;
    padding: 0 14px;
    box-sizing: border-box;
  }
  .toolbar-skeleton {
    width: 300px;
    height: 26px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }
</style>
