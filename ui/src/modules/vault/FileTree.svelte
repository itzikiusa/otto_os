<script lang="ts">
  // Vault file explorer — lazy directory tree over the store's TreeNode roots,
  // virtualized (big vaults stay cheap), with ctx-menu file ops and
  // drag-to-folder moves.
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { vault, type TreeNode } from './vault.svelte';

  let {
    onNewNote,
  }: {
    /** Open the new-note dialog seeded with a folder. */
    onNewNote: (dir: string) => void;
  } = $props();

  // Re-derive the flat list from tree state (open/loaded live on the nodes).
  const flat = $derived.by(() => {
    // Touch roots deeply so toggles re-run this.
    void vault.roots;
    return vault.flatTree();
  });

  let renaming = $state<string | null>(null);
  let renameValue = $state('');
  let dragOver = $state<string | null>(null);

  // Multi-select (notes/files, not dirs): hover checkboxes; any selection
  // keeps every checkbox visible and shows the group action bar.
  let selected = $state<Set<string>>(new Set());

  function toggleSelect(path: string): void {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }

  function clearSelection(): void {
    selected = new Set();
  }

  function groupReviewFix(): void {
    vault.sendGroupToAgent([...selected], null);
    clearSelection();
  }

  // Free-form group instruction, typed INLINE in the selection bar —
  // window.prompt() does not exist in the desktop webview (silent no-op).
  let groupPrompt = $state('');
  let groupInputEl = $state<HTMLInputElement | null>(null);

  function groupSend(): void {
    const inst = groupPrompt.trim();
    if (!inst) {
      groupInputEl?.focus();
      return;
    }
    vault.sendGroupToAgent([...selected], inst);
    groupPrompt = '';
    clearSelection();
  }

  /** Mode-aware selection — only the pane actually shown highlights its row. */
  function isActive(n: TreeNode): boolean {
    return vault.centerMode === 'note'
      ? vault.notePath === n.entry.path
      : vault.centerMode === 'file' && vault.filePath === n.entry.path;
  }

  function rowClick(n: TreeNode, e?: MouseEvent | KeyboardEvent): void {
    if (n.entry.kind === 'dir') {
      void vault.toggleDir(n);
      return;
    }
    // ⌘/Ctrl-click → toggle multi-select (OS file-manager convention). Open
    // in a new tab moved to middle-click (onauxclick) / the context menu.
    if (e && (e.metaKey || e.ctrlKey)) {
      toggleSelect(n.entry.path);
      return;
    }
    if (n.entry.kind === 'note') void vault.open(n.entry.path);
    else void vault.openFile(n.entry.path);
  }

  function rowAuxClick(n: TreeNode, e: MouseEvent): void {
    if (e.button !== 1 || n.entry.kind === 'dir') return;
    e.preventDefault();
    if (n.entry.kind === 'note') void vault.open(n.entry.path, { newTab: true });
    else void vault.openFile(n.entry.path, { newTab: true });
  }

  function startRename(n: TreeNode): void {
    renaming = n.entry.path;
    renameValue = n.entry.name;
  }

  function commitRename(n: TreeNode): void {
    const name = renameValue.trim();
    renaming = null;
    if (!name || name === n.entry.name) return;
    const dir = n.entry.path.includes('/')
      ? n.entry.path.slice(0, n.entry.path.lastIndexOf('/') + 1)
      : '';
    void vault.rename(n.entry.path, dir + name);
  }

  function menu(e: MouseEvent, n: TreeNode): void {
    const isDir = n.entry.kind === 'dir';
    ctxMenu.show(e, [
      ...(isDir
        ? []
        : [
            {
              label: 'Open in new tab',
              icon: n.entry.kind === 'note' ? 'note' : 'file',
              action: () =>
                n.entry.kind === 'note'
                  ? void vault.open(n.entry.path, { newTab: true })
                  : void vault.openFile(n.entry.path, { newTab: true }),
            },
            // With a multi-selection containing this note, Review + fix acts
            // on the WHOLE selection (same as the selection bar) — a single
            // right-clicked note out of N selected being fixed alone is never
            // what the user meant.
            ...(n.entry.kind === 'note'
              ? [
                  selected.size > 1 && selected.has(n.entry.path)
                    ? {
                        label: `Review + fix ${selected.size} selected (agent)`,
                        icon: 'zap',
                        action: () => groupReviewFix(),
                      }
                    : {
                        label: 'Review + fix (agent)',
                        icon: 'zap',
                        action: () => vault.reviewFixNote(n.entry.path),
                      },
                ]
              : []),
            { separator: true },
          ]),
      ...(isDir
        ? [
            { label: 'New note here', icon: 'note', action: () => onNewNote(n.entry.path) },
            {
              label: 'Docs agent here',
              icon: 'zap',
              action: () => vault.openDocsAgents(n.entry.path),
            },
            {
              label: 'Review + fix docs (agent)',
              icon: 'zap',
              action: () => vault.reviewFixBundle(n.entry.path),
            },
            {
              // Opens the docs-agent form scoped to the folder — the form's
              // prompt box is where the instruction is written (window.prompt
              // does not exist in the desktop webview).
              label: 'Send to agent…',
              icon: 'zap',
              action: () => vault.sendDirToAgent(n.entry.path, ''),
            },
            {
              label: 'New folder here',
              icon: 'folder',
              action: () => {
                const name = prompt('Folder name');
                if (name?.trim()) void vault.createFolder(`${n.entry.path}/${name.trim()}`);
              },
            },
            { separator: true },
          ]
        : []),
      { label: 'Rename', icon: 'edit', action: () => startRename(n) },
      {
        label: 'Move to…',
        icon: 'branch',
        action: () => {
          const to = prompt('Move to path', n.entry.path);
          if (to?.trim() && to.trim() !== n.entry.path) void vault.rename(n.entry.path, to.trim());
        },
      },
      { separator: true },
      {
        label: 'Delete (→ .trash)',
        icon: 'trash',
        danger: true,
        action: () => void vault.trash(n.entry.path),
      },
    ]);
  }

  // Drag a file onto a folder row → move.
  function onDragStart(e: DragEvent, n: TreeNode): void {
    e.dataTransfer?.setData('text/vault-path', n.entry.path);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDrop(e: DragEvent, n: TreeNode): void {
    e.preventDefault();
    dragOver = null;
    const from = e.dataTransfer?.getData('text/vault-path');
    if (!from || n.entry.kind !== 'dir') return;
    const base = from.split('/').pop()!;
    const to = `${n.entry.path}/${base}`;
    if (to !== from && !to.startsWith(`${from}/`)) void vault.rename(from, to);
  }
</script>

<div class="tree" role="tree">
  {#if flat.length === 0}
    <div class="empty">No notes yet — create one.</div>
  {:else}
    <VirtualList items={flat} estimateHeight={26} class="tree-list">
      {#snippet row(n: TreeNode)}
        <div
          class="row {n.entry.kind}"
          class:active={isActive(n)}
          class:reserved={n.entry.reserved}
          class:drag-over={dragOver === n.entry.path}
          style="padding-inline-start: {8 + n.depth * 14}px"
          role="treeitem"
          class:checked={selected.has(n.entry.path)}
          aria-selected={isActive(n)}
          tabindex="0"
          draggable={n.entry.kind !== 'dir'}
          onclick={(e) => rowClick(n, e)}
          onauxclick={(e) => rowAuxClick(n, e)}
          onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && rowClick(n, e)}
          oncontextmenu={(e) => menu(e, n)}
          ondragstart={(e) => onDragStart(e, n)}
          ondragover={(e) => {
            if (n.entry.kind === 'dir') {
              e.preventDefault();
              dragOver = n.entry.path;
            }
          }}
          ondragleave={() => (dragOver = null)}
          ondrop={(e) => onDrop(e, n)}
        >
          {#if n.entry.kind === 'dir'}
            <span class="chev" class:open={n.open}>▸</span>
            <Icon name="folder" size={14} />
          {:else}
            <input
              type="checkbox"
              class="sel"
              class:vis={selected.size > 0}
              checked={selected.has(n.entry.path)}
              aria-label="Select for group agent actions"
              onclick={(e) => {
                e.stopPropagation();
                toggleSelect(n.entry.path);
              }}
            />
            <Icon name={n.entry.kind === 'note' ? 'note' : 'file'} size={14} />
          {/if}
          <span class="name" title={n.entry.path}>
            {n.entry.kind === 'note' ? n.entry.name.replace(/\.md$/i, '') : n.entry.name}
          </span>
          {#if n.entry.kind === 'dir'}
            {@const prov = vault.draftAgentLabel(n.entry.path)}
            {#if prov}
              <span class="prov" title="Writer agent">{prov}</span>
            {/if}
          {/if}
          {#if renaming === n.entry.path}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="rename"
              bind:value={renameValue}
              autofocus
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => {
                if (e.key === 'Enter') commitRename(n);
                if (e.key === 'Escape') renaming = null;
                e.stopPropagation();
              }}
              onblur={() => commitRename(n)}
            />
          {/if}
          {#if n.entry.kind === 'dir' && n.entry.children > 0}
            <span class="count">{n.entry.children}</span>
          {/if}
        </div>
      {/snippet}
    </VirtualList>
  {/if}

  {#if selected.size > 0}
    <div class="sel-bar">
      <div class="sel-row">
        <span class="sel-count">{selected.size} selected</span>
        <button
          class="ghost"
          title="Review + fix ALL {selected.size} selected notes as one coherent set (starts the agent immediately)"
          onclick={groupReviewFix}
        >
          Review + fix
        </button>
        <button class="ghost dim" onclick={clearSelection}>Clear</button>
      </div>
      <div class="sel-row">
        <input
          class="sel-input"
          bind:this={groupInputEl}
          bind:value={groupPrompt}
          placeholder="Instruction for the {selected.size} notes… (Enter to send)"
          onkeydown={(e) => {
            if (e.key === 'Enter') groupSend();
          }}
        />
        <button
          class="ghost"
          title="Send ALL {selected.size} selected notes to an agent with this instruction"
          disabled={!groupPrompt.trim()}
          onclick={groupSend}
        >
          Send
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .tree {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  .tree :global(.tree-list) {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .empty {
    padding: 16px 12px;
    color: var(--text-dim);
    font-size: 12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding-inline-end: 8px;
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    border-radius: 5px;
    user-select: none;
    position: relative;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .row.active {
    background: var(--accent-dim, rgba(90, 120, 255, 0.18));
  }
  .row.reserved .name {
    opacity: 0.65;
    font-style: italic;
  }
  .row.drag-over {
    outline: 1px dashed var(--accent, #7a9cff);
    outline-offset: -1px;
  }
  .chev {
    display: inline-flex;
    transition: transform 0.12s;
    width: 12px;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .pad {
    width: 12px;
  }
  /* Selection checkbox lives in the .pad slot: invisible until hover or an
     active selection, so rows never shift. */
  .sel {
    width: 12px;
    height: 12px;
    margin: 0;
    flex-shrink: 0;
    visibility: hidden;
    accent-color: var(--accent, #7a9cff);
    cursor: pointer;
  }
  .row:hover .sel,
  .sel.vis,
  .sel:checked {
    visibility: visible;
  }
  .row.checked {
    background: var(--accent-dim, rgba(90, 120, 255, 0.12));
  }
  .sel-bar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 8px;
    border-top: 1px solid var(--border);
    background: var(--panel, #1c1c1e);
  }
  .sel-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .sel-count {
    font-size: 11px;
    color: var(--text-dim);
    margin-inline-end: auto;
  }
  .sel-input {
    flex: 1;
    min-width: 0;
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    font-size: 11.5px;
    padding: 4px 8px;
  }
  .sel-bar .ghost:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .sel-bar .ghost {
    background: none;
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 6px;
    padding: 3px 9px;
    font-size: 11.5px;
    cursor: pointer;
  }
  .sel-bar .ghost:hover {
    border-color: var(--accent, #7a9cff);
    color: var(--accent, #9ab4ff);
  }
  .sel-bar .ghost.dim {
    color: var(--text-dim);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }
  .count {
    font-size: 10px;
    color: var(--text-dim);
  }
  .prov {
    font-size: 10px;
    color: var(--accent, #9ab4ff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.12));
    border-radius: 999px;
    padding: 0 7px;
    white-space: nowrap;
    max-width: 110px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rename {
    position: absolute;
    inset-inline-start: 40px;
    inset-inline-end: 8px;
    background: var(--panel-2, #222);
    border: 1px solid var(--accent, #7a9cff);
    border-radius: 4px;
    color: var(--text);
    font-size: 12px;
    padding: 2px 6px;
    z-index: 2;
  }
</style>
