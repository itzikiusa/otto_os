<script lang="ts">
  // Interactive Side-by-Side Comparison & Local History Modal (`local-history`,
  // `git-revision`, `git-branch`, `clipboard`, and `file` modes).
  import { api, isAbortError } from '../../../lib/api/client';
  import { gitApi } from '../../../lib/api/git';
  import type { CommitInfo, RefBranch, Repo } from '../../../lib/api/types';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { startMouseDrag } from '../../../lib/dragCursor';
  import { loadErrorText } from '../../../lib/loadError';
  import {
    loadPaneWidth,
    paneResizer,
    RESIZE_TITLE,
    savePaneWidth,
  } from '../../../lib/paneResizer';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { ideGit } from '../git/ideGitController.svelte';
  import { ide, ideBasename } from '../ideStore.svelte';
  import IdeCompareDiffPane from './IdeCompareDiffPane.svelte';
  import IdeComparisonSidebar from './IdeComparisonSidebar.svelte';
  import {
    fetchGitRefFileContent,
    promptPutLocalHistoryLabel,
    readClipboardForCompare,
    readCurrentDiskOrTab,
    refreshTreeForRestoredPath,
    writeCurrentAndSyncIde,
    type HistoryEntryResp,
    type HistoryListResp,
    type HistoryRestoreResp,
    type IdeCompareMode,
    type IdeLocalHistoryEntry,
  } from './ideComparisonLoader';

  interface Props {
    mode?: IdeCompareMode;
    root: string;
    targetPath: string;
    secondPath?: string | null;
    isDir?: boolean;
    initialFilter?: 'all' | 'deleted';
    initialRef?: string | null;
    initialClipboardText?: string;
    onclose: () => void;
  }

  let {
    mode = 'local-history',
    root,
    targetPath,
    secondPath = null,
    isDir = false,
    initialFilter = 'all',
    initialRef = null,
    initialClipboardText,
    onclose,
  }: Props = $props();

  const LS_SIDE_W = 'otto.ide.compareSideW';
  let sideW = $state(loadPaneWidth(LS_SIDE_W, 290, 210, 480));
  let reloadSeq = $state(0);
  let restoring = $state(false);

  let entries = $state.raw<IdeLocalHistoryEntry[]>([]);
  let listLoading = $state(false);
  let listError = $state<string | null>(null);
  let filterMode = $state<'all' | 'deleted'>('all');
  let searchInput = $state('');
  let debouncedQuery = $state('');
  let selectedEntryId = $state<string | null>(null);

  let activeRepo = $state.raw<Repo | null>(null);
  let commits = $state.raw<CommitInfo[]>([]);
  let branches = $state.raw<RefBranch[]>([]);
  let selectedRef = $state<string>('');
  let refFilter = $state('');

  let detailLoading = $state(false);
  let detailError = $state<string | null>(null);
  let leftText = $state('');
  let rightText = $state('');
  let rightExists = $state(true);
  let leftLabel = $state('Before');
  let activeComparePath = $state('');

  $effect(() => {
    filterMode = initialFilter;
  });

  $effect(() => {
    const raw = searchInput.trim();
    const timer = setTimeout(() => (debouncedQuery = raw), 160);
    return () => clearTimeout(timer);
  });

  const hasSidebar = $derived(
    mode === 'local-history' || mode === 'git-revision' || mode === 'git-branch',
  );

  function setSideW(next: number): void {
    sideW = Math.max(210, Math.min(480, Math.round(next)));
    savePaneWidth(LS_SIDE_W, sideW);
  }

  function startSideDrag(e: MouseEvent): void {
    const sX = e.clientX;
    const sW = sideW;
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => setSideW(sW + (ev.clientX - sX)),
      onEnd: () => savePaneWidth(LS_SIDE_W, sideW),
    });
  }

  // 1. Load Local History entries (`GET /ide/history/list?...&q=`).
  $effect(() => {
    void reloadSeq;
    if (mode !== 'local-history') return;
    const ctl = new AbortController();
    listLoading = true;
    listError = null;
    const qParam = debouncedQuery ? `&q=${encodeURIComponent(debouncedQuery)}` : '';
    api
      .get<HistoryListResp>(
        `/ide/history/list?root=${encodeURIComponent(root)}&path=${encodeURIComponent(targetPath)}&include_deleted=true&limit=100${qParam}`,
        ctl.signal,
      )
      .then((res) => {
        if (!ctl.signal.aborted) entries = res.entries ?? [];
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        entries = [];
        listError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) listLoading = false;
      });
    return () => ctl.abort();
  });

  const filteredEntries = $derived(
    filterMode === 'deleted' ? entries.filter((e) => e.event_kind === 'deleted') : entries,
  );
  const deletedCount = $derived(entries.filter((e) => e.event_kind === 'deleted').length);

  $effect(() => {
    if (mode !== 'local-history') return;
    const list = filteredEntries;
    if (list.length === 0) selectedEntryId = null;
    else if (!selectedEntryId || !list.some((e) => e.id === selectedEntryId)) {
      selectedEntryId = list[0].id;
    }
  });

  const selectedEntry = $derived(filteredEntries.find((e) => e.id === selectedEntryId) ?? null);

  // 2. Load Git commits or branches when in `git-revision` or `git-branch` mode.
  $effect(() => {
    void reloadSeq;
    if (mode !== 'git-revision' && mode !== 'git-branch') return;
    let canceled = false;
    listLoading = true;
    listError = null;
    void (async () => {
      try {
        const repo = await ideGit.resolveRepoForPath(targetPath);
        if (canceled) return;
        activeRepo = repo;
        if (!repo) {
          listError = 'File is not inside a detected Git repository.';
          return;
        }
        const rel = ideGit.repoRelativePath(repo, targetPath);
        if (mode === 'git-revision') {
          const pathQ = rel ? `&follow=true&path=${encodeURIComponent(rel)}` : '';
          const rows = await api.get<CommitInfo[]>(
            `/repos/${encodeURIComponent(repo.id)}/log?all=true&limit=120&skip=0${pathQ}`,
          );
          if (canceled) return;
          commits = rows;
          selectedRef = initialRef || rows[0]?.sha || 'HEAD';
        } else {
          const refsResp = await gitApi.readRefs(repo.id);
          if (canceled) return;
          const allBr = [...(refsResp.refs?.local ?? []), ...(refsResp.refs?.remote ?? [])];
          branches = allBr;
          const fallback =
            allBr.find((b) => !b.is_current && (b.name === 'main' || b.name === 'master'))?.name ??
            allBr.find((b) => !b.is_current)?.name ??
            allBr[0]?.name ??
            'HEAD';
          selectedRef = initialRef || fallback;
        }
      } catch (e) {
        if (!canceled) listError = loadErrorText(e);
      } finally {
        if (!canceled) listLoading = false;
      }
    })();
    return () => {
      canceled = true;
    };
  });

  const filteredCommits = $derived.by(() => {
    const q = refFilter.trim().toLowerCase();
    return q
      ? commits.filter(
          (c) =>
            c.subject.toLowerCase().includes(q) ||
            c.author.toLowerCase().includes(q) ||
            c.short_sha.toLowerCase().includes(q),
        )
      : commits;
  });

  const filteredBranches = $derived.by(() => {
    const q = refFilter.trim().toLowerCase();
    return q ? branches.filter((b) => b.name.toLowerCase().includes(q)) : branches;
  });

  // 3. Load left/right comparison contents for the active mode & selection.
  $effect(() => {
    const curMode = mode;
    const entry = selectedEntry;
    const ref = selectedRef;
    const repo = activeRepo;
    const ctl = new AbortController();

    if (curMode === 'local-history') {
      if (!entry) {
        leftText = '';
        rightText = '';
        return;
      }
      detailLoading = true;
      detailError = null;
      api
        .get<HistoryEntryResp>(
          `/ide/history/entry?root=${encodeURIComponent(root)}&id=${encodeURIComponent(entry.id)}`,
          ctl.signal,
        )
        .then((res) => {
          if (ctl.signal.aborted) return;
          activeComparePath = res.entry.path || targetPath;
          leftText = res.content ?? '';
          rightText = res.current_content ?? '';
          rightExists = res.current_exists;
          const ts = new Date(res.entry.timestamp_ms).toLocaleTimeString();
          leftLabel = `Before · ${ts} (${res.entry.label || res.entry.event_kind})`;
        })
        .catch((e: unknown) => {
          if (!ctl.signal.aborted && !isAbortError(e)) detailError = loadErrorText(e);
        })
        .finally(() => {
          if (!ctl.signal.aborted) detailLoading = false;
        });
      return () => ctl.abort();
    }

    if (curMode === 'git-revision' || curMode === 'git-branch') {
      if (!repo || !ref) return;
      detailLoading = true;
      detailError = null;
      fetchGitRefFileContent(repo, targetPath, ref, ctl.signal)
        .then((res) => {
          if (ctl.signal.aborted) return;
          activeComparePath = targetPath;
          leftText = res.leftText;
          rightText = res.rightText;
          rightExists = res.rightExists;
          leftLabel = curMode === 'git-branch' ? `Branch · ${ref}` : `Revision · ${ref.slice(0, 8)}`;
        })
        .catch((e: unknown) => {
          if (!ctl.signal.aborted && !isAbortError(e)) detailError = loadErrorText(e);
        })
        .finally(() => {
          if (!ctl.signal.aborted) detailLoading = false;
        });
      return () => ctl.abort();
    }

    detailLoading = true;
    detailError = null;
    void (async () => {
      try {
        const cur = await readCurrentDiskOrTab(targetPath, ctl.signal);
        const left =
          curMode === 'file' && secondPath
            ? (await readCurrentDiskOrTab(secondPath, ctl.signal)).content
            : await readClipboardForCompare(initialClipboardText);
        if (ctl.signal.aborted) return;
        activeComparePath = targetPath;
        leftText = left;
        rightText = cur.content;
        rightExists = cur.exists;
        leftLabel = curMode === 'file' && secondPath ? ideBasename(secondPath) : 'Clipboard';
      } catch (e) {
        if (!ctl.signal.aborted && !isAbortError(e)) detailError = loadErrorText(e);
      } finally {
        if (!ctl.signal.aborted) detailLoading = false;
      }
    })();
    return () => ctl.abort();
  });

  async function applyToCurrent(nextText: string, reason: string): Promise<void> {
    const dest = activeComparePath || targetPath;
    if (!dest) return;
    const ok = await writeCurrentAndSyncIde(dest, nextText, !rightExists);
    if (ok) {
      rightText = nextText;
      rightExists = true;
      if (reason !== 'Edited Current') toasts.success(reason, ideBasename(dest));
    }
  }

  async function restoreSelectedWholeFile(): Promise<void> {
    const entry = selectedEntry;
    if (!entry || restoring) return;
    restoring = true;
    try {
      const res = await api.post<HistoryRestoreResp>('/ide/history/restore', { root, id: entry.id });
      const rawPath = res.path || entry.path;
      const restoredPath = await refreshTreeForRestoredPath(root, rawPath, entry.rel_path);
      if (ide.tabs.some((t) => t.path === restoredPath)) {
        await ideGit.reloadIdeTabIfOpen(restoredPath);
      } else {
        await ide.openFile(restoredPath, { preview: false });
      }
      void ideGit.refreshActive();
      toasts.success(
        res.recreated ? 'Recreated deleted file from Local History' : 'Reverted file from Local History',
        entry.rel_path || ideBasename(restoredPath),
      );
      onclose();
    } catch (e) {
      toastError('Couldn’t restore snapshot', e);
    } finally {
      restoring = false;
    }
  }

  async function onPutLabel(): Promise<void> {
    if (await promptPutLocalHistoryLabel(root, targetPath)) reloadSeq += 1;
  }

  const targetDisplay = $derived(
    targetPath === root ? ideBasename(root) : ide.relativePath(targetPath) || ideBasename(targetPath),
  );
  const modalTitle = $derived(
    mode === 'local-history'
      ? `Local History — ${targetDisplay}`
      : mode === 'clipboard'
        ? `Compare with Clipboard — ${ideBasename(targetPath)}`
        : mode === 'git-branch'
          ? `Compare with Branch — ${ideBasename(targetPath)}`
          : `Compare Revisions — ${ideBasename(targetPath)}`,
  );
