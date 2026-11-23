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
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { loadPaneWidth } from '../../../lib/paneResizer';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { ideGit } from '../git/ideGitController.svelte';
  import { ide, ideBasename } from '../ideStore.svelte';
  import { ideLayout } from '../shell/ideLayout.svelte';
  import IdeCompareDiffPane from './IdeCompareDiffPane.svelte';
  import IdeComparisonSidebar from './IdeComparisonSidebar.svelte';
  import { ideCompareLayout, ideCompareListName, ideCompareStepAfterPick, ideCompareTitle, type IdeCompareStep } from './ideCompareLayout';
  import {
    fetchGitRefFileContent,
    promptPutLocalHistoryLabel,
    readClipboardForCompare,
    openTabBuffer,
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
    /** What the left side is when it is not the clipboard ("On disk" from the disk-changed banner). */
    sourceLabel?: string | null;
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
    sourceLabel = null,
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
  // The first pick is automatic: if the newest entry equals the current file, move on to the
  // newest one that differs, so the modal never opens on "No differences".
  let autoPicked = false;

  let activeRepo = $state.raw<Repo | null>(null);
  let commits = $state.raw<CommitInfo[]>([]);
  let branches = $state.raw<RefBranch[]>([]);
  let selectedRef = $state<string>('');
  let refFilter = $state('');

  let detailLoading = $state(false);
  let detailError = $state<string | null>(null);
  let leftText = $state('');
  let rightText = $state('');
  // What the "Current" side read from disk: the precondition when it is written back.
  let rightDisk = $state<string | null>(null);
  let rightExists = $state(true);
  let leftLabel = $state('Before');
  let activeComparePath = $state('');
  const diskStateByPath = new Map<string, { exists: boolean; disk: string | null }>();

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
  // Stacked (phone): list → detail. It opens on the diff of the newest pick.
  let step = $state<IdeCompareStep>('diff');
  const layout = $derived(ideCompareLayout(ideLayout.stack, hasSidebar, step));
  const pick = (): void => { step = ideCompareStepAfterPick(ideLayout.stack, step); };

  // 1. Load Local History entries (`GET /ide/history/list?...&q=`).
  $effect(() => {
    void reloadSeq;
    if (mode !== 'local-history') return;
    if (ide.target.kind !== 'local') {
      // Local History records local files only; an SSH file has none.
      entries = [];
      return;
    }
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
        if (ctl.signal.aborted) return;
        entries = res.entries ?? [];
        // A file with no snapshots yet: say so instead of a large modal of empty panes.
        if (!isDir && !debouncedQuery && entries.length === 0) {
          toasts.info(`No local history for ${ideBasename(targetPath)} yet`, 'Saves and edits are recorded automatically.');
          onclose();
        }
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
      autoPicked = true;
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
    void reloadSeq;
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
          rightDisk = res.current_content ?? '';
          // An open tab's live buffer is "Current" (unsaved edits included), not the disk copy.
          rightText = openTabBuffer(activeComparePath) ?? rightDisk;
          rightExists = res.current_exists;
          diskStateByPath.set(activeComparePath, { exists: rightExists, disk: rightDisk });
          const ts = new Date(res.entry.timestamp_ms).toLocaleTimeString();
          leftLabel = `Before · ${ts} (${res.entry.label || res.entry.event_kind})`;
          if (autoPicked) {
            autoPicked = false;
            const older = leftText === rightText ? filteredEntries.find((e) => e.timestamp_ms < res.entry.timestamp_ms && e.sha256 !== res.entry.sha256) : null;
            if (older) selectedEntryId = older.id;
          }
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
          rightDisk = res.rightText;
          rightExists = res.rightExists;
          diskStateByPath.set(activeComparePath, { exists: rightExists, disk: rightDisk });
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
        rightDisk = cur.content;
        rightExists = cur.exists;
        diskStateByPath.set(activeComparePath, { exists: rightExists, disk: rightDisk });
        leftLabel = curMode === 'file' && secondPath ? ideBasename(secondPath) : (sourceLabel ?? 'Clipboard');
      } catch (e) {
        if (!ctl.signal.aborted && !isAbortError(e)) detailError = loadErrorText(e);
      } finally {
        if (!ctl.signal.aborted) detailLoading = false;
      }
    })();
    return () => ctl.abort();
  });

  async function applyToCurrent(nextText: string, reason: string, destPath?: string): Promise<void> {
    const dest = destPath || activeComparePath || targetPath;
    if (!dest) return;
    const isCurrentView = dest === (activeComparePath || targetPath);
    const snapshot = diskStateByPath.get(dest);
    const existsBefore = snapshot ? snapshot.exists : (isCurrentView ? rightExists : true);
    const diskBefore = snapshot ? snapshot.disk : (isCurrentView ? rightDisk : null);
    const written = await writeCurrentAndSyncIde(dest, nextText, !existsBefore, diskBefore);
    if (written !== null) {
      diskStateByPath.set(dest, { exists: true, disk: written });
      if (dest === (activeComparePath || targetPath)) {
        rightDisk = written;
        rightText = nextText;
        rightExists = true;
      }
      if (reason !== 'Edited Current') toasts.success(reason, ideBasename(dest));
    }
  }

  async function restoreSelectedWholeFile(): Promise<void> {
    const entry = selectedEntry;
    if (!entry || restoring) return;
    const openTab = ide.tabs.find((t) => t.path === ide.resolvePath(entry.path));
    const live = openTab ? openTabBuffer(openTab.path) : null;
    if (openTab && live !== null && live !== openTab.savedContent) {
      const ok = await confirmer.ask(`“${openTab.name}” has unsaved changes. Reverting replaces them with this snapshot.`, {
        title: 'Unsaved changes', confirmLabel: 'Revert anyway', danger: true,
      });
      if (!ok) return;
    }
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
  const modalTitle = $derived(ideCompareTitle(mode, ideBasename(targetPath), targetDisplay, sourceLabel, ideLayout.stack));
  const restoreLabel = $derived(restoring ? 'Restoring…' : !rightExists ? 'Restore deleted file' : 'Revert whole file');
</script>

<Modal title={modalTitle} width={1160} height={layout.footer ? 680 : 4000} dismissable={!restoring} {onclose} footer={layout.footer ? footer : undefined}>
  <div class="icm-root" class:compact={!layout.split && ideLayout.stack} data-testid="ide-comparison-modal">
    {#if layout.showList}
      <IdeComparisonSidebar
        {mode}
        {isDir}
        widthPx={layout.split ? sideW : 0}
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
        onSelectEntry={(id) => { autoPicked = false; selectedEntryId = id; pick(); }}
        onPutLabel={() => void onPutLabel()}
        onRefFilter={(next) => (refFilter = next)}
        onSelectRef={(ref) => { selectedRef = ref; pick(); }}
        onRetry={() => (reloadSeq += 1)}
      />
      {#if layout.split}<PaneDivider bind:width={sideW} storageKey={LS_SIDE_W} min={210} max={480} defaultWidth={290} label="Resize revision list" />{/if}
    {/if}

    {#if layout.showDiff}
    <section class="icm-right">
      {#if layout.showBack}
        <div class="icm-step-bar" data-testid="ide-compare-step-bar">
          <button type="button" class="btn ghost icm-back" onclick={() => (step = 'list')} data-testid="ide-compare-back">
            <Icon name="chevronLeft" size={14} />
            <span>{ideCompareListName(mode)}</span>
          </button>
          <span class="icm-step-what mono ellipsis" dir="ltr">{leftLabel}</span>
          {#if mode === 'local-history'}
            <button
              type="button"
              class="btn primary icm-step-act"
              disabled={!selectedEntry || restoring}
              onclick={() => void restoreSelectedWholeFile()}
              data-testid="ide-local-history-restore"
            >
              <Icon name="undo" size={13} />
              <span>{restoreLabel}</span>
            </button>
          {/if}
        </div>
      {/if}
      {#if mode === 'local-history' && !selectedEntry}
        <EmptyState icon="clock" title="Select a snapshot" body="Pick any timestamped snapshot on the start pane to compare and revert individual hunks or the whole file." headingLevel={3} />
      {:else if detailError}
        <LoadState what="comparison" error={detailError} empty={true} variant="panel" onretry={() => (reloadSeq += 1)} />
      {:else if detailLoading && !leftText && !rightText}
        <LoadState what="comparison" loading={true} variant="panel" />
      {:else}
        {#key activeComparePath || targetPath}
          <IdeCompareDiffPane
            filePath={activeComparePath || targetPath}
            {leftLabel}
            rightLabel={`Current · ${ideBasename(activeComparePath || targetPath)}`}
            {leftText}
            {rightText}
            {rightExists}
            canEditRight={Boolean(activeComparePath || !isDir)}
            onApplyRight={applyToCurrent}
            unifiedOnly={layout.unifiedOnly}
          />
        {/key}
      {/if}
    </section>
    {/if}
  </div>
</Modal>

{#snippet footer()}
    <button type="button" class="btn ghost" disabled={restoring} onclick={onclose}>Close</button>
    {#if mode === 'local-history'}
      <button
        type="button"
        class="btn primary"
        disabled={!selectedEntry || restoring || detailLoading || (rightExists && leftText === rightText)}
        title={rightExists && leftText === rightText ? 'This version matches the current file' : undefined}
        onclick={() => void restoreSelectedWholeFile()}
        data-testid="ide-local-history-restore"
      >
        <Icon name="undo" size={13} />
        <span>{restoreLabel}</span>
      </button>
    {/if}
{/snippet}

<style>
  .icm-root {
    display: flex; block-size: 540px; max-block-size: 76dvh; min-block-size: 340px;
    margin-block: -8px; margin-inline: -12px;
    border-block: 1px solid var(--border); overflow: hidden; background: var(--surface);
  }
  .icm-right { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; overflow: hidden; }
  /* Stacked (phone): the sheet is as tall as the window allows and the diff (or
     the list step) fills all of it — no fixed 540 px box, no 76dvh cap. */
  /* It also covers the sheet body's padding (4 px top, 16 px bottom, 16 px sides). */
  .icm-root.compact { block-size: calc(100% + 20px); max-block-size: none; min-block-size: 0; margin-block: -4px -16px; margin-inline: -16px; border-block-end: 0; }
  .icm-step-bar {
    display: flex; align-items: center; gap: 8px; flex-shrink: 0;
    min-block-size: 44px; padding-inline: 4px 8px; border-block-end: 1px solid var(--border);
  }
  .icm-back, .icm-step-act { min-block-size: var(--hit-min); flex-shrink: 0; }
  .icm-step-what { flex: 1; min-inline-size: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  /* The sheet's ✕ is a 24 px glyph; on this phone sheet it is the only way out,
     so it gets a full 44 px target. */
  :global(.sheet:has(> .sheet-body > .icm-root.compact) > header .icon-btn) { min-inline-size: 44px; min-block-size: 44px; }
  /* A phone on its side is ~380 px tall: the sheet keeps a 12 px margin (not
     32 px) and its header no extra padding, so the diff keeps most of the height. */
  :global(.sheet:has(> .sheet-body > .icm-root.compact)) { max-block-size: calc(100% - 24px); max-inline-size: calc(100% - 24px); }
  :global(.sheet:has(> .sheet-body > .icm-root.compact) > header) { padding-block: 0; padding-inline-end: 4px; }
</style>
