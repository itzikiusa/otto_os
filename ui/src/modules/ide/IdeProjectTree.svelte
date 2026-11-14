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
  import { rowMenu } from '../../lib/rowMenu';
  import { ideDockPrefs } from './debug/ideDockPrefs.svelte';
  import type { ideGit as IdeGitSingleton, IdeGitLetter } from './git/ideGitController.svelte';
  import type { IdeCompareMode } from './history/ideComparisonLoader';
  import type { FlatSymbolNode } from './history/IdeTreeSecondaryTabs.svelte';
  import type { FlatTreeNode } from './history/ideTreeNodeMenus';
  import { ide, type IdeFsEntry, type IdeSymbolNode } from './ideStore.svelte';

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
  const ROW_H = 26;
  const OVERSCAN = 20;

  let activeTab = $state<ExplorerTab>('files');
  let treeScrollEl = $state<HTMLDivElement | undefined>(undefined);
  let scrollTop = $state(0);
  let clientHeight = $state(720);
  let addFolderPickerOpen = $state(false);
  let selectedDirPath = $state<string | null>(null);
  let manifestTreeModal = $state<{ title: string; output: string } | null>(null);
  let liveGit = $state<typeof IdeGitSingleton | null>(null);

  $effect(() => {
    void import('./git/ideGitController.svelte').then((m) => {
      liveGit = m.ideGit;
    });
  });

  const normPath = (p: string): string => p.replace(/\/+$/, '') || '/';
  const shortenHome = (p: string): string => p.replace(/^\/(?:Users|home)\/[^/]+/, '~');
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
    const matchesTree = (entry: IdeFsEntry): boolean =>
      !q || entry.name.toLowerCase().includes(q) || Boolean(ide.childrenByDir[entry.path]?.some(matchesTree));
    const walk = (entries: IdeFsEntry[], depth: number): void => {
      for (let i = 0; i < entries.length; i++) {
        const entry = entries[i];
        if ((!ide.showHidden && entry.name.startsWith('.')) || !matchesTree(entry)) continue;
        out.push({ entry, depth });
        if (entry.kind === 'dir' && (ide.expandedDirs[entry.path] || Boolean(q && ide.childrenByDir[entry.path]))) {
          const kids = ide.childrenByDir[entry.path];
          if (kids) walk(kids, depth + 1);
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
        entry: { name: root.name, path: root.path, kind: 'dir' },
        depth: 0,
        isProjectRoot: true,
        isPrimaryRoot: root.isPrimary,
        defaultExpanded,
        subtitle: shortenHome(root.path),
      });
      if (isExpanded || Boolean(q)) walk(rootKids, 1);
    }
    return out;
  });

  const treeWin = $derived.by(() => {
    const total = visibleNodes.length;
    if (total <= VIRTUAL_MIN) return { virtual: false, items: visibleNodes, topPx: 0, bottomPx: 0 };
    const viewH = Math.max(clientHeight, 260);
    const top = Math.max(0, Math.min(scrollTop, Math.max(0, total * ROW_H - viewH)));
    const start = Math.max(0, Math.floor(top / ROW_H) - OVERSCAN);
    const end = Math.min(total, Math.ceil((top + viewH) / ROW_H) + OVERSCAN);
    return { virtual: true, items: visibleNodes.slice(start, end), topPx: start * ROW_H, bottomPx: Math.max(0, (total - end) * ROW_H) };
  });

  let lastRevealSeq = 0;
  $effect(() => {
    const req = ide.treeRevealRequest;
    if (!req || req.seq === lastRevealSeq) return;
    lastRevealSeq = req.seq;
    activeTab = 'files';
    void (async () => {
      await tick();
      const idx = visibleNodes.findIndex((n) => n.entry.path === req.path);
      if (idx >= 0 && treeScrollEl) {
        treeScrollEl.scrollTo({ top: Math.max(0, idx * ROW_H - clientHeight / 3), behavior: scrollBehavior() });
      }
    })();
  });

  function resolveGitLetter(node: FlatTreeNode): IdeGitLetter | null {
    if (!ideDockPrefs.gitStatusInTree || node.entry.kind === 'dir') return null;
    const live = liveGit?.treeGitOverlay.files.get(normPath(node.entry.path));
    if (live) return live;
    const raw = node.entry.git_status;
    if (raw === 'M' || raw === 'A' || raw === 'D' || raw === 'U' || raw === 'R' || raw === 'C') return raw;
    return raw === '?' ? 'U' : null;
  }

  function isDirtyFolder(node: FlatTreeNode): boolean {
    return Boolean(ideDockPrefs.gitStatusInTree && node.entry.kind === 'dir' && liveGit?.treeGitOverlay.dirtyDirs.has(normPath(node.entry.path)));
  }

  function onNodeClick(node: FlatTreeNode): void {
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
    if (node.isProjectRoot) return;
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
    void import('./history/ideTreeNodeMenus').then((m) =>
      m.showTreeNodeContextMenu(savedAnchor, node, {
        openCompare: openCompareModal,
        onOpenFolderPicker: () => (addFolderPickerOpen = true),
        onManifestOutput: (title, output) => (manifestTreeModal = { title, output }),
      }),
    );
  }
