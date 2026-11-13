<script lang="ts">
  import { tick } from 'svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { scrollBehavior } from '../../lib/motion';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { rowMenu } from '../../lib/rowMenu';

  const FolderPickerLazy = lazyComponent(() => import('../../lib/components/FolderPicker.svelte'));
  import type { BadgeTone } from '../../lib/status';
  import {
    ide,
    ideBasename,
    type IdeBreakpoint,
    type IdeFsEntry,
    type IdeSymbolNode,
  } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';

  type ExplorerTab = 'files' | 'structure' | 'breakpoints';

  interface FlatTreeNode {
    entry: IdeFsEntry;
    depth: number;
    isProjectRoot?: boolean;
    isPrimaryRoot?: boolean;
    subtitle?: string;
  }

  interface FlatSymbolNode {
    sym: IdeSymbolNode;
    depth: number;
  }

  const VIRTUAL_MIN = 150;
  const ROW_H = 26;
  const OVERSCAN = 20;

  let activeTab = $state<ExplorerTab>('files');
  let treeScrollEl = $state<HTMLDivElement | undefined>(undefined);
  let scrollTop = $state(0);
  let clientHeight = $state(720);
  let addFolderPickerOpen = $state(false);
  let selectedDirPath = $state<string | null>(null);

  function shortenHome(p: string): string {
    return p.replace(/^\/(?:Users|home)\/[^/]+/, '~');
  }

  $effect(() => {
    const el = treeScrollEl;
    if (!el) return;
    clientHeight = el.clientHeight || 720;
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      clientHeight = el.clientHeight || 720;
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  const flatSymbols = $derived.by((): FlatSymbolNode[] => {
    const out: FlatSymbolNode[] = [];
    const walk = (nodes: IdeSymbolNode[], depth: number): void => {
      for (let i = 0; i < nodes.length; i++) {
        const sym = nodes[i];
        out.push({ sym, depth });
        if (sym.children?.length) walk(sym.children, depth + 1);
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

  // IntelliJ Project view: always renders the main project folder at depth 0
  // (expanded by default) plus any sibling attached folders!
  const visibleNodes = $derived.by((): FlatTreeNode[] => {
    const q = ide.treeFilter.trim().toLowerCase();
    const out: FlatTreeNode[] = [];
    const matchesTree = (entry: IdeFsEntry): boolean => {
      if (!q) return true;
      if (entry.name.toLowerCase().includes(q)) return true;
      const kids = ide.childrenByDir[entry.path];
      return Boolean(kids?.some(matchesTree));
    };
    const walk = (entries: IdeFsEntry[], depth: number): void => {
      for (let i = 0; i < entries.length; i++) {
        const entry = entries[i];
        if (!ide.showHidden && entry.name.startsWith('.')) continue;
        if (!matchesTree(entry)) continue;
        out.push({ entry, depth });
        if (entry.kind === 'dir' && (ide.expandedDirs[entry.path] || Boolean(q && ide.childrenByDir[entry.path]))) {
          const kids = ide.childrenByDir[entry.path];
          if (kids) walk(kids, depth + 1);
        }
      }
    };

    const roots = ide.projectRoots;
    for (let r = 0; r < roots.length; r++) {
      const root = roots[r];
      const rootEntry: IdeFsEntry = { name: root.name, path: root.path, kind: 'dir' };
      const rootKids = root.isPrimary
        ? ide.treeRootEntries
        : (ide.childrenByDir[root.path] ?? []);
      const isExpanded = ide.expandedDirs[root.path] ?? true;
      out.push({
        entry: rootEntry,
        depth: 0,
        isProjectRoot: true,
        isPrimaryRoot: root.isPrimary,
        subtitle: shortenHome(root.path),
      });
      if (isExpanded || Boolean(q)) {
        walk(rootKids, 1);
      }
    }
    return out;
  });

  const treeWin = $derived.by(() => {
    const total = visibleNodes.length;
    if (total <= VIRTUAL_MIN) {
      return { virtual: false, items: visibleNodes, topPx: 0, bottomPx: 0 };
    }
    const viewH = Math.max(clientHeight, 260);
    const maxScroll = Math.max(0, total * ROW_H - viewH);
    const top = Math.max(0, Math.min(scrollTop, maxScroll));
    const start = Math.max(0, Math.floor(top / ROW_H) - OVERSCAN);
    const end = Math.min(total, Math.ceil((top + viewH) / ROW_H) + OVERSCAN);
    return {
      virtual: true,
      items: visibleNodes.slice(start, end),
      topPx: start * ROW_H,
      bottomPx: Math.max(0, (total - end) * ROW_H),
    };
  });

  // Scroll to & highlight revealed file when "Select Opened File" fires.
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
        const targetTop = Math.max(0, idx * ROW_H - clientHeight / 3);
        treeScrollEl.scrollTo({ top: targetTop, behavior: scrollBehavior() });
      }
    })();
  });

  function onTreeScroll(e: Event): void {
    const el = e.currentTarget as HTMLDivElement;
    scrollTop = el.scrollTop;
    clientHeight = el.clientHeight || 720;
  }

  function gitTone(status: string | null | undefined): BadgeTone {
    if (status === 'A') return 'ok';
    if (status === 'M') return 'warn';
    if (status === 'D') return 'bad';
    return 'neutral';
  }

  function symbolTone(kind: string): BadgeTone {
    const k = kind.toLowerCase();
    if (k.includes('func') || k.includes('method')) return 'info';
    if (k.includes('class') || k.includes('struct') || k.includes('interface') || k.includes('type')) return 'accent';
    if (k.includes('const') || k.includes('enum')) return 'ok';
    return 'neutral';
  }

  function shortSymbolKind(kind: string): string {
    const k = kind.toLowerCase();
    if (k.includes('method') || k.includes('func')) return 'fn';
    if (k.includes('struct')) return 'struct';
    if (k.includes('interface')) return 'iface';
    if (k.includes('class')) return 'class';
    if (k.includes('type')) return 'type';
    if (k.includes('const')) return 'const';
    if (k.includes('field') || k.includes('prop') || k.includes('key') || k.includes('col')) return 'prop';
    return kind.slice(0, 5);
  }

  function parentDirOf(fullPath: string): string {
    const idx = fullPath.lastIndexOf('/');
    return idx > 0 ? fullPath.slice(0, idx) : ide.target.root;
  }

  function defaultCreateDir(): string {
    if (selectedDirPath) return selectedDirPath;
    if (ide.activePath) return parentDirOf(ide.activePath);
    return ide.target.root;
  }

  function onNodeClick(node: FlatTreeNode): void {
    if (node.entry.kind === 'dir') {
      selectedDirPath = node.entry.path;
      void ide.toggleDir(node.entry.path);
      return;
    }
    selectedDirPath = parentDirOf(node.entry.path);
    void ide.openFile(node.entry.path, { preview: false });
  }

  function onNodeDblClick(node: FlatTreeNode): void {
    if (node.entry.kind === 'dir') return;
    void ide.openFile(node.entry.path, { preview: false });
  }

  function onNodeKeyDown(e: KeyboardEvent, node: FlatTreeNode): void {
    if (node.isProjectRoot) return;
    if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F6') {
      e.preventDefault();
      e.stopPropagation();
      void promptRename(node.entry);
    } else if ((e.key === 'Delete' || (e.metaKey && e.key === 'Backspace')) && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      void confirmDelete(node.entry);
    }
  }

  async function promptNewEntry(baseDir: string, op: 'create_file' | 'mkdir', extHint = ''): Promise<void> {
    const isDir = op === 'mkdir';
    const relLabel = ide.relativePath(baseDir);
    const displayDir = relLabel === baseDir ? ideBasename(baseDir) : (relLabel || '.');
    const name = await confirmer.promptText(
      isDir ? `Folder name inside ${displayDir}` : `File name inside ${displayDir}`,
      {
        title: isDir ? 'New folder' : extHint === '.go' ? 'New Go file' : 'New file',
        confirmLabel: 'Create',
        placeholder: isDir ? 'pkg/handlers' : extHint === '.go' ? 'service.go' : 'handler.go',
        validate: (v) => (v.includes('..') ? 'Relative parent segments (..) are not allowed.' : null),
      },
    );
    if (!name) return;
    const cleanName = extHint && !name.includes('.') ? `${name}${extHint}` : name;
    const cleanBase = baseDir.replace(/\/+$/, '');
    const targetPath = `${cleanBase}/${cleanName.replace(/^\/+/, '')}`;
    if (isDir) await ide.mutateFs('mkdir', targetPath);
    else await ide.createFileWithTemplate(targetPath);
  }

  async function promptRename(entry: IdeFsEntry): Promise<void> {
    const nextName = await confirmer.promptText(`New name for “${entry.name}”`, {
      title: 'Rename',
      confirmLabel: 'Rename',
      initial: entry.name,
      validate: (v) => (v.includes('/') || v.includes('..') ? 'Enter a single file or folder name.' : null),
    });
    if (!nextName || nextName === entry.name) return;
    const dest = `${parentDirOf(entry.path).replace(/\/+$/, '')}/${nextName}`;
    await ide.mutateFs('rename', entry.path, dest);
  }

  async function confirmDelete(entry: IdeFsEntry): Promise<void> {
    const ok = await confirmer.ask(
      `Permanently delete “${entry.name}” from ${ide.target.kind === 'ssh' ? 'the remote host' : 'disk'}?`,
      { title: `Delete ${entry.kind === 'dir' ? 'folder' : 'file'}`, confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    await ide.mutateFs('delete', entry.path);
  }

  async function confirmDetachRoot(rootPath: string, rootName: string): Promise<void> {
    const ok = await confirmer.ask(
      `Detach “${rootName}” from this project? No files on disk will be deleted.`,
      { title: 'Remove folder from project?', confirmLabel: 'Remove folder', danger: true },
    );
    if (!ok) return;
    await ide.removeProjectFolder(rootPath);
  }

  function openNodeMenu(e: MouseEvent, node: FlatTreeNode): void {
    const entry = node.entry;
    const baseDir = entry.kind === 'dir' ? entry.path : parentDirOf(entry.path);
    const items: MenuItem[] = [];
    if (entry.kind !== 'dir') {
      items.push(
        { label: 'Open', icon: 'file', action: () => void ide.openFile(entry.path, { preview: false }) },
        { label: 'Split right', icon: 'split', action: () => void ide.openFile(entry.path, { preview: false, inSplit: true }) },
        {
          label: 'Add to Agent context',
          icon: 'assistant',
          action: () => void ideAgent.attachFile(entry.path, { openPanel: true }),
        },
        { separator: true },
      );
    }
    items.push(
      { label: 'New file…', icon: 'plus', action: () => void promptNewEntry(baseDir, 'create_file') },
      { label: 'New Go file…', icon: 'file', action: () => void promptNewEntry(baseDir, 'create_file', '.go') },
      { label: 'New folder…', icon: 'folder', action: () => void promptNewEntry(baseDir, 'mkdir') },
    );
    if (ide.target.kind === 'local') {
      items.push({
        label: 'Add folder to project…',
        icon: 'worktree',
        action: () => (addFolderPickerOpen = true),
      });
    }
    items.push(
      { separator: true },
      {
        label: 'Find in folder…',
        icon: 'search',
        hint: '⌘⇧F',
        action: () => ide.openFindInPath({ scopePath: baseDir, replace: false }),
      },
      {
        label: 'Replace in folder…',
        icon: 'edit',
        hint: '⌘⇧R',
        action: () => ide.openFindInPath({ scopePath: baseDir, replace: true }),
      },
      {
        label: 'Open in terminal',
        icon: 'terminal',
        action: () => ide.openTerminalAt(baseDir),
      },
      { separator: true },
      {
        label: 'Copy path',
        icon: 'copy',
        action: () => void copyWithToast(entry.path, 'Copied path'),
      },
      {
        label: 'Copy relative path',
        icon: 'link',
        action: () => void copyWithToast(ide.relativePath(entry.path), 'Copied relative path'),
      },
    );

    if (node.isProjectRoot) {
      items.push(
        { separator: true },
        {
          label: ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles',
          icon: ide.showHidden ? 'eyeOff' : 'eye',
          action: () => toggleHidden(),
        },
        { label: 'Refresh project tree', icon: 'refresh', action: () => void ide.loadTree() },
      );
      if (!node.isPrimaryRoot) {
        items.push(
          { separator: true },
          {
            label: 'Remove folder from project…',
            icon: 'trash',
            danger: true,
            action: () => void confirmDetachRoot(entry.path, entry.name),
          },
        );
      }
    } else {
      items.push(
        { separator: true },
        { label: 'Rename…', icon: 'edit', hint: '⇧F6', action: () => void promptRename(entry) },
      );
      if (entry.kind !== 'dir') {
        items.push({ label: 'Duplicate', icon: 'copy', action: () => void ide.mutateFs('duplicate', entry.path) });
      }
      items.push(
        { separator: true },
        { label: 'Delete…', icon: 'trash', hint: '⌘⌫', danger: true, action: () => void confirmDelete(entry) },
      );
    }
    ctxMenu.show(e, items);
  }

  function openSymbolMenu(e: MouseEvent, sym: IdeSymbolNode): void {
    const path = ide.activePath;
    if (!path) return;
    ctxMenu.show(e, [
      { label: 'Jump to symbol', icon: 'target', action: () => ide.jumpTo(path, sym.line, 1) },
      {
        label: 'Add to Agent context',
        icon: 'assistant',
        action: () => ideAgent.attachSymbol(path, sym, { openPanel: true }),
      },
    ]);
  }

  async function editBreakpointCondition(bp: IdeBreakpoint): Promise<void> {
    const cond = await confirmer.promptText(
      `Break when expression is true at ${ideBasename(bp.path)}:${bp.line} (leave empty to clear)`,
      {
        title: 'Breakpoint condition',
        confirmLabel: 'Save condition',
        initial: bp.condition ?? '',
        placeholder: 'err != nil && count > 0',
      },
    );
    if (cond === null) return;
    ide.updateBreakpoint(bp.path, bp.line, { condition: cond.trim() || null });
  }

  function toggleHidden(): void {
    ide.showHidden = !ide.showHidden;
    void ide.loadTree();
  }
</script>

<aside class="ide-tree" aria-label="Project explorer" data-testid="ide-project-tree">
  <Tabs size="s" label="Project explorer views" idBase="ide-explorer" tabs={explorerTabs} value={activeTab} onchange={(id) => (activeTab = id)} />

  {#if activeTab === 'files'}
    <div class="tree-panel" role="tabpanel" id="ide-explorer-panel-files" aria-labelledby="ide-explorer-tab-files">
      <div class="tree-toolbar">
        <input class="tree-filter" type="search" dir="ltr" aria-label="Filter files" placeholder="Filter…" bind:value={ide.treeFilter} data-testid="ide-tree-filter" />
        <button type="button" class="icon-btn" aria-label="New file" title="New file…" onclick={() => void promptNewEntry(ide.target.root || defaultCreateDir(), 'create_file')}>
          <Icon name="plus" size={12} />
        </button>
        {#if ide.target.kind === 'local'}
          <button type="button" class="icon-btn" aria-label="Add folder to project" title="Add folder to project…" onclick={() => (addFolderPickerOpen = true)} data-testid="ide-add-folder-btn">
            <Icon name="worktree" size={12} />
          </button>
        {/if}
        <button type="button" class="icon-btn" aria-label="Select opened file" title="Select opened file in tree" disabled={!ide.activePath} onclick={() => void ide.revealInProjectTree()} data-testid="ide-select-opened-file">
          <Icon name="target" size={12} />
        </button>
        <button type="button" class="icon-btn" aria-label="Collapse all" title="Collapse all folders" onclick={() => ide.collapseAllDirs()} data-testid="ide-collapse-all">
          <Icon name="fold" size={12} />
        </button>
        <button type="button" class="icon-btn" aria-label={ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles'} title={ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles'} aria-pressed={ide.showHidden} onclick={toggleHidden}>
          <Icon name={ide.showHidden ? 'eye' : 'eyeOff'} size={12} />
        </button>
        <button type="button" class="icon-btn" aria-label="Refresh project tree" title="Refresh project tree" onclick={() => void ide.loadTree()}>
          <Icon name="refresh" size={12} />
        </button>
      </div>

      <div class="tree-scroll" bind:this={treeScrollEl} onscroll={onTreeScroll}>
        <LoadState what="project files" variant="compact" loading={ide.treeLoading} error={ide.treeError} empty={visibleNodes.length === 0} onretry={() => void ide.loadTree()}>
          {#snippet emptyView()}
            <EmptyState
              headingLevel={2}
              icon="folder"
              title={ide.treeFilter.trim() ? 'No matching files' : 'Folder is empty'}
              body={ide.treeFilter.trim() ? 'Clear the filter or press ⇧⇧ to search across the project.' : 'Create your first source file to start editing.'}
              actionLabel={ide.treeFilter.trim() ? undefined : 'New file'}
              actionKind="secondary"
              onaction={ide.treeFilter.trim() ? undefined : () => void promptNewEntry(ide.target.root, 'create_file')}
            />
          {/snippet}

          <div class="tree-list" class:is-virtual={treeWin.virtual} role="list">
            {#if treeWin.topPx > 0}<div style:block-size="{treeWin.topPx}px" aria-hidden="true"></div>{/if}
            {#each treeWin.items as node (`${node.isProjectRoot ? 'root:' : ''}${node.entry.path}`)}
              {@const isExpanded = node.entry.kind === 'dir' ? Boolean(ide.expandedDirs[node.entry.path] ?? node.isProjectRoot) : false}
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
                  ondblclick={() => onNodeDblClick(node)}
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
                  <span class="tree-name" class:is-root-name={Boolean(node.isProjectRoot)}>{node.entry.name}</span>
                  {#if node.subtitle}<span class="tree-root-sub">{node.subtitle}</span>{/if}
                  {#if node.entry.git_status}<Badge variant="outline" tone={gitTone(node.entry.git_status)} label={node.entry.git_status} />{/if}
                </button>
                <button type="button" class="icon-btn tree-more" aria-label="Actions for {node.entry.name}" title="Actions" onclick={(e) => openNodeMenu(e, node)}>
                  <Icon name="more" size={12} />
                </button>
              </div>
            {/each}
            {#if treeWin.bottomPx > 0}<div style:block-size="{treeWin.bottomPx}px" aria-hidden="true"></div>{/if}
          </div>
        </LoadState>
      </div>
    </div>
  {:else if activeTab === 'structure'}
    <div class="tree-panel" role="tabpanel" id="ide-explorer-panel-structure" aria-labelledby="ide-explorer-tab-structure">
      <div class="tree-scroll">
        {#if !ide.activeTab}
          <EmptyState headingLevel={2} icon="layers" title="No file open" body="Select a source file to inspect its classes, functions, keys, and columns." />
        {:else}
          <LoadState what="file outline" variant="compact" loading={ide.symbolsLoading} empty={flatSymbols.length === 0}>
            {#snippet emptyView()}
              <EmptyState headingLevel={2} icon="layers" title="No outline symbols" body="Symbols appear automatically for Go, TypeScript, Java, Rust, Python, Shell, JSON, and CSV files." />
            {/snippet}

            <div class="tree-list">
              {#each flatSymbols as item, idx (`${item.sym.name}:${item.sym.line}:${idx}`)}
                {@const isCurrent = ide.cursorInfo.line >= item.sym.line && ide.cursorInfo.line <= Math.max(item.sym.line, item.sym.end_line)}
                <div class="struct-line-wrap" class:active={isCurrent}>
                  <button
                    type="button"
                    class="struct-row"
                    aria-current={isCurrent ? 'true' : undefined}
                    dir="ltr"
                    style="padding-inline-start: calc(10px + {item.depth * 12}px)"
                    onclick={() => ide.activePath && ide.jumpTo(ide.activePath, item.sym.line, 1)}
                    oncontextmenu={(e) => openSymbolMenu(e, item.sym)}
                    use:rowMenu
                  >
                    <Badge variant="outline" tone={symbolTone(item.sym.kind)} label={shortSymbolKind(item.sym.kind)} />
                    <span class="tree-name">{item.sym.name}</span>
                    <span class="struct-line">:{item.sym.line}</span>
                  </button>
                  <button
                    type="button"
                    class="icon-btn struct-agent-btn"
                    aria-label="Add {item.sym.name} to Agent context"
                    title="Add to Agent context"
                    onclick={() => ide.activePath && ideAgent.attachSymbol(ide.activePath, item.sym, { openPanel: true })}
                    data-testid="ide-symbol-add-agent"
                  >
                    <Icon name="assistant" size={12} />
                  </button>
                </div>
              {/each}
            </div>
          </LoadState>
        {/if}
      </div>
    </div>
  {:else}
    <div class="tree-panel" role="tabpanel" id="ide-explorer-panel-breakpoints" aria-labelledby="ide-explorer-tab-breakpoints">
      <div class="bp-bar">
        <div class="bp-exc-toggle">
          <Switch checked={ide.breakpointsMuted} label="Mute all breakpoints" onchange={() => ide.toggleMuteBreakpoints()} />
          <span class="bp-toggle-text">Mute all</span>
        </div>
        <div class="bp-exc-toggle">
          <Switch checked={ide.breakOnExceptions} label="Break on uncaught exceptions" onchange={(next) => { ide.breakOnExceptions = next; void ide.syncBreakpointsToSession(); }} />
          <span class="bp-toggle-text">Exceptions</span>
        </div>
      </div>
      <div class="tree-scroll">
        {#if ide.allBreakpoints.length === 0}
          <EmptyState headingLevel={2} icon="bug" title="No breakpoints" body="Click the editor line gutter or press ⌘F8 to toggle a breakpoint." />
        {:else}
          <div class="tree-list">
            {#each ide.allBreakpoints as bp (`${bp.path}:${bp.line}`)}
              <div class="bp-row">
                <Switch checked={bp.enabled} label="Enable breakpoint at {ideBasename(bp.path)} line {bp.line}" onchange={(next) => ide.updateBreakpoint(bp.path, bp.line, { enabled: next })} />
                <button type="button" class="bp-jump" dir="ltr" onclick={() => void ide.openFile(bp.path, { line: bp.line })} title={ide.relativePath(bp.path)}>
                  <span class="tree-name">{ideBasename(bp.path)}:{bp.line}</span>
                  {#if bp.condition}<Badge variant="outline" tone="warn" label="if {bp.condition}" />{:else if bp.log_message}<Badge variant="outline" tone="info" label="log" />{/if}
                </button>
                <button type="button" class="icon-btn" aria-label="Edit breakpoint condition" title="Edit breakpoint condition…" onclick={() => void editBreakpointCondition(bp)}>
                  <Icon name="edit" size={12} />
                </button>
                <button type="button" class="icon-btn" aria-label="Remove breakpoint" title="Remove breakpoint" onclick={() => ide.removeBreakpoint(bp.path, bp.line)}>
                  <Icon name="x" size={12} />
                </button>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}
</aside>

{#if addFolderPickerOpen}
  <LazyMount
    lazy={FolderPickerLazy}
    what="folder picker"
    quiet
    props={{
      title: 'Add folder to project',
      start: ide.target.kind === 'local' ? ide.target.root : '',
      onpick: (picked: string) => {
        addFolderPickerOpen = false;
        void ide.addProjectFolder(picked);
      },
      onclose: () => (addFolderPickerOpen = false),
    }}
    onerror={() => (addFolderPickerOpen = false)}
  />
{/if}

<style>
  .ide-tree {
    display: flex; flex-direction: column; min-inline-size: 0; min-block-size: 0; block-size: 100%;
    background: var(--surface); border-inline-end: 1px solid var(--border);
  }
  .tree-panel { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; }
  .tree-toolbar {
    display: flex; align-items: center; gap: 2px; padding-block: 4px; padding-inline: 6px;
    border-block-end: 1px solid var(--border); min-inline-size: 0;
  }
  .tree-filter {
    flex: 1; min-inline-size: 48px; block-size: 24px; padding-inline: 6px; font-size: var(--fs-xs);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-s);
  }
  .tree-scroll { flex: 1; min-block-size: 0; overflow-y: auto; overflow-x: hidden; padding-block: 4px; }
  .tree-list { display: flex; flex-direction: column; gap: 1px; }
  .tree-list.is-virtual { gap: 0; }
  .tree-row { display: flex; align-items: center; padding-inline-end: 4px; min-inline-size: 0; }
  .tree-row.is-root { background: var(--surface-2); border-block-end: 1px solid var(--separator); }
  .tree-list.is-virtual .tree-row { block-size: 26px; box-sizing: border-box; }
  .tree-row:hover, .tree-row.active { background: var(--hover); }
  .tree-row.active .tree-name { color: var(--accent-text); font-weight: 500; }
  .tree-item {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 6px;
    padding-block: 4px; padding-inline: 2px 4px; background: transparent; border: none;
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .tree-list.is-virtual .tree-item { padding-block: 2px; }
  .tree-chevron { inline-size: 12px; display: inline-flex; align-items: center; justify-content: center; color: var(--text-dim); flex-shrink: 0; }
  .tree-kind { display: inline-flex; color: var(--text-dim); flex-shrink: 0; }
  .tree-kind.is-dir { color: var(--accent-text); }
  .tree-name { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tree-name.is-root-name { font-weight: 600; flex: 0 1 auto; }
  .tree-root-sub {
    flex: 1; min-inline-size: 0; font-family: var(--font-mono); font-size: var(--fs-xs);
    color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .tree-more, .struct-agent-btn { opacity: 0; flex-shrink: 0; }
  .tree-row:hover .tree-more, .tree-row:focus-within .tree-more,
  .struct-line-wrap:hover .struct-agent-btn, .struct-line-wrap:focus-within .struct-agent-btn { opacity: 1; }
  .struct-line-wrap { display: flex; align-items: center; padding-inline-end: 4px; min-inline-size: 0; }
  .struct-line-wrap:hover, .struct-line-wrap.active { background: var(--surface-2); }
  .struct-row {
    display: flex; align-items: center; gap: 6px; flex: 1; min-inline-size: 0;
    padding-block: 4px; padding-inline-end: 4px; background: transparent; border: none;
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .struct-line { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); flex-shrink: 0; }
  .bp-bar {
    display: flex; flex-wrap: wrap; align-items: center; gap: 12px;
    padding-block: 6px; padding-inline: 10px; border-block-end: 1px solid var(--border);
  }
  .bp-exc-toggle { display: flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-dim); }
  .bp-toggle-text { font-size: var(--fs-xs); color: var(--text-dim); }
  .bp-row { display: flex; align-items: center; gap: 6px; padding-block: 4px; padding-inline: 8px; min-inline-size: 0; }
  .bp-row:hover { background: var(--surface-2); }
  .bp-jump {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 6px;
    background: transparent; border: none; color: var(--text);
    font-family: var(--font-mono); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  @media (hover: none) {
    .tree-more, .struct-agent-btn { opacity: 1; }
  }
</style>
