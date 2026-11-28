<script lang="ts">
  // GoLand-style Local Changes inside the IDE Git dock: a list pane (Staged and
  // Changes sections, the commit composer as its footer) beside Otto's real
  // `DiffViewer.svelte` (hunk-level stage/unstage/discard).
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ListPane from '../../../lib/components/side/ListPane.svelte';
  import ListDetail from '../../../lib/components/side/ListDetail.svelte';
  import SideSection from '../../../lib/components/side/SideSection.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { rowMenu as rowMenuAction } from '../../../lib/rowMenu';
  import { api, isAbortError } from '../../../lib/api/client';
  import type { DiffResp, FileChange, Repo, RepoStatusResp } from '../../../lib/api/types';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { plural } from '../../../lib/plural';
  import { git } from '../../../lib/stores/git.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import DiffViewer from '../../../lib/diff/DiffViewer.svelte';
  import { repoDiffFileLoader } from '../../../lib/diff/diff-load';
  import { ide, ideBasename } from '../ideStore.svelte';
  import { ideGit } from './ideGitController.svelte';

  interface Props {
    repo: Repo;
    status: RepoStatusResp | null;
    oncommitted?: () => void;
  }

  let { repo, status, oncommitted }: Props = $props();
  // Touch devices have no ⌘ to press: no shortcut hint in the commit box there.
  const coarsePointer = typeof window !== 'undefined' && Boolean(window.matchMedia?.('(pointer: coarse)').matches);

  // The shared list range and key; the old key is read once.
  let phoneOpen = $state(false);

  let selectedPath = $state<string | null>(null);
  let selectedSection = $state<'staged' | 'unstaged'>('unstaged');
  let diff = $state.raw<DiffResp | null>(null);
  let diffLoading = $state(false);
  let diffError = $state<string | null>(null);
  let diffRetry = $state(0);

  let commitMessage = $state('');
  let commitBoxEl = $state<HTMLTextAreaElement | undefined>(undefined);
  let amend = $state(false);
  let committing = $state(false);
  let busyAction = $state(false);

  const changes = $derived(status?.changes ?? []);
  const statusError = $derived(git.statusErrorById[repo.id] ?? null);
  const stagedList = $derived(changes.filter((c) => c.staged));
  const unstagedList = $derived(changes.filter((c) => c.unstaged || c.kind === 'untracked' || c.kind === 'conflicted'));

  // Honor external focus requests from `Git > Show Diff` / `Git > Commit File…`.
  $effect(() => {
    const reqPath = ideGit.focusedChangePath;
    if (reqPath && changes.some((c) => c.path === reqPath)) {
      selectedPath = reqPath;
      const ch = changes.find((c) => c.path === reqPath);
      if (ch) selectedSection = ch.staged && !ch.unstaged ? 'staged' : 'unstaged';
      ideGit.focusedChangePath = null;
    }
  });
  let lastCommitSeq = 0;
  $effect(() => {
    const seq = ideGit.focusCommitSeq;
    if (seq > lastCommitSeq) {
      lastCommitSeq = seq;
      commitBoxEl?.focus();
    }
  });

  // Auto-select the first changed file so the diff pane never opens blank when changes exist.
  $effect(() => {
    const all = changes;
    if (all.length === 0) {
      selectedPath = null;
      return;
    }
    if (selectedPath && all.some((c) => c.path === selectedPath)) return;
    const first = unstagedList[0] ?? stagedList[0] ?? all[0];
    if (first) {
      selectedPath = first.path;
      selectedSection = first.unstaged || first.kind === 'untracked' ? 'unstaged' : 'staged';
    }
  });

  const selTarget = $derived.by((): 'working' | 'worktree' | 'staged' => {
    const p = selectedPath;
    if (!p || !status) return 'working';
    const ch = status.changes.find((c) => c.path === p);
    if (!ch || ch.kind === 'untracked' || ch.kind === 'conflicted') return 'working';
    if (ch.staged && ch.unstaged) return selectedSection === 'staged' ? 'staged' : 'worktree';
    if (ch.staged) return 'staged';
    if (ch.unstaged) return 'worktree';
    return 'working';
  });

  // Fetch per-file diff via Otto's `/repos/:id/diff`, refreshing live on `git.liveRev`.
  $effect(() => {
    const repoId = repo.id;
    const path = selectedPath;
    const target = selTarget;
    void git.liveRev[repoId];
    void diffRetry;
    if (!path) {
      diff = null;
      diffError = null;
      diffLoading = false;
      return;
    }
    diffLoading = true;
    diffError = null;
    const ctl = new AbortController();
    api
      .get<DiffResp>(
        `/repos/${encodeURIComponent(repoId)}/diff?target=${target}&path=${encodeURIComponent(path)}`,
        ctl.signal,
      )
      .then((res) => {
        if (!ctl.signal.aborted) diff = res;
      })
      .catch((e) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        diff = { files: [] };
        diffError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) diffLoading = false;
      });
    return () => ctl.abort();
  });

  function badgeLetter(c: FileChange): string {
    if (c.kind === 'untracked') return '?';
    if (c.kind === 'added') return 'A';
    if (c.kind === 'deleted') return 'D';
    if (c.kind === 'renamed') return 'R';
    if (c.kind === 'conflicted') return '!';
    return 'M';
  }

  function badgeClass(c: FileChange): string {
    if (c.kind === 'added') return 'is-add';
    if (c.kind === 'deleted' || c.kind === 'conflicted') return 'is-del';
    if (c.kind === 'untracked') return 'is-untracked';
    return 'is-mod';
  }

  function openInEditor(relPath: string): void {
    const abs = `${repo.path.replace(/\/+$/, '')}/${relPath.replace(/^\/+/, '')}`;
    void ide.openFile(abs, { preview: false });
  }

  // A row's actions past its stage / unstage button (⋯, right-click, long-press).
  function rowMenu(c: FileChange, section: 'staged' | 'unstaged'): MenuItem[] {
    const items: MenuItem[] = [];
    if (c.kind !== 'deleted') items.push({ label: 'Open in Editor', icon: 'external', action: () => openInEditor(c.path) });
    items.push({ label: c.kind === 'untracked' ? 'Delete File…' : 'Discard Changes…', icon: 'undo', danger: true, disabled: busyAction, action: () => void confirmDiscard(c, section === 'staged') });
    return items;
  }

  async function stageFiles(paths: string[], stage: boolean): Promise<void> {
    if (paths.length === 0 || busyAction) return;
    busyAction = true;
    try {
      await ideGit.stagePaths(repo.id, paths, stage);
      diffRetry += 1;
    } catch (e) {
      toastError(stage ? 'Couldn’t stage files' : 'Couldn’t unstage files', e);
    } finally {
      busyAction = false;
    }
  }

  async function confirmDiscard(c: FileChange, fromStaged: boolean): Promise<void> {
    if (busyAction) return;
    busyAction = true;
    try {
      const isUntracked = c.kind === 'untracked';
      const keepStaged = !fromStaged && !isUntracked;
      const ok = await confirmer.ask(
        isUntracked
          ? `Delete untracked file “${c.path}”? Untracked files aren’t in git yet, so deleting it cannot be undone.`
          : keepStaged
            ? `Discard unstaged changes in “${c.path}”? Staged hunks are kept, and unstaged edits are reverted.`
            : `Discard all changes in “${c.path}” back to HEAD? This cannot be undone.`,
        {
          title: isUntracked ? 'Delete untracked file?' : 'Discard changes?',
          confirmLabel: isUntracked ? 'Delete' : 'Discard',
          danger: true,
        },
      );
      if (!ok) return;
      await ideGit.discardPaths(repo.id, [c.path], keepStaged);
      toasts.info(`Discarded ${ideBasename(c.path)}`);
      diffRetry += 1;
    } catch (e) {
      toastError('Couldn’t discard changes', e);
    } finally {
      busyAction = false;
    }
  }

  async function submitCommit(e?: Event): Promise<void> {
    e?.preventDefault();
    const msg = commitMessage.trim();
    if (!msg || committing || (stagedList.length === 0 && !amend)) return;
    committing = true;
    try {
      const res = await git.commit(repo.id, { message: msg, amend });
      commitMessage = '';
      amend = false;
      toasts.success('Committed', `${res.sha.slice(0, 7)} on ${res.status.branch}`);
      oncommitted?.();
    } catch (err) {
      toastError('Couldn’t create commit', err);
    } finally {
      committing = false;
    }
  }

  function onComposerKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void submitCommit();
    }
  }
