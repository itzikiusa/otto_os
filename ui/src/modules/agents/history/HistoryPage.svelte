<script lang="ts">
  import { toastError } from '../../../lib/toastError';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  // History (`#/history[/<sessionId>]`) — every past Claude/Codex conversation,
  // grouped by repo/cwd like the Codex/Claude app sidebars, with a read-only
  // conversation on the right (docs/design/conversation-view.md §5.3).
  //
  // Rows are Otto sessions (any status, archived included) merged with
  // transcripts found on disk that no session claims (`on_disk`). An on_disk
  // row is read through the path route (`transcriptPath` mode); every other row
  // through its session. "Resume in Otto" imports an on_disk transcript as a
  // reconnectable session and then rides the existing restart/resume path.
  import { untrack } from 'svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import { LIST_PANE, loadPaneWidth } from '../../../lib/paneResizer';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../../lib/stores/workspace.svelte';
  import { activity } from '../../../lib/stores/activity.svelte';
  import { router } from '../../../lib/router.svelte';
  import { PHONE_MAX } from '../../../lib/stores/viewport.svelte';
  import { registry } from '../../../lib/commands.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { api } from '../../../lib/api/client';
  import { transcript } from '../../../lib/stores/transcript.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import { recallSelection, rememberSelection } from '../../../lib/lastSelection';
  import { ConversationView } from '../conversation';
  import OutputsPanel from '../../panels/OutputsPanel.svelte';
  import {
    history,
    entryKey,
    entryTitle,
    shortCwd,
    type DateWindow,
    type ProviderFilter,
    type StatusFilter,
  } from './history.svelte';
  import type { Artifact, HistoryEntry, HistoryStatus } from '../../../lib/api/types';

  let scope = $state<'workspace' | 'scratch'>('workspace');
  // The conversation list's width — drag / ←→ on the divider, remembered.
  let listW = $state(loadPaneWidth('history.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  const wsId = $derived(scope === 'scratch' ? SCRATCH_WORKSPACE_ID : (ws.currentId ?? SCRATCH_WORKSPACE_ID));
  const canEdit = $derived(wsId === SCRATCH_WORKSPACE_ID || ws.myRole !== 'viewer');
  const sel = $derived(history.selected);
  const selKey = $derived(sel ? entryKey(sel) : null);

  // ── Loading: workspace + filter changes; search is debounced ────────────────
  $effect(() => {
    const w = wsId;
    // Read the select-driven filters so the effect re-runs when they change.
    // `history.load` itself touches store state (and `q`, which is debounced
    // below) — untrack it so typing never fires an immediate extra request.
    void history.provider;
    void history.status;
    void history.cwd;
    if (w) untrack(() => void history.load(w));
  });

  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  function onSearchInput(): void {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void history.refresh(), 250);
  }

  // Deep link: `#/history/<sessionId>` preselects that row once it is loaded.
  $effect(() => {
    const want = router.parts[1];
    if (!want || history.entries.length === 0) return;
    const hit = history.visible.find((e) => e.session_id === want);
    if (hit && history.selectedKey !== entryKey(hit)) history.select(hit);
  });

  function pick(e: HistoryEntry): void {
    history.select(e);
    rememberSelection('history', entryKey(e));
    if (e.session_id) router.replace(`history/${e.session_id}`);
    else if (router.parts[1]) router.replace('history');
  }

  function clearSelection(): void {
    history.select(null);
    rememberSelection('history', null);
    if (router.parts[1]) router.replace('history');
  }

  // List/detail never opens onto an empty "pick one" pane: whenever the list
  // has rows and nothing is selected (first load, a scope switch, or a filter
  // that dropped the open row), restore the last conversation read here or
  // fall back to the newest visible one. A deep link that still resolves wins.
  // Skipped at the narrow list-OR-detail breakpoint, where selecting would hide
  // the list the user came to see (and "Back" there must stick).
  $effect(() => {
    const w = wsId;
    const groups = history.groups;
    if (!w || history.loading) return;
    const visible = groups.flatMap((g) => g.entries);
    if (history.selectedKey && visible.some((e) => entryKey(e) === history.selectedKey)) return;
    untrack(() => {
      if (history.selectedKey) clearSelection();
      const want = router.parts[1];
      if (want && visible.some((e) => e.session_id === want)) return;
      // Phone (one pane at a time, push navigation): no auto-open.
      if (typeof window !== 'undefined' && window.matchMedia?.(`(max-width: ${PHONE_MAX}px)`).matches) return;
      if (visible.length === 0) return;
      const last = recallSelection('history');
      pick(visible.find((e) => entryKey(e) === last) ?? visible[0]);
    });
  });

  // ── Rescan progress (WS `history_index_progress` via the activity store) ─────
  let rescanning = $state(false);
  const idx = $derived(activity.historyIndex);
  $effect(() => {
    if (rescanning && idx?.done) {
      rescanning = false;
      void history.refresh();
    }
  });
  async function rescan(): Promise<void> {
    if (!wsId || rescanning) return;
    rescanning = true;
    try {
      await history.rescan(wsId);
    } catch (e) {
      rescanning = false;
      toastError('Couldn’t rescan', e);
    }
  }

  // ── Actions ─────────────────────────────────────────────────────────────────
  let busy = $state(false);
  let searchEl = $state<HTMLInputElement | null>(null);

  // ⌘K verbs while History is open (resume runs the same path as the button).
  $effect(() =>
    registry.register('history', [
      { id: 'history.search', title: 'Search conversation history', group: 'History', keywords: 'find past conversation transcript prompt', run: () => searchEl?.focus() },
      { id: 'history.rescan', title: 'Rescan conversation history', group: 'History', keywords: 'refresh import disk transcripts', run: () => void rescan() },
      { id: 'history.scope-workspace', title: 'Show this workspace’s conversations', group: 'History', keywords: 'scope filter', run: () => (scope = 'workspace') },
      { id: 'history.scope-scratch', title: 'Show conversations with no workspace', group: 'History', keywords: 'scope scratch filter', run: () => (scope = 'scratch') },
      ...(sel && canEdit ? [{ id: 'history.resume', title: resumeLabel(sel), detail: entryTitle(sel), group: 'History', keywords: 'continue resume conversation', run: () => void resume(sel) }] : []),
    ]),
  );

  function isLiveStatus(status: HistoryStatus): boolean {
    return status === 'working' || status === 'running' || status === 'idle';
  }

  /** Import if needed, safely ensure the session is live, then open in Chat. */
  async function resume(e: HistoryEntry): Promise<void> {
    if (!wsId || busy) return;
    busy = true;
    try {
      let sid = e.session_id;
      if (e.status === 'on_disk' || !sid) {
        sid = (await history.importEntry(wsId, e)).id;
        await ws.refreshSessions();
      }
      // The row may be stale in either direction. The server checks the live
      // PTY under its resume lock and never replaces an already-live process.
      const resumed = await ws.resumeSession(sid);
      history.patchSession(sid, { status: resumed.status });
      openInChat(sid);
    } catch (err) {
      toastError('Couldn’t resume', err);
    } finally {
      busy = false;
    }
  }

  /** Update both the reactive view preference and its persisted value. */
  function openInChat(sid: string): void {
    transcript.setView(sid, 'chat');
    ws.setViewMode('tabs');
    ws.navigateToSession(sid);
  }

  async function copyText(v: string, what = 'Copied'): Promise<void> {
    try {
      await navigator.clipboard.writeText(v);
      toasts.info(what, v);
    } catch {
      toasts.error('Couldn’t copy', v);
    }
  }

  /** Reveal the folder in the OS file manager (desktop app); in a plain
   *  browser there is no bridge, so the path is copied instead — said so. */
  async function openFolder(e: HistoryEntry): Promise<void> {
    if ('__TAURI_INTERNALS__' in window) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('plugin:opener|reveal_item_in_dir', { path: e.cwd });
        return;
      } catch {
        /* permission not granted in this build — fall through to copy */
      }
    }
    await copyText(e.cwd, 'Path copied (no file-manager bridge here)');
  }

  async function archive(e: HistoryEntry): Promise<void> {
    if (!e.session_id || busy) return;
    busy = true;
    try {
      await ws.archiveSession(e.session_id);
      history.patchSession(e.session_id, { status: 'exited' });
    } catch (err) {
      toastError('Couldn’t archive', err);
    } finally {
      busy = false;
    }
  }

  function resumeLabel(e: HistoryEntry): string {
    return isLiveStatus(e.status) ? 'Open in Otto' : 'Resume in Otto';
  }

  function menuFor(e: HistoryEntry): MenuItem[] {
    const live = isLiveStatus(e.status);
    return [
      {
        label: resumeLabel(e),
        icon: 'play',
        disabled: !canEdit || (!live && !e.resumable && e.status !== 'on_disk'),
        action: () => void resume(e),
      },
      { label: 'Open folder', icon: 'folder', action: () => void openFolder(e) },
      { label: 'Copy transcript path', icon: 'copy', action: () => void copyText(e.transcript_path) },
      { label: 'Copy folder path', icon: 'copy', action: () => void copyText(e.cwd) },
      { separator: true },
      {
        label: 'Archive',
        icon: 'archive',
        disabled: !canEdit || !e.session_id || e.status === 'on_disk',
        action: () => void archive(e),
      },
    ];
  }

  // ── Outputs under the conversation (collapsed until asked) ───────────────────
  let outputsOpen = $state(false);
  let diskArtifacts = $state<Artifact[] | null>(null);
  let diskArtifactsFor: string | null = null;

  $effect(() => {
    // Reset per selection.
    void selKey;
    outputsOpen = false;
    diskArtifacts = null;
    diskArtifactsFor = null;
  });

  /** on_disk rows have no session → the server lists the artifacts its fold
   *  of the transcript collected (`GET …/history/artifacts`). This used to
   *  pull 500 full turns (up to a 2 MB page) just to pick the chips out of
   *  their blocks (SA-07). */
  async function loadDiskArtifacts(e: HistoryEntry): Promise<void> {
    if (!wsId || diskArtifactsFor === e.transcript_path) return;
    const path = e.transcript_path;
    diskArtifactsFor = path;
    try {
      const arts = await api.get<Artifact[]>(
        `/workspaces/${wsId}/history/artifacts?path=${encodeURIComponent(path)}`,
      );
      if (diskArtifactsFor === path) diskArtifacts = arts;
    } catch {
      if (diskArtifactsFor === path) diskArtifacts = [];
    }
  }

  function toggleOutputs(): void {
    outputsOpen = !outputsOpen;
    if (outputsOpen && sel && sel.status === 'on_disk') void loadDiskArtifacts(sel);
  }

  // ── Display helpers ─────────────────────────────────────────────────────────
  let collapsed = $state<Record<string, boolean>>({});

  /** Sentence-case status words (content.md), matching the session vocabulary. */
  const STATUS_LABEL: Record<HistoryStatus, string> = {
    working: 'Working',
    running: 'Running',
    idle: 'Idle',
    exited: 'Ended',
    reconnectable: 'Resumable',
    on_disk: 'On disk only',
  };

  const PROVIDERS: { id: ProviderFilter; label: string }[] = [
    { id: 'all', label: 'All providers' },
    { id: 'claude', label: 'Claude' },
    { id: 'codex', label: 'Codex' },
  ];
  const STATUSES: { id: StatusFilter; label: string }[] = [
    { id: 'all', label: 'Any status' },
    { id: 'working', label: 'Working' },
    { id: 'running', label: 'Running' },
    { id: 'idle', label: 'Idle' },
    { id: 'exited', label: 'Exited' },
    { id: 'reconnectable', label: 'Resumable' },
    { id: 'on_disk', label: 'On disk only' },
  ];
  const DATES: { id: DateWindow; label: string }[] = [
    { id: 'all', label: 'Any time' },
    { id: 'today', label: 'Today' },
    { id: '7d', label: 'Last 7 days' },
    { id: '30d', label: 'Last 30 days' },
  ];

  const shown = $derived(history.groups.reduce((n, g) => n + g.entries.length, 0));
  /** A server-side filter is narrowing the list (an empty result is a miss, not "no history"). */
  const serverFiltered = $derived(
    !!history.q.trim() || history.provider !== 'all' || history.status !== 'all' || !!history.cwd,
  );
  const filtersActive = $derived(serverFiltered || history.date !== 'all');
  /** Nothing to list at all (first load, a failed load, or truly no history):
   *  the list pane — search, filters and an empty column — is hidden and the
   *  page speaks once, from the right (loading / error + Retry / empty + CTA). */
  // …except when the scope toggle lives there (a workspace is open): an empty
  // or still-loading scope must not hide the way to the other scope.
  const nothingToList = $derived(history.entries.length === 0 && !serverFiltered && !history.hasMore);
  const hideList = $derived(nothingToList && !ws.currentId);
  const isEmpty = $derived(nothingToList && !history.error && !history.loading);
  /** The conversation the header names (only once it can actually be shown). */
  const headSel = $derived(sel && wsId ? sel : null);

  function clearFilters(): void {
    const hadQuery = !!history.q.trim();
    history.q = '';
    history.provider = 'all';
    history.status = 'all';
    history.cwd = '';
    history.date = 'all';
    // provider/status/cwd re-run the load effect; the debounced query does not.
    if (hadQuery) void history.refresh();
  }