</script>

<aside class="ide-tree" aria-label="Project explorer" data-testid="ide-project-tree">
  <Tabs size="s" label="Project explorer views" idBase="ide-explorer" tabs={explorerTabs} value={activeTab} onchange={(id) => (activeTab = id)} />

  {#if activeTab === 'files'}
    <div class="tree-panel" role="tabpanel" id="ide-explorer-panel-files" aria-labelledby="ide-explorer-tab-files">
      <div class="tree-toolbar">
        <input class="tree-filter" type="search" dir="ltr" aria-label="Filter files" placeholder="Filter…" bind:value={ide.treeFilter} data-testid="ide-tree-filter" />
        <button type="button" class="tree-act" aria-label="New file" title="New file…" onclick={() => triggerNewTreeEntry(ide.target.root || selectedDirPath || '', 'create_file')}>
          <Icon name="plus" size={12} />
        </button>
        {#if ide.target.kind === 'local'}
          <button type="button" class="tree-act" aria-label="Add folder to project" title="Add folder to project…" onclick={() => (addFolderPickerOpen = true)} data-testid="ide-add-folder-btn">
            <Icon name="worktree" size={12} />
          </button>
          <button type="button" class="tree-act" aria-label="Show project local history" title="Show project local history and deleted files" onclick={() => openCompareModal({ mode: 'local-history', path: ide.target.root || selectedDirPath || '', isDir: true, filter: 'all' })} data-testid="ide-tree-local-history-btn">
            <Icon name="clock" size={12} />
          </button>
        {/if}
        <button type="button" class="tree-act" aria-label="Select opened file" title="Select opened file in tree" disabled={!ide.activePath} onclick={() => void ide.revealInProjectTree()} data-testid="ide-select-opened-file">
          <Icon name="target" size={12} />
        </button>
        <button type="button" class="tree-act" aria-label="Collapse all" title="Collapse all folders" onclick={() => ide.collapseAllDirs()} data-testid="ide-collapse-all">
          <Icon name="fold" size={12} />
        </button>
        <button type="button" class="tree-act" aria-label={ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles'} title={ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles'} aria-pressed={ide.showHidden} onclick={() => { ide.showHidden = !ide.showHidden; void ide.loadTree(); }}>
          <Icon name={ide.showHidden ? 'eye' : 'eyeOff'} size={12} />
        </button>
        <button type="button" class="tree-act" aria-label="Refresh project tree" title="Refresh project tree" onclick={() => void ide.loadTree()}>
          <Icon name="refresh" size={12} />
        </button>
      </div>

      <div class="tree-scroll" bind:this={treeScrollEl} onscroll={(e) => { const el = e.currentTarget as HTMLDivElement; scrollTop = el.scrollTop; clientHeight = el.clientHeight || 720; }}>
        <LoadState what="project files" variant="compact" loading={ide.treeLoading} error={ide.treeError} empty={visibleNodes.length === 0} onretry={() => void ide.loadTree()}>
          {#snippet emptyView()}
            <EmptyState
              headingLevel={2}
              icon="folder"
              title={ide.treeFilter.trim() ? 'No matching files' : 'Folder is empty'}
              body={ide.treeFilter.trim() ? 'Clear the filter or press ⇧⇧ to search across the project.' : 'Create your first source file to start editing.'}
              actionLabel={ide.treeFilter.trim() ? undefined : 'New file'}
              actionKind="secondary"
              onaction={ide.treeFilter.trim() ? undefined : () => triggerNewTreeEntry(ide.target.root, 'create_file')}
            />
          {/snippet}

          <div class="tree-list" class:is-virtual={treeWin.virtual} role="list">
            {#if treeWin.topPx > 0}<div style:block-size="{treeWin.topPx}px" aria-hidden="true"></div>{/if}
            {#each treeWin.items as node (`${node.isProjectRoot ? 'root:' : ''}${node.entry.path}`)}
              {@const isExpanded = node.entry.kind === 'dir' ? Boolean(ide.expandedDirs[node.entry.path] ?? node.defaultExpanded ?? false) : false}
              {@const gitLetter = resolveGitLetter(node)}
              {@const dirtyDir = isDirtyFolder(node)}
              <div
                class="tree-row"
                class:active={!node.isProjectRoot && ide.activePath === node.entry.path}
                class:is-root={Boolean(node.isProjectRoot)}
                role="listitem"
                style="padding-inline-start: calc(6px + {node.depth * 12}px)"
                data-testid={node.isProjectRoot ? (node.isPrimaryRoot ? 'ide-tree-root-node' : 'ide-tree-attached-root') : undefined}
              >
                <button
                  type="button"
                  class="tree-item"
                  dir="ltr"
                  aria-expanded={node.entry.kind === 'dir' ? isExpanded : undefined}
                  aria-current={node.entry.kind !== 'dir' && ide.activePath === node.entry.path ? 'true' : undefined}
                  onclick={() => onNodeClick(node)}
                  ondblclick={() => node.entry.kind !== 'dir' && void ide.openFile(node.entry.path, { preview: false })}
                  onkeydown={(e) => onNodeKeyDown(e, node)}
                  oncontextmenu={(e) => openNodeMenu(e, node)}
                  use:rowMenu
                  title={node.isProjectRoot ? node.entry.path : ide.relativePath(node.entry.path)}
                >
                  {#if node.entry.kind === 'dir'}
                    <span class="tree-chevron"><Icon name={isExpanded ? 'chevronDown' : 'chevronRight'} size={12} noflip /></span>
                  {:else}
                    <span class="tree-chevron" aria-hidden="true"></span>
                  {/if}
                  <span class="tree-kind" class:is-dir={node.entry.kind === 'dir'}>
                    <Icon name={node.isProjectRoot && !node.isPrimaryRoot ? 'worktree' : node.entry.kind === 'dir' ? 'folder' : 'file'} size={13} />
                  </span>
                  <span class="tree-name" class:is-root-name={Boolean(node.isProjectRoot)} data-git={gitLetter ?? undefined}>{node.entry.name}</span>
                  {#if node.subtitle}<span class="tree-root-sub">{node.subtitle}</span>{/if}
                  {#if dirtyDir}<span class="tree-dirty-dot" title="Contains uncommitted Git changes" aria-hidden="true"></span>{/if}
                  {#if gitLetter}<span class="tree-git-mark mono" data-git={gitLetter} data-testid="ide-tree-git-status">{gitLetter === 'U' ? '?' : gitLetter}</span>{/if}
                </button>
                <button type="button" class="tree-act tree-more" aria-label="Actions for {node.entry.name}" title="More actions" onclick={(e) => openNodeMenu(e, node)}>
                  <Icon name="more" size={12} />
                </button>
              </div>
            {/each}
            {#if treeWin.bottomPx > 0}<div style:block-size="{treeWin.bottomPx}px" aria-hidden="true"></div>{/if}
          </div>
        </LoadState>
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
    <pre class="dep-tree-pre mono" dir="ltr">{manifestTreeModal.output}</pre>
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
    flex: 1; min-inline-size: 40px; block-size: 22px; padding-inline: 6px; font-size: var(--fs-xs);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-s);
  }
  .tree-scroll { flex: 1; min-block-size: 0; overflow-y: auto; overflow-x: hidden; padding-block: 4px; }
  .tree-list { display: flex; flex-direction: column; gap: 2px; }
  .tree-list.is-virtual { gap: 0; }
  .tree-row { display: flex; align-items: center; padding-inline-end: 4px; min-inline-size: 0; overflow: hidden; }
  .tree-row.is-root { background: var(--surface-2); border-block-end: 1px solid var(--separator); }
  .tree-list.is-virtual .tree-row { block-size: 26px; box-sizing: border-box; }
  .tree-row:hover, .tree-row.active { background: var(--hover); }
  .tree-item {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 6px;
    padding-block: 4px; padding-inline: 2px 4px; background: transparent; border: none;
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer; overflow: hidden;
  }
  .tree-list.is-virtual .tree-item { padding-block: 2px; }
  .tree-chevron { inline-size: 12px; display: inline-flex; align-items: center; justify-content: center; color: var(--text-dim); flex-shrink: 0; }
  .tree-kind { display: inline-flex; color: var(--text-dim); flex-shrink: 0; }
  .tree-kind.is-dir { color: var(--accent-text); }
  .tree-name { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tree-name.is-root-name { font-weight: 600; flex: 0 1 auto; }
  .tree-name[data-git='M'], .tree-git-mark[data-git='M'] { color: var(--info); }
  .tree-name[data-git='A'], .tree-git-mark[data-git='A'] { color: var(--success); }
  .tree-name[data-git='U'], .tree-git-mark[data-git='U'] { color: var(--warning); }
  .tree-name[data-git='D'], .tree-name[data-git='C'], .tree-git-mark[data-git='D'], .tree-git-mark[data-git='C'] { color: var(--danger); }
  .tree-name[data-git='R'], .tree-git-mark[data-git='R'] { color: var(--accent-text); }
  .tree-row.active .tree-name { font-weight: 600; }
  .tree-dirty-dot { inline-size: 6px; block-size: 6px; border-radius: 50%; background: var(--info); flex-shrink: 0; }
  .tree-git-mark { font-size: var(--fs-xs); font-weight: 600; flex-shrink: 0; }
  .tree-root-sub { flex: 1; min-inline-size: 0; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tree-more { opacity: 0; block-size: 20px; inline-size: 20px; }
  .tree-row:hover .tree-more, .tree-row:focus-within .tree-more { opacity: 1; }
  .dep-tree-pre { max-block-size: 60vh; overflow: auto; padding: 10px; background: var(--surface-2); border-radius: var(--radius-s); font-size: var(--fs-xs); white-space: pre; }
  @media (hover: none) { .tree-more { opacity: 1; } }
</style>
