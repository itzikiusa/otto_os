<script lang="ts">
  // Vault v3 — the docs home. Obsidian-style three-pane layout: left sidebar
  // (Files / Search / Tags over the active vault), center (note edit⇄read or
  // the graph), right panel (backlinks / outgoing / outline / properties /
  // OKF). Files on disk are the truth; the daemon keeps a derived index.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { lsGet, lsSet } from '../../lib/storage';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import DocsAgentsView from './DocsAgentsView.svelte';
  import FileTree from './FileTree.svelte';
  import FileViewer from './FileViewer.svelte';
  import GraphView from './GraphView.svelte';
  import RecoveryView from './RecoveryView.svelte';
  import NewNoteDialog from './NewNoteDialog.svelte';
  import NoteView from './NoteView.svelte';
  import RightPanel from './RightPanel.svelte';
  import SearchPanel from './SearchPanel.svelte';
  import Switcher from './Switcher.svelte';
  import TagsPanel from './TagsPanel.svelte';
  import { vault } from './vault.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  // -- pane widths (drag-resizable, persisted) ---------------------------------
  const LEFT_W_KEY = 'otto_vault_left_w';
  const RIGHT_W_KEY = 'otto_vault_right_w';
  // Storage is a convenience cache: lsGet/lsSet swallow a blocked accessor or
  // a full quota, so a private window never blanks the page.
  let leftW = $state(Number(lsGet(LEFT_W_KEY)) || 250);
  let rightW = $state(Number(lsGet(RIGHT_W_KEY)) || 280);
  let rightOpen = $state(lsGet('otto_vault_right_open') !== '0');
  let leftOpen = $state(lsGet('otto_vault_left_open') !== '0');

  function toggleLeft(): void {
    leftOpen = !leftOpen;
    lsSet('otto_vault_left_open', leftOpen ? '1' : '0');
  }
  function toggleRight(): void {
    rightOpen = !rightOpen;
    lsSet('otto_vault_right_open', rightOpen ? '1' : '0');
  }
  let resizing = $state(false);

  function startResize(e: MouseEvent, side: 'left' | 'right'): void {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = side === 'left' ? leftW : rightW;
    const onMove = (ev: MouseEvent) => {
      const d = ev.clientX - startX;
      const w = Math.max(180, Math.min(520, Math.round(side === 'left' ? startW + d : startW - d)));
      if (side === 'left') {
        leftW = w;
        lsSet(LEFT_W_KEY, String(w));
      } else {
        rightW = w;
        lsSet(RIGHT_W_KEY, String(w));
      }
    };
    const onUp = () => {
      resizing = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }

  // -- create-vault dialog -------------------------------------------------------
  let createOpen = $state(false);
  let cName = $state('');
  let cPath = $state('');
  let cOkf = $state(true);
  let creating = $state(false);
  let createError = $state('');
  // Folder selection uses the shared daemon-side FolderPicker (/fs/browse).
  let browsing = $state(false);

  async function submitCreate(): Promise<void> {
    if (!cName.trim() || creating) return;
    creating = true;
    createError = '';
    try {
      await vault.create(cName.trim(), cPath.trim() || undefined, cOkf);
      createOpen = false;
      cName = '';
      cPath = '';
    } catch (e) {
      // Surface the daemon's reason inline — a silently-failing dialog is the
      // worst kind of "did nothing".
      createError = e instanceof Error ? e.message : String(e);
    } finally {
      creating = false;
    }
  }

  // -- new-note dialog -------------------------------------------------------------
  let newNoteOpen = $state(false);
  let newNoteDir = $state('');

  function openNewNote(dir: string): void {
    newNoteDir = dir;
    newNoteOpen = true;
  }

  function vaultMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      ...vault.vaults.map((v) => ({
        label: v.name + (v.id === vault.current?.id ? '  ✓' : ''),
        icon: 'globe',
        action: () => void vault.select(v.id),
      })),
      { separator: true },
      { label: 'Add vault…', icon: 'plus', action: () => (createOpen = true) },
      ...(vault.current
        ? [
            {
              label: vault.current.okf ? 'Disable OKF mode' : 'Enable OKF mode',
              icon: 'check',
              action: () => void vault.toggleOkf(),
            },
            { label: 'Rescan', icon: 'refresh', action: () => void vault.rescan() },
            {
              label: 'Unregister vault (keeps files)',
              icon: 'trash',
              danger: true,
              action: async () => {
                if (await confirmer.ask(`Unregister "${vault.current?.name}"? Files on disk are untouched.`, { title: 'Unregister vault', confirmLabel: 'Unregister' }) && vault.current) {
                  void vault.unregister(vault.current!.id);
                }
              },
            },
          ]
        : []),
    ]);
  }

  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'o') {
      e.preventDefault();
      vault.switcherOpen = true;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'n' && vault.current) {
      e.preventDefault();
      openNewNote('');
    }
  }

  /** Header ⋯: the less-used views (history, trash) and the pane toggles —
   *  the toolbar keeps ≤5 controls (graph, docs agent, switcher, new note, ⋯). */
  function moreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Edit history', icon: 'clock', action: () => void vault.openHistory() },
      { label: 'Trash and restore', icon: 'trash', action: () => void vault.openTrash() },
      { separator: true },
      { label: 'File tree', checked: leftOpen, action: toggleLeft },
      { label: 'Right panel', checked: rightOpen, action: toggleRight },
    ];
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }

  function toggleGraph(): void {
    vault.centerMode = vault.centerMode === 'graph' ? (vault.note ? 'note' : 'empty') : 'graph';
    vault.persistView();
  }
  function toggleDocsAgents(): void {
    if (vault.centerMode === 'docs-agents') {
      vault.centerMode = vault.note ? 'note' : 'empty';
      vault.persistView();
    } else {
      vault.openDocsAgents('');
    }
  }

  // -- landing note ----------------------------------------------------------------
  // A vault never opens onto a bare "open a note" pane: on a first visit (no
  // saved tabs/view for this vault) its root index/README note opens by itself.
  // Once per vault per mount, so closing it lands on the empty state instead.
  const INDEX_NAMES = ['index.md', 'readme.md', '_index.md'];
  const indexNote = $derived.by(() => {
    for (const name of INDEX_NAMES) {
      const hit = vault.roots.find((r) => r.entry.kind === 'note' && r.entry.name.toLowerCase() === name);
      if (hit) return hit.entry.path;
    }
    return null;
  });
  let autoIndexFor: number | null = null;
  $effect(() => {
    const id = vault.current?.id;
    const path = indexNote;
    if (id == null || !path || autoIndexFor === id) return;
    if (vault.centerMode !== 'empty' || vault.tabs.length > 0) return;
    // A saved view (tabs, or graph/docs-agents/trash) is restored by the
    // store — don't race it.
    let saved: { tabs?: unknown; mode?: unknown } = {};
    try {
      saved = JSON.parse(lsGet(`otto_vault_tabs:${id}`) ?? '{}') ?? {};
    } catch {
      /* corrupt blob — treat as a first visit */
    }
    const hasTabs = Array.isArray(saved.tabs) && saved.tabs.length > 0;
    const mode = typeof saved.mode === 'string' ? saved.mode : 'empty';
    if (hasTabs || (mode !== 'empty' && mode !== 'note')) return;
    autoIndexFor = id;
    void vault.open(path);
  });

  const scanning = $derived(vault.status?.scan_state === 'scanning');
  const scanError = $derived(vault.status?.scan_state.startsWith('error') ?? false);

  // The first load comes from the workspace effect below (it runs on mount
  // too) — loading here as well fetched every vault request twice.
  onMount(() => {
    vault.startPolling();
    return () => vault.stopPolling();
  });

  // Reload when the workspace changes.
  let lastWs = $state('');
  $effect(() => {
    const id = ws.current?.id ?? '';
    if (id && id !== lastWs) {
      lastWs = id;
      void vault.load();
    }
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="vault-page" class:resizing>
  <!-- Unified header: vault switcher as the title, status chips as badges,
       tools right-aligned (they collapse into ⋯ when the pane is narrow). -->
  <PageHeader title={vault.current?.name ?? 'Vault'} class="vault-header">
    {#snippet titleContent()}
      <button class="vault-pick" onclick={(e) => vaultMenu(e)} title="Switch vault">
        <Icon name="globe" size={14} />
        <span>{vault.current?.name ?? 'No vault'}</span>
        <span class="tri">▾</span>
      </button>
    {/snippet}
    {#snippet badge()}
      {#if vault.current?.okf}
        <span class="okf-chip" title="OKF (Open Knowledge Format) vault">OKF</span>
      {/if}
      {#if scanning}
        <span class="scan-chip">Indexing vault…</span>
      {:else if scanError}
        <span class="scan-chip err" title={vault.status?.scan_state}>index error</span>
      {/if}
      {#if vault.activeDocsRuns.length > 0}
        <!-- Always-visible signal that agents are writing into this vault right
             now (runs may be launched from here, MCP, or a workflow) — clicking
             jumps to the Docs agent view where each run can be watched. -->
        <button
          class="run-chip"
          title={vault.activeDocsRuns
            .map((r) => (r.kind === 'refine' ? `refine: ${r.note_path}` : r.prompt))
            .join('\n')}
          onclick={() => vault.openDocsAgents('')}
        >
          <Icon name="zap" size={12} />
          {vault.activeDocsRuns.length === 1
            ? '1 agent run active'
            : `${vault.activeDocsRuns.length} agent runs active`}
        </button>
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if vault.current}
        <button
          class="icon-btn vh-tool"
          class:active={vault.centerMode === 'graph'}
          aria-pressed={vault.centerMode === 'graph'}
          title="Graph view"
          aria-label="Graph view"
          data-icon="share"
          onclick={toggleGraph}
        >
          <Icon name="share" size={14} />
        </button>
        <button
          class="icon-btn vh-tool"
          class:active={vault.centerMode === 'docs-agents'}
          aria-pressed={vault.centerMode === 'docs-agents'}
          title="Docs agent — have agents write documentation into this vault"
          aria-label="Docs agent"
          data-label="Docs agent"
          data-icon="zap"
          onclick={toggleDocsAgents}
        >
          <Icon name="zap" size={14} />
        </button>
        <button class="icon-btn vh-tool" title="Quick switcher (⌘O)" aria-label="Quick switcher" data-icon="search" onclick={() => (vault.switcherOpen = true)}>
          <Icon name="search" size={14} />
        </button>
        <button class="icon-btn vh-tool" title="New note (⌘N)" aria-label="New note" data-icon="plus" data-overflow="1" onclick={() => openNewNote('')}>
          <Icon name="plus" size={14} />
        </button>
        <button
          class="icon-btn vh-tool"
          title="More vault actions"
          aria-label="More vault actions"
          aria-haspopup="menu"
          data-keep
          onclick={moreMenu}
        >
          <Icon name="more" size={14} />
        </button>
      {/if}
    {/snippet}
  </PageHeader>

  {#if !vault.current && !vault.loading}
    <EmptyState
      variant="page"
      icon="note"
      title="The docs home"
      body="A vault is a folder of markdown files on disk — point Otto at an existing Obsidian vault or create a fresh one. Files stay yours; Otto indexes links, tags and full text, and agents read/write it over MCP in OKF."
      actionLabel="Add a vault"
      actionIcon="plus"
      onaction={() => (createOpen = true)}
    />
  {:else if vault.current}
    <div class="panes">
      {#if leftOpen}
      <aside class="left" style="width:{leftW}px">
        <div class="left-modes" role="group" aria-label="Sidebar view">
          <button
            class="icon-btn vh-tool"
            class:active={vault.leftMode === 'files'}
            aria-pressed={vault.leftMode === 'files'}
            title="Files"
            aria-label="Files"
            onclick={() => (vault.leftMode = 'files')}><Icon name="folder" size={14} /></button
          >
          <button
            class="icon-btn vh-tool"
            class:active={vault.leftMode === 'search'}
            aria-pressed={vault.leftMode === 'search'}
            title="Search"
            aria-label="Search"
            onclick={() => (vault.leftMode = 'search')}><Icon name="search" size={14} /></button
          >
          <button
            class="icon-btn vh-tool"
            class:active={vault.leftMode === 'tags'}
            aria-pressed={vault.leftMode === 'tags'}
            title="Tags"
            aria-label="Tags"
            onclick={() => {
              vault.leftMode = 'tags';
              void vault.loadTags();
            }}><Icon name="tag" size={14} /></button
          >
          <div class="spacer"></div>
          {#if vault.leftMode === 'files'}
            <button class="icon-btn" title="Collapse all" aria-label="Collapse all" onclick={() => vault.collapseAll()}>
              <Icon name="minimize" size={13} />
            </button>
            <button class="icon-btn" title="Rescan vault" aria-label="Rescan vault" onclick={() => void vault.rescan()}>
              <Icon name="refresh" size={13} />
            </button>
          {/if}
        </div>
        {#if vault.leftMode === 'files'}
          <FileTree onNewNote={openNewNote} />
        {:else if vault.leftMode === 'search'}
          <SearchPanel />
        {:else}
          <TagsPanel />
        {/if}
      </aside>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="resizer" onmousedown={(e) => startResize(e, 'left')}></div>
      {/if}

      <main class="center">
        {#if vault.tabs.length > 0}
          <div class="tabstrip" role="tablist">
            {#each vault.tabs as t, i (t.kind + ':' + t.path)}
              <div
                class="vtab"
                class:active={i === vault.activeTab &&
                  (vault.centerMode === 'note' || vault.centerMode === 'file')}
                role="tab"
                tabindex="0"
                aria-selected={i === vault.activeTab}
                title={t.path}
                onclick={() => void vault.activateTab(i)}
                onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && void vault.activateTab(i)}
                onauxclick={(e) => {
                  if (e.button === 1) void vault.closeTab(i);
                }}
              >
                <Icon name={t.kind === 'note' ? 'note' : 'file'} size={12} />
                <span class="vtab-name">
                  {t.kind === 'note'
                    ? (t.path.split('/').pop() ?? t.path).replace(/\.md$/i, '')
                    : (t.path.split('/').pop() ?? t.path)}
                </span>
                <button
                  class="vtab-close"
                  title="Close tab"
                  aria-label="Close tab"
                  onclick={(e) => {
                    e.stopPropagation();
                    void vault.closeTab(i);
                  }}>×</button
                >
              </div>
            {/each}
          </div>
        {/if}
        {#if vault.centerMode === 'graph'}
          <div class="graph-scope">
            <label>Graph scope <select aria-label="Graph scope" bind:value={vault.graphLocal} onchange={() => vault.persistView()}>
              <option value={false}>Whole vault</option><option value={true} disabled={!vault.notePath}>Around current note</option>
            </select></label>
            {#if vault.graphLocal}<span>{vault.notePath}</span>{/if}
          </div>
          <GraphView local={vault.graphLocal} />
        {:else if vault.centerMode === 'trash' || vault.centerMode === 'history'}
          {#key vault.centerMode}<RecoveryView mode={vault.centerMode} />{/key}
        {:else if vault.centerMode === 'docs-agents'}
          <DocsAgentsView />
        {:else if vault.centerMode === 'file' && vault.filePath}
          <FileViewer />
        {:else if vault.centerMode === 'note' && vault.note}
          <NoteView />
        {:else}
          <div class="center-empty">
            <EmptyState
              icon="note"
              title="No note open"
              body="Open a note from the file tree or search, press ⌘O to jump to one, or start a new note."
              actionLabel="New note"
              actionIcon="plus"
              onaction={() => openNewNote('')}
            >
              {#if indexNote}
                <button class="btn ghost" onclick={() => void vault.open(indexNote!)}>Open {indexNote}</button>
              {:else}
                <button class="btn ghost" onclick={() => (vault.switcherOpen = true)}>Quick switcher (⌘O)</button>
              {/if}
            </EmptyState>
          </div>
        {/if}
      </main>

      {#if rightOpen && vault.centerMode === 'note'}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="resizer resizer-right" onmousedown={(e) => startResize(e, 'right')}></div>
        <aside class="right-pane" style="width:{rightW}px">
          <RightPanel />
        </aside>
      {/if}
    </div>

    <footer class="vault-statusbar">
      <span class="vs-counts">
        {vault.status?.notes ?? vault.current.notes} notes · {vault.status?.links ?? vault.current.links} links{#if (vault.status?.unresolved ?? 0) > 0}
          · {vault.status?.unresolved} unresolved{/if}
      </span>
      {#if vault.note}
        <span>{vault.backlinks.length} backlinks</span>
        <span>{vault.note.meta.word_count} words</span>
        <span>{(vault.editing ? vault.draft : vault.note.raw).length} characters</span>
        {#if vault.current.okf && vault.okfReport}
          <span class:ok={vault.okfReport.conformant} class:bad={!vault.okfReport.conformant}>
            OKF {vault.okfReport.conformant ? '✓' : `✗ ${vault.okfReport.errors.length}`}
          </span>
        {/if}
      {/if}
      <span class="grow"></span>
      <span class="dim">{vault.current.root_path}</span>
    </footer>
  {/if}
</div>

{#if createOpen}
  <Modal title="Add a vault" onclose={() => (createOpen = false)}>
    <div class="av-body">
      <label class="av-fld">
        <span>Name</span>
        <input bind:value={cName} placeholder="Team Docs" />
      </label>
      <label class="av-fld">
        <span>Folder (blank → create under ~/.otto/vault; a new path is created)</span>
        <div class="pathrow">
          <input bind:value={cPath} placeholder="~/Documents/Obsidian/MyVault" />
          <button class="btn" type="button" onclick={() => (browsing = true)}>Browse…</button>
        </div>
      </label>
      <label class="av-chk">
        <input type="checkbox" bind:checked={cOkf} />
        OKF vault (Open Knowledge Format validation + templates)
      </label>
      {#if createError}
        <div class="av-err" role="alert">{createError}</div>
      {/if}
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (createOpen = false)}>Cancel</button>
      <button class="btn primary" disabled={!cName.trim() || creating} onclick={() => void submitCreate()}>
        {creating ? 'Adding…' : 'Add vault'}
      </button>
    {/snippet}
  </Modal>
{/if}

{#if browsing}
  <FolderPicker
    title="Choose vault folder"
    start={cPath || '~'}
    onpick={(p: string) => {
      cPath = p;
      browsing = false;
    }}
    onclose={() => (browsing = false)}
  />
{/if}

<NewNoteDialog bind:open={newNoteOpen} bind:dir={newNoteDir} />
<Switcher />

<style>
  .graph-scope { display: flex; align-items: center; gap: 12px; padding: 8px 12px; flex-wrap: wrap; font-size: 12px; color: var(--text-dim); }
  .graph-scope select { background: var(--bg); color: var(--text); border: 1px solid var(--border); padding: 4px; border-radius: 4px; }
  .vault-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .vault-page.resizing {
    cursor: col-resize;
  }
  .vault-pick {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 8px;
    padding: 5px 10px;
    font-size: var(--fs-s);
    cursor: pointer;
    max-width: 260px;
  }
  .vault-pick span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tri {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .okf-chip {
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.4px;
    color: var(--accent-text);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    border-radius: 5px;
    padding: 1px 6px;
  }
  .scan-chip {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .scan-chip.err {
    color: var(--danger);
    animation: none;
  }
  .run-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: var(--accent-soft);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    border-radius: 999px;
    padding: 2px 9px;
    cursor: pointer;
    white-space: nowrap;
    animation: pulse 1.2s ease-in-out infinite;
  }
  .run-chip:hover {
    animation: none;
  }
  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .scan-chip,
    .run-chip {
      animation: none;
    }
  }
  .spacer,
  .grow {
    flex: 1;
  }
  /* Header + sidebar-mode toggles are global .icon-btn; "on" is the quiet
     selection tint, never the accent as text colour. */
  .vh-tool.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .panes {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .left {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-inline-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .left-modes {
    display: flex;
    gap: 2px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .resizer {
    width: 4px;
    cursor: col-resize;
    flex-shrink: 0;
  }
  .resizer:hover {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .center {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .center > :global(*) {
    flex: 1;
    min-height: 0;
  }
  /* The graph-scope bar is chrome too: under the `flex: 1` rule above it took
     HALF the pane, pushing the graph (and its controls panel) down by ~200px
     and off the bottom of the window. */
  .center > .graph-scope {
    flex: 0 0 auto;
  }
  /* The tab strip is chrome, not content — never let it stretch. */
  .center > .tabstrip {
    flex: 0 0 auto;
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 4px 8px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: thin;
  }
  .vtab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px 4px 10px;
    font-size: 12px;
    color: var(--text-dim);
    background: none;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 7px 7px 0 0;
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
    max-width: 220px;
  }
  .vtab:hover {
    background: var(--hover);
  }
  .vtab.active {
    color: var(--text);
    background: var(--surface-2);
    border-color: var(--border);
  }
  .vtab-name {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .vtab-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1;
    cursor: pointer;
    padding: 0;
  }
  .vtab-close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .center-empty {
    overflow-y: auto;
  }
  .right-pane {
    border-inline-start: 1px solid var(--border);
    flex-shrink: 0;
    min-height: 0;
  }
  .vault-statusbar {
    display: flex;
    gap: 14px;
    align-items: center;
    padding: 4px 14px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-dim);
  }
  .vault-statusbar .ok {
    color: var(--success);
  }
  .vault-statusbar .bad {
    color: var(--danger);
  }
  .vs-counts {
    white-space: nowrap;
  }
  .vault-statusbar .dim {
    opacity: 0.7;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .av-fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .av-fld input {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-m);
    padding: 8px 10px;
  }
  .pathrow {
    display: flex;
    gap: 6px;
  }
  .pathrow input {
    flex: 1;
    min-width: 0;
  }
  .av-err {
    color: var(--danger);
    font-size: var(--fs-s);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    background: var(--danger-soft);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    word-break: break-word;
  }
  .av-chk {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: var(--fs-s);
    color: var(--text);
  }

  /* Mid widths (narrow desktop / tablet portrait / phone landscape): the
   * fixed side panes would crush the note pane — drop the right panel. */
  @media (max-width: 1100px) {
    .right-pane,
    .resizer-right {
      display: none;
    }
  }

  /* Mobile: stack — left pane becomes a top strip, right panel hidden. */
  @media (max-width: 800px) {
    .panes {
      flex-direction: column;
    }
    .left {
      width: 100% !important;
      max-height: 40%;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    .resizer {
      display: none;
    }
    .right-pane {
      display: none;
    }
  }
</style>
