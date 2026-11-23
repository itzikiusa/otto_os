<script lang="ts" module>
  /** The open file the tree last revealed; outlives a remount (see the autoReveal effect). */
  let lastAutoReveal: string | null = null;
</script>

<script lang="ts">
  import { tick, untrack } from 'svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { scrollBehavior } from '../../lib/motion';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { ideDockPrefs } from './debug/ideDockPrefs.svelte';
  import type { ideGit as IdeGitSingleton } from './git/ideGitController.svelte';
  import { ideTreeDirIsDirty, ideTreeGitLetter, type IdeGitLetter } from './git/ideGitTreeOverlay';
  import type { IdeCompareMode } from './history/ideComparisonLoader';
  import type { FlatSymbolNode } from './history/IdeTreeSecondaryTabs.svelte';
  import type { FlatTreeNode } from './history/ideTreeNodeMenus';
  import { treeFilterMatches, treeNavAction, treeRevealScrollTop, treeRowHeight } from './history/ideTreeNav';
  import { nestSiblings } from './history/ideTreeNesting';
  import { ide, ideBasename, type IdeFsEntry, type IdeSymbolNode } from './ideStore.svelte';
  import { ideTreeDirErrors } from './store/ideTreeDirErrors.svelte';
  import { ideProjectHealth } from './store/ideProjectHealth.svelte';
  import { ideBreakpointsNav } from './debug/ideBreakpointsNav.svelte';
  import IdeProjectMissingState from './shell/IdeProjectMissingState.svelte';
  import { newScratchFile, onScratches } from './shell/ideScratches.svelte';
  import { ideTabRestore } from './shell/ideProjectHelpers';
  import { IdeTreeLive, LEASE_HEARTBEAT_MS } from './store/ideTreeLive';
  import { pollWhileVisible } from '../../lib/poll';
  import { appLive } from '../../lib/live';

  const FolderPickerLazy = lazyComponent(() => import('../../lib/components/FolderPicker.svelte'));
  const IdeTreeSecondaryTabsLazy = lazyComponent(
    () => import('./history/IdeTreeSecondaryTabs.svelte'),
  );

  type ExplorerTab = 'files' | 'structure' | 'breakpoints';

  interface CompareModalRequest {
    mode: IdeCompareMode;
    path?: string;
    secondPath?: string | null;
    isDir?: boolean;
    filter?: 'all' | 'deleted';
    ref?: string | null;
    clipboardText?: string;
  }

  interface Props {
    onOpenCompare?: (opts: CompareModalRequest) => void;
  }

  let { onOpenCompare }: Props = $props();

  const VIRTUAL_MIN = 150;
  // One row height for the virtualiser AND the CSS (`--tree-row-h`): 40 px touch rows on a
  // coarse pointer, the dense 26 px row otherwise; offsets stay exact either way.
  const ROW_H = treeRowHeight(typeof window !== 'undefined' && Boolean(window.matchMedia?.('(pointer: coarse)').matches));
  const OVERSCAN = 20;

  let activeTab = $state<ExplorerTab>('files');
  // "Breakpoints (n)…" in the debugger's ⋯ asks for this tab (`ideBreakpointsNav.seq` bumps).
  let seenBpSeq = ideBreakpointsNav.seq;
  $effect(() => { const seq = ideBreakpointsNav.seq; if (seq !== seenBpSeq) { seenBpSeq = seq; activeTab = 'breakpoints'; } });
  let treeScrollEl = $state<HTMLDivElement | undefined>(undefined);
  let scrollTop = $state(0);
  let clientHeight = $state(720);
  let addFolderPickerOpen = $state(false);
  let selectedDirPath = $state<string | null>(null);
  $effect(() => {
    void ide.target.root;
    selectedDirPath = null;
  });
  let manifestTreeModal = $state<{ title: string; output: string } | null>(null);
  let liveGit = $state<typeof IdeGitSingleton | null>(null);
  // One Tab stop for the whole tree (roving tabindex): the row last moved to with the arrow
  // keys, else the open file's row, else the first row.
  let focusKey = $state<string | null>(null);
  // Nested-file parents (go.mod over go.sum) the person opened; collapsed by default.
  let nestOpen = $state<Record<string, boolean>>({});
  // The filter input updates at once; the (whole-tree) filtering runs ~80 ms after typing stops.
  let filterDraft = $state(ide.treeFilter);
  let filterTimer: ReturnType<typeof setTimeout> | null = null;
  function onFilterInput(value: string): void {
    filterDraft = value;
    if (filterTimer) clearTimeout(filterTimer);
    filterTimer = setTimeout(() => { filterTimer = null; ide.treeFilter = value; }, 80);
  }
  $effect(() => {
    const ext = ide.treeFilter;
    untrack(() => { if (!filterTimer && ext !== filterDraft) filterDraft = ext; });
  });
  $effect(() => () => { if (filterTimer) clearTimeout(filterTimer); });

  // Live tree: the daemon watches the folders shown here and names the ones whose entries
  // changed on disk (ideTreeLive). The lease follows the project and its open folders.
  const treeLive = new IdeTreeLive(ide);
  let leaseTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    void ide.target.root;
    void ide.target.kind;
    void ide.projectRoots;
    void ide.expandedDirs;
    void ide.childrenByDir;
    untrack(() => {
      if (leaseTimer) clearTimeout(leaseTimer);
      leaseTimer = setTimeout(() => { leaseTimer = null; treeLive.sync(); }, 250);
    });
  });
  $effect(() => {
    // Back to the window (Remote SSH's only refresh), or events were lost.
    const catchUp = (): void => {
      if (document.visibilityState === 'visible') treeLive.onFocus();
    };
    const offChanged = appLive.on(['ide_fs_changed'], (ev) => treeLive.apply(String(ev.client_id), (ev.dirs as string[] | undefined) ?? []));
    const offResync = appLive.onResync(() => treeLive.onResync());
    // Paused while hidden: the lease then lapses and the return to the window re-posts it.
    const beat = pollWhileVisible(() => treeLive.sync(true), { ms: LEASE_HEARTBEAT_MS, immediate: false });
    window.addEventListener('focus', catchUp);
    document.addEventListener('visibilitychange', catchUp);
    return () => {
      offChanged();
      offResync();
      beat.stop();
      window.removeEventListener('focus', catchUp);
      document.removeEventListener('visibilitychange', catchUp);
      if (leaseTimer) clearTimeout(leaseTimer);
      treeLive.release();
    };
  });

  $effect(() => {
    void import('./git/ideGitController.svelte').then((m) => {
      liveGit = m.ideGit;
    });
  });

  const normPath = (p: string): string => p.replace(/\/+$/, '') || '/';
  const parentDirOfPath = (fullPath: string): string => {
    const idx = fullPath.lastIndexOf('/');
    return idx > 0 ? fullPath.slice(0, idx) : ide.target.root;
  };
  function triggerNewTreeEntry(baseDir: string, op: 'create_file' | 'mkdir', extHint = ''): void {
    void import('./history/ideTreeNodeMenus').then((m) => m.promptCreateTreeEntry(baseDir, op, extHint));
  }
  function openCompareModal(opts: CompareModalRequest): void {
    if (onOpenCompare) {
      onOpenCompare(opts);
      return;
    }
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('otto:ide-open-compare', { detail: opts }));
    }
  }

  $effect(() => {
    void ide.target.root;
    void ide.projectRoots.length;
    void ide.treeRootEntries.length;
    void ide.activePath;
    const enabled = ideDockPrefs.gitStatusInTree;
    const g = liveGit;
    if (enabled && g) untrack(() => g.syncVisibleRoots({ probeAll: false }));
  });

  $effect(() => {
    const el = treeScrollEl;
    if (!el) return;
    clientHeight = el.clientHeight || 720;
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => (clientHeight = el.clientHeight || 720));
    ro.observe(el);
    return () => ro.disconnect();
  });

  const flatSymbols = $derived.by((): FlatSymbolNode[] => {
    const out: FlatSymbolNode[] = [];
    const walk = (nodes: IdeSymbolNode[], depth: number): void => {
      for (let i = 0; i < nodes.length; i++) {
        out.push({ sym: nodes[i], depth });
        if (nodes[i].children?.length) walk(nodes[i].children!, depth + 1);
      }
    };
    walk(ide.activeSymbols, 0);
    return out;
  });

  const explorerTabs = $derived<readonly TabItem<ExplorerTab>[]>([
    { id: 'files', label: 'Files', testid: 'ide-tree-tab-files' },
    { id: 'structure', label: 'Structure', count: flatSymbols.length || undefined, testid: 'ide-tree-tab-structure' },
    { id: 'breakpoints', label: 'Breakpoints', count: ide.allBreakpoints.length || undefined, testid: 'ide-tree-tab-breakpoints' },
  ]);

  // Scale-disciplined multi-root project tree: in 30+ repo projects, attached
  // secondary roots start collapsed unless expanded or holding the active file.
  const visibleNodes = $derived.by((): FlatTreeNode[] => {
    const q = ide.treeFilter.trim().toLowerCase();
    const out: FlatTreeNode[] = [];
    const dirErrors = ideTreeDirErrors.byPath;
    const hits = treeFilterMatches(q, ide.projectRoots.map((r) => (r.isPrimary ? ide.treeRootEntries : (ide.childrenByDir[r.path] ?? []))), ide.childrenByDir);
    const matchesTree = (entry: IdeFsEntry): boolean => !hits || hits.has(entry.path);
    const pushDirError = (dirPath: string, depth: number): void => {
      const err = dirErrors[dirPath];
      if (err) out.push({ entry: { name: '', path: `${dirPath}\u0000error`, kind: 'file' }, depth, loadError: err, errorDir: dirPath });
    };
    const hidden = (entry: IdeFsEntry): boolean => /^__debug_bin\d*$/.test(entry.name) || (!ide.showHidden && entry.name.startsWith('.'));
    const walk = (entries: IdeFsEntry[], depth: number): void => {
      const nest = nestSiblings(entries, ideDockPrefs.nestRelatedFiles);
      for (let i = 0; i < entries.length; i++) {
        const entry = entries[i];
        if (nest.parentOf.has(entry.path) || hidden(entry)) continue;
        const kids = nest.childrenOf.get(entry.path)?.filter((k) => !hidden(k));
        // Filtering: a parent stays (as context) when one of its nested files matches.
        const shownKids = kids?.filter(matchesTree);
        if (!matchesTree(entry) && !shownKids?.length) continue;
        if (kids?.length) {
          const open = Boolean(nestOpen[entry.path] || (q && shownKids?.length));
          out.push({ entry, depth, nestKids: kids.length, nestOpen: open });
          if (open) for (const kid of q ? shownKids! : kids) out.push({ entry: kid, depth: depth + 1 });
          continue;
        }
        out.push({ entry, depth });
        if (entry.kind === 'dir' && (ide.expandedDirs[entry.path] || Boolean(q && ide.childrenByDir[entry.path]))) {
          const kids = ide.childrenByDir[entry.path];
          if (kids) walk(kids, depth + 1);
          else if (ide.expandedDirs[entry.path]) pushDirError(entry.path, depth + 1);
        }
      }
    };

    const roots = ide.projectRoots;
    const manyRoots = roots.length > 4;
    const curActive = ide.activePath ? normPath(ide.activePath) : '';
    for (let r = 0; r < roots.length; r++) {
      const root = roots[r];
      const rp = normPath(root.path);
      const rootKids = root.isPrimary ? ide.treeRootEntries : (ide.childrenByDir[root.path] ?? []);
      const holdsActive = Boolean(curActive) && (curActive === rp || curActive.startsWith(`${rp}/`));
      const defaultExpanded = root.isPrimary || !manyRoots || holdsActive;
      const isExpanded = ide.expandedDirs[root.path] ?? defaultExpanded;
      out.push({
        entry: { name: root.isPrimary && onScratches() ? 'Scratches' : root.name, path: root.path, kind: 'dir' },
        depth: 0,
        isProjectRoot: true,
        isPrimaryRoot: root.isPrimary,
        defaultExpanded,
      });
      if (isExpanded || Boolean(q)) walk(rootKids, 1);
      if (isExpanded && !root.isPrimary && !ide.childrenByDir[root.path]) pushDirError(root.path, 1);
    }
    return out;
  });

  const rowKey = (node: FlatTreeNode): string => `${node.isProjectRoot ? 'root:' : ''}${node.entry.path}`;
  const rowExpanded = (node: FlatTreeNode): boolean =>
    node.nestKids ? Boolean(node.nestOpen) : node.entry.kind === 'dir' && Boolean(ide.expandedDirs[node.entry.path] ?? node.defaultExpanded ?? false);
  const toggleNest = (path: string): void => { nestOpen = { ...nestOpen, [path]: !nestOpen[path] }; };
  /** Revealing a nested file (go.sum) opens its parent (go.mod) first. */
  function openNestFor(path: string): void {
    if (!ideDockPrefs.nestRelatedFiles) return;
    const dir = parentDirOfPath(path);
    const siblings = ide.childrenByDir[dir] ?? (normPath(dir) === normPath(ide.target.root) ? ide.treeRootEntries : []);
    const parent = nestSiblings(siblings).parentOf.get(path);
    if (parent && !nestOpen[parent]) nestOpen = { ...nestOpen, [parent]: true };
  }
  const tabStopKey = $derived.by((): string | null => {
    const nodes = visibleNodes;
    if (focusKey && nodes.some((n) => rowKey(n) === focusKey)) return focusKey;
    const active = ide.activePath ? nodes.find((n) => !n.isProjectRoot && n.entry.path === ide.activePath) : undefined;
    const first = nodes.find((n) => !n.loadError);
    return active ? rowKey(active) : first ? rowKey(first) : null;
  });

  /** Move keyboard focus to row `index`: scroll it into the (virtualized) window first. */
  async function focusRow(index: number): Promise<void> {
    const node = visibleNodes[index];
    if (!node) return;
    const key = rowKey(node);
    focusKey = key;
    if (treeScrollEl) {
      const top = index * ROW_H;
      if (top < treeScrollEl.scrollTop) treeScrollEl.scrollTop = top;
      else if (top + ROW_H > treeScrollEl.scrollTop + clientHeight) treeScrollEl.scrollTop = top + ROW_H - clientHeight;
      scrollTop = treeScrollEl.scrollTop;
    }
    await tick();
    treeScrollEl?.querySelector<HTMLElement>(`[data-tree-key="${CSS.escape(key)}"]`)?.focus();
  }

  function onTreeKeyDown(e: KeyboardEvent, node: FlatTreeNode): void {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const index = visibleNodes.indexOf(node);
    const rows = visibleNodes.map((n) => ({ depth: n.depth, isDir: n.entry.kind === 'dir' || Boolean(n.nestKids), expanded: rowExpanded(n), loadError: n.loadError }));
    const act = treeNavAction(rows, index, e.key, e.shiftKey);
    if (!act) return;
    e.preventDefault();
    e.stopPropagation();
    if (act.kind === 'focus') { void focusRow(act.index); return; }
    if (act.kind === 'menu') {
      const el = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
      if (el) void import('./history/ideTreeNodeMenus').then((m) => m.showTreeNodeContextMenu(el, node, menuCallbacks));
      return;
    }
    // Expand / collapse goes through the same path as a click (lazy child load included).
    focusKey = rowKey(node);
    if (node.nestKids) toggleNest(node.entry.path);
    else onNodeClick(node);
  }

  const treeWin = $derived.by(() => {
    const total = visibleNodes.length;
    if (total <= VIRTUAL_MIN) return { virtual: false, items: visibleNodes, topPx: 0, bottomPx: 0 };
    const viewH = Math.max(clientHeight, 260);
    const top = Math.max(0, Math.min(scrollTop, Math.max(0, total * ROW_H - viewH)));
    const start = Math.max(0, Math.floor(top / ROW_H) - OVERSCAN);
    const end = Math.min(total, Math.ceil((top + viewH) / ROW_H) + OVERSCAN);
    return { virtual: true, items: visibleNodes.slice(start, end), topPx: start * ROW_H, bottomPx: Math.max(0, (total - end) * ROW_H) };
  });

  // What is on screen drives the listings: a folder open but not listed yet (it came back
  // open with the project) is listed as it scrolls into view, a few at a time, and the live
  // tree re-lists folders that may have changed off screen once they show again.
  let lazyLoads = 0;
  $effect(() => {
    const items = treeWin.items;
    untrack(() => {
      const onScreen = new Set<string>();
      for (const n of items) {
        if (n.isProjectRoot || n.loadError) continue;
        onScreen.add(parentDirOfPath(n.entry.path));
        if (n.entry.kind !== 'dir' || !rowExpanded(n)) continue;
        onScreen.add(n.entry.path);
        if (ide.childrenByDir[n.entry.path] || ideTreeDirErrors.byPath[n.entry.path] || lazyLoads >= 4) continue;
        lazyLoads += 1;
        void ide.loadTree(n.entry.path).finally(() => { lazyLoads -= 1; });
      }
      treeLive.onScreen(onScreen);
    });
  });

  /** Scroll `path`'s row into view if it is not already (never moves focus). */
  async function scrollRowIntoView(path: string): Promise<void> {
    await tick();
    const el = treeScrollEl;
    const idx = visibleNodes.findIndex((n) => !n.isProjectRoot && n.entry.path === path);
    if (idx < 0 || !el) return;
    const row = el.querySelector<HTMLElement>(`[data-tree-key="${CSS.escape(path)}"]`);
    const rowTop = row ? row.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop : idx * ROW_H;
    const top = treeRevealScrollTop(rowTop, row?.offsetHeight || ROW_H, el.scrollTop, clientHeight);
    if (top !== null) el.scrollTo({ top, behavior: scrollBehavior() });
  }

  let lastRevealSeq = 0;
  $effect(() => {
    const req = ide.treeRevealRequest;
    if (!req || req.seq === lastRevealSeq) return;
    lastRevealSeq = req.seq;
    activeTab = 'files';
    untrack(() => openNestFor(req.path));
    void scrollRowIntoView(req.path);
  });

  // Reveal the active file (VS Code's autoReveal): when the open tab changes while the Files
  // view shows, open its folders and bring its row into view — without taking focus. Coming
  // back to the IDE (a remount) or tabs coming back with the project (a switch, a reload) is
  // not the open tab changing: the tree stays as the person left it.
  $effect(() => {
    const p = ide.activePath;
    if (!p || activeTab !== 'files' || p === lastAutoReveal) return;
    lastAutoReveal = p;
    if (ideTabRestore.active) return;
    // Already on screen in the flattened tree: just scroll; else the store opens its folders
    // (loading them) and sends a reveal request the effect above scrolls to.
    untrack(() => {
      openNestFor(p);
      if (visibleNodes.some((n) => !n.isProjectRoot && n.entry.path === p)) void scrollRowIntoView(p);
      else void ide.revealInProjectTree(p);
    });
  });

  // One key normaliser for the overlay and the rows (`gitTreeKey`): a loaded repo status is the
  // truth (a clean file shows nothing), the listing's own mark only covers repos not loaded yet.
  function resolveGitLetter(node: FlatTreeNode): IdeGitLetter | null {
    if (!ideDockPrefs.gitStatusInTree || node.entry.kind === 'dir') return null;
    const live = liveGit ? ideTreeGitLetter(liveGit.treeGitOverlay, node.entry.path) : undefined;
    if (live !== undefined) return live;
    const raw = node.entry.git_status;
    if (raw === 'M' || raw === 'A' || raw === 'D' || raw === 'U' || raw === 'R' || raw === 'C') return raw;
    return raw === '?' ? 'U' : null;
  }

  function isDirtyFolder(node: FlatTreeNode): boolean {
    return Boolean(ideDockPrefs.gitStatusInTree && node.entry.kind === 'dir' && liveGit && ideTreeDirIsDirty(liveGit.treeGitOverlay, node.entry.path));
  }

  function onNodeClick(node: FlatTreeNode, e?: MouseEvent): void {
    // A nested-file parent (go.mod): its chevron folds go.sum, the rest of the row opens it
    // (← / → fold from the keyboard).
    if (node.nestKids && e?.target instanceof Element && e.target.closest('.tree-chevron')) {
      toggleNest(node.entry.path);
      return;
    }
    if (node.entry.kind === 'dir') {
      selectedDirPath = node.entry.path;
      const curExpanded = ide.expandedDirs[node.entry.path] ?? node.defaultExpanded ?? false;
      if (node.isProjectRoot && ide.expandedDirs[node.entry.path] === undefined) {
        ide.expandedDirs = { ...ide.expandedDirs, [node.entry.path]: curExpanded };
      }
      if (!curExpanded && node.isProjectRoot && ideDockPrefs.gitStatusInTree) {
        void import('./git/ideGitController.svelte').then((m) => m.ideGit.ensureRootDetected(node.entry.path));
      }
      void ide.toggleDir(node.entry.path);
      return;
    }
    selectedDirPath = parentDirOfPath(node.entry.path);
    void ide.openFile(node.entry.path, { preview: false });
  }

  function onNodeKeyDown(e: KeyboardEvent, node: FlatTreeNode): void {
    onTreeKeyDown(e, node);
    if (e.defaultPrevented || node.isProjectRoot) return;
    if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F6') {
      e.preventDefault();
      e.stopPropagation();
      void import('./history/ideTreeNodeMenus').then((m) => m.promptRenameTreeEntry(node.entry));
    } else if ((e.key === 'Delete' || (e.metaKey && e.key === 'Backspace')) && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      void import('./history/ideTreeNodeMenus').then((m) => m.confirmDeleteTreeEntry(node.entry));
    }
  }

  function openNodeMenu(e: MouseEvent, node: FlatTreeNode): void {
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
    const savedAnchor =
      el && (e.detail === 0 || (e.type === 'click' && el.getBoundingClientRect().height <= 48))
        ? el
        : new MouseEvent('contextmenu', { clientX: e.clientX, clientY: e.clientY });
    void import('./history/ideTreeNodeMenus').then((m) => m.showTreeNodeContextMenu(savedAnchor, node, menuCallbacks));
  }

  /** The explorer's one ⋯: what used to be five icon buttons. */
  function openExplorerMenu(e: MouseEvent): void {
    const local = ide.target.kind === 'local';
    const items: MenuItem[] = [
      ...(local
        ? [
            { label: 'Add folder to project…', icon: 'worktree', action: () => (addFolderPickerOpen = true) },
            { label: 'Local history and deleted files…', icon: 'clock', action: () => openCompareModal({ mode: 'local-history', path: selectedDirPath || ide.target.root || '', isDir: true, filter: 'all' }) },
          ]
        : []),
      { label: 'Show dotfiles', checked: ide.showHidden, action: () => { ide.showHidden = !ide.showHidden; void ide.loadTree(); } },
      { separator: true },
      { label: 'Collapse all folders', icon: 'fold', action: () => ide.collapseAllDirs() },
      { label: 'Refresh', icon: 'refresh', action: () => void ide.loadTree() },
    ];
    ctxMenu.show(e, items);
  }

  const menuCallbacks = {
    openCompare: openCompareModal,
    onOpenFolderPicker: () => (addFolderPickerOpen = true),
    onManifestOutput: (title: string, output: string) => (manifestTreeModal = { title, output }),
  };
