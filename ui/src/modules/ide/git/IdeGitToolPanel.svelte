<script lang="ts">
  // Integrated Git tool window inside the IDE bottom dock (`Local Changes` |
  // `Log` | `File History / Blame` + closable `History: <filename> ×` tabs),
  // powered directly by Otto's `git` store and shared `DiffViewer` / `BlamePanel`.
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { isBroadNonProjectRootClient } from '../../../lib/fileUri';
  import { router } from '../../../lib/router.svelte';
  import { git } from '../../../lib/stores/git.svelte';
  import { ide, ideBasename } from '../ideStore.svelte';
  import IdeGitChangesView from './IdeGitChangesView.svelte';
  import { ideGit } from './ideGitController.svelte';
  import IdeGitLogView from './IdeGitLogView.svelte';
  import {
    gitFetchRepo,
    gitOpenBranchesMenu,
    gitPullRepo,
    gitPushRepo,
  } from './ideGitQuickMenu';

  let refreshTick = $state(0);
  let busyRefresh = $state(false);
  let branchBtnEl = $state<HTMLButtonElement | undefined>(undefined);

  $effect(() => {
    void ide.projectRoots.length;
    void ide.treeRootEntries.length;
    void ide.activePath;
    ideGit.syncVisibleRoots({ probeAll: false });
    const root = ideGit.activeRoot;
    if (root) void ideGit.ensureRootDetected(root.path);
  });

  const activeRepo = $derived(ideGit.activeRepo);
  const activeStatus = $derived(ideGit.activeStatus);
  const changeCount = $derived(activeStatus?.changes.length ?? 0);
  const activeFileBase = $derived(ide.activePath ? ideBasename(ide.activePath) : null);
  const subTab = $derived(ideGit.activeSubTab);
  const activeHistTab = $derived(ideGit.historyTabs.find((t) => t.key === subTab) ?? null);
  const histTabRepo = $derived.by(() => {
    if (!activeHistTab) return activeRepo;
    return ideGit.repos.find((r) => r.id === activeHistTab.repoId) ?? activeRepo;
  });

  async function refreshNow(): Promise<void> {
    if (busyRefresh) return;
    busyRefresh = true;
    try {
      await ideGit.refreshActive();
      refreshTick += 1;
    } finally {
      busyRefresh = false;
    }
  }

  async function openInGitModule(): Promise<void> {
    const targetPath = ide.activePath || ideGit.activeRoot?.path || ide.target.root;
    const repo = activeRepo ?? (targetPath ? await ideGit.resolveRepoForPath(targetPath) : null);
    if (repo && !isBroadNonProjectRootClient(repo.path)) {
      git.upsertRepo(repo);
      git.openRepoTab(repo.id, 'graph');
    }
    router.go('git');
  }
</script>

