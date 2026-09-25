<script lang="ts">
  // One repo: toolbar header + tabs (Graph / Pull Requests / Review). Staging
  // and history both live on the graph now (WIP row + detail panel), so there
  // are no separate Changes/History tabs.
  import type { GitOpInProgress, MergeResult, Repo, RepoStatusResp } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import GitToolbar from './GitToolbar.svelte';
  import GraphView from './GraphView.svelte';
  import GraphSearchBar from './GraphSearchBar.svelte';
  import BlamePanel from './BlamePanel.svelte';
  import FileHistoryPanel from './FileHistoryPanel.svelte';
  import RemotesPanel from './RemotesPanel.svelte';
  import RecoveryTools from './RecoveryTools.svelte';
  import { gitBridge } from './gitBridge.svelte';
  import FocusView from './FocusView.svelte';
  import PrList from './PrList.svelte';
  import LocalReviewPanel from './LocalReviewPanel.svelte';
  import MergeApprovalModal from './MergeApprovalModal.svelte';
  import ConflictResolverView from './ConflictResolverView.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    repo: Repo;
    tab: string; // graph | prs | review (legacy changes/history render the graph)
    /** Embedded in the agent-mode right panel: switch tabs via `onTab` (local
     *  state) instead of routing, and hide the "← Repos" back button. */
    embedded?: boolean;
    onTab?: (tab: string) => void;
    /** Git page (tab-store owned navigation): open another repo as a tab. The
     *  route-based fallback below is a no-op there once the route is `#/git`. */
    onopenrepo?: (repoId: string) => void;
    /** Git page: open its Add Repository sheet in the given mode. */
    onaddrepo?: (mode: 'register' | 'clone' | 'browse') => void;
  }
  let { repo, tab, embedded = false, onTab, onopenrepo, onaddrepo }: Props = $props();

  // Legacy deep links / persisted state may still say 'changes' or 'history';
  // both render the graph now, and the Graph tab must read as active for them.
  const effTab = $derived(tab === 'changes' || tab === 'history' ? 'graph' : tab);

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
  function loadStatus(id: string): void {
    statusLoading = true;
    void git.refreshStatus(id).finally(() => {
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
  $effect(() => {
    const id = repo.id;
    const seen = conflictKey;
    void git
      .getMergeStatus(id)
      .then((m) => {
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
    void git.refreshStatus(repo.id);
  }

  // Bumping this key re-mounts GraphView so its refs/commits effect re-runs
  // after a merge changes history.
  let graphKey = $state(0);

  // Remote CRUD lives behind a header button + modal (the branch bar is a
  // different owner's surface, and remotes are a repo-level setting, not a
  // per-branch one).
  let remotesOpen = $state(false);

  /** ⋯ in the toolbar row: the repo-level tools that don't earn a permanent
   *  button (remotes are a setting; recovery is rare and deliberate). */
  function openMoreMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Remotes…', icon: 'globe', action: () => (remotesOpen = true) },
      { label: 'Recovery tools…', icon: 'undo', action: () => gitBridge.openRecovery(repo.id) },
      { separator: true },
      {
        label: 'Copy repository path',
        icon: 'copy',
        action: () =>
          void copyTextOrThrow(repo.path)
            .then(() => toasts.success('Copied', repo.path))
            .catch(() => toasts.error('Copy failed', 'The clipboard is unavailable.')),
      },
    ]);
  }

  // History / blame open as a right-side drawer over the graph. The request
  // comes through `gitBridge` from whichever diff header asked for it, so a
  // stale request for ANOTHER repo never renders here.
  const fileTool = $derived(
    gitBridge.fileTool?.repoId === repo.id ? gitBridge.fileTool : null,
  );

  /** Host of a remote URL for the unsupported-forge message — handles
   *  https://, ssh:// and scp-like (git@host:path) forms (mirrors the daemon's
   *  detect.rs split). */
  function remoteHost(url: string | null): string {
    if (!url) return 'this remote';
    const u = url.trim();
    for (const scheme of ['https://', 'http://', 'ssh://', 'git://']) {
      if (u.startsWith(scheme)) {
        const rest = u.slice(scheme.length).replace(/^[^@/]+@/, '');
        return rest.split('/')[0]?.split(':')[0] || 'this remote';
      }
    }
    const scp = u.match(/^(?:[^@/]+@)?([^:/]+):/);
    return scp?.[1] ?? 'this remote';
  }

  const tabs = [
    { id: 'graph', label: 'Graph' },
    { id: 'prs', label: 'Pull requests' },
    { id: 'review', label: 'Review' },
    // Cross-repo: "my PRs" + "my Jira work" — not scoped to this repo, but it
    // lives here because the git page is where you think about this.
    { id: 'focus', label: 'Focus' },
  ];

  function selectTab(id: string): void {
    resolving = false;
    if (embedded) onTab?.(id);
    else router.go(`git/${repo.id}/${id}`);
  }

  /** ←/→ (Home/End) move between the view tabs, like any tablist. */
  function onTabKey(e: KeyboardEvent): void {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
    const list = (e.currentTarget as HTMLElement).closest<HTMLElement>('[role="tablist"]');
    if (!list) return;
    const btns = [...list.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    e.preventDefault();
    const rtl = getComputedStyle(list).direction === 'rtl';
    const fwd = e.key === (rtl ? 'ArrowLeft' : 'ArrowRight');
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? btns.length - 1 : (i + (fwd ? 1 : -1) + btns.length) % btns.length;
    btns[next].focus();
    btns[next].click();
  }

  // Repo switcher: jump between repositories without going back to the list.
  // Prefer the workspace-independent global list (Git page); fall back to the
  // per-workspace list when embedded in the agent right panel (global not loaded).
  const repoPool = $derived(git.allRepos.length ? git.allRepos : git.repos);
  function openRepoSwitcher(e: MouseEvent): void {
    const others = repoPool.filter((r) => r.id !== repo.id);
    // On the Git page the tab store owns navigation, so `router.go('git')` is a
    // no-op there (the route already IS `#/git`) — "Add repository…" / "All
    // repositories…" used to do nothing. Use the page's own callbacks when given.
    const tail = onaddrepo
      ? [
          { label: 'Clone a repository…', icon: 'download', pinned: true, action: () => onaddrepo('clone') },
          { label: 'Add a local repository…', icon: 'folder', pinned: true, action: () => onaddrepo('register') },
        ]
      : [
          { label: 'Add repository…', icon: 'plus', action: () => router.go('git') },
          { label: 'All repositories…', icon: 'folder', action: () => router.go('git') },
        ];
    ctxMenu.show(
      e,
      [
        ...others.map((r) => ({
          label: r.name,
          icon: 'branch',
          action: () => (onopenrepo ? onopenrepo(r.id) : router.go(`git/${r.id}/${tab}`)),
        })),
        ...(others.length > 0 ? [{ separator: true }] : []),
        ...tail,
      ],
      others.length > 8 ? { filter: true, filterPlaceholder: 'Search repositories…', maxVisible: 12 } : undefined,
    );
  }
</script>

<div class="repoview">
  <!-- ONE toolbar row: the view switcher, then the git verbs, then ⋯ for the
       repo-level tools. On the Git page the repo tab already names the repo, so
       the switcher only shows where there are no tabs (the agent right panel).
       It wraps (never overflows) when a long branch name or a narrow pane
       doesn't leave room. -->
  <header class="rv-head">
    {#if !embedded}
      <button class="btn ghost small" onclick={() => router.go('git')}><span class="rv-back-arrow" aria-hidden="true"><Icon name="chevronLeft" size={12} /></span> Repositories</button>
    {/if}
    {#if !onopenrepo}
      <button
        class="rv-name rv-switch"
        title="{repo.name} · {repo.path} — switch repository{repoPool.length > 1 ? ` (${repoPool.length} registered)` : ''}"
        onclick={openRepoSwitcher}
        oncontextmenu={openRepoSwitcher}
      >
        <Icon name="branch" size={14} />
        <span class="rv-name-text">{repo.name}</span>
        {#if repoPool.length > 1}<span class="rv-count">{repoPool.length}</span>{/if}
        <Icon name="chevronDown" size={12} />
      </button>
    {/if}
    <div class="rv-tabs segmented" role="tablist" aria-label="{repo.name} views">
      {#each tabs as t (t.id)}
        <button
          class="rv-tab"
          role="tab"
          aria-selected={effTab === t.id && !resolving}
          tabindex={effTab === t.id && !resolving ? 0 : -1}
          class:active={effTab === t.id && !resolving}
          onclick={() => selectTab(t.id)}
          onkeydown={onTabKey}
        >
          {t.label}
          {#if t.id === 'graph' && status && status.changes.length > 0}
            <span class="count" title="{status.changes.length} uncommitted change{status.changes.length === 1 ? '' : 's'} (WIP)">{status.changes.length}</span>
          {/if}
          {#if t.id === 'graph' && conflictedPaths.length > 0}
            <span class="count conflict-count" title="{conflictedPaths.length} conflicted file{conflictedPaths.length === 1 ? '' : 's'}"><Icon name="warning" size={12} />{conflictedPaths.length}</span>
          {/if}
        </button>
      {/each}
      <!-- The resolver is a view of its own while it's open (the banner below is
           the way in), so it only takes a tab slot then. -->
      {#if resolving}
        <button class="rv-tab conflict-tab active" role="tab" aria-selected="true" tabindex="0" onkeydown={onTabKey}>
          <Icon name="merge" size={12} />
          Resolve conflicts
          {#if conflictSeed.files.length > 0}
            <span class="count conflict-count">{conflictSeed.files.length}</span>
          {/if}
        </button>
      {/if}
    </div>
    <span class="grow"></span>
    {#if status}
      <GitToolbar repoId={repo.id} {status} onstatus={setStatus} onrefresh={() => graphKey++} />
    {:else if !statusError}
      <div class="toolbar-skeleton" aria-label="Loading repository status"></div>
    {/if}
    <button
      class="icon-btn rv-more"
      title="More repository actions"
      aria-label="More repository actions"
      aria-haspopup="menu"
      onclick={openMoreMenu}
    ><Icon name="more" size={16} /></button>
  </header>

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
      <ConflictResolverView
        repoId={repo.id}
        initialFiles={conflictSeed.files}
        initialSource={conflictSeed.source}
        initialOp={mergeOp}
        onleave={leaveResolver}
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
        <PrList repoId={repo.id} />
      {/if}
    {:else if effTab === 'review'}
      <div class="rv-tab-scroll">
        <LocalReviewPanel repoId={repo.id} />
      </div>
    {:else if effTab === 'focus'}
      <FocusView />
    {:else if status}
      <!-- Graph is the default for 'graph' AND any legacy tab id (changes /
           history) still living in persisted state or old deep links. -->
      <div class="rv-graph">
        <GraphSearchBar repoId={repo.id} />
        <div class="rv-graph-body">
          {#key graphKey}
            <GraphView
              repoId={repo.id}
              repoPath={repo.path}
              workspaceId={repo.workspace_id}
              {status}
              onstatus={setStatus}
              onmergerequest={requestMerge}
              onresolveconflicts={openResolver}
            />
          {/key}
          {#if fileTool}
            <aside class="rv-drawer">
              {#if fileTool.kind === 'history'}
                <FileHistoryPanel
                  repoId={repo.id}
                  path={fileTool.path}
                  onclose={() => gitBridge.closeFileTool()}
                />
              {:else}
                <BlamePanel
                  repoId={repo.id}
                  path={fileTool.path}
                  rev={fileTool.rev}
                  onclose={() => gitBridge.closeFileTool()}
                />
              {/if}
            </aside>
          {/if}
        </div>
      </div>
    {:else if statusError}
      <LoadState
        what={repo.name}
        variant="page"
        error={`${statusError} If the folder moved or was deleted, remove the repository from the Git page and add it again.`}
        empty
        loading={statusLoading}
        onretry={() => loadStatus(repo.id)}
      />
    {:else}
      <div style="padding: 16px"><Skeleton rows={5} height={36} /></div>
    {/if}
  </div>
</div>

{#if remotesOpen}
  <RemotesPanel
    repoId={repo.id}
    onclose={() => {
      remotesOpen = false;
      // The repo record's remote/forge are derived from origin — re-read them
      // so an added or changed remote lights up the PR view without a reload.
      void git.loadAllRepos(true);
    }}
  />
{/if}

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
  /* The one toolbar row under the page header. Wraps rather than overflowing:
     on a narrow window (or a long branch name) the verbs drop to a second line
     that keeps to the trailing edge, instead of Stash / Pop running off-screen. */
  .rv-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 10px;
    min-height: 46px;
    padding: 7px 14px;
    box-sizing: border-box;
    border-bottom: 1px solid var(--border);
  }
  .rv-head :global(.toolbar) {
    margin-inline-start: auto;
    min-width: 0;
  }
  .rv-more {
    flex-shrink: 0;
  }
  .rv-name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 320px;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .rv-name > :global(svg),
  .rv-name .rv-count {
    flex-shrink: 0;
  }
  .rv-name-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 0 5px;
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  /* View switcher: the shared segmented control (app.css), sized for the
     toolbar row. */
  .rv-tabs {
    flex-shrink: 0;
    max-width: 100%;
  }
  .rv-tabs > .rv-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    white-space: nowrap;
  }
  .rv-tabs > .rv-tab:hover:not(.active) {
    color: var(--text);
  }
  .rv-tabs > .rv-tab.active {
    font-weight: 500;
  }
  .count {
    font-size: var(--fs-xs);
    font-weight: 600;
    min-width: 16px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent-text);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
  }
  .rv-tabs > .conflict-tab,
  .rv-tabs > .conflict-tab.active {
    color: var(--warning);
    gap: 5px;
  }
  .conflict-count {
    background: var(--warning-soft);
    color: var(--warning);
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
  .rv-graph {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .rv-graph-body {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  /* History / blame slide over the graph rather than squeezing it — the graph
     keeps its lane layout, and the drawer never exceeds the viewport width. */
  .rv-drawer {
    position: absolute;
    inset-block: 0;
    inset-inline-end: 0;
    width: min(520px, 90vw);
    z-index: 6;
    background: var(--surface);
    box-shadow: var(--shadow);
  }
  /* The back arrow is a literal "←"; mirror it under RTL so it points the
     direction "back" actually goes (→) instead of always pointing left. */
  .rv-back-arrow {
    display: inline-flex;
  }
  :global([dir='rtl']) .rv-back-arrow {
    transform: scaleX(-1);
  }
  .rv-tab-scroll {
    height: 100%;
    overflow-y: auto;
    overscroll-behavior: contain;
    -webkit-overflow-scrolling: touch;
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

  /* ── Mobile + tablet (≤1024px): keep the header + toolbar + tabs usable on a
     narrow / short screen — the toolbar scrolls horizontally instead of
     overflowing the page, the tab strip scrolls, and tab labels stay legible.
     Matched to the body breakpoint (1024) so the tablet/landscape range gets the
     same scroll-don't-overflow header treatment. ── */
  @media (max-width: 1024px) {
    .rv-head {
      gap: 8px;
      padding: 8px 10px;
    }
    .rv-name { font-size: var(--fs-l); flex-shrink: 0; }
    /* The view switcher gets its own full-width line (scrolling if it must);
       the toolbar + ⋯ share the next line, the toolbar scrolling
       horizontally so it never forces the page wider than the viewport. */
    .rv-tabs {
      flex: 1 1 100%;
      overflow-x: auto;
      scrollbar-width: none;
    }
    .rv-tabs::-webkit-scrollbar { display: none; }
    .rv-tabs > .rv-tab {
      flex: 1 0 auto;
      justify-content: center;
      height: 34px;
      font-size: var(--fs-m);
    }
    .rv-head .grow { display: none; }
    .rv-head :global(.toolbar) {
      flex: 1 1 0;
      margin-inline-start: 0;
      overflow-x: auto;
      scrollbar-width: none;
    }
    .rv-head :global(.toolbar)::-webkit-scrollbar { display: none; }
    .rv-more {
      min-width: 36px;
      min-height: 36px;
    }
  }
</style>

{#if gitBridge.recovery?.repoId === repo.id}
  {#key repo.id}
    <RecoveryTools repoId={repo.id} initialMode={gitBridge.recovery.mode} initialOnto={gitBridge.recovery.onto}
      onclose={() => { gitBridge.recovery = null; graphKey++; }}
      onresolve={() => { gitBridge.recovery = null; resolving = true; }} />
  {/key}
{/if}