</script>

<aside class="ide-tree" aria-label="Project explorer" data-testid="ide-project-tree">
  <Tabs size="s" label="Project explorer views" idBase="ide-explorer" tabs={explorerTabs} value={activeTab} onchange={(id) => (activeTab = id)} />

  {#if activeTab === 'files'}
    <div class="tree-panel" role="tabpanel" id="ide-explorer-panel-files" aria-labelledby="ide-explorer-tab-files">
      <div class="tree-toolbar">
        <input class="tree-filter" type="search" dir="ltr" aria-label="Filter files" placeholder="Filter…" value={filterDraft} oninput={(e) => onFilterInput(e.currentTarget.value)} data-testid="ide-tree-filter" />
        <button type="button" class="tree-act" aria-label={onScratches() ? 'New scratch file…' : 'New file…'} title={onScratches() ? 'New scratch file…' : 'New file…'} onclick={() => (onScratches() ? void newScratchFile() : triggerNewTreeEntry(selectedDirPath || ide.target.root || '', 'create_file'))} data-testid="ide-tree-new-file">
          <Icon name="plus" size={12} />
        </button>
        <button type="button" class="tree-act" aria-label="Explorer options" title="Explorer options" aria-haspopup="menu" onclick={openExplorerMenu} data-testid="ide-tree-more-btn">
          <Icon name="more" size={12} />
        </button>
      </div>

      <div class="tree-scroll" bind:this={treeScrollEl} onscroll={(e) => { const el = e.currentTarget as HTMLDivElement; scrollTop = el.scrollTop; clientHeight = el.clientHeight || 720; }}>
        {#if ideProjectHealth.issue && ide.treeError}<IdeProjectMissingState issue={ideProjectHealth.issue} />{:else}
        <LoadState what="project files" variant="compact" loading={ide.treeLoading} error={ide.treeError} empty={visibleNodes.length === 0} onretry={() => void ide.loadTree()}>
          {#snippet emptyView()}
            <EmptyState
              headingLevel={2}
              icon="folder"
              title={ide.treeFilter.trim() ? 'No matching files' : onScratches() ? 'No scratches yet' : 'Folder is empty'}
              body={ide.treeFilter.trim() ? 'Clear the filter or press ⇧⇧ to search across the project.' : onScratches() ? 'Jot down queries, notes, JSON or snippets — every workspace sees the same scratches.' : 'Create your first source file to start editing.'}
              actionLabel={ide.treeFilter.trim() ? undefined : onScratches() ? 'New scratch file' : 'New file'}
              actionKind="secondary"
              onaction={ide.treeFilter.trim() ? undefined : () => (onScratches() ? void newScratchFile() : triggerNewTreeEntry(ide.target.root, 'create_file'))}
            />
          {/snippet}

          <div class="tree-list" class:is-virtual={treeWin.virtual} role="tree" aria-label="Project files" style:--tree-row-h="{ROW_H}px">
            {#if treeWin.topPx > 0}<div style:block-size="{treeWin.topPx}px" aria-hidden="true"></div>{/if}
            {#each treeWin.items as node (rowKey(node))}
              {#if node.loadError}
                <div class="tree-row tree-err-row" role="none" style="padding-inline-start: calc(6px + {node.depth * 12}px)" data-testid="ide-tree-dir-error">
                  <LoadState what={`“${ideBasename(node.errorDir ?? '')}”`} variant="compact" error={node.loadError} onretry={() => void ide.loadTree(node.errorDir)} />
                </div>
              {:else}
              {@const isExpanded = rowExpanded(node)}
              {@const gitLetter = resolveGitLetter(node)}
              {@const dirtyDir = isDirtyFolder(node)}
              <div
                class="tree-row"
                class:active={!node.isProjectRoot && ide.activePath === node.entry.path}
                class:is-root={Boolean(node.isProjectRoot)}
                role="none"
                style="padding-inline-start: calc(6px + {node.depth * 12}px)"
                data-testid={node.isProjectRoot ? (node.isPrimaryRoot ? 'ide-tree-root-node' : 'ide-tree-attached-root') : undefined}
              >
                <button
                  type="button"
                  class="tree-item"
                  dir="ltr"
                  role="treeitem"
                  aria-level={node.depth + 1}
                  aria-selected={!node.isProjectRoot && ide.activePath === node.entry.path}
                  tabindex={rowKey(node) === tabStopKey ? 0 : -1}
                  data-tree-key={rowKey(node)}
                  onfocus={() => (focusKey = rowKey(node))}
                  aria-expanded={node.entry.kind === 'dir' || node.nestKids ? isExpanded : undefined}
                  data-ignored={node.entry.ignored ? 'true' : undefined}
                  aria-current={node.entry.kind !== 'dir' && ide.activePath === node.entry.path ? 'true' : undefined}
                  onclick={(e) => onNodeClick(node, e)}
                  ondblclick={() => node.entry.kind !== 'dir' && void ide.openFile(node.entry.path, { preview: false })}
                  onkeydown={(e) => onNodeKeyDown(e, node)}
                  oncontextmenu={(e) => openNodeMenu(e, node)}
                  use:rowMenu
                  title={node.isProjectRoot ? node.entry.path : `${ide.relativePath(node.entry.path)}${node.entry.ignored ? ' — ignored by .gitignore' : ''}`}
                >
                  {#if node.entry.kind === 'dir'}
                    <span class="tree-chevron"><Icon name={isExpanded ? 'chevronDown' : 'chevronRight'} size={12} noflip /></span>
                  {:else if node.nestKids}
                    <span class="tree-chevron" data-testid="ide-tree-nest-toggle"><Icon name={isExpanded ? 'chevronDown' : 'chevronRight'} size={12} noflip /></span>
                  {:else}
                    <span class="tree-chevron" aria-hidden="true"></span>
                  {/if}
                  <span class="tree-kind" class:is-dir={node.entry.kind === 'dir'} class:is-ignored={Boolean(node.entry.ignored)}>
                    <Icon name={node.isProjectRoot && !node.isPrimaryRoot ? 'worktree' : node.entry.kind === 'dir' ? 'folder' : 'file'} size={13} />
                  </span>
                  <span class="tree-name" class:is-root-name={Boolean(node.isProjectRoot)} class:is-ignored={Boolean(node.entry.ignored)} data-git={gitLetter ?? undefined}>{node.entry.name}</span>
                  {#if node.entry.ignored}<span class="sr-only" data-testid="ide-tree-ignored-label">, ignored by .gitignore</span>{/if}
                  {#if dirtyDir}<span class="tree-dirty-dot" title="Contains uncommitted Git changes" aria-hidden="true"></span>{/if}
                  {#if gitLetter}<span class="tree-git-mark mono" data-git={gitLetter} data-testid="ide-tree-git-status">{gitLetter === 'U' ? '?' : gitLetter}</span>{/if}
                </button>
                <button type="button" class="tree-act tree-more" tabindex="-1" aria-hidden="true" aria-label="Actions for {node.entry.name}" title="Actions for {node.entry.name} (⇧F10)" onclick={(e) => openNodeMenu(e, node)}>
                  <Icon name="more" size={12} />
                </button>
              </div>
              {/if}
            {/each}
            {#if treeWin.bottomPx > 0}<div style:block-size="{treeWin.bottomPx}px" aria-hidden="true"></div>{/if}
          </div>
        </LoadState>
        {/if}
      </div>
    </div>
  {:else}
    <LazyMount lazy={IdeTreeSecondaryTabsLazy} what="structure & breakpoints" variant="panel" props={{ tab: activeTab, flatSymbols }} />
  {/if}
</aside>

{#if addFolderPickerOpen}
  <LazyMount
    lazy={FolderPickerLazy}
    what="folder picker"
    quiet
    props={{ title: 'Add folder to project', start: ide.target.kind === 'local' ? ide.target.root : '', onpick: (picked: string) => { addFolderPickerOpen = false; void ide.addProjectFolder(picked); }, onclose: () => (addFolderPickerOpen = false) }}
    onerror={() => (addFolderPickerOpen = false)}
  />
{/if}

{#if manifestTreeModal}
  <Modal title={manifestTreeModal.title} width={720} onclose={() => (manifestTreeModal = null)}>
    <pre
      class="dep-tree-pre mono"
      dir="ltr"
      style:max-block-size="{typeof window !== 'undefined' ? Math.max(240, Math.floor(window.innerHeight * 0.6)) : 420}px"
    >{manifestTreeModal.output}</pre>
  </Modal>
{/if}

<style>
  .ide-tree {
    position: relative; display: flex; flex-direction: column; min-inline-size: 0; min-block-size: 0;
    block-size: 100%; overflow: hidden; contain: layout paint;
    background: var(--surface); border-inline-end: 1px solid var(--border);
  }
  .ide-tree :global(.otabs) { block-size: 34px; min-block-size: 34px; max-block-size: 34px; flex-shrink: 0; }
  .tree-panel { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; overflow: hidden; }
  .tree-toolbar {
    display: flex; align-items: center; gap: 2px;
    block-size: 30px; min-block-size: 30px; max-block-size: 30px;
    padding-inline: 6px; border-block-end: 1px solid var(--border); min-inline-size: 0; overflow: hidden; flex-shrink: 0;
  }
  .tree-act {
    display: inline-flex; align-items: center; justify-content: center;
    block-size: 22px; inline-size: 22px; border: 0; border-radius: var(--radius-s);
    background: transparent; color: var(--text-dim); cursor: pointer; flex-shrink: 0;
  }
  .tree-act:hover:not(:disabled) { background: var(--hover); color: var(--text); }
  .tree-act:disabled { opacity: var(--disabled-opacity); cursor: default; }
  .tree-filter {
    flex: 1; min-inline-size: 96px; block-size: 22px; padding-inline: 6px; font-size: var(--fs-xs);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-s);
  }
  .tree-scroll { flex: 1; min-block-size: 0; overflow-y: auto; overflow-x: hidden; padding-block: 4px; }
  .tree-list { display: flex; flex-direction: column; gap: 2px; }
  .tree-list.is-virtual { gap: 0; }
  .tree-row { display: flex; align-items: center; padding-inline-end: 4px; min-inline-size: 0; overflow: hidden; }
  .tree-row.is-root { background: var(--surface-2); border-block-end: 1px solid var(--separator); }
  .tree-list.is-virtual .tree-row { block-size: var(--tree-row-h, 26px); box-sizing: border-box; }
  .tree-row:hover, .tree-row.active { background: var(--hover); }
  .tree-item {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 6px;
    padding-block: var(--ide-row-py, 4px); padding-inline: 2px 4px; background: transparent; border: none;
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer; overflow: hidden;
  }
  .tree-list.is-virtual .tree-item { padding-block: 2px; }
  .tree-chevron { inline-size: 12px; display: inline-flex; align-items: center; justify-content: center; color: var(--text-dim); flex-shrink: 0; }
  .tree-kind { display: inline-flex; color: var(--text-dim); flex-shrink: 0; }
  .tree-kind.is-dir { color: var(--accent-text); }
  .tree-name { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tree-name.is-root-name { font-weight: 600; flex: 0 1 auto; }
  /* Git-ignored entries recede (GoLand's dimmed ignored files); still 4.5:1 on the surface. */
  /* GoLand-style olive: the warning hue pushed toward the dim text colour. */
  .tree-name.is-ignored, .tree-kind.is-ignored { color: color-mix(in oklab, var(--warning) 55%, var(--text-dim)); }
  .tree-name[data-git='M'], .tree-git-mark[data-git='M'] { color: var(--info); }
  .tree-name[data-git='A'], .tree-git-mark[data-git='A'] { color: var(--success); }
  .tree-name[data-git='U'], .tree-git-mark[data-git='U'] { color: var(--warning); }
  .tree-name[data-git='D'], .tree-name[data-git='C'], .tree-git-mark[data-git='D'], .tree-git-mark[data-git='C'] { color: var(--danger); }
  .tree-name[data-git='R'], .tree-git-mark[data-git='R'] { color: var(--accent-text); }
  .tree-row.active .tree-name { font-weight: 600; }
  .tree-dirty-dot { inline-size: 6px; block-size: 6px; border-radius: 50%; background: var(--info); flex-shrink: 0; }
  .tree-git-mark { font-size: var(--fs-xs); font-weight: 600; flex-shrink: 0; }
  .tree-more { opacity: 0; block-size: 20px; inline-size: 20px; }
  .tree-item:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: var(--radius-s); }
  .tree-err-row { padding-inline-end: 6px; }
  .tree-list.is-virtual .tree-err-row { overflow: hidden; }
  .tree-row:hover .tree-more, .tree-row:focus-within .tree-more { opacity: 1; }
  .dep-tree-pre { overflow: auto; padding: 10px; background: var(--surface-2); border-radius: var(--radius-s); font-size: var(--fs-xs); white-space: pre; }
  @media (hover: none) { .tree-more { opacity: 1; } }
  /* Touch: rows and their buttons are comfortable targets (the row height is the same
     constant the virtualiser uses, see ROW_H). */
  @media (pointer: coarse) {
    .tree-row { min-block-size: var(--tree-row-h, 40px); box-sizing: border-box; }
    .tree-toolbar { block-size: auto; min-block-size: 40px; max-block-size: none; }
    .tree-act, .tree-more { block-size: 36px; inline-size: 36px; }
    .tree-filter { block-size: 32px; }
  }
</style>