</script>

{#snippet fileRow(c: FileChange, section: 'staged' | 'unstaged')}
  {@const isSel = selectedPath === c.path && selectedSection === section}
  {@const dirPart = c.path.includes('/') ? c.path.slice(0, c.path.lastIndexOf('/')) : ''}
  <div class="side-row reveal-host" class:selected={isSel}>
    <button
      type="button"
      class="side-row-main"
      data-side-row
      dir="ltr"
      aria-current={isSel ? 'true' : undefined}
      onclick={() => { selectedPath = c.path; selectedSection = section; phoneOpen = true; }}
      ondblclick={() => openInEditor(c.path)}
      oncontextmenu={(e) => { e.preventDefault(); ctxMenu.show(e, rowMenu(c, section)); }}
      use:rowMenuAction
      title={c.path}
    >
      <span class={`gc-status mono ${badgeClass(c)}`}>{badgeLetter(c)}</span>
      <span class="side-row-text">
        <span class="side-row-title"><span class={badgeClass(c)}>{ideBasename(c.path)}</span>{#if dirPart}<span class="gc-dir">{dirPart}</span>{/if}</span>
      </span>
    </button>
    {#if section === 'staged'}
      <button type="button" class="icon-btn" aria-label={`Unstage ${c.path}`} title="Unstage file" disabled={busyAction} onclick={() => void stageFiles([c.path], false)}><Icon name="minus" size={14} /></button>
    {:else}
      <button type="button" class="icon-btn" aria-label={`Stage ${c.path}`} title="Stage file" disabled={busyAction} onclick={() => void stageFiles([c.path], true)}><Icon name="plus" size={14} /></button>
    {/if}
    <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label={`${c.path} actions`} title="More" onclick={(e) => ctxMenu.showAt(e.currentTarget as HTMLElement, rowMenu(c, section), { align: 'end' })}><Icon name="more" size={14} /></button>
  </div>
{/snippet}

<div class="gc-split" data-testid="ide-git-changes-view">
  <!-- The changes are the list of a ListDetail nested in the tool window (the
       page's list is the explorer). A phone pushes: a tapped file shows its
       diff with "‹ Changes" in the page header (the one Back). -->
  <ListDetail module="ide" pane="gitChanges" name="Changes" open={phoneOpen} onback={() => (phoneOpen = false)} defaultWidth={320} legacy={{ w: 'otto.ide.gitChangesW' }}>
  {#snippet list()}
    <ListPane
      count={status ? changes.length : null}
      loading={!status && !statusError}
      error={!status ? statusError : null}
      onretry={() => void ideGit.refreshActive()}
    >
      {#if statusError}
        <LoadState what="working tree status" error={statusError} empty={true} onretry={() => void ideGit.refreshActive()} variant="compact" />
      {/if}
      {#if status && changes.length === 0}
        <EmptyState icon="check" title="Working tree clean" body={`No uncommitted changes on ${status.branch || 'HEAD'}.`} headingLevel={3} />
      {:else if status}
        {#if stagedList.length > 0}
          <SideSection title="Staged" count={stagedList.length}>
            {#snippet actions()}
              <button type="button" class="btn small ghost gc-mini-btn" disabled={busyAction} onclick={() => void stageFiles(stagedList.map((c) => c.path), false)} data-testid="ide-git-unstage-all">Unstage all</button>
            {/snippet}
            {#each stagedList as c (`s:${c.path}`)}
              {@render fileRow(c, 'staged')}
            {/each}
          </SideSection>
        {/if}
        <SideSection title="Changes" count={unstagedList.length}>
          {#snippet actions()}
            {#if unstagedList.length > 0}
              <button type="button" class="btn small ghost gc-mini-btn" disabled={busyAction} onclick={() => void stageFiles(unstagedList.filter((c) => c.kind !== 'conflicted').map((c) => c.path), true)} data-testid="ide-git-stage-all">Stage all</button>
            {/if}
          {/snippet}
          {#each unstagedList as c (`u:${c.path}`)}
            {@render fileRow(c, 'unstaged')}
          {/each}
        </SideSection>
      {/if}
      {#snippet footer()}
        <form class="gc-composer" onsubmit={(e) => void submitCommit(e)}>
          <textarea
            bind:this={commitBoxEl}
            class="input gc-msg"
            rows={2}
            dir="auto"
            aria-label="Commit message"
            placeholder={coarsePointer ? 'Commit message…' : 'Commit message (⌘Enter to commit)…'}
            bind:value={commitMessage}
            onkeydown={onComposerKeydown}
            data-testid="ide-git-commit-input"
          ></textarea>
          <div class="gc-composer-bar">
            <label class="gc-amend">
              <input type="checkbox" bind:checked={amend} />
              <span>Amend</span>
            </label>
            <span class="grow"></span>
            <button
              type="submit"
              class="btn small primary"
              disabled={committing || !commitMessage.trim() || (stagedList.length === 0 && !amend)}
              title={stagedList.length === 0 && !amend ? 'Stage at least one file to commit' : `Commit ${plural(stagedList.length, 'staged file')}`}
              data-testid="ide-git-commit-btn"
            >
              <Icon name="commit" size={12} />
              <span>{committing ? 'Committing…' : stagedList.length > 0 ? `Commit (${stagedList.length})` : 'Commit'}</span>
            </button>
          </div>
        </form>
      {/snippet}
    </ListPane>
  {/snippet}

  <section class="gc-right" aria-label="Selected file diff">
    {#if !selectedPath}
      <div class="gc-empty">
        <p class="dim">Select a modified file to inspect its diff.</p>
      </div>
    {:else}
      <header class="gc-diff-bar">
        <span class="mono gc-diff-path ellipsis" dir="ltr" title={selectedPath}>{selectedPath}</span>
        <span class="gc-target-label dim">{selTarget === 'staged' ? 'staged' : 'working tree'}</span>
        <span class="grow"></span>
        <button type="button" class="btn small ghost gc-mini-btn" onclick={() => selectedPath && openInEditor(selectedPath)}>
          <Icon name="external" size={12} /><span>Open in editor</span>
        </button>
      </header>
      <div class="gc-diff-scroll">
        {#if diffError}
          <div class="gc-pad">
            <LoadState what="file diff" error={diffError} empty={true} onretry={() => (diffRetry += 1)} variant="compact" />
          </div>
        {:else if diffLoading && !diff}
          <div class="gc-pad"><LoadState what="file diff" loading={true} variant="compact" rows={4} /></div>
        {:else if diff && diff.files.length > 0}
          <DiffViewer
            {diff}
            repoId={repo.id}
            loadFile={repoDiffFileLoader(repo.id, selTarget)}
            stateKey={`${repo.id}:${selTarget}:${selectedPath}`}
            wip={selTarget === 'worktree' || selTarget === 'staged'
              ? {
                  target: selTarget,
                  onapplied: (r) => {
                    ideGit.applyStatus(repo.id, r.status);
                    diff = r.diff;
                    if (selTarget === 'worktree' && selectedPath) {
                      void ideGit.reloadIdeTabIfOpen(
                        `${repo.path.replace(/\/+$/, '')}/${selectedPath.replace(/^\/+/, '')}`,
                        { onlyIfChangedOnDisk: true },
                      );
                    }
                  },
                }
              : undefined}
          />
        {:else}
          <div class="gc-empty">
            <p class="dim">No textual diff lines (new empty, binary, or mode-only change).</p>
          </div>
        {/if}
      </div>
    {/if}
  </section>
  </ListDetail>
</div>

<style>
  .gc-split { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; overflow: hidden; background: var(--surface); }
  .gc-pad { padding: 12px; }
  .gc-empty { flex: 1; display: flex; align-items: center; justify-content: center; padding: 16px; font-size: var(--fs-xs); }
  .gc-mini-btn { block-size: 22px; padding-block: 0; }
  .gc-status { inline-size: 14px; text-align: center; font-size: var(--fs-xs); font-weight: 600; flex-shrink: 0; }
  .is-mod { color: var(--info); }
  .is-add { color: var(--success); }
  .is-del { color: var(--danger); }
  .is-untracked { color: var(--warning); }
  .gc-dir { margin-inline-start: 6px; font-size: var(--fs-xs); color: var(--text-dim); font-weight: 400; }
  .gc-composer { flex: 1; display: flex; flex-direction: column; gap: 6px; padding-block: 8px; }
  .gc-msg { resize: none; font-size: var(--fs-xs); min-block-size: 44px; padding: 6px 8px; }
  .gc-composer-bar { display: flex; align-items: center; gap: 8px; }
  .gc-amend { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--text-dim); cursor: pointer; }
  .gc-right { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; overflow: hidden; }
  .gc-diff-bar {
    display: flex; align-items: center; gap: 8px; min-block-size: 28px;
    padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0;
  }
  .gc-diff-path { font-size: var(--fs-xs); font-weight: 500; }
  .gc-target-label { font-size: var(--fs-xs); flex-shrink: 0; }
  .gc-diff-scroll { flex: 1; min-block-size: 0; min-inline-size: 0; overflow: auto; }
  /* The one layout control is the diff's own Unified | Side by side. A narrow dock scrolls
     the diff sideways rather than wrapping a hunk header (`@@ -1,6 +1,3 @@`) over 4 lines. */
  .gc-diff-scroll :global(.hunk-header) { white-space: nowrap; }
  .gc-diff-bar .gc-mini-btn { flex-shrink: 0; white-space: nowrap; }
</style>
