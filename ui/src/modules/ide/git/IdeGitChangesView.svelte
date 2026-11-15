<script lang="ts">
  // GoLand-style Local Changes pane inside the IDE Git dock: resizable staged &
  // unstaged/untracked file lists + commit composer on the left, and Otto's real
  // `DiffViewer.svelte` (with hunk-level stage/unstage/discard) on the right.
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import { api, isAbortError } from '../../../lib/api/client';
  import type { DiffResp, FileChange, Repo, RepoStatusResp } from '../../../lib/api/types';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { loadPaneWidth } from '../../../lib/paneResizer';
  import { plural } from '../../../lib/plural';
  import { git } from '../../../lib/stores/git.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
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

  const LS_GIT_LEFT_W = 'otto.ide.gitChangesW';
  let leftW = $state(loadPaneWidth(LS_GIT_LEFT_W, 320, 220, 560));

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
    busyAction = true;
    try {
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
  <div class="gc-row reveal-host" class:is-selected={isSel}>
    <button
      type="button"
      class="gc-file-btn"
      dir="ltr"
      onclick={() => { selectedPath = c.path; selectedSection = section; }}
      ondblclick={() => openInEditor(c.path)}
      title={c.path}
    >
      <span class={`gc-status mono ${badgeClass(c)}`}>{badgeLetter(c)}</span>
      <span class={`gc-name ${badgeClass(c)}`}>{ideBasename(c.path)}</span>
      {#if dirPart}<span class="gc-dir dim ellipsis">{dirPart}</span>{/if}
    </button>
    <div class="gc-row-actions">
      {#if section === 'staged'}
        <button type="button" class="icon-btn" aria-label={`Unstage ${c.path}`} title="Unstage file" disabled={busyAction} onclick={() => void stageFiles([c.path], false)}><Icon name="minus" size={12} /></button>
      {:else}
        <button type="button" class="icon-btn" aria-label={`Stage ${c.path}`} title="Stage file" disabled={busyAction} onclick={() => void stageFiles([c.path], true)}><Icon name="plus" size={12} /></button>
      {/if}
      {#if c.kind !== 'deleted'}
        <button type="button" class="icon-btn reveal-on-hover" aria-label={`Open ${c.path} in editor`} title="Open file in editor" onclick={() => openInEditor(c.path)}><Icon name="external" size={12} /></button>
      {/if}
      <button type="button" class="icon-btn reveal-on-hover" aria-label={`Discard changes to ${c.path}`} title="Discard changes…" disabled={busyAction} onclick={() => void confirmDiscard(c, section === 'staged')}><Icon name="undo" size={12} /></button>
    </div>
  </div>
{/snippet}

<div class="gc-split" data-testid="ide-git-changes-view">
  <section
    class="gc-left"
    style:inline-size={!viewport.isPhone ? `${leftW}px` : undefined}
    aria-label="Local working tree changes"
  >
    <div class="gc-lists">
      {#if !status}
        <div class="gc-pad"><LoadState what="working tree status" loading={true} variant="compact" rows={3} /></div>
      {:else if changes.length === 0}
        <div class="gc-empty">
          <EmptyState
            icon="check"
            title="Working tree clean"
            body={`No uncommitted changes on ${status.branch || 'HEAD'}.`}
            headingLevel={3}
          />
        </div>
      {:else}
        {#if stagedList.length > 0}
          <div class="gc-sec-head">
            <span class="gc-sec-title">Staged ({stagedList.length})</span>
            <button
              type="button"
              class="btn small ghost gc-mini-btn"
              disabled={busyAction}
              onclick={() => void stageFiles(stagedList.map((c) => c.path), false)}
              data-testid="ide-git-unstage-all"
            >Unstage all</button>
          </div>
          {#each stagedList as c (`s:${c.path}`)}
            {@render fileRow(c, 'staged')}
          {/each}
        {/if}

        <div class="gc-sec-head">
          <span class="gc-sec-title">Changes ({unstagedList.length})</span>
          {#if unstagedList.length > 0}
            <button
              type="button"
              class="btn small ghost gc-mini-btn"
              disabled={busyAction}
              onclick={() => void stageFiles(unstagedList.filter((c) => c.kind !== 'conflicted').map((c) => c.path), true)}
              data-testid="ide-git-stage-all"
            >Stage all</button>
          {/if}
        </div>
        {#each unstagedList as c (`u:${c.path}`)}
          {@render fileRow(c, 'unstaged')}
        {/each}
      {/if}
    </div>

    <form class="gc-composer" onsubmit={(e) => void submitCommit(e)}>
      <textarea
        bind:this={commitBoxEl}
        class="input gc-msg"
        rows={2}
        dir="auto"
        aria-label="Commit message"
        placeholder="Commit message (⌘Enter to commit)…"
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
  </section>

  {#if !viewport.isPhone}
    <PaneDivider bind:width={leftW} storageKey={LS_GIT_LEFT_W} min={220} max={560} defaultWidth={320} label="Resize local changes list" />
  {/if}

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
        <button
          type="button"
          class="btn small ghost gc-mini-btn"
          onclick={() => selectedPath && window.dispatchEvent(new CustomEvent('otto:ide-open-compare', { detail: { mode: 'git-revision', path: `${repo.path.replace(/\/+$/, '')}/${selectedPath.replace(/^\/+/, '')}`, ref: 'HEAD' } }))}
        ><Icon name="columns" size={12} /><span>Side-by-side</span></button>
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
</div>

<style>
  .gc-split { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; overflow: hidden; background: var(--surface); }
  .gc-left { display: flex; flex-direction: column; flex-shrink: 0; min-block-size: 0; min-inline-size: 220px; border-inline-end: 1px solid var(--border); }
  .gc-lists { flex: 1; min-block-size: 0; overflow-y: auto; padding-block: 4px; }
  .gc-pad { padding: 12px; }
  .gc-empty { flex: 1; display: flex; align-items: center; justify-content: center; padding: 16px; font-size: var(--fs-xs); }
  .gc-sec-head {
    display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding-block: 4px; padding-inline: 10px 6px; background: var(--surface-2);
    border-block: 1px solid var(--separator);
  }
  .gc-sec-title { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.06em; }
  .gc-mini-btn { block-size: 22px; padding-block: 0; }
  .gc-row { display: flex; align-items: center; padding-inline-end: 4px; min-block-size: 24px; }
  .gc-row:hover { background: var(--hover); }
  .gc-row.is-selected { background: var(--accent-soft); }
  .gc-file-btn {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 6px;
    padding-block: 2px; padding-inline: 10px 4px; border: 0; background: transparent;
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .gc-status { inline-size: 14px; text-align: center; font-size: var(--fs-xs); font-weight: 600; flex-shrink: 0; }
  .gc-status.is-mod, .gc-name.is-mod { color: var(--info); }
  .gc-status.is-add, .gc-name.is-add { color: var(--success); }
  .gc-status.is-del, .gc-name.is-del { color: var(--danger); }
  .gc-status.is-untracked, .gc-name.is-untracked { color: var(--warning); }
  .gc-name { flex-shrink: 0; max-inline-size: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .gc-dir { flex: 1; min-inline-size: 0; font-size: var(--fs-xs); }
  .gc-row-actions { display: flex; align-items: center; gap: 2px; flex-shrink: 0; }
  .gc-composer { display: flex; flex-direction: column; gap: 6px; padding: 8px; border-block-start: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; }
  .gc-msg { resize: none; font-size: var(--fs-xs); min-block-size: 44px; padding: 6px 8px; }
  .gc-composer-bar { display: flex; align-items: center; gap: 8px; }
  .gc-amend { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--text-dim); cursor: pointer; }
  .gc-right { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; overflow: hidden; }
  .gc-diff-bar {
    display: flex; align-items: center; gap: 8px; block-size: 28px; min-block-size: 28px;
    padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0;
  }
  .gc-diff-path { font-size: var(--fs-xs); font-weight: 500; }
  .gc-target-label { font-size: var(--fs-xs); flex-shrink: 0; }
  .gc-diff-scroll { flex: 1; min-block-size: 0; min-inline-size: 0; overflow: auto; }
  @media (max-width: 640px) {
    .gc-split { flex-direction: column; overflow-y: auto; }
    .gc-left { inline-size: 100%; border-inline-end: 0; border-block-end: 1px solid var(--border); max-block-size: 260px; }
  }
</style>
