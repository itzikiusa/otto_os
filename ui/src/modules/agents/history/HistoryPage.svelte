<script lang="ts">
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
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { activity } from '../../../lib/stores/activity.svelte';
  import { router } from '../../../lib/router.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { api } from '../../../lib/api/client';
  import { winKey } from '../../../lib/win';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
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
  import type { Artifact, HistoryEntry, HistoryStatus, Transcript } from '../../../lib/api/types';

  const wsId = $derived(ws.currentId);
  const canEdit = $derived(ws.myRole !== 'viewer');
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
    const hit = history.entries.find((e) => e.session_id === want);
    if (hit && history.selectedKey !== entryKey(hit)) history.select(hit);
  });

  function pick(e: HistoryEntry): void {
    history.select(e);
    if (e.session_id) router.replace(`history/${e.session_id}`);
    else if (router.parts[1]) router.replace('history');
  }

  function clearSelection(): void {
    history.select(null);
    if (router.parts[1]) router.replace('history');
  }

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
      toasts.error('Rescan failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ── Actions ─────────────────────────────────────────────────────────────────
  let busy = $state(false);

  /** Resume in Otto: import (on_disk) → restart (exited/reconnectable) → open in Chat. */
  async function resume(e: HistoryEntry): Promise<void> {
    if (!wsId || busy) return;
    busy = true;
    try {
      let sid = e.session_id;
      if (e.status === 'on_disk' || !sid) {
        sid = (await history.importEntry(wsId, e)).id;
        await ws.refreshSessions();
      }
      if (e.status !== 'running' && e.status !== 'idle') {
        await ws.restartSession(sid);
        history.patchSession(sid, { status: 'running' });
      }
      openInChat(sid);
    } catch (err) {
      toasts.error('Could not resume', err instanceof Error ? err.message : String(err));
    } finally {
      busy = false;
    }
  }

  /** Open a live session in the Chat view (SessionView reads this key first). */
  function openInChat(sid: string): void {
    try {
      localStorage.setItem(winKey(`otto_session_view:${sid}`), 'chat');
    } catch {
      /* storage unavailable — SessionView falls back to its default */
    }
    ws.setViewMode('tabs');
    ws.navigateToSession(sid);
  }

  async function copyText(v: string, what = 'Copied'): Promise<void> {
    try {
      await navigator.clipboard.writeText(v);
      toasts.info(what, v);
    } catch {
      toasts.error('Could not copy', v);
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
      toasts.error('Could not archive', err instanceof Error ? err.message : String(err));
    } finally {
      busy = false;
    }
  }

  function resumeLabel(e: HistoryEntry): string {
    return e.status === 'running' || e.status === 'idle' ? 'Open in Otto' : 'Resume in Otto';
  }

  function menuFor(e: HistoryEntry): MenuItem[] {
    const live = e.status === 'running' || e.status === 'idle';
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
        danger: true,
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

  /** on_disk rows have no session → fold artifacts out of the transcript itself. */
  async function loadDiskArtifacts(e: HistoryEntry): Promise<void> {
    if (!wsId || diskArtifactsFor === e.transcript_path) return;
    diskArtifactsFor = e.transcript_path;
    try {
      const t = await api.get<Transcript>(
        `/workspaces/${wsId}/history/transcript?path=${encodeURIComponent(e.transcript_path)}&limit=500`,
      );
      const seen = new Map<string, Artifact>();
      for (const turn of t.turns)
        for (const b of turn.blocks) if (b.kind === 'artifact') seen.set(b.artifact.id, b.artifact);
      diskArtifacts = [...seen.values()];
    } catch {
      diskArtifacts = [];
    }
  }

  function toggleOutputs(): void {
    outputsOpen = !outputsOpen;
    if (outputsOpen && sel && sel.status === 'on_disk') void loadDiskArtifacts(sel);
  }

  // ── Display helpers ─────────────────────────────────────────────────────────
  let collapsed = $state<Record<string, boolean>>({});

  function relTime(iso: string): string {
    try {
      const secs = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 1000));
      if (secs < 60) return 'now';
      const mins = Math.floor(secs / 60);
      if (mins < 60) return `${mins}m`;
      const hrs = Math.floor(mins / 60);
      if (hrs < 24) return `${hrs}h`;
      const days = Math.floor(hrs / 24);
      if (days < 7) return `${days}d`;
      return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    } catch {
      return '';
    }
  }

  const STATUS_LABEL: Record<HistoryStatus, string> = {
    running: 'running',
    idle: 'idle',
    exited: 'exited',
    reconnectable: 'resumable',
    on_disk: 'on disk',
  };

  const PROVIDERS: { id: ProviderFilter; label: string }[] = [
    { id: 'all', label: 'All providers' },
    { id: 'claude', label: 'Claude' },
    { id: 'codex', label: 'Codex' },
  ];
  const STATUSES: { id: StatusFilter; label: string }[] = [
    { id: 'all', label: 'Any status' },
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
</script>

<div class="history" class:has-sel={!!sel} data-testid="history-page">
  <!-- ── Left: search, filters, grouped list ─────────────────────────────── -->
  <aside class="hlist">
    <div class="toolbar">
      <div class="search-wrap">
        <Icon name="search" size={12} />
        <input
          class="search"
          placeholder="Search titles and first prompts…"
          bind:value={history.q}
          oninput={onSearchInput}
          spellcheck="false"
          aria-label="Search history"
          data-testid="history-search"
        />
      </div>
      <button
        class="icon-btn"
        onclick={() => void rescan()}
        disabled={rescanning}
        title={rescanning ? 'Rescanning transcripts on disk…' : 'Rescan ~/.claude/projects and ~/.codex/sessions'}
        aria-label="Rescan transcripts"
        data-testid="history-rescan"
      >
        <Icon name="refresh" size={13} />
      </button>
    </div>
    <div class="filters">
      <select class="sel" bind:value={history.provider} aria-label="Provider">
        {#each PROVIDERS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
      </select>
      <select class="sel" bind:value={history.cwd} aria-label="Folder">
        <option value="">All folders</option>
        {#each history.folders as f (f.cwd)}<option value={f.cwd} title={f.cwd}>{f.label}</option>{/each}
      </select>
      <select class="sel" bind:value={history.status} aria-label="Status">
        {#each STATUSES as s (s.id)}<option value={s.id}>{s.label}</option>{/each}
      </select>
      <select class="sel" bind:value={history.date} aria-label="Date">
        {#each DATES as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
      </select>
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
        <p class="empty-line err">{history.error}</p>
      {:else if history.loading && history.entries.length === 0}
        <p class="empty-line dim">Loading…</p>
      {:else if shown === 0}
        <p class="empty-line dim">
          {history.entries.length === 0
            ? 'No conversations yet. Run claude or codex in this workspace, or rescan to pick up transcripts already on disk.'
            : 'Nothing matches these filters.'}
        </p>
      {:else}
        {#each history.groups as g (g.key)}
          <div class="group">
            <button class="group-head" onclick={() => (collapsed[g.key] = !collapsed[g.key])} title={g.cwd}>
              <Icon name={collapsed[g.key] ? 'chevronRight' : 'chevronDown'} size={10} />
              <Icon name="folder" size={12} />
              <span class="group-label">{g.label}</span>
              <span class="count">{g.entries.length}</span>
            </button>
            {#if !collapsed[g.key]}
              {#each g.entries as e (entryKey(e))}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="row"
                  class:on={selKey === entryKey(e)}
                  role="button"
                  tabindex="0"
                  onclick={() => pick(e)}
                  onkeydown={(k) => {
                    if (k.key === 'Enter' || k.key === ' ') {
                      k.preventDefault();
                      pick(e);
                    }
                  }}
                  oncontextmenu={(k) => ctxMenu.show(k, menuFor(e))}
                  data-testid="history-row"
                  data-status={e.status}
                  data-session-id={e.session_id}
                >
                  <span class="glyph {e.provider}" title={e.provider}>{e.provider === 'codex' ? '◇' : 'C'}</span>
                  <span class="row-body">
                    <span class="row-title">{entryTitle(e)}</span>
                    <span class="row-meta">
                      <span class="dot st-{e.status}" title={STATUS_LABEL[e.status]}></span>
                      <span class="mono">{relTime(e.last_active_at)}</span>
                      {#if e.turns != null}<span class="mono">· {e.turns} turns</span>{/if}
                      {#if e.status === 'on_disk'}<span class="on-disk">on disk</span>{/if}
                    </span>
                  </span>
                  <button
                    class="row-more"
                    onclick={(k) => ctxMenu.show(k, menuFor(e))}
                    onkeydown={(k) => {
                      if (k.key === 'Enter' || k.key === ' ') ctxMenu.show(k, menuFor(e));
                    }}
                    title="Actions"
                    aria-label="Actions"
                  >⋯</button>
                </div>
              {/each}
            {/if}
          </div>
        {/each}
        {#if history.hasMore}
          <button class="more" onclick={() => void history.loadMore()} disabled={history.loadingMore}>
            {history.loadingMore ? 'Loading…' : 'Load older'}
          </button>
        {/if}
      {/if}
    </div>
  </aside>

  <!-- ── Right: read-only conversation + outputs ─────────────────────────── -->
  <section class="hdetail">
    {#if !sel}
      <EmptyState
        icon="clock"
        title="Pick a conversation"
        body="Every Claude and Codex session in this workspace — and the transcripts already on disk — listed on the left. Select one to read it here; resume it to keep going."
      />
    {:else if wsId}
      <header class="dhead">
        <button class="back" onclick={clearSelection} title="Back to the list" aria-label="Back">
          <Icon name="chevronLeft" size={14} />
        </button>
        <div class="dtitle-wrap">
          <div class="dtitle" title={entryTitle(sel)}>{entryTitle(sel)}</div>
          <div class="dmeta">
            <span class="glyph {sel.provider}">{sel.provider === 'codex' ? '◇' : 'C'}</span>
            <span>{sel.provider}</span>
            <span class="dot st-{sel.status}"></span>
            <span>{STATUS_LABEL[sel.status]}</span>
            <span class="mono" title={sel.cwd}>· {sel.repo_name ?? shortCwd(sel.cwd)}</span>
            {#if sel.turns != null}<span class="mono">· {sel.turns} turns</span>{/if}
            <span class="mono">· {relTime(sel.last_active_at)}</span>
          </div>
        </div>
        <div class="dactions">
          {#if canEdit}
            <button
              class="act primary"
              onclick={() => sel && void resume(sel)}
              disabled={busy || (sel.status !== 'running' && sel.status !== 'idle' && !sel.resumable && sel.status !== 'on_disk')}
              data-testid="history-resume"
            >
              <Icon name="play" size={12} /> {resumeLabel(sel)}
            </button>
          {/if}
          <button class="act" onclick={() => sel && void openFolder(sel)} title="Reveal the working folder">
            <Icon name="folder" size={12} /> Open folder
          </button>
          <button class="act" onclick={() => sel && void copyText(sel.transcript_path)} title={sel.transcript_path}>
            <Icon name="copy" size={12} /> Copy path
          </button>
          {#if canEdit && sel.session_id && sel.status !== 'on_disk'}
            <button class="act danger" onclick={() => sel && void archive(sel)} disabled={busy}>
              <Icon name="archive" size={12} /> Archive
            </button>
          {/if}
        </div>
      </header>

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
          <Icon name={outputsOpen ? 'chevronDown' : 'chevronRight'} size={10} />
          <Icon name="layers" size={12} />
          <span>Outputs</span>
          <span class="dim small">
            {sel.status === 'on_disk' ? 'files, PRs and images this conversation produced' : 'artifacts of this session'}
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

<style>
  .history {
    display: flex;
    height: 100%;
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
    font-size: 10.5px;
    font-weight: 400;
  }
  .err {
    color: var(--status-exited, #e5534b);
  }
  .empty-line {
    font-size: 12px;
    line-height: 1.45;
    margin: 8px 12px;
  }

  /* ── list ──────────────────────────────────────────────────────────────── */
  .hlist {
    flex: 0 0 340px;
    min-width: 260px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--bg-sidebar, var(--bg));
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
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search-wrap:focus-within {
    border-color: var(--accent);
  }
  .search {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    outline: none;
  }
  .icon-btn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    color: var(--accent);
    border-color: var(--accent);
  }
  .icon-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .filters {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    padding: 4px 8px 8px;
    border-bottom: 1px solid var(--border);
  }
  .sel {
    min-width: 0;
    height: 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: 11px;
    padding: 0 4px;
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    font-size: 10.5px;
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
    transition: width 200ms ease-out;
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
    gap: 5px;
    padding: 6px 6px 3px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    text-align: start;
    cursor: pointer;
  }
  .group-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-transform: none;
    letter-spacing: 0;
    font-size: 11.5px;
  }
  .count {
    font-size: 10px;
    font-weight: 600;
    background: var(--surface-2);
    border-radius: 999px;
    padding: 1px 6px;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .row:hover {
    background: var(--surface-2);
  }
  .row.on {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .glyph {
    flex-shrink: 0;
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    font-size: 10px;
    font-weight: 700;
    line-height: 1;
    margin-top: 1px;
  }
  .glyph.claude {
    background: color-mix(in srgb, #d97757 22%, transparent);
    color: #d97757;
  }
  .glyph.codex {
    background: var(--surface-2);
    color: var(--text);
  }
  .row-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row-title {
    font-size: 12.5px;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-meta {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .on-disk {
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border: 1px dashed var(--border);
    border-radius: 999px;
    padding: 0 5px;
  }
  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .st-running {
    background: var(--status-working, #3fb950);
  }
  .st-idle {
    background: var(--status-idle, #8b949e);
  }
  .st-exited {
    background: var(--status-exited, #e5534b);
  }
  .st-reconnectable {
    background: var(--status-warn, #d29922);
  }
  .st-on_disk {
    background: transparent;
    border: 1px solid var(--text-dim);
    box-sizing: border-box;
  }
  .row-more {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
    opacity: 0;
  }
  .row:hover .row-more,
  .row.on .row-more,
  .row-more:focus-visible {
    opacity: 1;
  }
  .row-more:hover {
    background: var(--surface);
    color: var(--text);
  }
  .more {
    width: 100%;
    margin-top: 6px;
    height: 26px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
  }
  .more:hover:not(:disabled) {
    color: var(--accent);
    border-color: var(--accent);
  }

  /* ── detail ────────────────────────────────────────────────────────────── */
  .hdetail {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .dhead {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .back {
    display: none;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .dtitle-wrap {
    flex: 1;
    min-width: 0;
  }
  .dtitle {
    font-size: 13.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dmeta {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-top: 2px;
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    white-space: nowrap;
  }
  .dmeta .glyph {
    width: 14px;
    height: 14px;
    font-size: 8.5px;
    margin-top: 0;
  }
  .dactions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .act {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
    white-space: nowrap;
  }
  .act:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .act:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .act.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-contrast, #fff);
  }
  .act.danger:hover:not(:disabled) {
    color: var(--status-exited, #e5534b);
    border-color: var(--status-exited, #e5534b);
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
    font-size: 11.5px;
    font-weight: 600;
    text-align: start;
    cursor: pointer;
    flex-shrink: 0;
  }
  .doutputs-head:hover {
    background: var(--surface-2);
  }

  /* ── narrow: list OR detail (with a back button) ───────────────────────── */
  @media (max-width: 768px) {
    .hlist {
      flex: 1;
      border-inline-end: none;
    }
    .history.has-sel .hlist {
      display: none;
    }
    .history:not(.has-sel) .hdetail {
      display: none;
    }
    .back {
      display: inline-flex;
    }
    .dmeta .mono {
      display: none;
    }
  }
</style>
