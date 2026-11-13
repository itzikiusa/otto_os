<script lang="ts">
  import { copyWithToast } from '../../lib/clipboard';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { rowMenu } from '../../lib/rowMenu';
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
    {
      id: 'structure',
      label: 'Structure',
      count: flatSymbols.length || undefined,
      testid: 'ide-tree-tab-structure',
    },
    {
      id: 'breakpoints',
      label: 'Breakpoints',
      count: ide.allBreakpoints.length || undefined,
      testid: 'ide-tree-tab-breakpoints',
    },
  ]);

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
    walk(ide.treeRootEntries, 0);
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
    if (k.includes('method')) return 'fn';
    if (k.includes('func')) return 'fn';
    if (k.includes('struct')) return 'struct';
    if (k.includes('interface')) return 'iface';
    if (k.includes('class')) return 'class';
    if (k.includes('type')) return 'type';
    if (k.includes('const')) return 'const';
    if (k.includes('field') || k.includes('prop')) return 'prop';
    return kind.slice(0, 5);
  }

  function parentDirOf(fullPath: string): string {
    const idx = fullPath.lastIndexOf('/');
    return idx > 0 ? fullPath.slice(0, idx) : ide.target.root;
  }

  function onNodeClick(entry: IdeFsEntry): void {
    if (entry.kind === 'dir') {
      void ide.toggleDir(entry.path);
      return;
    }
    void ide.openFile(entry.path, { preview: true });
  }

  function onNodeDblClick(entry: IdeFsEntry): void {
    if (entry.kind === 'dir') return;
    void ide.openFile(entry.path, { preview: false });
  }

  async function promptNewEntry(baseDir: string, op: 'create_file' | 'mkdir'): Promise<void> {
    const isDir = op === 'mkdir';
    const name = await confirmer.promptText(
      isDir ? `Folder name inside ${ide.relativePath(baseDir) || '.'}` : `File name inside ${ide.relativePath(baseDir) || '.'}`,
      {
        title: isDir ? 'New folder' : 'New file',
        confirmLabel: 'Create',
        placeholder: isDir ? 'pkg/handlers' : 'handler.go',
        validate: (v) => (v.includes('..') ? 'Relative parent segments (..) are not allowed.' : null),
      },
    );
    if (!name) return;
    const cleanBase = baseDir.replace(/\/+$/, '');
    const targetPath = `${cleanBase}/${name.replace(/^\/+/, '')}`;
    await ide.mutateFs(op, targetPath);
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

  function openNodeMenu(e: MouseEvent, entry: IdeFsEntry): void {
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
      { label: 'New folder…', icon: 'folder', action: () => void promptNewEntry(baseDir, 'mkdir') },
      { separator: true },
      { label: 'Rename…', icon: 'edit', action: () => void promptRename(entry) },
    );
    if (entry.kind !== 'dir') {
      items.push({ label: 'Duplicate', icon: 'copy', action: () => void ide.mutateFs('duplicate', entry.path) });
    }
    items.push(
      {
        label: 'Copy relative path',
        icon: 'link',
        action: () => void copyWithToast(ide.relativePath(entry.path), 'Copied relative path'),
      },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void confirmDelete(entry) },
    );
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
  <Tabs
    size="s"
    label="Project explorer views"
    idBase="ide-explorer"
    tabs={explorerTabs}
    value={activeTab}
    onchange={(id) => (activeTab = id)}
  />

  {#if activeTab === 'files'}
    <div
      class="tree-panel"
      role="tabpanel"
      id="ide-explorer-panel-files"
      aria-labelledby="ide-explorer-tab-files"
    >
      <div class="tree-toolbar">
        <input
          class="tree-filter"
          type="search"
          dir="ltr"
          aria-label="Filter files"
          placeholder="Filter files…"
          bind:value={ide.treeFilter}
          data-testid="ide-tree-filter"
        />
        <button
          type="button"
          class="icon-btn"
          aria-label="New file"
          title="New file…"
          onclick={() => void promptNewEntry(ide.target.root, 'create_file')}
        >
          <Icon name="plus" size={12} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="New folder"
          title="New folder…"
          onclick={() => void promptNewEntry(ide.target.root, 'mkdir')}
        >
          <Icon name="folder" size={12} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label={ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles'}
          title={ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles'}
          aria-pressed={ide.showHidden}
          onclick={toggleHidden}
        >
          <Icon name={ide.showHidden ? 'eye' : 'eyeOff'} size={12} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Refresh project tree"
          title="Refresh project tree"
          onclick={() => void ide.loadTree()}
        >
          <Icon name="refresh" size={12} />
        </button>
      </div>

      <div class="tree-scroll" bind:this={treeScrollEl} onscroll={onTreeScroll}>
        <LoadState
          what="project files"
          variant="compact"
          loading={ide.treeLoading}
          error={ide.treeError}
          empty={visibleNodes.length === 0}
          onretry={() => void ide.loadTree()}
        >
          {#snippet emptyView()}
            <EmptyState
              headingLevel={2}
              icon="folder"
              title={ide.treeFilter.trim() ? 'No matching files' : 'Folder is empty'}
              body={ide.treeFilter.trim()
                ? 'Clear the filter or press ⇧⇧ to search across the project.'
                : 'Create your first source file to start editing.'}
              actionLabel={ide.treeFilter.trim() ? undefined : 'New file'}
              actionKind="secondary"
              onaction={ide.treeFilter.trim() ? undefined : () => void promptNewEntry(ide.target.root, 'create_file')}
            />
          {/snippet}

          <div class="tree-list" class:is-virtual={treeWin.virtual} role="list">
            {#if treeWin.topPx > 0}
              <div style:block-size="{treeWin.topPx}px" aria-hidden="true"></div>
            {/if}
            {#each treeWin.items as node (node.entry.path)}
              <div
                class="tree-row"
                class:active={ide.activePath === node.entry.path}
                role="listitem"
                style="padding-inline-start: calc(6px + {node.depth * 12}px)"
              >
                <button
                  type="button"
                  class="tree-item"
                  dir="ltr"
                  aria-expanded={node.entry.kind === 'dir' ? Boolean(ide.expandedDirs[node.entry.path]) : undefined}
                  aria-current={node.entry.kind !== 'dir' && ide.activePath === node.entry.path ? 'true' : undefined}
                  onclick={() => onNodeClick(node.entry)}
                  ondblclick={() => onNodeDblClick(node.entry)}
                  oncontextmenu={(e) => openNodeMenu(e, node.entry)}
                  use:rowMenu
                  title={ide.relativePath(node.entry.path)}
                >
                  {#if node.entry.kind === 'dir'}
                    <span class="tree-chevron">
                      <Icon
                        name={ide.expandedDirs[node.entry.path] ? 'chevronDown' : 'chevronRight'}
                        size={12}
                        noflip
                      />
                    </span>
                  {:else}
                    <span class="tree-chevron" aria-hidden="true"></span>
                  {/if}
                  <span class="tree-kind" class:is-dir={node.entry.kind === 'dir'}>
                    <Icon name={node.entry.kind === 'dir' ? 'folder' : 'file'} size={13} />
                  </span>
                  <span class="tree-name">{node.entry.name}</span>
                  {#if node.entry.git_status}
                    <Badge
                      variant="outline"
                      tone={gitTone(node.entry.git_status)}
                      label={node.entry.git_status}
                    />
                  {/if}
                </button>
                <button
                  type="button"
                  class="icon-btn tree-more"
                  aria-label="Actions for {node.entry.name}"
                  title="Actions"
                  onclick={(e) => openNodeMenu(e, node.entry)}
                >
                  <Icon name="more" size={12} />
                </button>
              </div>
            {/each}
            {#if treeWin.bottomPx > 0}
              <div style:block-size="{treeWin.bottomPx}px" aria-hidden="true"></div>
            {/if}
          </div>
        </LoadState>
      </div>
    </div>
  {:else if activeTab === 'structure'}
    <div
      class="tree-panel"
      role="tabpanel"
      id="ide-explorer-panel-structure"
      aria-labelledby="ide-explorer-tab-structure"
    >
      <div class="tree-scroll">
        {#if !ide.activeTab}
          <EmptyState
            headingLevel={2}
            icon="layers"
            title="No file open"
            body="Select a source file to inspect its classes, functions, and types."
          />
        {:else}
          <LoadState
            what="file outline"
            variant="compact"
            loading={ide.symbolsLoading}
            empty={flatSymbols.length === 0}
          >
            {#snippet emptyView()}
              <EmptyState
                headingLevel={2}
                icon="layers"
                title="No outline symbols"
                body="Symbols appear automatically for supported code files."
              />
            {/snippet}

            <div class="tree-list">
              {#each flatSymbols as item, idx (`${item.sym.name}:${item.sym.line}:${idx}`)}
                {@const isCurrent =
                  ide.cursorInfo.line >= item.sym.line &&
                  ide.cursorInfo.line <= Math.max(item.sym.line, item.sym.end_line)}
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
    <div
      class="tree-panel"
      role="tabpanel"
      id="ide-explorer-panel-breakpoints"
      aria-labelledby="ide-explorer-tab-breakpoints"
    >
      <div class="bp-bar">
        <div class="bp-exc-toggle">
          <Switch
            checked={ide.breakOnExceptions}
            label="Break on uncaught exceptions"
            onchange={(next) => {
              ide.breakOnExceptions = next;
              void ide.syncBreakpointsToSession();
            }}
          />
          <button
            type="button"
            tabindex="-1"
            class="bp-exc-label"
            onclick={() => {
              ide.breakOnExceptions = !ide.breakOnExceptions;
              void ide.syncBreakpointsToSession();
            }}
          >Break on uncaught exceptions</button>
        </div>
      </div>
      <div class="tree-scroll">
        {#if ide.allBreakpoints.length === 0}
          <EmptyState
            headingLevel={2}
            icon="bug"
            title="No breakpoints"
            body="Click the editor line gutter or press ⌘F8 to toggle a breakpoint."
          />
        {:else}
          <div class="tree-list">
            {#each ide.allBreakpoints as bp (`${bp.path}:${bp.line}`)}
              <div class="bp-row">
                <Switch
                  checked={bp.enabled}
                  label="Enable breakpoint at {ideBasename(bp.path)} line {bp.line}"
                  onchange={(next) => ide.updateBreakpoint(bp.path, bp.line, { enabled: next })}
                />
                <button
                  type="button"
                  class="bp-jump"
                  dir="ltr"
                  onclick={() => void ide.openFile(bp.path, { line: bp.line })}
                  title={ide.relativePath(bp.path)}
                >
                  <span class="tree-name">{ideBasename(bp.path)}:{bp.line}</span>
                  {#if bp.condition}
                    <Badge variant="outline" tone="warn" label="if {bp.condition}" />
                  {:else if bp.log_message}
                    <Badge variant="outline" tone="info" label="log" />
                  {/if}
                </button>
                <button
                  type="button"
                  class="icon-btn"
                  aria-label="Edit breakpoint condition"
                  title="Edit breakpoint condition…"
                  onclick={() => void editBreakpointCondition(bp)}
                >
                  <Icon name="edit" size={12} />
                </button>
                <button
                  type="button"
                  class="icon-btn"
                  aria-label="Remove breakpoint"
                  title="Remove breakpoint"
                  onclick={() => ide.removeBreakpoint(bp.path, bp.line)}
                >
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

<style>
  .ide-tree {
    display: flex; flex-direction: column; min-inline-size: 0; min-block-size: 0; block-size: 100%;
    background: var(--surface); border-inline-end: 1px solid var(--border);
  }
  .tree-panel { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; }
  .tree-toolbar {
    display: flex; align-items: center; gap: 4px; padding-block: 6px; padding-inline: 8px;
    border-block-end: 1px solid var(--border);
  }
  .tree-filter {
    flex: 1; min-inline-size: 0; block-size: 26px; padding-inline: 8px; font-size: var(--fs-xs);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-s);
  }
  .tree-scroll { flex: 1; min-block-size: 0; overflow-y: auto; overflow-x: hidden; padding-block: 4px; }
  .tree-list { display: flex; flex-direction: column; gap: 2px; }
  .tree-list.is-virtual { gap: 0; }
  .tree-row { display: flex; align-items: center; padding-inline-end: 4px; min-inline-size: 0; }
  .tree-list.is-virtual .tree-row { block-size: 26px; box-sizing: border-box; }
  .tree-row:hover, .tree-row.active { background: var(--surface-2); }
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
  .bp-bar { padding-block: 6px; padding-inline: 10px; border-block-end: 1px solid var(--border); }
  .bp-exc-toggle { display: flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-dim); }
  .bp-exc-label { border: 0; background: transparent; padding: 0; font-size: var(--fs-xs); color: var(--text-dim); cursor: pointer; }
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