</script>

<Modal title={modalTitle} width={1160} height={680} dismissable={!restoring} {onclose}>
  <div class="icm-root" data-testid="ide-comparison-modal">
    {#if hasSidebar}
      <IdeComparisonSidebar
        {mode}
        {isDir}
        widthPx={sideW}
        {entries}
        {filteredEntries}
        {deletedCount}
        {filterMode}
        {searchInput}
        {selectedEntryId}
        commits={filteredCommits}
        branches={filteredBranches}
        {selectedRef}
        {refFilter}
        loading={listLoading}
        error={listError}
        onFilterMode={(next) => (filterMode = next)}
        onSearchInput={(next) => (searchInput = next)}
        onSelectEntry={(id) => (selectedEntryId = id)}
        onPutLabel={() => void onPutLabel()}
        onRefFilter={(next) => (refFilter = next)}
        onSelectRef={(ref) => (selectedRef = ref)}
        onRetry={() => (reloadSeq += 1)}
      />
      <div
        class="icm-resizer"
        role="separator"
        aria-label="Resize revision list"
        title={RESIZE_TITLE}
        use:paneResizer={{
          value: sideW, min: 210, max: 480, orientation: 'vertical',
          text: (v) => `${Math.round(v)} pixels wide`,
          onChange: setSideW, onReset: () => setSideW(290), onDragStart: startSideDrag,
        }}
      ></div>
    {/if}

    <section class="icm-right">
      {#if mode === 'local-history' && !selectedEntry}
        <EmptyState icon="clock" title="Select a snapshot" body="Pick any timestamped snapshot on the start pane to compare and revert individual hunks or the whole file." headingLevel={3} />
      {:else if detailError}
        <LoadState what="comparison" error={detailError} empty={true} variant="panel" />
      {:else if detailLoading && !leftText && !rightText}
        <LoadState what="comparison" loading={true} variant="panel" />
      {:else}
        <IdeCompareDiffPane
          filePath={activeComparePath || targetPath}
          {leftLabel}
          rightLabel={`Current · ${ideBasename(activeComparePath || targetPath)}`}
          {leftText}
          {rightText}
          {rightExists}
          canEditRight={Boolean(activeComparePath || !isDir)}
          onApplyRight={applyToCurrent}
        />
      {/if}
    </section>
  </div>

  {#snippet footer()}
    <button type="button" class="btn ghost" disabled={restoring} onclick={onclose}>Close</button>
    {#if mode === 'local-history'}
      <button
        type="button"
        class="btn primary"
        disabled={!selectedEntry || restoring}
        onclick={() => void restoreSelectedWholeFile()}
        data-testid="ide-local-history-restore"
      >
        <Icon name="undo" size={13} />
        <span>{restoring ? 'Restoring…' : !rightExists ? 'Restore deleted file' : 'Revert whole file'}</span>
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .icm-root {
    display: flex; block-size: 540px; max-block-size: 76vh; min-block-size: 340px;
    margin-block: -8px; margin-inline: -12px;
    border-block: 1px solid var(--border); overflow: hidden; background: var(--surface);
  }
  .icm-resizer { inline-size: 4px; margin-inline: -2px; cursor: col-resize; flex-shrink: 0; z-index: 2; }
  .icm-resizer:hover, .icm-resizer:focus-visible { background: var(--accent-soft); }
  .icm-right { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; overflow: hidden; }
</style>