</script>

<div class="history-page">
<PageHeader
  title={headSel ? entryTitle(headSel) : 'History'}
  subtitle={headSel
    ? `${headSel.repo_name ?? shortCwd(headSel.cwd)}${headSel.turns != null ? ` · ${headSel.turns} ${headSel.turns === 1 ? 'turn' : 'turns'}` : ''} · ${rel(headSel.last_active_at)}`
    : 'Past Claude and Codex conversations: Otto sessions and transcripts on disk'}
>
  {#snippet leading()}
    {#if headSel}
      <button class="icon-btn back" onclick={clearSelection} title="Back to the list" aria-label="Back to the list">
        <Icon name="chevronLeft" size={16} />
      </button>
    {/if}
  {/snippet}
  {#snippet badge()}
    {#if headSel}
      <span class="dmeta">
        <span class="glyph {headSel.provider}"><ProviderIcon provider={headSel.provider} size={14} /></span>
        <span class="cap">{headSel.provider}</span>
        <span class="dot st-{headSel.status}" aria-hidden="true"></span>
        <span>{STATUS_LABEL[headSel.status]}</span>
      </span>
    {/if}
  {/snippet}
  {#snippet actions()}
    <!-- The empty page's own CTA is "Rescan transcripts" — no second copy up here. -->
    {#if !isEmpty}
      <button
        class="btn small"
        onclick={() => void rescan()}
        disabled={rescanning}
        title={rescanning ? 'Rescanning transcripts on disk…' : 'Rescan ~/.claude/projects and ~/.codex/sessions for new transcripts'}
        aria-label="Rescan transcripts"
        data-icon="refresh"
        data-overflow="-5"
        data-testid="history-rescan"
      >
        <Icon name="refresh" size={12} /> {rescanning ? 'Rescanning…' : 'Rescan'}
      </button>
    {/if}
    {#if headSel}
      {@const cur = headSel}
      {#if canEdit && cur.session_id && cur.status !== 'on_disk'}
        <button class="icon-btn" data-overflow="-3" data-icon="archive" data-label="Archive session" onclick={() => void archive(cur)} disabled={busy}
          aria-label="Archive" title="Archive the session — restore it any time from the sidebar’s Archived list">
          <Icon name="archive" size={14} />
        </button>
      {/if}
      <button class="btn small" data-overflow="-2" data-icon="copy" data-label="Copy transcript path" onclick={() => void copyText(cur.transcript_path, 'Transcript path copied')} title={cur.transcript_path}>
        <Icon name="copy" size={12} /> Copy path
      </button>
      <button class="btn small" data-overflow="-1" data-icon="folder" data-label="Open folder" onclick={() => void openFolder(cur)} title="Reveal {cur.cwd}">
        <Icon name="folder" size={12} /> Open folder
      </button>
      <!-- The primary is last (the trailing edge is where the eye lands). -->
      {#if canEdit}
        {@const resumable = isLiveStatus(cur.status) || cur.resumable || cur.status === 'on_disk'}
        <button
          class="btn small primary"
          onclick={() => void resume(cur)}
          disabled={busy || !resumable}
          title={resumable ? (cur.status === 'on_disk' ? 'Import this transcript as an Otto session and continue it' : 'Continue this conversation in Otto') : 'This conversation can’t be resumed (the CLI left no resumable state)'}
          data-icon="play"
          data-label={resumeLabel(cur)}
          data-testid="history-resume"
        >
          <Icon name="play" size={12} /> {resumeLabel(cur)}
        </button>
      {/if}
    {/if}
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
<div class="history" class:has-sel={!!sel} class:list-hidden={hideList} data-testid="history-page">
  <!-- ── Left: search, filters, grouped list ─────────────────────────────── -->
  {#if !hideList}
  <aside class="hlist" aria-label="Conversations" style="--list-pane-w:{listW}px">
    {#if ws.currentId}
      <div class="segmented scope-seg" role="group" aria-label="Which conversations">
        <button class:active={scope === 'workspace'} aria-pressed={scope === 'workspace'}
          onclick={() => (scope = 'workspace')} title="Conversations in the current workspace">This workspace</button>
        <button class:active={scope === 'scratch'} aria-pressed={scope === 'scratch'}
          onclick={() => (scope = 'scratch')} title="Conversations started outside any workspace">No workspace</button>
      </div>
    {/if}
    <div class="toolbar">
      <div class="search-wrap">
        <Icon name="search" size={12} />
        <input
          class="search"
          bind:this={searchEl}
          placeholder="Search titles and first prompts…"
          bind:value={history.q}
          oninput={onSearchInput}
          spellcheck="false"
          aria-label="Search history"
          data-testid="history-search"
        />
      </div>
    </div>
    <div class="filters" role="group" aria-label="Filters">
      <select class="sel" class:set={history.provider !== 'all'} bind:value={history.provider} aria-label="Provider">
        {#each PROVIDERS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
      </select>
      <select class="sel" class:set={history.status !== 'all'} bind:value={history.status} aria-label="Status">
        {#each STATUSES as st (st.id)}<option value={st.id}>{st.label}</option>{/each}
      </select>
      <select class="sel" class:set={history.date !== 'all'} bind:value={history.date} aria-label="Date">
        {#each DATES as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
      </select>
      <select class="sel folder" class:set={!!history.cwd} bind:value={history.cwd} aria-label="Folder"
        title={history.cwd || 'All folders'}>
        <option value="">All folders</option>
        {#each history.folders as f (f.cwd)}<option value={f.cwd} title={f.cwd}>{f.label}</option>{/each}
      </select>
      {#if filtersActive}
        <button class="btn small ghost clear" onclick={clearFilters} title="Show every conversation again">Clear</button>
      {/if}
    </div>
    {#if rescanning || (idx && !idx.done)}
      <div class="progress" data-testid="history-progress">
        <span class="dim">Indexing transcripts…</span>
        <span class="mono dim">{idx ? `${idx.scanned}/${idx.total || '?'}` : '…'}</span>
        <span class="ptrack" aria-hidden="true">
          <span class="pfill" style="width:{idx && idx.total ? Math.round((idx.scanned / idx.total) * 100) : 0}%"></span>
        </span>
      </div>
    {/if}

    <div class="rows" data-testid="history-list">
      {#if history.error}
        <p class="empty-line err" role="alert">
          <span>Refresh failed: {history.error}</span>
          <button class="btn small" onclick={() => void history.refresh()}>Retry</button>
        </p>
      {/if}
      {#if history.loading && history.entries.length === 0}
        <Skeleton rows={5} height={40} label="conversations" />
      {:else if shown === 0 && !history.error}
        <!-- The miss is explained (with its fix) by the right pane; this line
             only shows at the narrow list-only layout, where that pane is hidden. -->
        <p class="empty-line dim narrow-only">
          {history.hasMore ? 'No matches in the loaded part of history yet.' : 'Nothing matches these filters.'}
          {#if filtersActive && !history.hasMore}
            <button class="btn small" onclick={clearFilters}>Clear filters</button>
          {/if}
        </p>
      {:else}
        {#each history.groups as g (g.key)}
          <div class="group">
            <button class="group-head" onclick={() => (collapsed[g.key] = !collapsed[g.key])} title={g.cwd}
              aria-expanded={!collapsed[g.key]}>
              <Icon name={collapsed[g.key] ? 'chevronRight' : 'chevronDown'} size={12} />
              <Icon name="folder" size={12} />
              <span class="group-label">{g.label}</span>
              <span class="count">{g.entries.length}</span>
            </button>
            {#if !collapsed[g.key]}
              {#each g.entries as e (entryKey(e))}
                <div
                  class="row"
                  class:on={selKey === entryKey(e)}
                  data-testid="history-row"
                  data-status={e.status}
                  data-session-id={e.session_id}
                >
                  <button
                    class="row-main"
                    aria-current={selKey === entryKey(e) ? 'true' : undefined}
                    onclick={() => pick(e)}
                    oncontextmenu={(k) => ctxMenu.show(k, menuFor(e))}
                  >
                    <span class="glyph {e.provider}" title={e.provider}><ProviderIcon provider={e.provider} size={18} /></span>
                    <span class="row-body">
                      <span class="row-title" title={entryTitle(e)}>{entryTitle(e)}</span>
                      <span class="row-meta">
                        <span class="dot st-{e.status}" aria-hidden="true"></span>
                        <span class="sr-only">{STATUS_LABEL[e.status]},</span>
                        <span title={new Date(e.last_active_at).toLocaleString()}>{rel(e.last_active_at)}</span>
                        {#if e.turns != null}<span>· {e.turns} {e.turns === 1 ? 'turn' : 'turns'}</span>{/if}
                        {#if e.status === 'on_disk'}<span class="on-disk" title="A transcript on disk that no Otto session owns">on disk</span>{/if}
                      </span>
                    </span>
                  </button>
                  <button
                    class="icon-btn row-more"
                    onclick={(k) => ctxMenu.show(k, menuFor(e))}
                    title="More actions"
                    aria-label="More actions for {entryTitle(e)}"
                  ><Icon name="more" size={14} /></button>
                </div>
              {/each}
            {/if}
          </div>
        {/each}
      {/if}
      {#if history.hasMore && !history.loading}
        <button class="btn small more" onclick={() => void history.loadMore()} disabled={history.loadingMore}>
          {history.loadingMore ? 'Loading older conversations…' : 'Load older conversations'}
        </button>
      {/if}
    </div>
  </aside>
  <PaneDivider bind:width={listW} storageKey="history.listW" label="Resize the conversations list" max={LIST_PANE.max} />
  {/if}

  <!-- ── Right: read-only conversation + outputs ─────────────────────────── -->
  <section class="hdetail">
    {#if nothingToList || (!sel && history.error)}
      <LoadState
        variant="page"
        what="history"
        loading={history.loading}
        error={history.error}
        empty={true}
        onretry={() => void history.refresh()}
      >
        {#snippet emptyView()}
          <EmptyState
            variant="page"
            icon="clock"
            title="No conversations yet"
            body={rescanning
              ? `Indexing transcripts on disk… ${idx ? `${idx.scanned}/${idx.total || '?'}` : ''}`
              : scope === 'scratch'
                ? 'Conversations started outside a workspace show up here. Rescan to pick up transcripts already on disk.'
                : 'Run claude or codex in this workspace, or rescan to pick up transcripts already on disk.'}
            actionLabel={rescanning ? 'Rescanning…' : 'Rescan transcripts'}
            actionIcon="refresh"
            onaction={() => void rescan()}
          />
        {/snippet}
      </LoadState>
    {:else if !sel && shown === 0}
      {#if history.hasMore}
        <EmptyState
          variant="page"
          icon="search"
          title="No matches yet"
          body="No matches in this part of history. Load more to keep looking."
          actionLabel={history.loadingMore ? 'Loading more history…' : 'Load more'}
          onaction={() => void history.loadMore()}
        />
      {:else}
        <EmptyState
          variant="page"
          icon="filter"
          title="No matching conversations"
          body="Nothing matches these filters. Clear them to see every conversation."
          actionLabel="Clear filters"
          onaction={clearFilters}
        />
      {/if}
    {:else if !sel}
      <!-- Transient on desktop: a row is auto-picked as soon as the list settles. -->
      {#if history.loading}
        <LoadState variant="page" what="conversations" loading={true} empty={true} />
      {:else}
        <EmptyState
          variant="page"
          icon="clock"
          title="{history.entries.length}{history.hasMore ? '+' : ''} {history.entries.length === 1 ? 'conversation' : 'conversations'} in {history.groups.length} {history.groups.length === 1 ? 'folder' : 'folders'}"
          body="Pick one on the left to read it here; resume it to keep going."
        />
      {/if}
    {:else if wsId}
      <!-- Phone: the header title ellipsizes and has no hover, so the full
           conversation title wraps here (hidden from AT: the h1 already names it). -->
      <p class="dfull" aria-hidden="true">{entryTitle(sel)}</p>
      <div class="dconv" data-testid="history-conversation">
        {#key selKey}
          {#if sel.status === 'on_disk' || !sel.session_id}
            <ConversationView transcriptPath={sel.transcript_path} workspaceId={wsId} readonly />
          {:else}
            <ConversationView sessionId={sel.session_id} workspaceId={wsId} readonly />
          {/if}
        {/key}
      </div>

      <div class="doutputs" class:open={outputsOpen}>
        <button class="doutputs-head" onclick={toggleOutputs} aria-expanded={outputsOpen}>
          <Icon name={outputsOpen ? 'chevronDown' : 'chevronRight'} size={12} />
          <Icon name="layers" size={12} />
          <span>Outputs</span>
          <span class="dim small">
            {sel.status === 'on_disk' ? 'Files, PRs and images this conversation produced' : 'Artifacts of this session'}
          </span>
        </button>
        {#if outputsOpen}
          {#if sel.status === 'on_disk' || !sel.session_id}
            {#if diskArtifacts === null}
              <p class="empty-line dim">Reading the transcript…</p>
            {:else}
              <OutputsPanel artifacts={diskArtifacts} embedded />
            {/if}
          {:else}
            <OutputsPanel sessionId={sel.session_id} embedded />
          {/if}
        {/if}
      </div>
    {/if}
  </section>
</div>
</PageBody>
</div>

<style>
  .history-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .history {
    display: flex;
    flex: 1;
    min-height: 0;
    background: var(--bg);
    color: var(--text);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .small {
    font-size: var(--fs-xs);
    font-weight: 400;
  }
  .cap {
    text-transform: capitalize;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .err {
    color: var(--danger);
  }
  .empty-line {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    font-size: var(--fs-s);
    line-height: 1.45;
    margin: 8px 6px;
  }
  .empty-line.narrow-only {
    display: none;
  }

  /* ── list ──────────────────────────────────────────────────────────────── */
  .hlist {
    flex: 0 0 var(--list-pane-w, 280px);
    min-width: 220px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-sidebar);
  }
  .scope-seg {
    display: flex;
    margin: 8px 8px 0;
  }
  .scope-seg > button {
    flex: 1;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 4px;
  }
  .search-wrap {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search-wrap:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .search {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    outline: none;
  }
  /* A filter bar of pill selects, each as wide as its value; a non-default
     filter is tinted so a narrowed list never looks like "all". */
  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 4px 8px 8px;
    border-bottom: 1px solid var(--border);
  }
  .sel {
    min-width: 0;
    max-width: 100%;
    height: 22px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    padding: 0 6px;
    cursor: pointer;
  }
  .sel.folder {
    max-width: 150px;
  }
  .sel.set {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .sel:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .clear {
    margin-inline-start: auto;
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    font-size: var(--fs-xs);
    border-bottom: 1px solid var(--border);
  }
  .ptrack {
    flex: 1;
    height: 3px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .pfill {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 6px 10px;
  }
  .group-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 6px 2px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-align: start;
    cursor: pointer;
  }
  .group-head:hover {
    color: var(--text);
  }
  .group-head:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .group-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-size: var(--fs-xs);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    background: var(--surface-2);
    border-radius: 999px;
    padding: 0 6px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius-s);
    padding-inline-end: 4px;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.on {
    background: var(--accent-soft);
  }
  .row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding-block: 6px; padding-inline: 8px 4px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .row-main:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  /* Provider mark — the shared ProviderIcon (the sidebar/pane header one), not
     a private letter glyph. */
  .glyph {
    flex-shrink: 0;
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    margin-top: 1px;
  }
  .row-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row-title {
    font-size: var(--fs-m);
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-meta {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    min-width: 0;
  }
  .on-disk {
    font-size: var(--fs-xs);
    border: 1px dashed var(--border-strong);
    border-radius: 999px;
    padding: 0 6px;
  }
  /* Session vocabulary (patterns.md §1): running = accent, idle = grey, ended =
     faded grey (a normal stop is NOT red), resumable = a hollow ring like
     "suspended" (amber is reserved for "needs you"), on disk = dashed ring. */
  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-idle);
    flex-shrink: 0;
    box-sizing: border-box;
  }
  .st-running {
    background: var(--accent);
  }
  .st-idle {
    background: var(--status-idle);
  }
  .st-exited {
    background: var(--status-idle);
    opacity: 0.5;
  }
  .st-reconnectable {
    background: transparent;
    border: 1.5px solid var(--status-idle);
  }
  .st-on_disk {
    background: transparent;
    border: 1px dashed var(--text-dim);
  }
  /* Quiet until the row is hovered/selected/focused — but always reachable by
     keyboard and always shown on touch (no hover there). */
  .row-more {
    opacity: 0;
  }
  .row:hover .row-more,
  .row.on .row-more,
  .row:focus-within .row-more {
    opacity: 1;
  }
  @media (hover: none) {
    .row-more {
      opacity: 1;
    }
  }
  .more {
    width: 100%;
    margin-top: 6px;
  }

  /* ── detail ────────────────────────────────────────────────────────────── */
  .hdetail {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .back {
    display: none;
  }
  .dmeta {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .dmeta .glyph {
    width: 14px;
    height: 14px;
    margin-top: 0;
  }
  .dconv {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .dconv > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .doutputs {
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    max-height: 45%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .doutputs.open {
    overflow-y: auto;
  }
  .doutputs-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 12px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    text-align: start;
    cursor: pointer;
    flex-shrink: 0;
  }
  .doutputs-head:hover {
    background: var(--hover);
  }
  .doutputs-head:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }

  /* ── narrow: list OR detail (with a back button) ───────────────────────── */
  @media (max-width: 640px) {
    .hlist {
      flex: 1;
    }
    /* One pane at a time here — nothing to drag. */
    .history :global(.pane-divider) {
      display: none;
    }
    .history.has-sel .hlist {
      display: none;
    }
    .history:not(.has-sel):not(.list-hidden) .hdetail {
      display: none;
    }
    .back {
      display: inline-flex;
    }
    .empty-line.narrow-only {
      display: flex;
    }
  }
  .dfull {
    display: none;
  }
  @media (max-width: 640px) {
    .dfull {
      display: block;
      margin: 0;
      padding-block: 8px;
      padding-inline: 14px;
      font-size: var(--fs-m);
      font-weight: 600;
      overflow-wrap: anywhere;
      border-block-end: 1px solid var(--separator);
    }
  }
</style>