<div class="ig-root" data-testid="ide-git-panel">
  <header class="ig-bar">
    {#if ideGit.repos.length > 1}
      <select
        class="ig-repo-select"
        dir="ltr"
        aria-label="Select repository"
        value={activeRepo?.id ?? ''}
        onchange={(e) => ideGit.selectRepo((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-git-repo-select"
      >
        {#each ideGit.repos as repoOpt (repoOpt.id)}
          <option value={repoOpt.id}>{repoOpt.name}</option>
        {/each}
      </select>
    {:else if ide.projectRoots.length > 1}
      <select
        class="ig-repo-select"
        dir="ltr"
        aria-label="Select repository"
        value={ideGit.activeRoot?.path ?? ''}
        onchange={(e) => ideGit.selectRoot((e.currentTarget as HTMLSelectElement).value)}
        data-testid="ide-git-repo-select"
      >
        {#each ide.projectRoots as root (root.path)}
          <option value={root.path}>{root.name}</option>
        {/each}
      </select>
    {:else if activeRepo}
      <span class="ig-repo-label mono ellipsis" dir="ltr" title={activeRepo.path}>
        {activeRepo.name}
      </span>
    {/if}

    {#if activeStatus?.branch && activeRepo}
      <button
        bind:this={branchBtnEl}
        type="button"
        class="ig-branch"
        dir="ltr"
        title={activeStatus.upstream ? `Branch ${activeStatus.branch} (tracking ${activeStatus.upstream}) — click to switch branches` : `Branch ${activeStatus.branch} — click to switch branches`}
        onclick={() => branchBtnEl && void gitOpenBranchesMenu(activeRepo.path, branchBtnEl)}
        data-testid="ide-git-branch-btn"
      >
        <Icon name="branch" size={12} />
        <span class="mono ellipsis">{activeStatus.branch}</span>
        {#if activeStatus.ahead > 0}<span class="mono ig-sync">↑{activeStatus.ahead}</span>{/if}
        {#if activeStatus.behind > 0}<span class="mono ig-sync">↓{activeStatus.behind}</span>{/if}
      </button>
    {/if}

    <div class="segmented ig-tabs" role="group" aria-label="Git tool views">
      <button
        type="button"
        class:active={subTab === 'changes'}
        aria-pressed={subTab === 'changes'}
        onclick={() => (ideGit.activeSubTab = 'changes')}
        data-testid="ide-git-tab-changes"
      >
        <span>Local Changes</span>
        {#if changeCount > 0}
          <Badge tone="info" label={String(changeCount)} />
        {/if}
      </button>
      <button
        type="button"
        class:active={subTab === 'log'}
        aria-pressed={subTab === 'log'}
        onclick={() => (ideGit.activeSubTab = 'log')}
        data-testid="ide-git-tab-log"
      >
        Log
      </button>
      <button
        type="button"
        class:active={subTab === 'file'}
        aria-pressed={subTab === 'file'}
        onclick={() => (ideGit.activeSubTab = 'file')}
        data-testid="ide-git-tab-file-history"
      >
        <span>File History / Blame</span>
        {#if activeFileBase}
          <span class="dim mono">· {activeFileBase}</span>
        {/if}
      </button>
    </div>

    {#if ideGit.historyTabs.length > 0}
      <div class="ig-hist-tabs" role="group" aria-label="Pinned file history tabs">
        {#each ideGit.historyTabs as tab (tab.key)}
          <div class="ig-hist-pill" class:active={subTab === tab.key} data-testid="ide-git-history-tab">
            <button
              type="button"
              class="ig-hist-select"
              dir="ltr"
              title={tab.relPath}
              onclick={() => (ideGit.activeSubTab = tab.key)}
            >
              <Icon name="clock" size={12} />
              <span class="ellipsis">History: {tab.name}</span>
            </button>
            <button
              type="button"
              class="ig-hist-close"
              aria-label="Close history tab for {tab.name}"
              title="Close history tab"
              onclick={() => ideGit.closeFileHistoryTab(tab.key)}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>
    {/if}

    <span class="grow"></span>

    {#if activeRepo}
      <button
        type="button"
        class="ig-act"
        aria-label="Fetch from remote"
        title="Fetch from remote"
        onclick={() => void gitFetchRepo(activeRepo.path)}
        data-testid="ide-git-fetch-btn"
      >
        <Icon name="fetch" size={12} />
      </button>
      <button
        type="button"
        class="ig-act"
        aria-label="Pull changes"
        title="Pull from remote"
        onclick={() => void gitPullRepo(activeRepo.path)}
        data-testid="ide-git-pull-btn"
      >
        <Icon name="arrowDown" size={12} />
      </button>
      <button
        type="button"
        class="ig-act"
        aria-label="Push commits"
        title="Push commits to remote…"
        onclick={() => void gitPushRepo(activeRepo.path)}
        data-testid="ide-git-push-btn"
      >
        <Icon name="arrowUp" size={12} />
      </button>
    {/if}

    <button
      type="button"
      class="ig-act"
      aria-label="Refresh Git status"
      title="Refresh Git status"
      disabled={busyRefresh}
      onclick={() => void refreshNow()}
      data-testid="ide-git-refresh"
    >
      <Icon name="refresh" size={12} />
    </button>

    <button
      type="button"
      class="btn small ghost ig-open-mod"
      onclick={() => void openInGitModule()}
      data-testid="ide-git-open-module"
    >
      <Icon name="external" size={12} />
      <span>Open in Git Module</span>
    </button>
  </header>

  <div class="ig-body">
    {#if ide.target.kind !== 'local'}
      <EmptyState
        icon="branch"
        title="Local Git repositories only"
        body="Integrated Git views operate on local project folders. Use the integrated SSH terminal for remote repository commands."
        headingLevel={2}
      />
    {:else if !activeRepo}
      <EmptyState
        icon="branch"
        title={ideGit.detectingCount > 0 ? 'Detecting Git repository…' : 'Not a Git repository'}
        body={ideGit.detectingCount > 0
          ? 'Resolving the active project folder against Otto’s repository store.'
          : 'Initialize Git in this folder or select a Git-backed repository from the project switcher.'}
        actionLabel="Detect repository"
        actionIcon="refresh"
        actionKind="secondary"
        onaction={() => void refreshNow()}
        headingLevel={2}
      />
    {:else if activeHistTab && histTabRepo}
      <IdeGitLogView
        repo={histTabRepo}
        mode="file"
        explicitRelPath={activeHistTab.relPath}
        explicitAbsPath={activeHistTab.absPath}
        initialFileMode={activeHistTab.initialMode}
        modeSeq={activeHistTab.modeSeq}
        {refreshTick}
      />
    {:else if subTab === 'changes'}
      <IdeGitChangesView repo={activeRepo} status={activeStatus} />
    {:else if subTab === 'log'}
      <IdeGitLogView repo={activeRepo} mode="log" {refreshTick} />
    {:else}
      <IdeGitLogView repo={activeRepo} mode="file" {refreshTick} />
    {/if}
  </div>
</div>

<style>
  .ig-root { display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; overflow: hidden; background: var(--surface); }
  .ig-bar {
    display: flex; align-items: center; gap: 6px;
    block-size: 32px; min-block-size: 32px; max-block-size: 32px;
    padding-inline: 10px; border-block-end: 1px solid var(--border);
    background: var(--surface); flex-shrink: 0;
    overflow-x: auto; overflow-y: hidden; scrollbar-width: none;
  }
  .ig-bar::-webkit-scrollbar { display: none; }
  .ig-repo-select {
    max-inline-size: 150px; block-size: 22px; padding-inline: 6px;
    font-size: var(--fs-xs); font-weight: 500; color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: var(--radius-s); flex-shrink: 0;
  }
  .ig-repo-label { font-size: var(--fs-xs); font-weight: 600; color: var(--text); max-inline-size: 150px; flex-shrink: 0; }
  .ig-branch {
    display: inline-flex; align-items: center; gap: 4px;
    block-size: 22px; padding-inline: 8px; border: 0;
    font-size: var(--fs-xs); color: var(--accent-text);
    background: var(--accent-soft); border-radius: var(--radius-s);
    max-inline-size: 170px; flex-shrink: 0; cursor: pointer;
  }
  .ig-branch:hover { background: var(--hover); }
  .ig-sync { font-size: var(--fs-xs); color: var(--text-dim); }
  .ig-tabs { flex-shrink: 0; }
  .ig-tabs button { display: inline-flex; align-items: center; gap: 4px; block-size: 22px; padding-inline: 8px; font-size: var(--fs-xs); }
  .ig-hist-tabs { display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0; }
  .ig-hist-pill {
    display: inline-flex; align-items: center; block-size: 22px;
    border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--surface-2); color: var(--text-dim);
  }
  .ig-hist-pill.active { background: var(--accent-soft); color: var(--accent-text); border-color: var(--accent-text); }
  .ig-hist-select {
    display: inline-flex; align-items: center; gap: 4px;
    block-size: 20px; padding-inline: 6px 4px; border: 0; background: transparent;
    color: inherit; font-size: var(--fs-xs); max-inline-size: 180px; cursor: pointer;
  }
  .ig-hist-close {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 18px; block-size: 20px; border: 0; background: transparent;
    color: inherit; cursor: pointer; border-radius: var(--radius-s);
  }
  .ig-hist-close:hover { background: var(--hover); color: var(--text); }
  .ig-act {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 22px; block-size: 22px; border: 0; border-radius: var(--radius-s);
    background: transparent; color: var(--text-dim); cursor: pointer; flex-shrink: 0;
  }
  .ig-act:hover:not(:disabled) { background: var(--hover); color: var(--text); }
  .ig-act:disabled { opacity: var(--disabled-opacity); cursor: default; }
  .ig-open-mod { block-size: 22px; padding-block: 0; flex-shrink: 0; }
  .ig-body { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; overflow: hidden; }
</style>
