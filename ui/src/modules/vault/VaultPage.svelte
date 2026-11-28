<script lang="ts">
  import { loadErrorText } from '../../lib/loadError';
  import { announce, announceShown } from '../../lib/announce.svelte';
  // Vault v3 — the docs home: a list pane with Files | Search | Tags views
  // (ListDetail: push on a phone), the note or graph in the content, and Note
  // details as the page's Detail pane. Files on disk are the truth; the
  // daemon keeps a derived index.
  import { onMount, untrack } from 'svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { lsGet } from '../../lib/storage';
  import PageBody from '../../lib/components/PageBody.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
  import { DetailOpen } from '../../lib/components/side/detailOpen.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import FileTree from './FileTree.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { DocsAgentsLazy, FileViewerLazy, GraphLazy, RecoveryLazy } from './lazyViews';
  import NewNoteDialog from './NewNoteDialog.svelte';
  import NoteView from './NoteView.svelte';
  import RightPanel from './RightPanel.svelte';
  import SearchPanel from './SearchPanel.svelte';
  import SearchIndexChip, { rebuildSearchIndex } from './SearchIndexChip.svelte';
  import Switcher from './Switcher.svelte';
  import TagsPanel from './TagsPanel.svelte';
  import { vault } from './vault.svelte';
  import { shouldIgnoreVaultShortcut } from './vaultShortcuts';
  import { consumeVaultDeepLink } from './ottoRefs';
  import { registerVaultPort } from '../../lib/uiCommands/vaultPort';
  import PathField from '../../lib/components/PathField.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  // -- Note details: the page's Detail pane ---------------------------------------
  // Beside the note (inline) it follows the person's last choice, kept under
  // otto_vault_right_open (default on). Where it would be a sheet over the
  // note (a phone, a narrow window) it opens only when asked, and that choice
  // isn't remembered — a sheet never covers a note by itself.
  const details = new DetailOpen('otto_vault_right_open');
  const detailsShown = $derived(vault.centerMode === 'note' && !!vault.note && details.open);
  // The pane's two views: the note's details, or "Refine with Otto" (a live
  // agent on this note; NoteView's sparkle button and the tree's Review + fix
  // open it). Another note starts on Details again.
  let detailView = $state<'details' | 'refine'>('details');
  const DETAIL_VIEWS = [
    { id: 'details', label: 'Details' },
    { id: 'refine', label: 'Refine with Otto' },
  ] as const;
  const RefineLazy = lazyComponent(() => import('./RefineDrawer.svelte'));
  function toggleRefine(): void {
    if (detailsShown && detailView === 'refine') return setDetails(false);
    detailView = 'refine';
    setDetails(true);
  }
  $effect(() => {
    void vault.notePath;
    untrack(() => (detailView = 'details'));
  });
  // "Review + fix" queues a pending refine: open the Refine view on that
  // note; RefineDrawer consumes it (the person presses Send).
  $effect(() => {
    const pending = vault.pendingRefine;
    if (pending && vault.notePath === pending.path) untrack(() => {
      detailView = 'refine';
      setDetails(true);
    });
  });
  const setDetails = (show: boolean): void => details.set(show);
  // a11y-6-01: a backlinks refresh failure is announced at its edge (the status-bar glyph is no live region).
  const backlinksError = $derived(vault.note ? (vault.backlinksError ?? '') : '');
  $effect(() => { if (backlinksError) announce(`Couldn’t refresh the backlinks. ${backlinksError}`); });
  $effect(() => registerVaultPort(vault)); // otto.ui_vault_* drive the store while the page is up (lib/uiCommands/vaultPort.ts)

  let createOpen = $state(false);
  let cName = $state('');
  let cPath = $state('');
  let cOkf = $state(true);
  let creating = $state(false);
  let createError = $state('');

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
      createError = loadErrorText(e);
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
        label: v.name,
        checked: v.id === vault.current?.id,
        action: () => void vault.select(v.id),
      })),
      { separator: true },
      { label: 'Add vault…', icon: 'plus', action: () => (createOpen = true) },
      ...(vault.current
        ? [
            {
              label: 'OKF mode (validation + templates)',
              checked: vault.current.okf,
              action: () => void vault.toggleOkf(),
            },
            { label: 'Rescan', icon: 'refresh', action: () => void vault.rescan() },
            {
              // Reversible (the files, their edit history and trash all live
              // in the folder; re-adding it rebuilds the index), so no confirm
              // — the toast says what happened and how to undo it.
              label: 'Unregister vault',
              icon: 'x',
              action: () => {
                if (vault.current) void vault.unregister(vault.current.id);
              },
            },
          ]
        : []),
    ]);
  }

  function onKeydown(e: KeyboardEvent): void {
    if ((!e.metaKey && !e.ctrlKey) || shouldIgnoreVaultShortcut(e)) return;
    const key = e.key.toLowerCase();
    if (key === 'o') { e.preventDefault(); vault.switcherOpen = true; }
    else if (key === 'n' && vault.current) { e.preventDefault(); openNewNote(''); }
  }

  // ⌘K: the page's verbs (the same ones the toolbar and ⌘O / ⌘N offer).
  $effect(() => {
    const hasVault = !!vault.current;
    return registry.register('vault', hasVault
      ? [
          { id: 'vault.new-note', title: 'New note…', group: 'Vault', keywords: 'create markdown doc page', shortcut: '⌘N', run: () => openNewNote('') },
          { id: 'vault.open-note', title: 'Open note…', group: 'Vault', keywords: 'quick switcher jump find file', shortcut: '⌘O', run: () => (vault.switcherOpen = true) },
        ]
      : []);
  });

  /** Header ⋯: less-used views and actions (the toolbar keeps ≤5 controls).
   *  On a phone list page the toolbar's buttons give way to the pane's
   *  header, so its views come along here. */
  function moreItems(): MenuItem[] {
    if (!vault.current) return [];
    const listPage = viewport.navDrawer && !phoneOpen;
    return [
      ...(listPage
        ? [
            { label: 'Graph view', icon: 'share', checked: vault.centerMode === 'graph', action: toggleGraph },
            { label: 'Docs agent', icon: 'zap', checked: vault.centerMode === 'docs-agents', action: toggleDocsAgents },
            { label: 'Quick switcher', icon: 'search', hint: '⌘O', action: () => (vault.switcherOpen = true) },
            { separator: true },
          ] satisfies MenuItem[]
        : []),
      { label: 'Edit history', icon: 'clock', action: () => void vault.openHistory() },
      { label: 'Trash and restore', icon: 'trash', action: () => void vault.openTrash() },
      { label: 'Rebuild search index', icon: 'refresh', action: () => void rebuildSearchIndex() },
    ];
  }

  // The list pane's views (its header): Files | Search | Tags.
  const MODES = [
    { id: 'files', label: 'Files' },
    { id: 'search', label: 'Search' },
    { id: 'tags', label: 'Tags' },
  ] as const;
  const modeLabel = $derived(MODES.find((m) => m.id === vault.leftMode)?.label ?? 'Files');
  function setMode(id: 'files' | 'search' | 'tags'): void {
    vault.leftMode = id;
    if (id === 'tags') void vault.loadTags();
  }
  const newNote = { icon: 'plus' as const, label: 'New note (⌘N)', onclick: () => openNewNote('') };

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
  // Phone push (ListDetail): whatever the content shows is the page; Back
  // returns to the list with the open note still marked. Opening anything
  // (a row, a tab, a view) pushes again.
  let phoneBack = $state(false);
  const phoneOpen = $derived(vault.centerMode !== 'empty' && !phoneBack);
  $effect(() => {
    void vault.note;
    void vault.filePath;
    void vault.activeTab;
    void vault.centerMode;
    untrack(() => (phoneBack = false));
  });
  // The empty rule: a vault with nothing in it has no list pane; the
  // content's "This vault is empty" owns the page.
  const vaultEmpty = $derived(
    !!vault.current && vault.roots.length === 0 && !vault.treeLoading && !vault.treeError && (vault.status?.notes ?? vault.current.notes) === 0,
  );

  let autoIndexFor: number | null = null;
  $effect(() => {
    if (viewport.navDrawer) return;
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
  const trackedRecovery = $derived(vault.status?.tracked_recovery ?? []);
  function explainTrackedRecovery(): void {
    const cmd = `git rm --cached -r ${trackedRecovery.join(' ')}`;
    toasts.info(
      'Note history is tracked by git',
      `Otto now ignores ${trackedRecovery.join(' and ')}, but files committed earlier are still in the repository. ` +
        `In the vault folder run:\n${cmd}\nthen commit. Earlier commits keep their copies until the history is rewritten.`,
    );
  }

  // The first load comes from the workspace effect below (it runs on mount
  // too) — loading here as well fetched every vault request twice.
  onMount(() => {
    vault.startPolling();
    return () => vault.stopPolling();
  });

  // Leaving the Vault lands a pending autosave first (never asks: the vault
  // auto-saves and keeps a local draft). Moves inside the Vault are the
  // store's own business (it saves before switching notes).
  $effect(() => router.guard((to) => (to === 'vault' || to.startsWith('vault/') ? true : vault.flushBeforeLeave())));

  // Reload when the workspace changes.
  let lastWs = $state('');
  $effect(() => {
    const id = ws.current?.id ?? '';
    if (id && id !== lastWs) { lastWs = id; void vault.load(); }
  });
  $effect(() => { if (vault.vaults.length) void consumeVaultDeepLink(router.parts, vault, router); });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="vault-page">
  <!-- Unified header: vault switcher as the title, status chips as badges,
       tools right-aligned (they collapse into ⋯ when the pane is narrow). -->
  <PageHeader title={vault.current?.name ?? 'Vault'} class="vault-header" more={vault.current ? moreItems : undefined}>
    {#snippet titleContent()}
      <button
        class="vault-pick"
        onclick={(e) => vaultMenu(e)}
        title={vault.current ? `${vault.current.name} — switch vault` : 'Switch vault'}
        aria-haspopup="menu"
      >
        <Icon name="book" size={14} />
        <span class="vp-name">{vault.current?.name ?? 'No vault'}</span>
        <span class="tri"><Icon name="chevronDown" size={12} /></span>
      </button>
    {/snippet}
    {#snippet badge()}
      {#if vault.current?.okf}
        <span class="okf-chip" title="OKF (Open Knowledge Format) vault">OKF</span>
      {/if}
      {#if scanning}
        <span class="scan-chip">Indexing vault…</span>
      {:else if scanError}
        <button class="scan-chip err" title={`${vault.status?.scan_state ?? ''} — click to rescan`} onclick={() => void vault.rescan()}>
          <Icon name="warning" size={12} /> Indexing failed · Rescan
        </button>
      {/if}
      <SearchIndexChip />
      {#if (vault.status?.tracked_recovery?.length ?? 0) > 0}
        <!-- S7-07: private note history / trash committed before Otto ignored
             them keeps being pushed with the repo. -->
        <button
          class="scan-chip err"
          title={`Git already tracks ${trackedRecovery.join(' and ')} (private note history). Click for the command that stops tracking it.`}
          onclick={explainTrackedRecovery}
        >
          <Icon name="warning" size={12} /> Note history is in git
        </button>
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
        {#if vault.centerMode === 'note' && vault.note}
          <!-- The Detail pane's toggle (⌥⌘B): what it holds, not the panel glyph. -->
          <button
            class="icon-btn vh-tool"
            class:active={detailsShown}
            aria-pressed={detailsShown}
            title={detailsShown ? 'Hide note details (⌥⌘B)' : 'Show note details: backlinks, outline, properties (⌥⌘B)'}
            aria-label="Note details"
            data-icon="info"
            data-keep
            data-testid="vault-details-toggle"
            onclick={() => setDetails(!detailsShown)}
          >
            <Icon name="info" size={14} />
          </button>
        {/if}
      {/if}
    {/snippet}
  </PageHeader>

  {#if !vault.current && vault.loadError}
    <LoadState what="vaults" variant="page" loading={vault.loading} error={vault.loadError} empty onretry={() => void vault.load()} />
  {:else if !vault.current && vault.loading}
    <LoadState what="vaults" variant="page" loading empty />
  {:else if !vault.current && !vault.loading}
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
    <PageBody fill padded={false}>
    {#if vault.noteOpenError}
      {@const failed = vault.noteOpenError}
      <div class="note-open-error" data-testid="note-open-error"><LoadState what={failed.path} errorTitle={`Couldn’t open ${failed.path}`} variant="compact" error={failed.message} empty onretry={() => void vault.open(failed.path, failed.opts)}>{#snippet errorActions()}<button class="btn small ghost" onclick={() => (vault.noteOpenError = null)}>Dismiss</button>{/snippet}</LoadState></div>
    {/if}
    <ListDetail
      module="vault"
      name={modeLabel}
      open={phoneOpen}
      onback={() => (phoneBack = true)}
      empty={vaultEmpty}
      legacy={{ w: 'otto_vault_left_w', hidden: () => { const o = lsGet('otto_vault_left_open'); return o === null ? null : o === '0'; } }}
      testid="vault-panes"
    >
      {#snippet list()}
        {#if vault.leftMode === 'files'}
          <FileTree onNewNote={openNewNote} onpick={() => (phoneBack = false)} {views} primary={newNote} />
        {:else if vault.leftMode === 'search'}
          <SearchPanel {views} primary={newNote} onpick={() => (phoneBack = false)} />
        {:else}
          <TagsPanel {views} primary={newNote} />
        {/if}
      {/snippet}
      <main class="center">
        {#if vault.tabs.length > 0}
          <!-- Each tab is a presentational wrapper around TWO real buttons (the
               tab + its close), so the close control isn't nested inside an
               interactive role=tab. Middle-click closes too. -->
          <div class="tabstrip scroll-thin" role="tablist" aria-label="Open notes"><!-- ui-guards: allow — closeable document tabstrip; keyboard-complete via onTabKey -->
            {#each vault.tabs as t, i (t.kind + ':' + t.path)}
              {@const tabName = t.kind === 'note'
                ? (t.path.split('/').pop() ?? t.path).replace(/\.md$/i, '')
                : (t.path.split('/').pop() ?? t.path)}
              <div
                class="vtab"
                class:active={i === vault.activeTab &&
                  (vault.centerMode === 'note' || vault.centerMode === 'file')}
                role="presentation"
              >
                <button
                  class="vtab-main"
                  role="tab"
                  aria-selected={i === vault.activeTab}
                  tabindex={i === vault.activeTab ? 0 : -1}
                  title={t.path}
                  onkeydown={onTabKey}
                  onclick={() => void vault.activateTab(i)}
                  onauxclick={(e) => {
                    if (e.button === 1) void vault.closeTab(i);
                  }}
                >
                  <Icon name={t.kind === 'note' ? 'note' : 'file'} size={12} />
                  <span class="vtab-name">{tabName}</span>
                </button>
                <button
                  class="vtab-close"
                  title="Close tab"
                  aria-label="Close {tabName}"
                  onclick={() => void vault.closeTab(i)}><Icon name="x" size={12} /></button
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
            {#if vault.graphLocal}<span class="gs-path" title={vault.notePath ?? ''}>{vault.notePath}</span>{/if}
          </div>
          <LazyMount lazy={GraphLazy} what="the graph" variant="panel" props={{ local: vault.graphLocal, onNewNote: () => openNewNote('') }} />
        {:else if vault.centerMode === 'trash' || vault.centerMode === 'history'}
          {#key vault.centerMode}<LazyMount lazy={RecoveryLazy} what="recovery" variant="panel" props={{ mode: vault.centerMode }} />{/key}
        {:else if vault.centerMode === 'docs-agents'}
          <LazyMount lazy={DocsAgentsLazy} what="the docs agent" variant="panel" />
        {:else if vault.centerMode === 'file' && vault.filePath}
          <LazyMount lazy={FileViewerLazy} what="the file" variant="panel" />
        {:else if vault.centerMode === 'note' && vault.note}
          <NoteView refineShown={detailsShown && detailView === 'refine'} ontogglerefine={toggleRefine} />
        {:else}
          {@const emptyVault = (vault.status?.notes ?? vault.current.notes) === 0}
          <div class="center-empty">
            <EmptyState
              icon="note"
              title={emptyVault ? 'This vault is empty' : 'No note open'}
              body={emptyVault
                ? 'Start a new markdown note or run the Docs agent to populate this vault.'
                : 'Open a note from the file tree or search, press ⌘O to jump to one, or start a new note.'}
              actionLabel="New note"
              actionIcon="plus"
              onaction={() => openNewNote('')}
            >
              {#if indexNote}
                <button class="btn ghost" onclick={() => void vault.open(indexNote!)}>Open {indexNote}</button>
              {:else if !emptyVault}
                <button class="btn ghost" onclick={() => (vault.switcherOpen = true)}>Quick switcher (⌘O)</button>
              {/if}
            </EmptyState>
          </div>
        {/if}
      </main>
      {#snippet detail()}
        {#if vault.centerMode === 'note' && vault.note}
          <DetailPane
            module="vault"
            pane="details"
            title={detailView === 'refine' ? 'Refine with Otto' : 'Note details'}
            open={vault.centerMode === 'note' && !!vault.note}
            control={details}
            testid="vault-details"
          >
            {#snippet views()}
              <Tabs size="s" label="Note details views" idBase="vault-detail" tabs={DETAIL_VIEWS} value={detailView} onchange={(id) => (detailView = id)} />
            {/snippet}
            <div class="detail-view" role="tabpanel" id="vault-detail-panel-{detailView}" aria-labelledby="vault-detail-tab-{detailView}">
              {#if detailView === 'refine' && vault.notePath}
                <!-- Keyed by note: the same note reloaded after the agent's
                     edit keeps the terminal; another note gets its own session. -->
                {#key `${vault.wsId}:${vault.current?.id}:${vault.notePath}`}
                  <LazyMount lazy={RefineLazy} what="refine" variant="panel" props={{ path: vault.notePath }} />
                {/key}
              {:else}
                <RightPanel />
              {/if}
            </div>
          </DetailPane>
        {/if}
      {/snippet}
    </ListDetail>

    <footer class="vault-statusbar">
      <span class="vs-counts">
        {vault.status?.notes ?? vault.current.notes} notes · {vault.status?.links ?? vault.current.links} links{#if (vault.status?.unresolved ?? 0) > 0}{' · '}{vault.status?.unresolved} unresolved{/if}
      </span>
      {#if vault.note}
        <span>
          {vault.backlinksTotal.toLocaleString()} backlinks{#if vault.backlinksLoading}<span class="spinner vs-inline" role="img" aria-label="Loading backlinks"></span>{:else if vault.backlinksError}<span class="vs-warn" title={vault.backlinksError}><Icon name="warning" size={12} /> couldn’t refresh</span>{/if}
        </span>
        <span>{vault.note.meta.word_count} words</span>
        <span>{(vault.editing ? vault.draft : vault.note.raw).length} characters</span>
        {#if vault.current.okf && vault.okfReport}
          <span class:ok={vault.okfReport.conformant} class:bad={!vault.okfReport.conformant}
            title={vault.okfReport.conformant ? 'This note passes OKF validation' : 'See the OKF section in Note details (Side panel)'}>
            <Icon name={vault.okfReport.conformant ? 'check' : 'warning'} size={12} />
            {vault.okfReport.conformant ? 'OKF valid' : `OKF: ${vault.okfReport.errors.length} ${vault.okfReport.errors.length === 1 ? 'issue' : 'issues'}`}
          </span>
        {/if}
      {/if}
      <span class="grow"></span>
      <span class="dim vs-path" title={vault.current.root_path}>{vault.current.root_path}</span>
    </footer>
    </PageBody>
  {/if}
</div>

{#snippet views()}
  <Tabs size="s" label="Vault views" idBase="vault-left" tabs={MODES} value={vault.leftMode} onchange={setMode} testid="vault-left-modes" />
{/snippet}

{#if createOpen}
  <Modal title="Add a vault" onclose={() => (createOpen = false)}>
    <form class="av-body" onsubmit={(e) => { e.preventDefault(); void submitCreate(); }}>
      <div class="field">
        <label for="av-name">Name</label>
        <input dir="auto" id="av-name" class="input" bind:value={cName} placeholder="Team Docs" use:focusOnMount />
      </div>
      <div class="field">
        <label for="av-path">Folder</label>
        <PathField bind:value={cPath} start={cPath || '~'}>
          <input dir="ltr" id="av-path" class="input" bind:value={cPath} placeholder="~/Documents/Obsidian/MyVault" spellcheck="false" />
        </PathField>
        <span class="hint">An existing folder (an Obsidian vault works as is) or a new path to create. Leave it blank to create one under ~/.otto/vault.</span>
      </div>
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={cOkf} />
        OKF vault (Open Knowledge Format validation and templates)
      </label>
      {#if createError}
        <div class="av-err" use:announceShown={{ text: `Couldn’t add the vault. ${createError}`, assertive: true }}>Couldn’t add the vault. {createError}</div>
      {/if}
    </form>
    {#snippet footer()}
      <button class="btn" onclick={() => (createOpen = false)}>Cancel</button>
      <button class="btn primary" disabled={!cName.trim() || creating} onclick={() => void submitCreate()}>
        {creating ? 'Adding…' : 'Add vault'}
      </button>
    {/snippet}
  </Modal>
{/if}

<NewNoteDialog bind:open={newNoteOpen} bind:dir={newNoteDir} />
<Switcher />

<style>
  .note-open-error {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 12px;
    background: var(--danger-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .note-open-error span { flex: 1; min-width: 0; overflow-wrap: anywhere; }

  .graph-scope { display: flex; align-items: center; gap: 12px; padding: 8px 12px; flex-wrap: wrap; font-size: var(--fs-s); color: var(--text-dim); border-bottom: 1px solid var(--border); }
  .gs-path { min-width: 0; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .graph-scope select { background: var(--bg); color: var(--text); border: 1px solid var(--border); padding: 4px; border-radius: var(--radius-s); }
  .vault-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .vault-pick {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: var(--radius-m);
    padding: 4px 10px;
    font-size: var(--fs-s);
    cursor: pointer;
    max-width: 260px;
  }
  /* The name is a flex item: without min-width:0 it never shrinks below
     its full text, so a long vault name pushed past the 260px cap with no
     ellipsis (the chevron was the part that got cut). */
  .vp-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tri {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .okf-chip {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 1px 6px;
  }
  .scan-chip {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  .scan-chip.err {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--danger);
    background: var(--danger-soft);
    border: none;
    border-radius: 999px;
    padding: 2px 8px;
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    animation: none;
  }
  .run-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: var(--accent-soft);
    border: 1px solid var(--accent-line);
    border-radius: 999px;
    padding: 2px 8px;
    cursor: pointer;
    white-space: nowrap;
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  .run-chip:hover {
    animation: none;
  }
  
  @media (prefers-reduced-motion: reduce) {
    .scan-chip,
    .run-chip {
      animation: none;
    }
  }
  .grow {
    flex: 1;
  }
  /* Header + sidebar-mode toggles are global .icon-btn; "on" is the quiet
     selection tint, never the accent as text colour. */
  .vh-tool.active {
    background: var(--accent-soft);
    color: var(--accent-text);
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
  }
  .vtab {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-inline-end: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    background: none;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-m) var(--radius-m) 0 0;
    user-select: none;
    white-space: nowrap;
    max-width: 220px;
  }
  .vtab-main {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding-block: 4px; padding-inline: 10px 4px;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    border-radius: var(--radius-s);
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
    border-radius: var(--radius-s);
    background: none;
    color: var(--text-dim);
    flex-shrink: 0;
    cursor: pointer;
    padding: 0;
  }
  .vtab-close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .detail-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .center-empty {
    overflow-y: auto;
  }
  .vs-inline {
    margin-inline-start: 6px;
    vertical-align: middle;
  }
  .vs-warn {
    margin-inline-start: 6px;
    color: var(--warning);
  }
  .vault-statusbar {
    display: flex;
    /* A phone wraps whole items onto a second row ("0 backlinks" never
       splits into "0" over "backlinks"). */
    flex-wrap: wrap;
    gap: 2px 14px;
    align-items: center;
    padding: 4px 14px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .vault-statusbar > span:has(> :global(svg)) {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .vault-statusbar .ok {
    color: var(--success);
  }
  .vault-statusbar .bad {
    color: var(--danger);
  }
  .vault-statusbar > span {
    white-space: nowrap;
  }
  .vault-statusbar .dim {
    min-width: 0;
    max-width: 100%;
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-body {
    display: flex;
    flex-direction: column;
  }
  .av-body .checkbox-row {
    margin-bottom: 4px;
  }
  .av-err {
    margin-top: 8px;
    color: var(--danger);
    font-size: var(--fs-s);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    background: var(--danger-soft);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    word-break: break-word;
  }

  @media (pointer: coarse), (max-width: 640px) {
    .vtab-main {
      min-height: var(--hit-min);
    }
    .vtab-close {
      width: var(--hit-min);
      height: var(--hit-min);
    }
  }

  @media (max-width: 640px) {
    .vs-path { display: none; }
  }
</style>
