<script lang="ts">
  // One repo: History / Pull requests / Review. Staging and history both
  // live on History's graph (WIP row + detail panel): there is no separate
  // Changes view.
  import type { GitOpInProgress, MergeResult, Repo, RepoStatusResp } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import GraphView from './GraphView.svelte';
  import { gitBridge } from '../../lib/diff/gitBridge.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { toasts } from '../../lib/toast.svelte';
  import { remoteHost } from './gitUiHelpers';
  import { readOverview } from '../../lib/api/gitOverview';
  import { prefetchFirstPage } from './graph/graphData.svelte';
  import { untrack } from 'svelte';

  // Views only some visits open stay out of the Git chunk (perf R7): the PR
  // list (+ CreatePr), the local review, the merge dialog and the conflict
  // resolver load on first use, then mount synchronously (lazyComponent).
  // Graph and WIP — what every visit shows — stay eager.
  const PrWorkspaceLazy = lazyComponent(() => import('./PrWorkspace.svelte'));
  const LocalReviewLazy = lazyComponent(() => import('./LocalReviewPanel.svelte'));
  const MergeApprovalLazy = lazyComponent(() => import('./MergeApprovalModal.svelte'));
  const ConflictResolverLazy = lazyComponent(() => import('./ConflictResolverView.svelte'));
  // Opened panels (remotes, recovery tools).
  const RemotesLazy = lazyComponent(() => import('./RemotesPanel.svelte'));
  const RecoveryLazy = lazyComponent(() => import('./RecoveryTools.svelte'));

  interface Props {
    repo: Repo;
    tab: string; // graph (History) | prs | review (legacy changes / history render the graph)
    /** The Git page owns the view switch (its header) and the tab store. */
    onTab: (tab: string) => void;
  }
  let { repo, tab, onTab }: Props = $props();

  // Legacy ids render the graph: 'changes' (its WIP row is the working tree),
  // 'history', and 'focus' (Focus is a page-level view now).
  const effTab = $derived(tab === 'changes' || tab === 'history' || tab === 'focus' ? 'graph' : tab);

  // Status is owned by the git store (single source of truth) so the auto-fetch
  // loop, the tab strip and this toolbar all stay in sync from one fetch.
  const status = $derived(git.statusById[repo.id] ?? null);

  // ── Merge / conflict-resolution state ──────────────────────────────────────
  // The pending merge approval (set when a branch is dropped onto another).
  let mergeReq = $state<{ source: string; target: string } | null>(null);
  // True while the conflict resolver view is showing instead of the tabs.
  let resolving = $state(false);
  // Seed data carried from a conflicting merge into the resolver.
  let conflictSeed = $state<{ files: string[]; source: string | null }>({ files: [], source: null });
  // True when ANY resolvable operation is in progress (merge / rebase /
  // cherry-pick / revert, or conflicted files with no state file, e.g. a
  // conflicting stash pop) — surfaces the "Resolve conflicts" banner.
  let merging = $state(false);
  // Which operation it is; null for state-file-less conflicts (stash pop).
  let mergeOp = $state<GitOpInProgress | null>(null);

  /** Human label for the in-progress operation, banner + resolver wording. */
  const opLabel = $derived(
    mergeOp === 'rebase'
      ? 'rebase'
      : mergeOp === 'cherry_pick'
        ? 'cherry-pick'
        : mergeOp === 'revert'
          ? 'revert'
          : mergeOp === 'merge'
            ? 'merge'
            : null,
  );

  // A failed FIRST status load (folder moved or deleted, daemon down) used to
  // leave the toolbar and graph skeletons pulsing forever. The store records the
  // failure (`statusErrorById`); it only replaces the view while there is no
  // status at all — once any status exists, a later failure keeps the stale one.
  const statusError = $derived(status ? null : (git.statusErrorById[repo.id] ?? null));
  let statusLoading = $state(false);
  /** First read: the repo's ONE overview (shared with the graph); a retry re-reads the status. */
  function loadStatus(id: string, retry = false): void {
    statusLoading = true;
    const read = retry ? git.refreshStatus(id) : readOverview(id).then((o) => void git.setStatus(id, o.status), () => git.refreshStatus(id));
    void read.finally(() => {
      if (id === repo.id) statusLoading = false;
    });
  }

  $effect(() => {
    const id = repo.id;
    resolving = false;
    merging = false;
    mergeOp = null;
    mergeReq = null;
    // Status lives in the store; (re)load it for this repo. The auto-fetch loop
    // keeps it fresh thereafter, and the tab strip shares the same value.
    loadStatus(id);
    // History mounts once the status is in: its first log page starts now,
    // beside the overview (a known status mounts the graph at once).
    if (untrack(() => effTab === 'graph' && !git.statusById[id])) prefetchFirstPage(id);
  });

  // Unmerged paths as the status reports them. A merge in progress ALWAYS shows
  // up here (kind="conflicted"), whoever started it — the merge modal, a pull
  // whose merge conflicted, an agent session, or git on the command line.
  const conflictedPaths = $derived(
    (status?.changes ?? []).filter((c) => c.kind === 'conflicted').map((c) => c.path),
  );
  // Primitive key so the effect below re-runs when the conflict SET changes —
  // not on every status poll (each poll hands back a fresh array reference).
  const conflictKey = $derived(conflictedPaths.join(' '));
  // Re-checked whenever that set changes, so the banner + "Resolve conflicts"
  // tab appear as soon as the repo enters a merge — not only on mount. (This
  // used to be a mount-only effect keyed on repo.id: a mid-session conflict left
  // the incoming files sitting in WIP with no way into the resolver until you
  // navigated away and back.)
  const opKey = $derived(status?.op_in_progress ?? '');
  $effect(() => {
    const id = repo.id;
    const seen = conflictKey;
    // Nothing conflicted and no operation in progress: nothing to ask the daemon.
    if (!seen && !opKey) {
      merging = false;
      mergeOp = null;
      return;
    }
    void git
      .getMergeStatus(id)
      .then((m) => {
        // A late reply for a repo this view no longer shows must not paint
        // its conflicts here.
        if (id !== repo.id) return;
        merging = m.merging;
        mergeOp = m.op ?? null;
        if (m.merging) {
          conflictSeed = {
            files: m.conflicted_files.length > 0 ? m.conflicted_files : conflictedPaths,
            source: m.source ?? conflictSeed.source,
          };
        } else if (!seen) {
          conflictSeed = { files: [], source: null };
        }
      })
      .catch(() => {
        if (id !== repo.id) return;
        // Fall back to what the status already told us rather than hiding a
        // merge the user is standing in.
        merging = conflictedPaths.length > 0;
        mergeOp = status?.op_in_progress ?? null;
      });
  });

  function setStatus(s: RepoStatusResp): void {
    git.setStatus(repo.id, s);
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
    // Re-read refs + log in place (keeps the graph's selection and scroll).
    gitBridge.refreshGraph(repo.id);
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
    // The graph remounts when the resolver closes; its cached snapshot is
    // revalidated against `/refs`, and this covers a graph that stayed mounted.
    gitBridge.refreshGraph(repo.id);
  }

  function setStatusFromDaemon(): void {
    void git.refreshStatus(repo.id);
  }

  // Remote CRUD lives behind a header button + modal (the branch bar is a
  // different owner's surface, and remotes are a repo-level setting, not a
  // per-branch one).
  let remotesOpen = $state(false);

  /** The repo's own overflow rows (Remotes, Recovery, Copy path). In the Git
   *  page header they ride PageHeader's single ⋯ (`more`). */
  export function moreMenuItems(): MenuItem[] {
    return [
      { label: 'Remotes…', icon: 'globe', action: () => (remotesOpen = true) },
      { label: 'Recovery tools…', icon: 'undo', action: () => gitBridge.openRecovery(repo.id) },
      { separator: true },
      {
        label: 'Copy repository path',
        icon: 'copy',
        action: () =>
          void copyTextOrThrow(repo.path)
            .then(() => toasts.success('Copied', repo.path))
            .catch(() => toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.')),
      },
    ];
  }
  /** The Git page's header draws the view switch (GitPage): what it shows. */
  export function viewState(): { tab: string; resolving: boolean; conflictFiles: number } {
    return { tab: effTab, resolving, conflictFiles: conflictSeed.files.length };
  }
  export function selectTab(id: string): void {
    resolving = false;
    onTab(id);
  }


</script>

<div class="repoview">
  <!-- In-progress merge banner (shown when not already in the resolver). -->
  {#if merging && !resolving}
    <div class="merge-banner">
      <Icon name="merge" size={13} />
      <span>
        {#if opLabel}
          A {opLabel} is in progress{#if conflictSeed.source}{' '}(<span class="mono">{conflictSeed.source}</span>){/if}.
        {:else}
          Conflicted files need resolution (e.g. from a stash pop).
        {/if}
        {#if conflictSeed.files.length > 0}
          {conflictSeed.files.length === 1 ? '1 file needs' : `${conflictSeed.files.length} files need`} resolution.
        {/if}
      </span>
      <span class="grow"></span>
      <button class="btn small primary" onclick={openResolver}>Resolve conflicts</button>
    </div>
  {/if}

  <div class="rv-body">
    {#if resolving}
      <LazyMount
        lazy={ConflictResolverLazy}
        what="the conflict resolver"
        props={{
          repoId: repo.id,
          initialFiles: conflictSeed.files,
          initialSource: conflictSeed.source,
          initialOp: mergeOp,
          onleave: leaveResolver,
        }}
      />
    {:else if effTab === 'prs'}
      {#if repo.forge === null && !repo.remote_url}
        <!-- A local-only repo: nothing to open a PR against. Say so and offer
             the fix, instead of a "provider unreachable" error whose Retry
             can never succeed. -->
        <EmptyState
          icon="pr"
          title="No remote yet"
          body="Pull requests need a remote on GitHub, Bitbucket Cloud or GitLab. Add one, then publish your branch."
          actionLabel="Add a remote…"
          actionIcon="globe"
          onaction={() => (remotesOpen = true)}
        />
      {:else if repo.forge === 'unrecognized'}
        <!-- Honest dead-end instead of a silent one: the remote host isn't a
             forge Otto can open PRs on (e.g. Bitbucket Server / Data Center). -->
        <EmptyState
          icon="pr"
          title="Pull requests aren’t available for {remoteHost(repo.remote_url)}"
          body="Otto supports GitHub, Bitbucket Cloud, and GitLab. This repository’s remote isn’t one of them, so there’s no pull request view here."
        />
      {:else}
        <div class="rv-prs">
          <LazyMount lazy={PrWorkspaceLazy} what="pull requests" props={{ repoId: repo.id }} />
        </div>
      {/if}
    {:else if effTab === 'review'}
      <div class="rv-tab-scroll">
        <LazyMount lazy={LocalReviewLazy} what="the local review" variant="panel" props={{ repoId: repo.id }} />
      </div>
    {:else if status}
      <!-- History: Branches · the graph · the Detail pane (it lands on the
           WIP row or the HEAD commit; a file's history / blame opens there). -->
      <GraphView
        repoId={repo.id}
        repoPath={repo.path}
        workspaceId={repo.workspace_id}
        {status}
        onstatus={setStatus}
        onmergerequest={requestMerge}
        onresolveconflicts={openResolver}
      />
    {:else if statusError}
      <LoadState
        what={repo.name}
        variant="page"
        error={`${statusError} If the folder moved or was deleted, remove the repository from the Git page and add it again.`}
        empty
        loading={statusLoading}
        onretry={() => loadStatus(repo.id, true)}
      />
    {:else}
      <div style="padding: 16px"><Skeleton rows={5} height={36} /></div>
    {/if}
  </div>
</div>

{#if remotesOpen}
  <LazyMount lazy={RemotesLazy} what="the remotes" quiet onerror={() => (remotesOpen = false)} props={{
    repoId: repo.id,
    onclose: () => {
      remotesOpen = false;
      // The repo record's remote/forge are derived from origin — re-read them
      // so an added or changed remote lights up the PR view without a reload.
      void git.loadAllRepos(true);
    },
  }} />
{/if}

{#if mergeReq}
  <LazyMount
    lazy={MergeApprovalLazy}
    what="the merge dialog"
    quiet
    props={{
      repoId: repo.id,
      source: mergeReq.source,
      target: mergeReq.target,
      onclose: () => (mergeReq = null),
      onmerged: onMerged,
      onconflicts: onConflicts,
    }}
  />
{/if}

<style>

  .repoview {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .merge-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    background: var(--warning-soft);
    border-bottom: 1px solid var(--border);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .merge-banner > :global(svg) {
    color: var(--warning);
    flex-shrink: 0;
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
  /* The PR workspace fills the pane (list | open PRs), each side scrolls. */
  .rv-prs {
    height: 100%;
    min-height: 0;
  }
  .rv-tab-scroll {
    height: 100%;
    overflow-y: auto;
    overscroll-behavior: contain;
    -webkit-overflow-scrolling: touch;
    padding: 0 14px;
    box-sizing: border-box;
  }
</style>

{#if gitBridge.recovery?.repoId === repo.id}
  {#key repo.id}
    <LazyMount lazy={RecoveryLazy} what="the recovery tools" quiet onerror={() => (gitBridge.recovery = null)} props={{
      repoId: repo.id, initialMode: gitBridge.recovery.mode, initialOnto: gitBridge.recovery.onto,
      onclose: () => { gitBridge.recovery = null; gitBridge.refreshGraph(repo.id); },
      onresolve: () => { gitBridge.recovery = null; resolving = true; } }} />
  {/key}
{/if}
