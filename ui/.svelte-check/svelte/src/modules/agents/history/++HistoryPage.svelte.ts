///<reference types="svelte" />
;
import { copyText } from '../../../lib/clipboard';
import { rowMenu } from '../../../lib/rowMenu';
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
import { onDestroy, untrack } from 'svelte';
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
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
  let alive = true;
  let actionGeneration = 0;
  // A resumed process may finish starting after this History visit has ended.
  // Its completion must never replace a newer route or workspace decision.
  $effect(() => {
    void wsId; void scope; void router.parts; void router.pendingTarget;
    return () => { ++actionGeneration; };
  });
  onDestroy(() => {
    alive = false;
    ++actionGeneration;
    if (searchTimer) clearTimeout(searchTimer);
  });
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
    ++actionGeneration;
    history.select(e);
    rememberSelection('history', entryKey(e));
    if (e.session_id) router.replace(`history/${e.session_id}`);
    else if (router.parts[1]) router.replace('history');
  }

  function clearSelection(): void {
    ++actionGeneration;
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
      // Same predicate as the header's Resume button — never offer a resume
      // the button would refuse (archived / nothing resumable).
      ...(sel && canEdit && canResume(sel) ? [{ id: 'history.resume', title: resumeLabel(sel), detail: entryTitle(sel), group: 'History', keywords: 'continue resume conversation', run: () => void resume(sel) }] : []),
      ...(sel && canEdit && sel.archived && sel.session_id ? [{ id: 'history.unarchive', title: 'Unarchive conversation', detail: entryTitle(sel), group: 'History', keywords: 'restore archived resume', run: () => void unarchive(sel) }] : []),
    ]),
  );

  function isLiveStatus(status: HistoryStatus): boolean {
    return status === 'working' || status === 'running' || status === 'idle';
  }

  /** "Resume in Otto" is possible: live, resumable (an ended conversation
   *  with a provider id counts — the daemon resumes it), or an on-disk
   *  transcript to import. Never for an archived row: resume refuses it
   *  (409) — Unarchive first. */
  function canResume(e: HistoryEntry): boolean {
    if (e.archived) return false;
    return isLiveStatus(e.status) || e.resumable || e.status === 'on_disk';
  }

  /** Import if needed, safely ensure the session is live, then open in Chat. */
  async function resume(e: HistoryEntry): Promise<void> {
    if (!wsId || busy) return;
    busy = true;
    const generation = actionGeneration, workspace = wsId, originScope = scope, originHash = window.location.hash;
    // Accepted navigation can keep this page mounted while its destination
    // chunk loads. The hash also fences the interval before hashchange parses it.
    const current = () => alive && generation === actionGeneration && wsId === workspace && scope === originScope && window.location.hash === originHash && !router.pendingTarget;
    try {
      let sid = e.session_id;
      if (e.status === 'on_disk' || !sid) {
        sid = (await history.importEntry(workspace, e)).id;
        if (!current()) return;
        await ws.refreshSessions();
        if (!current()) return;
      }
      // The row may be stale in either direction. The server checks the live
      // PTY under its resume lock and never replaces an already-live process.
      const resumed = await ws.resumeSession(sid);
      if (!current()) return;
      history.patchSession(sid, { status: resumed.status });
      openInChat(sid);
    } catch (err) {
      if (current()) toastError('Couldn’t resume', err);
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

  async function copyNotify(v: string, what = 'Copied'): Promise<void> {
    if (await copyText(v)) toasts.info(what, v);
    else toasts.error('Couldn’t copy', `The clipboard isn’t available here. Select the text and copy it manually: ${v}`);
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
    await copyNotify(e.cwd, 'Path copied (no file-manager bridge here)');
  }

  async function archive(e: HistoryEntry): Promise<void> {
    if (!e.session_id || busy) return;
    busy = true;
    try {
      // Asks first when the agent is mid-turn (archive stops it).
      if (await ws.requestArchive(e.session_id)) history.patchSession(e.session_id, { status: 'exited', archived: true });
    } catch (err) {
      toastError('Couldn’t archive', err);
    } finally {
      busy = false;
    }
  }

  /** Archived rows come back here (History had no way out before). */
  async function unarchive(e: HistoryEntry): Promise<void> {
    if (!e.session_id || busy) return;
    busy = true;
    try {
      await ws.unarchiveSession(e.session_id);
      history.patchSession(e.session_id, { archived: false });
      toasts.success('Session unarchived', 'You can resume it now.');
    } catch (err) {
      toastError('Couldn’t unarchive', err);
    } finally {
      busy = false;
    }
  }

  function resumeLabel(e: HistoryEntry): string {
    return isLiveStatus(e.status) ? 'Open in Otto' : 'Resume in Otto';
  }

  function menuFor(e: HistoryEntry): MenuItem[] {
    return [
      e.archived
        ? { label: 'Unarchive', icon: 'archive', disabled: !canEdit || !e.session_id, action: () => void unarchive(e) }
        : { label: resumeLabel(e), icon: 'play', disabled: !canEdit || !canResume(e), action: () => void resume(e) },
      { label: 'Open folder', icon: 'folder', action: () => void openFolder(e) },
      { label: 'Copy transcript path', icon: 'copy', action: () => void copyNotify(e.transcript_path) },
      { label: 'Copy folder path', icon: 'copy', action: () => void copyNotify(e.cwd) },
      { separator: true },
      {
        label: 'Archive',
        icon: 'archive',
        disabled: !canEdit || !e.session_id || e.status === 'on_disk' || !!e.archived,
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
;
async () => {

 { svelteHTML.createElement("div", { "class":`history-page`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":headSel ? entryTitle(headSel) : 'History',"subtitle":headSel
    ? `${headSel.repo_name ?? shortCwd(headSel.cwd)}${headSel.turns != null ? ` · ${headSel.turns} ${headSel.turns === 1 ? 'turn' : 'turns'}` : ''} · ${rel(headSel.last_active_at)}`
    : 'Past Claude and Codex conversations: Otto sessions and transcripts on disk',leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(headSel){
       { svelteHTML.createElement("button", {       "class":`icon-btn back`,"onclick":clearSelection,"title":`Back to the list`,"aria-label":`Back to the list`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
       }
    }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(headSel){
       { svelteHTML.createElement("span", { "class":`dmeta`,});
         { svelteHTML.createElement("span", { "class":`glyph ${headSel.provider}`,}); { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":headSel.provider,"size":14,}});} }
         { svelteHTML.createElement("span", { "class":`cap`,});headSel.provider; }
         { svelteHTML.createElement("span", {   "class":`dot st-${headSel.status}`,"aria-hidden":`true`,}); }
         { svelteHTML.createElement("span", {});STATUS_LABEL[headSel.status]; }
       }
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    
    if(!isEmpty){
       { svelteHTML.createElement("button", {                 "class":`btn small`,"onclick":() => void rescan(),"disabled":rescanning,"title":rescanning ? 'Rescanning transcripts on disk…' : 'Rescan ~/.claude/projects and ~/.codex/sessions for new transcripts',"aria-label":`Rescan transcripts`,...__sveltets_2_empty({"data-icon":`refresh`}),...__sveltets_2_empty({"data-overflow":`-5`}),...__sveltets_2_empty({"data-testid":`history-rescan`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} rescanning ? 'Rescanning…' : 'Rescan';
       }
    }
    if(headSel){
      const cur = headSel;
      if(canEdit && cur.session_id && cur.status !== 'on_disk' && !cur.archived){
         { svelteHTML.createElement("button", {               "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-3`}),...__sveltets_2_empty({"data-icon":`archive`}),...__sveltets_2_empty({"data-label":`Archive session`}),"onclick":() => void archive(cur),"disabled":busy,"aria-label":`Archive`,"title":`Archive the session — restore it any time from the sidebar’s Archived list`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":14,}});}
         }
      }
       { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`-2`}),...__sveltets_2_empty({"data-icon":`copy`}),...__sveltets_2_empty({"data-label":`Copy transcript path`}),"onclick":() => void copyNotify(cur.transcript_path, 'Transcript path copied'),"title":cur.transcript_path,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  
       }
       { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`folder`}),...__sveltets_2_empty({"data-label":`Open folder`}),"onclick":() => void openFolder(cur),"title":`Reveal ${cur.cwd}`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}  
       }
      
      if(canEdit && cur.archived && cur.session_id){
         { svelteHTML.createElement("button", {               "class":`btn small primary`,"onclick":() => void unarchive(cur),"disabled":busy,"title":`Archived — unarchive it to resume this conversation`,...__sveltets_2_empty({"data-icon":`archive`}),...__sveltets_2_empty({"data-label":`Unarchive`}),...__sveltets_2_empty({"data-testid":`history-unarchive`}),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":12,}});} 
         }
      } else if (canEdit){
        const resumable = canResume(cur);
         { svelteHTML.createElement("button", {               "class":`btn small primary`,"onclick":() => void resume(cur),"disabled":busy || !resumable,"title":resumable ? (cur.status === 'on_disk' ? 'Import this transcript as an Otto session and continue it' : 'Continue this conversation in Otto') : 'This conversation can’t be resumed — the CLI never recorded a session id for it',...__sveltets_2_empty({"data-icon":`play`}),...__sveltets_2_empty({"data-label":resumeLabel(cur)}),...__sveltets_2_empty({"data-testid":`history-resume`}),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} resumeLabel(cur);
         }
      }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
 { svelteHTML.createElement("div", {     "class":`history`,...__sveltets_2_empty({"data-testid":`history-page`}),});!!sel;hideList;
  
  if(!hideList){
   { svelteHTML.createElement("aside", {     "class":`hlist`,"aria-label":`Conversations`,"style":`--list-pane-w:${listW}px`,});
    if(ws.currentId){
       { svelteHTML.createElement("div", {     "class":`segmented scope-seg`,"role":`group`,"aria-label":`Which conversations`,});
         { svelteHTML.createElement("button", {       "aria-pressed":scope === 'workspace',"onclick":() => (scope = 'workspace'),"title":`Conversations in the current workspace`,});scope === 'workspace';  }
         { svelteHTML.createElement("button", {       "aria-pressed":scope === 'scratch',"onclick":() => (scope = 'scratch'),"title":`Conversations started outside any workspace`,});scope === 'scratch';  }
       }
    }
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("div", { "class":`search-wrap`,});
         { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
         { const $$_input6 = svelteHTML.createElement("input", {                 "dir":`auto`,"class":`search`,"placeholder":`Search titles and first prompts…`,"bind:value":history.q,"oninput":onSearchInput,"spellcheck":`false`,"aria-label":`Search history`,...__sveltets_2_empty({"data-testid":`history-search`}),});searchEl = $$_input6;/*Ωignore_startΩ*/() => history.q = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
     }
     { svelteHTML.createElement("div", {     "class":`filters`,"role":`group`,"aria-label":`Filters`,});
       { svelteHTML.createElement("select", {      "class":`sel`,"bind:value":history.provider,"aria-label":`Provider`,});history.provider !== 'all';/*Ωignore_startΩ*/() => history.provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(PROVIDERS)){p.id; { svelteHTML.createElement("option", { "value":p.id,});p.label; }}
       }
       { svelteHTML.createElement("select", {      "class":`sel`,"bind:value":history.status,"aria-label":`Status`,});history.status !== 'all';/*Ωignore_startΩ*/() => history.status = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let st of __sveltets_2_ensureArray(STATUSES)){st.id; { svelteHTML.createElement("option", { "value":st.id,});st.label; }}
       }
       { svelteHTML.createElement("select", {      "class":`sel`,"bind:value":history.date,"aria-label":`Date`,});history.date !== 'all';/*Ωignore_startΩ*/() => history.date = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let d of __sveltets_2_ensureArray(DATES)){d.id; { svelteHTML.createElement("option", { "value":d.id,});d.label; }}
       }
       { svelteHTML.createElement("select", {        "class":`sel folder`,"bind:value":history.cwd,"aria-label":`Folder`,"title":history.cwd || 'All folders',});!!history.cwd;/*Ωignore_startΩ*/() => history.cwd = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let f of __sveltets_2_ensureArray(history.folders)){f.cwd; { svelteHTML.createElement("option", {   "value":f.cwd,"title":f.cwd,});f.label; }}
       }
      if(filtersActive){
         { svelteHTML.createElement("button", {     "class":`btn small ghost clear`,"onclick":clearFilters,"title":`Show every conversation again`,});  }
      }
     }
    if(rescanning || (idx && !idx.done)){
       { svelteHTML.createElement("div", {   "class":`progress`,...__sveltets_2_empty({"data-testid":`history-progress`}),});
         { svelteHTML.createElement("span", { "class":`dim`,});  }
         { svelteHTML.createElement("span", { "class":`mono dim`,});idx ? `${idx.scanned}/${idx.total || '?'}` : '…'; }
         { svelteHTML.createElement("span", {   "class":`ptrack`,"aria-hidden":`true`,});
           { svelteHTML.createElement("span", {   "class":`pfill`,"style":`width:${idx && idx.total ? Math.round((idx.scanned / idx.total) * 100) : 0}%`,}); }
         }
       }
    }

     { svelteHTML.createElement("div", {   "class":`rows`,...__sveltets_2_empty({"data-testid":`history-list`}),});
      if(history.error){
         { svelteHTML.createElement("p", {   "class":`empty-line err`,"role":`alert`,});
           { svelteHTML.createElement("span", {});  history.error; }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void history.refresh(),});  }
         }
      }
      if(history.loading && history.entries.length === 0){
         { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {      "rows":5,"height":40,"label":`conversations`,}});}
      } else if (shown === 0 && !history.error){
        
         { svelteHTML.createElement("p", { "class":`empty-line dim narrow-only`,});
          history.hasMore ? 'No matches in the loaded part of history yet.' : 'Nothing matches these filters.';
          if(filtersActive && !history.hasMore){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":clearFilters,});  }
          }
         }
      }else{
           for(let g of __sveltets_2_ensureArray(history.groups)){g.key;
           { svelteHTML.createElement("div", { "class":`group`,});
             { svelteHTML.createElement("button", {       "class":`group-head`,"onclick":() => (collapsed[g.key] = !collapsed[g.key]),"title":g.cwd,"aria-expanded":!collapsed[g.key],});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":collapsed[g.key] ? 'chevronRight' : 'chevronDown',"size":12,}});}
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
               { svelteHTML.createElement("span", { "class":`group-label`,});g.label; }
               { svelteHTML.createElement("span", { "class":`count`,});g.entries.length; }
             }
            if(!collapsed[g.key]){
                 for(let e of __sveltets_2_ensureArray(g.entries)){entryKey(e);
                 { svelteHTML.createElement("div", {          "class":`hist-row`,...__sveltets_2_empty({"data-testid":`history-row`}),...__sveltets_2_empty({"data-status":e.status}),...__sveltets_2_empty({"data-session-id":e.session_id}),});selKey === entryKey(e);
                   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {          "class":`row-main`,"aria-current":selKey === entryKey(e) ? 'true' : undefined,"onclick":() => pick(e),"oncontextmenu":(k) => ctxMenu.show(k, menuFor(e)),});
                     { svelteHTML.createElement("span", {   "class":`glyph ${e.provider}`,"title":e.provider,}); { const $$_nocIredivorP9C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP9C({ target: __sveltets_2_any(), props: {    "provider":e.provider,"size":18,}});} }
                     { svelteHTML.createElement("span", { "class":`row-body`,});
                       { svelteHTML.createElement("span", {   "class":`row-title`,"title":entryTitle(e),});entryTitle(e); }
                       { svelteHTML.createElement("span", { "class":`row-meta`,});
                         { svelteHTML.createElement("span", {   "class":`dot st-${e.status}`,"aria-hidden":`true`,}); }
                         { svelteHTML.createElement("span", { "class":`sr-only`,});STATUS_LABEL[e.status];  }
                         { svelteHTML.createElement("span", { "title":new Date(e.last_active_at).toLocaleString(),});rel(e.last_active_at); }
                        if(e.turns != null){ { svelteHTML.createElement("span", {}); e.turns; e.turns === 1 ? 'turn' : 'turns'; }}
                        if(e.status === 'on_disk'){ { svelteHTML.createElement("span", {   "class":`on-disk`,"title":`A transcript on disk that no Otto session owns`,});  }}
                        if(e.archived){ { svelteHTML.createElement("span", {   "class":`on-disk`,"title":`Archived — unarchive it to resume`,});  }}
                       }
                     }
                   }}
                   { svelteHTML.createElement("button", {         "class":`icon-btn row-more`,"onclick":(k) => ctxMenu.showAt(k.currentTarget as HTMLElement, menuFor(e)),"title":`More actions for ${entryTitle(e)}`,"aria-label":`More actions for ${entryTitle(e)}`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
                 }
              }
            }
           }
        }
      }
      if(history.hasMore && !history.loading){
         { svelteHTML.createElement("button", {     "class":`btn small more`,"onclick":() => void history.loadMore(),"disabled":history.loadingMore,});
          history.loadingMore ? 'Loading older conversations…' : 'Load older conversations';
         }
      }
     }
   }
   { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {         width:listW,"storageKey":`history.listW`,"label":`Resize the conversations list`,"max":LIST_PANE.max,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
  }

  
   { svelteHTML.createElement("section", { "class":`hdetail`,});
    if(nothingToList || (!sel && history.error)){
       { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL4 = new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {             "variant":`page`,"what":`history`,"loading":history.loading,"error":history.error,"empty":true,"onretry":() => void history.refresh(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`clock`,"title":`No conversations yet`,"body":rescanning
              ? `Indexing transcripts on disk… ${idx ? `${idx.scanned}/${idx.total || '?'}` : ''}`
              : scope === 'scratch'
                ? 'Conversations started outside a workspace show up here. Rescan to pick up transcripts already on disk.'
                : 'Run claude or codex in this workspace, or rescan to pick up transcripts already on disk.',"actionLabel":rescanning ? 'Rescanning…' : 'Rescan transcripts',"actionIcon":`refresh`,"onaction":() => void rescan(),}});}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL4.$$prop_def;/*Ωignore_endΩ*/
        
       LoadState}
    } else if (!sel && shown === 0){
      if(history.hasMore){
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`search`,"title":`No matches yet`,"body":`No matches in this part of history. Load more to keep looking.`,"actionLabel":history.loadingMore ? 'Loading more history…' : 'Load more',"onaction":() => void history.loadMore(),}});}
      }else{
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`filter`,"title":`No matching conversations`,"body":`Nothing matches these filters. Clear them to see every conversation.`,"actionLabel":`Clear filters`,"onaction":clearFilters,}});}
      }
    } else if (!sel){
      
      if(history.loading){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "variant":`page`,"what":`conversations`,"loading":true,"empty":true,}});}
      }else{
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`clock`,"title":`${history.entries.length}${history.hasMore ? '+' : ''} ${history.entries.length === 1 ? 'conversation' : 'conversations'} in ${history.groups.length} ${history.groups.length === 1 ? 'folder' : 'folders'}`,"body":`Pick one on the left to read it here; resume it to keep going.`,}});}
      }
    } else if (wsId){
      
       { svelteHTML.createElement("p", {   "class":`dfull`,"aria-hidden":`true`,});entryTitle(sel); }
       { svelteHTML.createElement("div", {   "class":`dconv`,...__sveltets_2_empty({"data-testid":`history-conversation`}),});
        selKey; {
          if(sel.status === 'on_disk' || !sel.session_id){
              { const $$_weiVnoitasrevnoC5C = __sveltets_2_ensureComponent(ConversationView); new $$_weiVnoitasrevnoC5C({ target: __sveltets_2_any(), props: {    "transcriptPath":sel.transcript_path,"workspaceId":wsId,"readonly":true,}});}
          }else{
              { const $$_weiVnoitasrevnoC5C = __sveltets_2_ensureComponent(ConversationView); new $$_weiVnoitasrevnoC5C({ target: __sveltets_2_any(), props: {    "sessionId":sel.session_id,"workspaceId":wsId,"readonly":true,}});}
          }
        }
       }

       { svelteHTML.createElement("div", {  "class":`doutputs`,});outputsOpen;
         { svelteHTML.createElement("button", {     "class":`doutputs-head`,"onclick":toggleOutputs,"aria-expanded":outputsOpen,});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":outputsOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`layers`,"size":12,}});}
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", { "class":`dim small`,});
            sel.status === 'on_disk' ? 'Files, PRs and images this conversation produced' : 'Artifacts of this session';
           }
         }
        if(outputsOpen){
          if(sel.status === 'on_disk' || !sel.session_id){
            if(diskArtifacts === null){
               { svelteHTML.createElement("p", { "class":`empty-line dim`,});   }
            }else{
                { const $$_lenaPstuptuO5C = __sveltets_2_ensureComponent(OutputsPanel); new $$_lenaPstuptuO5C({ target: __sveltets_2_any(), props: {  "artifacts":diskArtifacts,"embedded":true,}});}
            }
          }else{
              { const $$_lenaPstuptuO5C = __sveltets_2_ensureComponent(OutputsPanel); new $$_lenaPstuptuO5C({ target: __sveltets_2_any(), props: {  "sessionId":sel.session_id,"embedded":true,}});}
          }
        }
       }
    }
   }
 }
 PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HistoryPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HistoryPage__SvelteComponent_ = ReturnType<typeof HistoryPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default HistoryPage__SvelteComponent_;