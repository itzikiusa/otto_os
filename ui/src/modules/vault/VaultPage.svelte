<script lang="ts">
  // Vault v3 — the docs home. Obsidian-style three-pane layout: left sidebar
  // (Files / Search / Tags over the active vault), center (note edit⇄read or
  // the graph), right panel (backlinks / outgoing / outline / properties /
  // OKF). Files on disk are the truth; the daemon keeps a derived index.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import DocsAgentsView from './DocsAgentsView.svelte';
  import FileTree from './FileTree.svelte';
  import FileViewer from './FileViewer.svelte';
  import GraphView from './GraphView.svelte';
  import NewNoteDialog from './NewNoteDialog.svelte';
  import NoteView from './NoteView.svelte';
  import RightPanel from './RightPanel.svelte';
  import SearchPanel from './SearchPanel.svelte';
  import Switcher from './Switcher.svelte';
  import TagsPanel from './TagsPanel.svelte';
  import { vault } from './vault.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';

  // -- pane widths (drag-resizable, persisted) ---------------------------------
  const LEFT_W_KEY = 'otto_vault_left_w';
  const RIGHT_W_KEY = 'otto_vault_right_w';
  let leftW = $state(Number(localStorage.getItem(LEFT_W_KEY)) || 250);
  let rightW = $state(Number(localStorage.getItem(RIGHT_W_KEY)) || 280);
  let rightOpen = $state(localStorage.getItem('otto_vault_right_open') !== '0');
  let leftOpen = $state(localStorage.getItem('otto_vault_left_open') !== '0');
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
        localStorage.setItem(LEFT_W_KEY, String(w));
      } else {
        rightW = w;
        localStorage.setItem(RIGHT_W_KEY, String(w));
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
              action: () => {
                if (confirm(`Unregister "${vault.current?.name}"? Files on disk are untouched.`)) {
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

  const scanning = $derived(vault.status?.scan_state === 'scanning');
  const scanError = $derived(vault.status?.scan_state.startsWith('error') ?? false);

  onMount(() => {
    void vault.load();
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
  <header class="topbar">
    <button
      class="tool"
      title={leftOpen ? 'Hide file tree' : 'Show file tree'}
      onclick={() => {
        leftOpen = !leftOpen;
        localStorage.setItem('otto_vault_left_open', leftOpen ? '1' : '0');
      }}
    >
      <Icon name="panel" size={14} />
    </button>
    <button class="vault-pick" onclick={(e) => vaultMenu(e)} title="Switch vault">
      <Icon name="globe" size={14} />
      <span>{vault.current?.name ?? 'No vault'}</span>
      <span class="tri">▾</span>
    </button>
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
    <div class="spacer"></div>
    {#if vault.current}
      <div class="counts">
        {vault.status?.notes ?? vault.current.notes} notes · {vault.status?.links ??
          vault.current.links} links
        {#if (vault.status?.unresolved ?? 0) > 0}
          · {vault.status?.unresolved} unresolved
        {/if}
      </div>
      <button
        class="tool"
        class:active={vault.centerMode === 'graph'}
        title="Graph view"
        onclick={() => {
          vault.centerMode = vault.centerMode === 'graph' ? (vault.note ? 'note' : 'empty') : 'graph';
          vault.persistView();
        }}
      >
        <Icon name="share" size={14} />
      </button>
      <button
        class="tool"
        class:active={vault.centerMode === 'docs-agents'}
        title="Docs agent — have agents write documentation into this vault"
        onclick={() => {
          if (vault.centerMode === 'docs-agents') {
            vault.centerMode = vault.note ? 'note' : 'empty';
            vault.persistView();
          } else {
            vault.openDocsAgents('');
          }
        }}
      >
        <Icon name="zap" size={14} />
      </button>
      <button class="tool" title="Quick switcher (⌘O)" onclick={() => (vault.switcherOpen = true)}>
        <Icon name="search" size={14} />
      </button>
      <button class="tool" title="New note (⌘N)" onclick={() => openNewNote('')}>
        <Icon name="plus" size={14} />
      </button>
      <button
        class="tool"
        title="Toggle right panel"
        onclick={() => {
          rightOpen = !rightOpen;
          localStorage.setItem('otto_vault_right_open', rightOpen ? '1' : '0');
        }}
      >
        <Icon name="sidebar" size={14} />
      </button>
    {/if}
  </header>

  {#if !vault.current && !vault.loading}
    <div class="onboard">
      <h2>The docs home</h2>
      <p>
        A vault is a folder of markdown files on disk — point Otto at an existing Obsidian vault or
        create a fresh one. Files stay yours; Otto indexes links, tags and full text, and agents
        read/write it over MCP in OKF.
      </p>
      <button class="primary" onclick={() => (createOpen = true)}>Add a vault</button>
    </div>
  {:else if vault.current}
    <div class="panes">
      {#if leftOpen}
      <aside class="left" style="width:{leftW}px">
        <div class="left-modes">
          <button
            class:active={vault.leftMode === 'files'}
            title="Files"
            onclick={() => (vault.leftMode = 'files')}><Icon name="folder" size={14} /></button
          >
          <button
            class:active={vault.leftMode === 'search'}
            title="Search"
            onclick={() => (vault.leftMode = 'search')}><Icon name="search" size={14} /></button
          >
          <button
            class:active={vault.leftMode === 'tags'}
            title="Tags"
            onclick={() => {
              vault.leftMode = 'tags';
              void vault.loadTags();
            }}><Icon name="tag" size={14} /></button
          >
          <div class="spacer"></div>
          {#if vault.leftMode === 'files'}
            <button title="Collapse all" onclick={() => vault.collapseAll()}>
              <Icon name="minimize" size={13} />
            </button>
            <button title="Rescan vault" onclick={() => void vault.rescan()}>
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
          <GraphView />
        {:else if vault.centerMode === 'docs-agents'}
          <DocsAgentsView />
        {:else if vault.centerMode === 'file' && vault.filePath}
          <FileViewer />
        {:else if vault.centerMode === 'note' && vault.note}
          <NoteView />
        {:else}
          <div class="center-empty">
            <p>Open a note from the tree, search, or press ⌘O.</p>
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
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => (createOpen = false)}>
    <div class="dialog" role="dialog" tabindex="-1" aria-label="Add vault" onclick={(e) => e.stopPropagation()}>
      <h3>Add a vault</h3>
      <label class="fld">
        <span>Name</span>
        <input bind:value={cName} placeholder="Team Docs" />
      </label>
      <label class="fld">
        <span>Folder (blank → create under ~/.otto/vault; a new path is created)</span>
        <div class="pathrow">
          <input bind:value={cPath} placeholder="~/Documents/Obsidian/MyVault" />
          <button class="browse" type="button" onclick={() => (browsing = true)}>Browse…</button>
        </div>
      </label>
      <label class="chk">
        <input type="checkbox" bind:checked={cOkf} />
        OKF vault (Open Knowledge Format validation + templates)
      </label>
      {#if createError}
        <div class="err" role="alert">{createError}</div>
      {/if}
      <div class="actions">
        <button onclick={() => (createOpen = false)}>Cancel</button>
        <button class="primary" disabled={!cName.trim() || creating} onclick={() => void submitCreate()}>
          {creating ? 'Adding…' : 'Add vault'}
        </button>
      </div>
    </div>
  </div>
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
  .vault-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .vault-page.resizing {
    cursor: col-resize;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .vault-pick {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 8px;
    padding: 5px 10px;
    font-size: 12.5px;
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
    font-size: 10px;
  }
  .okf-chip {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.4px;
    color: var(--accent, #9ab4ff);
    border: 1px solid var(--accent, #9ab4ff);
    border-radius: 5px;
    padding: 1px 6px;
  }
  .scan-chip {
    font-size: 11px;
    color: var(--accent, #9ab4ff);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .scan-chip.err {
    color: #e88;
    animation: none;
  }
  .run-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--accent, #9ab4ff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
    border: 1px solid var(--accent, #7a9cff);
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
  .spacer,
  .grow {
    flex: 1;
  }
  .counts {
    font-size: 11.5px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .tool {
    display: inline-flex;
    background: none;
    border: 1px solid transparent;
    border-radius: 7px;
    color: var(--text);
    padding: 5px 7px;
    cursor: pointer;
  }
  .tool:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .tool.active {
    border-color: var(--accent, #7a9cff);
    color: var(--accent, #9ab4ff);
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
  .left-modes button {
    display: inline-flex;
    background: none;
    border: none;
    border-radius: 6px;
    color: var(--text-dim);
    padding: 5px 7px;
    cursor: pointer;
  }
  .left-modes button:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .left-modes button.active {
    color: var(--accent, #9ab4ff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
  }
  .resizer {
    width: 4px;
    cursor: col-resize;
    flex-shrink: 0;
  }
  .resizer:hover {
    background: var(--accent-dim, rgba(90, 120, 255, 0.3));
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
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .vtab.active {
    color: var(--text);
    background: var(--panel-2, #222);
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
    background: var(--hover, rgba(127, 127, 127, 0.2));
    color: var(--text);
  }
  .center-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    font-size: 13px;
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
    color: #7fc97f;
  }
  .vault-statusbar .bad {
    color: #e88;
  }
  .vault-statusbar .dim {
    opacity: 0.7;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .onboard {
    max-width: 480px;
    margin: 12vh auto;
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 0 20px;
  }
  .onboard h2 {
    margin: 0;
  }
  .onboard p {
    color: var(--text-dim);
    font-size: 13.5px;
    line-height: 1.55;
  }
  .onboard .primary,
  .actions .primary {
    background: var(--accent, #4c6fff);
    border: none;
    color: #fff;
    border-radius: 8px;
    padding: 8px 18px;
    font-size: 13px;
    cursor: pointer;
    align-self: center;
  }
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 90;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 16vh;
  }
  .dialog {
    width: min(460px, 92vw);
    background: var(--panel, #1c1c1e);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .dialog h3 {
    margin: 0;
    font-size: 14px;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .fld input {
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 13px;
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
  .browse {
    border: 1px solid var(--border);
    background: var(--panel-2, #222);
    color: var(--text);
    border-radius: 7px;
    padding: 0 12px;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .err {
    color: #e88;
    font-size: 12px;
    border: 1px solid rgba(214, 86, 72, 0.4);
    background: rgba(214, 86, 72, 0.08);
    border-radius: 7px;
    padding: 6px 10px;
    word-break: break-word;
  }
  .chk {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    border: 1px solid var(--border);
    background: var(--panel-2, #222);
    color: var(--text);
    border-radius: 7px;
    padding: 6px 14px;
    cursor: pointer;
    font-size: 12.5px;
  }
  .actions .primary:disabled {
    opacity: 0.5;
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
