<script lang="ts">
  // Integrated Git Log + File History / Blame view for the IDE bottom dock.
  // Reuses Otto's shared `DiffViewer.svelte`, `BlamePanel.svelte`, and `diff-load.ts`.
  import { api, isAbortError } from '../../../lib/api/client';
  import { gitApi } from '../../../lib/api/git';
  import type { CommitInfo, DiffResp, RefBranch, Repo } from '../../../lib/api/types';
  import { copyWithToast } from '../../../lib/clipboard';
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { startMouseDrag } from '../../../lib/dragCursor';
  import { loadErrorText } from '../../../lib/loadError';
  import {
    loadPaneWidth,
    paneResizer,
    RESIZE_TITLE,
    savePaneWidth,
  } from '../../../lib/paneResizer';
  import BlamePanel from '../../git/BlamePanel.svelte'; // ui-guards: allow — reuse Otto's shared Git BlamePanel
  import { repoDiffFileLoader } from '../../git/diff-load'; // ui-guards: allow — reuse Otto's shared Git diff loader
  import DiffViewer from '../../git/DiffViewer.svelte'; // ui-guards: allow — reuse Otto's shared Git DiffViewer
  import { ide, ideBasename } from '../ideStore.svelte';
  import { ideGit } from './ideGitController.svelte';

  interface Props {
    repo: Repo;
    mode: 'log' | 'file';
    refreshTick?: number;
    explicitRelPath?: string | null;
    explicitAbsPath?: string | null;
    initialFileMode?: 'history' | 'blame';
    modeSeq?: number;
  }

  let {
    repo,
    mode,
    refreshTick = 0,
    explicitRelPath = null,
    explicitAbsPath = null,
    initialFileMode = 'history',
    modeSeq = 0,
  }: Props = $props();

  const LS_LOG_W = 'otto.ide.gitLogW';
  const PANE_MIN = 240;
  const PANE_MAX = 640;
  const LOG_LIMIT = 250;

  let listW = $state(loadPaneWidth(LS_LOG_W, 380, PANE_MIN, PANE_MAX));
  let branchFilter = $state<string>('__all__');
  let searchQuery = $state('');
  let branches = $state.raw<RefBranch[]>([]);
  let commits = $state.raw<CommitInfo[]>([]);
  let logLoading = $state(false);
  let logError = $state<string | null>(null);
  let selectedSha = $state<string | null>(null);
  let commitDiff = $state.raw<DiffResp | null>(null);
  let diffLoading = $state(false);
  let diffError = $state<string | null>(null);
  let fileMode = $state<'history' | 'blame'>('history');
  let jumpLineInput = $state('1');

  $effect(() => {
    void modeSeq;
    fileMode = initialFileMode;
  });

  const targetAbsPath = $derived(explicitAbsPath ?? ide.activePath);

  const relFilePath = $derived.by((): string | null => {
    if (mode !== 'file') return null;
    if (explicitRelPath) return explicitRelPath;
    if (!ide.activePath) return null;
    const rel = ideGit.repoRelativePath(repo, ide.activePath);
    return rel ? rel : null;
  });

  function setListW(next: number): void {
    listW = Math.max(PANE_MIN, Math.min(PANE_MAX, Math.round(next)));
    savePaneWidth(LS_LOG_W, listW);
  }

  function startListResize(e: MouseEvent): void {
    const startX = e.clientX;
    const startW = listW;
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => setListW(startW + (ev.clientX - startX)),
      onEnd: () => savePaneWidth(LS_LOG_W, listW),
    });
  }

  function compactDate(iso: string): string {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return '';
    const diffSec = Math.max(0, Math.round((Date.now() - d.getTime()) / 1000));
    if (diffSec < 60) return 'now';
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m`;
    if (diffSec < 86_400) return `${Math.floor(diffSec / 3600)}h`;
    if (diffSec < 86_400 * 14) return `${Math.floor(diffSec / 86_400)}d`;
    return d.toISOString().slice(0, 10);
  }

  // Load branch refs when in `log` mode.
  $effect(() => {
    void refreshTick;
    const repoId = repo.id;
    if (mode !== 'log') return;
    let canceled = false;
    void gitApi.readRefs(repoId).then((res) => {
      if (canceled || !res.refs) return;
      branches = [...res.refs.local, ...res.refs.remote];
    });
    return () => {
      canceled = true;
    };
  });

  // Load commit history for either repo-wide Log or file History.
  $effect(() => {
    void refreshTick;
    const repoId = repo.id;
    const curMode = mode;
    const fPath = relFilePath;
    const bFilter = branchFilter;

    if (curMode === 'file' && (!fPath || fileMode === 'blame')) {
      commits = [];
      logLoading = false;
      logError = null;
      return;
    }

    const ctl = new AbortController();
    logLoading = true;
    logError = null;
    const url =
      curMode === 'file' && fPath
        ? `/repos/${encodeURIComponent(repoId)}/log?all=true&limit=150&skip=0&follow=true&path=${encodeURIComponent(fPath)}`
        : bFilter === '__head__'
          ? `/repos/${encodeURIComponent(repoId)}/log?limit=${LOG_LIMIT}&skip=0`
          : `/repos/${encodeURIComponent(repoId)}/log?all=true&limit=${LOG_LIMIT}&skip=0`;

    api
      .get<CommitInfo[]>(url, ctl.signal)
      .then((rows) => {
        if (ctl.signal.aborted) return;
        commits = rows;
        if (!selectedSha || !rows.some((c) => c.sha === selectedSha)) {
          selectedSha = rows[0]?.sha ?? null;
        }
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        commits = [];
        logError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) logLoading = false;
      });

    return () => ctl.abort();
  });

  // Filter commits by selected branch ancestry + text query.
  const filteredCommits = $derived.by((): CommitInfo[] => {
    let list = commits;
    if (mode === 'log' && branchFilter !== '__all__' && branchFilter !== '__head__') {
      const targetBranch = branches.find((b) => b.name === branchFilter);
      if (targetBranch?.sha) {
        const bySha = new Map<string, CommitInfo>();
        for (let i = 0; i < list.length; i++) bySha.set(list[i].sha, list[i]);
        if (bySha.has(targetBranch.sha)) {
          const reachable = new Set<string>();
          const stack = [targetBranch.sha];
          while (stack.length > 0) {
            const cur = stack.pop()!;
            if (reachable.has(cur)) continue;
            reachable.add(cur);
            const node = bySha.get(cur);
            if (node) for (let p = 0; p < node.parents.length; p++) stack.push(node.parents[p]);
          }
          list = list.filter((c) => reachable.has(c.sha));
        }
      }
    }
    const q = searchQuery.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (c) =>
        c.subject.toLowerCase().includes(q) ||
        c.author.toLowerCase().includes(q) ||
        c.sha.toLowerCase().startsWith(q) ||
        c.short_sha.toLowerCase().includes(q),
    );
  });

  const selectedCommit = $derived(
    filteredCommits.find((c) => c.sha === selectedSha) ??
      commits.find((c) => c.sha === selectedSha) ??
      null,
  );

  // Load diff of the selected commit (scoped to `relFilePath` in file history mode).
  $effect(() => {
    const repoId = repo.id;
    const sha = selectedCommit?.sha ?? null;
    const fPath = mode === 'file' ? relFilePath : null;
    if (!sha || (mode === 'file' && fileMode === 'blame')) {
      commitDiff = null;
      diffLoading = false;
      diffError = null;
      return;
    }
    const ctl = new AbortController();
    diffLoading = true;
    diffError = null;
    const pathParam = fPath ? `&path=${encodeURIComponent(fPath)}` : '&summary=true';
    api
      .get<DiffResp>(
        `/repos/${encodeURIComponent(repoId)}/diff?target=${encodeURIComponent(`commit:${sha}`)}${pathParam}`,
        ctl.signal,
      )
      .then((d) => {
        if (!ctl.signal.aborted) commitDiff = d;
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        commitDiff = null;
        diffError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) diffLoading = false;
      });
    return () => ctl.abort();
  });

  function jumpToEditorLine(): void {
    if (!targetAbsPath) return;
    const line = Math.max(1, Number.parseInt(jumpLineInput, 10) || 1);
    void ide.openFile(targetAbsPath, { line, preview: false });
  }

  function compareRevisionWithCurrent(sha: string): void {
    if (!targetAbsPath) return;
    window.dispatchEvent(
      new CustomEvent('otto:ide-open-compare', {
        detail: { mode: 'git-revision', path: targetAbsPath, ref: sha },
      }),
    );
  }
</script>

<div class="gl-root" data-testid={mode === 'log' ? 'ide-git-log-view' : 'ide-git-file-view'}>
  {#if mode === 'file' && !relFilePath}
    <EmptyState
      icon="file"
      title="Open a repository file"
      body="Select any file inside this repository in the editor to inspect its Git commit history and line-by-line blame."
      headingLevel={2}
    />
  {:else if mode === 'file' && relFilePath}
    <div class="gl-file-bar">
      <Icon name="file" size={12} />
      <span class="gl-file-path mono ellipsis" dir="ltr" title={relFilePath}>{relFilePath}</span>
      <div class="segmented" role="group" aria-label="File Git view">
        <button
          type="button"
          class:active={fileMode === 'history'}
          aria-pressed={fileMode === 'history'}
          onclick={() => (fileMode = 'history')}
          data-testid="ide-git-file-mode-history"
        >History</button>
        <button
          type="button"
          class:active={fileMode === 'blame'}
          aria-pressed={fileMode === 'blame'}
          onclick={() => (fileMode = 'blame')}
          data-testid="ide-git-file-mode-blame"
        >Blame</button>
      </div>
      <span class="grow"></span>
      <div class="gl-jump">
        <input
          class="gl-jump-input mono"
          type="number"
          min="1"
          dir="ltr"
          aria-label="Line number to jump to in editor"
          bind:value={jumpLineInput}
          onkeydown={(e) => e.key === 'Enter' && jumpToEditorLine()}
        />
        <button type="button" class="btn small ghost" onclick={jumpToEditorLine}>
          <Icon name="external" size={12} />
          <span>Jump to line</span>
        </button>
      </div>
    </div>
  {/if}

  {#if mode === 'file' && relFilePath && fileMode === 'blame'}
    <div class="gl-blame-wrap">
      <BlamePanel repoId={repo.id} path={relFilePath} onclose={() => (fileMode = 'history')} />
    </div>
  {:else if mode === 'log' || (mode === 'file' && relFilePath)}
    <div class="gl-split">
      <aside class="gl-left" style:inline-size="{listW}px">
        <div class="gl-filters">
          {#if mode === 'log'}
            <select
              class="gl-branch-select"
              dir="ltr"
              aria-label="Filter commits by branch"
              bind:value={branchFilter}
              data-testid="ide-git-branch-filter"
            >
              <option value="__all__">All branches</option>
              <option value="__head__">HEAD (current)</option>
              {#each branches as br (br.remote ? `remote:${br.name}` : `local:${br.name}`)}
                <option value={br.name}>{br.name}</option>
              {/each}
            </select>
          {/if}
          <div class="input-group gl-search">
            <Icon name="search" size={12} />
            <input
              type="search"
              dir="ltr"
              aria-label="Filter commits"
              placeholder="Search message, author, SHA…"
              bind:value={searchQuery}
              data-testid="ide-git-log-search"
            />
          </div>
        </div>

        <div class="gl-commits">
          <LoadState what="commits" loading={logLoading} error={logError} empty={filteredCommits.length === 0} variant="compact">
            {#snippet emptyView()}
              <EmptyState
                icon="commit"
                title={searchQuery.trim() ? 'No matching commits' : 'No commits yet'}
                body={searchQuery.trim() ? 'Clear the commit filter to see earlier history.' : 'Commits on this repository will appear here.'}
                headingLevel={2}
              />
            {/snippet}

            {#each filteredCommits as c (c.sha)}
              <button
                type="button"
                class="gl-commit-row"
                class:active={selectedCommit?.sha === c.sha}
                onclick={() => (selectedSha = c.sha)}
                data-testid="ide-git-commit-row"
              >
                <div class="gl-commit-top">
                  <span class="gl-subject ellipsis" dir="auto" title={c.subject}>{c.subject || '(no commit message)'}</span>
                  {#if c.refs.length > 0}<Badge tone="accent" variant="outline" label={c.refs[0]} />{/if}
                </div>
                <div class="gl-commit-meta">
                  <span class="ellipsis">{c.author}</span>
                  <span class="mono">{c.short_sha}</span>
                  <span class="mono">{compactDate(c.date)}</span>
                </div>
              </button>
            {/each}
          </LoadState>
        </div>
      </aside>

      <div
        class="gl-resizer"
        role="separator"
        aria-label="Resize commit list"
        title={RESIZE_TITLE}
        use:paneResizer={{
          value: listW, min: PANE_MIN, max: PANE_MAX, orientation: 'vertical',
          text: (v) => `${Math.round(v)} pixels wide`,
          onChange: setListW, onReset: () => setListW(380), onDragStart: startListResize,
        }}
      ></div>

      <section class="gl-right" aria-label="Commit diff">
        {#if !selectedCommit}
          <EmptyState icon="commit" title="Select a commit" body="Pick a commit on the start pane to inspect its changed files and syntax-highlighted diff." headingLevel={2} />
        {:else}
          <header class="gl-commit-head">
            <button type="button" class="btn small ghost mono" title="Copy full commit SHA" onclick={() => void copyWithToast(selectedCommit.sha, 'Copied commit SHA')}>
              <Icon name="commit" size={12} />
              <span>{selectedCommit.short_sha}</span>
            </button>
            <span class="gl-head-subject ellipsis" dir="auto" title={selectedCommit.subject}>{selectedCommit.subject}</span>
            <span class="gl-head-author dim ellipsis">{selectedCommit.author}</span>
            {#if mode === 'file' && targetAbsPath}
              <button type="button" class="btn small ghost" onclick={() => compareRevisionWithCurrent(selectedCommit.sha)}>
                <Icon name="columns" size={12} />
                <span>Compare with Current</span>
              </button>
              <button type="button" class="btn small ghost" onclick={() => void ide.openFile(targetAbsPath, { preview: false })}>
                <Icon name="file" size={12} />
                <span>{ideBasename(targetAbsPath)}</span>
              </button>
            {/if}
          </header>

          <div class="gl-diff-body">
            <LoadState what="commit diff" loading={diffLoading && !commitDiff} error={diffError} empty={!commitDiff || commitDiff.files.length === 0} variant="panel">
              {#snippet emptyView()}
                <EmptyState icon="commit" title="Empty commit diff" body="This commit did not modify any tracked file lines." headingLevel={2} />
              {/snippet}
              {#if commitDiff}
                <DiffViewer
                  diff={commitDiff}
                  repoId={repo.id}
                  loadFile={repoDiffFileLoader(repo.id, `commit:${selectedCommit.sha}`)}
                  stateKey={`${repo.id}:${selectedCommit.sha}:${relFilePath ?? 'all'}`}
                />
              {/if}
            </LoadState>
          </div>
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .gl-root { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; overflow: hidden; background: var(--surface); }
  .gl-file-bar { display: flex; align-items: center; gap: 8px; block-size: 30px; min-block-size: 30px; padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; }
  .gl-file-path { font-size: var(--fs-xs); color: var(--text); max-inline-size: 320px; }
  .gl-jump { display: inline-flex; align-items: center; gap: 4px; }
  .gl-jump-input { inline-size: 56px; block-size: 22px; padding-inline: 6px; font-size: var(--fs-xs); color: var(--text); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-s); }
  .gl-blame-wrap { flex: 1; min-block-size: 0; overflow: auto; }
  .gl-split { display: flex; flex: 1; min-block-size: 0; min-inline-size: 0; overflow: hidden; }
  .gl-left { display: flex; flex-direction: column; flex-shrink: 0; min-block-size: 0; min-inline-size: 200px; border-inline-end: 1px solid var(--border); background: var(--surface); }
  .gl-filters { display: flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 8px; border-block-end: 1px solid var(--border); flex-shrink: 0; }
  .gl-branch-select { max-inline-size: 136px; block-size: 24px; padding-inline: 6px; font-size: var(--fs-xs); color: var(--text); background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-s); }
  .gl-search { flex: 1; min-inline-size: 80px; block-size: 24px; font-size: var(--fs-xs); }
  .gl-commits { flex: 1; min-block-size: 0; overflow-y: auto; }
  .gl-commit-row { inline-size: 100%; display: flex; flex-direction: column; gap: 2px; padding-block: 6px; padding-inline: 10px; border: 0; border-block-end: 1px solid var(--separator); background: transparent; color: var(--text); text-align: start; cursor: pointer; }
  .gl-commit-row:hover { background: var(--hover); }
  .gl-commit-row.active { background: var(--accent-soft); }
  .gl-commit-top { display: flex; align-items: center; gap: 6px; min-inline-size: 0; font-size: var(--fs-xs); font-weight: 500; }
  .gl-subject { flex: 1; min-inline-size: 0; }
  .gl-commit-meta { display: flex; align-items: center; gap: 8px; font-size: var(--fs-xs); color: var(--text-dim); min-inline-size: 0; }
  .gl-commit-meta > :first-child { flex: 1; }
  .gl-resizer { inline-size: 4px; margin-inline: -2px; cursor: col-resize; flex-shrink: 0; z-index: 2; }
  .gl-resizer:hover, .gl-resizer:focus-visible { background: var(--accent-soft); }
  .gl-right { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; overflow: hidden; }
  .gl-commit-head { display: flex; align-items: center; gap: 8px; block-size: 30px; min-block-size: 30px; padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; min-inline-size: 0; }
  .gl-head-subject { flex: 1; font-size: var(--fs-xs); font-weight: 600; }
  .gl-head-author { font-size: var(--fs-xs); max-inline-size: 180px; }
  .gl-diff-body { flex: 1; min-block-size: 0; overflow: auto; }
</style>
