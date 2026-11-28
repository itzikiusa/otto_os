<script lang="ts">
  // The Agents inbox (patterns.md §10 — the Threads concept, where the owner
  // works): terminal sessions and structured threads in one inbox, grouped by
  // workspace by default in All workspaces. Closed sessions stay in History.
  // A ListPane (design.md §3): "Sessions N · New session · ⋯"; one search row
  // (filter · Select · Show / sort), the workspace switcher row, the folds;
  // the usage meter is the footer. Rows are InboxRow (↑ ↓ through listNav). It replaced the session tree that
  // hung under "Agents" in the app sidebar; everything that tree did is here
  // (scope by workspace, multi-select → Tile / Archive / Delete, drag to
  // reorder in Manual, the row menu, rename, delete with undo).
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import InboxFold from '../../lib/components/InboxFold.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import ProjectBadge from '../../lib/components/ProjectBadge.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { lazyComponent, whenIdle } from '../../lib/lazy-component.svelte';
  import { popoutItems } from '../../lib/popoutMenu';
  import { rowMenu as longPress } from '../../lib/rowMenu';
  import { router } from '../../lib/router.svelte';
  import { events } from '../../lib/events.svelte';
  import { sessionState } from '../../lib/status';
  import { inboxClass, sessionWord } from '../../lib/statusWords';
  import { isForeground } from '../../lib/stores/sessionBuckets';
  import { auth } from '../../lib/stores/auth.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { sessionOrder, applyOrder, applyTokenOrder } from '../../lib/stores/sessionOrder.svelte';
  import { sessionUsage } from '../../lib/stores/sessionUsage.svelte';
  import { pollWhileVisible } from '../../lib/poll';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
  import { lsGet, lsSet } from '../../lib/storage';
  import type { Session } from '../../lib/api/types';
  import { ShareModalChunk } from '../../shell/overlays.svelte';
  import { RemovedWorkspacesChunk } from '../../shell/lazyDialogs';
  import { closeConfirm } from '../../shell/closeConfirm';
  import { checkOpenTabs } from '../../shell/openTabsCheck';
  import { LS_WS_RECENT, parseRecent, switcherItems, workspaceActivity } from '../../shell/workspaceSwitch';
  import { newScratchSession, newSessionIn, workspaceMenuItems } from '../../shell/workspaceActions';
  import AgentsInboxRow from './AgentsInboxRow.svelte';
  import {
    byRecent, groupAgentSessions, matchesQuery, sessionListStatus,
    SESSION_GROUPING_LABELS, SESSION_LIST_STATUS_LABELS, type InboxGroup, type SessionGrouping,
  } from './agentsInbox';
  import { startThreadFeeds } from './threadFeeds.svelte';

  // The inbox lists the same structured threads as Threads (spec A: one set of sessions, two views).
  type RuntimeStore = typeof import('../threads/runtime/runtimeThreads.svelte').runtimeThreads; // ui-guards: allow — one store
  // …and read their state the Threads way (threadNav `threadClass`: one needs-you derivation).
  type ThreadNav = typeof import('../threads/threadNav.svelte'); // ui-guards: allow — one derivation
  type InboxLive = typeof import('../threads/threadInbox.svelte').inboxLive; // ui-guards: allow — one store
  // The Threads footer's meter (the focused session's agent): after first paint.
  const UsageMeter = lazyComponent(() => import('../threads/ThreadUsageMeter.svelte')); // ui-guards: allow — the Threads footer meter, one implementation

  let query = $state('');
  let historyOpen = $state(false);
  let telegramOpen = $state(false);
  let slackOpen = $state(false);
  let selectMode = $state(false);
  let sel = $state<Set<string>>(new Set());
  let shareId = $state<string | null>(null);
  let removedOpen = $state(false);
  let archivedError = $state<string | null>(null);

  // Structured threads come from the Threads runtime store — its own chunk,
  // loaded after first paint so the terminal is never behind it.
  let rt = $state<RuntimeStore | null>(null);
  let nav = $state<ThreadNav | null>(null);
  let live = $state<InboxLive | null>(null);
  let meterReady = $state(false);
  onMount(() => {
    let gone = false;
    const cancel = whenIdle(() => {
      meterReady = true;
      void startThreadFeeds().then(([m, n, i]) => {
        if (gone) return;
        rt = m.runtimeThreads;
        nav = n;
        live = i.inboxLive;
      }, () => {}); // a failed chunk load is retried by the next visit (threadFeeds)
    });
    return () => {
      gone = true;
      cancel();
    };
  });

  // Grouping and folded workspaces belong to the signed-in person, across
  // workspace switches and sidebar remounts.
  interface GroupPreferences {
    groupBy: SessionGrouping;
    collapsed: string[];
  }
  const preferencesKey = $derived(`otto_agents_inbox:${auth.me?.id ?? 'anonymous'}`);
  let preferencesEdit = $state.raw<{ key: string; value: GroupPreferences } | null>(null);
  const preferences = $derived.by((): GroupPreferences => {
    if (preferencesEdit?.key === preferencesKey) return preferencesEdit.value;
    try {
      const saved = JSON.parse(lsGet(preferencesKey) ?? 'null') as Partial<GroupPreferences> | null;
      return {
        groupBy: saved?.groupBy === 'status' || saved?.groupBy === 'none' ? saved.groupBy : 'workspace',
        collapsed: Array.isArray(saved?.collapsed) ? saved.collapsed.filter((id) => typeof id === 'string') : [],
      };
    } catch {
      return { groupBy: 'workspace', collapsed: [] };
    }
  });
  function setPreferences(value: GroupPreferences): void {
    preferencesEdit = { key: preferencesKey, value };
    lsSet(preferencesKey, JSON.stringify(value));
  }
  const collapsed = $derived(new Set(preferences.collapsed));
  function isFiltering(): boolean {
    return query.trim() !== '' || ws.needsYouFilter;
  }
  function isCollapsed(group: InboxGroup): boolean {
    return group.kind === 'workspace' && collapsed.has(group.key) && !isFiltering();
  }
  function toggleWorkspace(id: string): void {
    const next = new Set(collapsed);
    if (!next.delete(id)) next.add(id);
    setPreferences({ ...preferences, collapsed: [...next] });
  }
  const multiWorkspace = $derived(ws.multiWorkspaceScope);
  const grouping = $derived(multiWorkspace ? preferences.groupBy : 'single');
  const grouped = $derived(grouping === 'workspace' || grouping === 'status');

  // ── Single-workspace sort (Recent · Manual · Tokens) ────────────────────
  $effect(() => sessionOrder.load(ws.currentId ?? SCRATCH_WORKSPACE_ID));
  const canUsage = $derived(auth.can('usage', 'view'));
  const usageWs = $derived(ws.currentId ?? SCRATCH_WORKSPACE_ID);
  const manualSort = $derived(!multiWorkspace && sessionOrder.mode === 'manual');
  const tokensSort = $derived(!multiWorkspace && sessionOrder.mode === 'tokens' && canUsage && sessionUsage.available);
  // The 30-day rollup feeds the Tokens sort only; polled while it is on.
  $effect(() => {
    if (!tokensSort) return;
    const wid = usageWs;
    const p = pollWhileVisible(() => sessionUsage.loadWorkspace(wid), { ms: 60_000 });
    return () => p.stop();
  });

  const stale = $derived(events.state !== 'connected');
  const workspaceNames = $derived(new Map(ws.workspaces.map((w) => [w.id, w.name])));
  const wsName = (id: string | null | undefined): string =>
    id === SCRATCH_WORKSPACE_ID || !id ? 'No workspace' : (workspaceNames.get(id) ?? 'Workspace');
  const isThread = (s: Session): boolean => !!rt && !!rt.get(s.id);

  const ownPtyRows = $derived(
    manualSort ? applyOrder(ws.plainAgentSessions, sessionOrder.order)
      : tokensSort ? applyTokenOrder(ws.plainAgentSessions, sessionUsage.tokensFor(usageWs))
        : ws.plainAgentSessions,
  );
  // The current workspace's terminals, scratch, and the other workspaces'
  // foreground terminals when All workspaces is on.
  const ptyRows = $derived([
    ...ownPtyRows, ...ws.scratchSessions, ...(multiWorkspace ? ws.otherAgentSessions : []),
  ]);
  const scope = (s: Session): boolean =>
    multiWorkspace || s.workspace_id === ws.currentId || s.workspace_id === SCRATCH_WORKSPACE_ID;
  const threadRows = $derived(rt ? rt.rows().filter((s) => isForeground(s) && scope(s)) : []);

  function stateOf(s: Session, forDisplay = true) {
    return nav && isThread(s)
      ? (forDisplay ? nav.threadRowState(s) : nav.threadLastKnownState(s))
      : sessionState(s, ws.statusMap[s.id] ?? s.status, ws.needsYou[s.id] === true, { stale: forDisplay && stale });
  }
  const unread = (s: Session): boolean => ws.unread[s.id] === true && ws.activeSessionId !== s.id;
  const classOf = (s: Session, state = stateOf(s, false)) =>
    nav && isThread(s) ? nav.threadClass(s, unread(s), state) : inboxClass(state.key, unread(s));
  /** A thread's pending requests' kinds (Approval vs Question in its word). */
  const kindsOf = (s: Session): string[] => (live && isThread(s) ? live.kindsOf(s.id) : []);
  const branchOf = (s: Session): string | null => {
    const m = s.meta as Record<string, unknown> | null | undefined;
    const b = m?.git_branch ?? m?.branch ?? m?.worktree_branch;
    if (typeof b === 'string' && b.trim()) return b.trim();
    const repo = s.cwd ? git.repos.find((r) => s.cwd === r.path || s.cwd!.startsWith(r.path.replace(/\/+$/, '') + '/')) : null;
    return repo ? (git.statusById[repo.id]?.branch ?? null) : null;
  };
  // The branch / workspace lookups run only while a query asks (not per row on every change).
  const matches = (s: Session): boolean =>
    (!ws.needsYouFilter || classOf(s) === 'needs') && (!query.trim() || matchesQuery(s, query, [branchOf(s), wsName(s.workspace_id)]));

  // Use last-known state for ordering. Reconnecting changes the row's display
  // hint, not its position or group.
  const inbox = $derived(groupAgentSessions(
    [...ptyRows, ...threadRows].filter(matches),
    (s) => {
      const state = stateOf(s, false);
      return sessionListStatus(state.key, classOf(s, state));
    },
    grouping,
    ws.currentId,
    manualSort || tokensSort,
  ));
  const historyRows = $derived(
    [...ws.archivedAgentSessions, ...(rt ? rt.rows(true).filter((s) => isForeground(s) && scope(s)) : [])].filter(matches),
  );
  const telegram = $derived(ws.telegramSessions.filter(matches));
  const slack = $derived(ws.slackSessions.filter(matches));

  // ── Select mode: Tile · Archive · Delete for the checked rows ────────────
  // This workspace's rows (and "No workspace"), as the old list's select mode.
  const own = (s: Session): boolean => s.workspace_id === ws.currentId || s.workspace_id === SCRATCH_WORKSPACE_ID;
  const selectable = $derived(
    [...inbox.flatMap((group) => isCollapsed(group) ? [] : group.sessions), ...(historyOpen ? historyRows : [])]
      .filter((s) => !isThread(s) && own(s) && ws.canEditSession(s)),
  );
  const selIds = $derived(selectable.filter((s) => sel.has(s.id)).map((s) => s.id));
  const liveSel = $derived(selIds.filter((id) => !ws.archivedAgentSessions.some((a) => a.id === id)));
  function toggleSel(id: string): void {
    const next = new Set(sel);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    sel = next;
  }
  function setSelectMode(on: boolean): void {
    selectMode = on;
    if (!on) sel = new Set();
  }
  function askClose(ids: string[], permanent: boolean): Promise<boolean> {
    const titles = ids.map((id) => selectable.find((s) => s.id === id)?.title ?? id);
    const working = ids.filter((id) => ws.isAgentMidTurn(id)).length;
    const { message, ...opts } = closeConfirm({ titles, working, permanent });
    return confirmer.ask(message, opts);
  }
  async function archiveSelected(): Promise<void> {
    const ids = liveSel;
    if (ids.length === 0) return;
    if (ids.some((id) => ws.isAgentMidTurn(id)) && !(await askClose(ids, false))) return;
    await ws.archiveSessions(ids);
    checkOpenTabs(true);
    setSelectMode(false);
  }
  async function deleteSelected(): Promise<void> {
    const ids = selIds;
    if (ids.length === 0 || !(await askClose(ids, true))) return;
    await ws.killSessions(ids);
    checkOpenTabs(true);
    setSelectMode(false);
  }
  function tileSelected(): void {
    if (liveSel.length === 0) return;
    ws.tileSessions(liveSel);
    setSelectMode(false);
  }

  // ── Manual order: drag rows (pointer only; the menu has "Move to top"). The
  // first drag implies Manual, seeded from the order as shown. ──────────────
  const dragOn = $derived(!multiWorkspace && !query.trim() && !selectMode && !ws.needsYouFilter && !viewport.isPhone);
  const shownOwn = (): string[] =>
    (manualSort || tokensSort ? ownPtyRows : byRecent(ownPtyRows)).map((x) => x.id);
  let dragId = $state<string | null>(null);
  let dragOver = $state<string | null>(null);
  const ownIds = $derived(new Set(ws.plainAgentSessions.map((s) => s.id)));
  function dragFor(s: Session) {
    if (!dragOn || !ownIds.has(s.id)) return null;
    return {
      over: dragOver === s.id,
      onstart: (e: DragEvent) => {
        dragId = s.id;
        e.dataTransfer?.setData('text/plain', s.id);
        if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
      },
      onover: (e: DragEvent) => {
        if (!dragId || dragId === s.id) return;
        e.preventDefault();
        dragOver = s.id;
      },
      onleave: () => {
        if (dragOver === s.id) dragOver = null;
      },
      ondrop: (e: DragEvent) => {
        e.preventDefault();
        if (dragId && dragId !== s.id) sessionOrder.dragTo(shownOwn(), dragId, s.id);
        dragId = dragOver = null;
      },
      onend: () => (dragId = dragOver = null),
    };
  }

  // ── Row actions ────────────────────────────────────────────────────────
  function open(e: MouseEvent, s: Session): void {
    if (selectMode && !isThread(s)) return toggleSel(s.id);
    // A thread opens (or focuses) as a chat pane here; ⌘-click takes it to Threads.
    if (isThread(s) && (e.metaKey || e.ctrlKey)) return router.go(`threads/${s.id}`);
    if (isThread(s)) {
      ws.openSession(s.id, 'thread');
      ws.navigateToSession(s.id);
    } else if (s.archived) void ws.unarchiveAndOpen(s.id).catch((err) => toastError('Couldn’t reopen the session', err));
    else if (s.workspace_id !== ws.currentId && s.workspace_id !== SCRATCH_WORKSPACE_ID) void ws.openInWorkspace(s.workspace_id, s.id);
    else if (e.metaKey || e.ctrlKey) ws.openInSplit(s.id);
    else ws.navigateToSession(s.id);
  }
  async function rename(s: Session): Promise<void> {
    if (isThread(s) || !ws.canEditSession(s)) return;
    const next = await confirmer.promptText('Session name', { title: 'Rename session', initial: s.title, confirmLabel: 'Rename' });
    if (next?.trim() && next.trim() !== s.title) await ws.renameSession(s.id, next.trim()).catch((e) => toastError('Couldn’t rename the session', e));
  }
  function rowMenu(e: MouseEvent, s: Session): void {
    e.preventDefault();
    if (isThread(s)) {
      // Here first (open beside the terminals), then Threads; the rest is the thread's own menu.
      const here: MenuItem[] = [
        { label: 'Open beside', icon: 'split', action: () => void ws.openInSplit(s.id, 'thread') },
        { label: 'Open in Threads', icon: 'threads', action: () => router.go(`threads/${s.id}`) },
        { separator: true },
      ];
      // Threads' Active / History tidy changes nothing in this list: Close thread is the way to History here.
      const drop = new Set(['Open beside', 'Move to History', 'Move back to Active']);
      void import('../threads/runtime/runtimeMenu').then((m) => ctxMenu.show(e, [...here, ...m.runtimeThreadMenu(s.id).filter((i) => !drop.has(i.label ?? ''))])); // ui-guards: allow — a thread's own menu
      return;
    }
    const edit = ws.canEditSession(s);
    const other = s.workspace_id !== ws.currentId && s.workspace_id !== SCRATCH_WORKSPACE_ID;
    const status = ws.statusMap[s.id] ?? s.status;
    const items: MenuItem[] = s.archived
      ? edit
        ? [
            { label: 'Unarchive', icon: 'refresh', action: () => void ws.unarchiveSession(s.id) },
            { label: 'Delete…', icon: 'trash', danger: true, action: () => void ws.requestDeleteSession(s.id) },
          ]
        : []
      : [
          // Session · Share · Danger — the tab and pane menus' order.
          ...(edit ? [{ label: 'Rename…', icon: 'edit' as const, action: () => void rename(s) }] : []),
          ...(other ? [] : [{ label: 'Open beside', icon: 'split' as const, action: () => void ws.openInSplit(s.id) }]),
          ...(manualSort && ownIds.has(s.id)
            ? [{ label: 'Move to top', icon: 'arrowUp' as const, action: () => { const ids = shownOwn(); sessionOrder.dragTo(ids, s.id, ids[0]); } }]
            : []),
          { separator: true },
          ...(other ? [] : [{ label: 'Share…', icon: 'share' as const, action: () => (shareId = s.id) }]),
          ...(other ? [] : popoutItems(`agents/${s.id}`, s.title)),
          ...(edit
            ? [
                { separator: true },
                ...(s.kind === 'agent' && (status === 'running' || status === 'working')
                  ? [{ label: 'Restart session', icon: 'refresh' as const, action: () => void ws.requestRestart(s.id) }]
                  : []),
                { label: 'Archive', icon: 'archive' as const, action: () => void ws.requestArchive(s.id).catch((err: unknown) => toastError('Couldn’t archive the session', err)) },
                { label: 'Delete…', icon: 'trash' as const, danger: true as const, action: () => void ws.requestDeleteSession(s.id) },
              ]
            : []),
        ];
    const home = s.workspace_id && s.workspace_id !== SCRATCH_WORKSPACE_ID ? s.workspace_id : ws.currentId;
    items.push(
      { separator: true },
      home ? { label: `New session in “${wsName(home)}”…`, icon: 'plus', action: () => void newSessionIn(home) } : { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
      { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
      { label: 'Usage', icon: 'chart', action: () => router.go('usage') },
    );
    ctxMenu.show(e, items);
  }

  const sortLabel = $derived(grouping === 'single'
    ? sessionOrder.mode === 'manual' ? 'Manual order' : sessionOrder.mode === 'tokens' ? 'Sorted by tokens' : 'Recent first'
    : grouping === 'none' ? 'Recent first' : `Grouped by ${SESSION_GROUPING_LABELS[grouping].toLowerCase()}`);
  function sortMenu(e: MouseEvent): void {
    const order: MenuItem[] = multiWorkspace
      ? (Object.keys(SESSION_GROUPING_LABELS) as SessionGrouping[]).map((groupBy) => ({
          label: `Group by ${SESSION_GROUPING_LABELS[groupBy]}`,
          checked: grouping === groupBy,
          action: () => setPreferences({ ...preferences, groupBy }),
        }))
      : [
          { label: 'Sort: Recent', checked: sessionOrder.mode === 'recent', action: () => sessionOrder.setMode('recent') },
          { label: 'Sort: Manual (drag rows)', checked: manualSort, action: () => sessionOrder.setMode('manual', byRecent(ws.plainAgentSessions).map((x) => x.id)) },
          ...(canUsage ? [{ label: 'Sort: Tokens (most first, 30 days)', checked: sessionOrder.mode === 'tokens', action: () => sessionOrder.setMode('tokens') }] : []),
          { label: 'Reset to recent', icon: 'refresh', disabled: sessionOrder.mode === 'recent', action: () => sessionOrder.reset() },
        ];
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'This workspace', checked: !ws.allWorkspaces, action: () => ws.setAllWorkspaces(false) },
      { label: 'All workspaces', checked: ws.allWorkspaces, action: () => ws.setAllWorkspaces(true) },
      { label: 'Only what needs you', checked: ws.needsYouFilter, action: () => (ws.needsYouFilter = !ws.needsYouFilter) },
      { separator: true },
      ...order,
    ]);
  }
  function wsSwitcher(e: MouseEvent): void {
    const activity = workspaceActivity([...ws.foregroundActive, ...ws.otherWsSessions], ws.statusMap, ws.needsYou);
    const items = switcherItems(ws.workspaces, parseRecent(lsGet(LS_WS_RECENT)), activity, ws.currentId, (id) => void ws.select(id));
    if (items.length > 0) items.push({ separator: true });
    items.push({ label: 'Add workspace…', icon: 'plus', pinned: true, action: () => (ui.newWorkspaceOpen = true) });
    items.push({ label: 'Removed workspaces…', icon: 'archive', pinned: true, action: () => (removedOpen = true) });
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, ws.workspaces.length >= 5 ? { filter: true, filterPlaceholder: 'Switch workspace…', maxVisible: 12 } : undefined);
  }
  let searchEl = $state<HTMLInputElement | null>(null);
  function clearSearch(): void {
    query = '';
    searchEl?.focus();
  }
  function clearFilters(): void {
    ws.needsYouFilter = false;
    clearSearch();
  }
  // ⋯ after "Hide Sessions ⌘B" (the pane adds it).
  function paneMenu(): MenuItem[] {
    return [
      { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
      { label: 'Add workspace…', icon: 'folder', action: () => (ui.newWorkspaceOpen = true) },
      { label: 'Removed workspaces…', icon: 'archive', action: () => (removedOpen = true) },
    ];
  }
  // The selection bar's ⋯: Tile and Delete (Archive is its one button).
  function selectionMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'Tile', icon: 'grid', disabled: liveSel.length < 2, action: tileSelected },
      { label: 'Delete…', icon: 'trash', danger: true, disabled: selIds.length === 0, action: () => void deleteSelected() },
    ], { align: 'end' });
  }
  function toggleHistory(): void {
    historyOpen = !historyOpen;
    if (historyOpen && !ws.archivedLoaded) void loadArchived(false);
  }
  async function loadArchived(more: boolean): Promise<void> {
    archivedError = null;
    try {
      await ws.loadArchived(more);
    } catch (e) {
      archivedError = loadErrorText(e);
    }
  }
</script>

{#snippet row(s: Session, showWorkspace = true)}
  {@const thread = isThread(s)}
  {@const st = stateOf(s)}
  {@const wordState = st.key === 'stale' && grouped ? stateOf(s, false) : st}
  {@const word = s.archived ? { word: 'Stopped', tone: '' as const } : sessionWord(wordState, { unread: unread(s), kinds: kindsOf(s) })}
  {@const cls = s.archived ? 'rest' : classOf(s, wordState)}
  <AgentsInboxRow
    session={s}
    project={showWorkspace && (ws.allWorkspaces || s.workspace_id !== ws.currentId) ? wsName(s.workspace_id) : null}
    status={word}
    strong={cls === 'needs' || cls === 'done'}
    kind={s.archived ? 'archived' : thread ? 'chat' : 'terminal'}
    branch={s.archived ? null : branchOf(s)}
    selected={ws.activeSessionId === s.id && router.module !== 'threads'}
    inPane={ws.panes.includes(s.id)}
    tip={[s.title, st.hint ?? st.label, s.cwd].filter(Boolean).join('\n')}
    stateKey={s.archived ? 'archived' : st.key}
    select={selectMode && !thread && own(s) && ws.canEditSession(s) ? { on: true, checked: sel.has(s.id), ontoggle: () => toggleSel(s.id) } : null}
    drag={dragFor(s)}
    onclick={(e) => open(e, s)}
    onmenu={(e) => rowMenu(e, s)}
    onrename={() => void rename(s)}
    close={!s.archived && !thread && ws.canEditSession(s) ? { title: ws.closeTabTitle(s.id, 'session'), onclose: () => void ws.requestCloseTab(s.id) } : null}
  />
{/snippet}

{#snippet selectionBar()}
  <div class="ai-tools" data-testid="agents-select-tools">
    <label class="ai-count">
      <input type="checkbox" aria-label="Select all sessions" checked={selIds.length > 0 && selIds.length === selectable.length} indeterminate={selIds.length > 0 && selIds.length < selectable.length} onchange={() => (sel = selIds.length === selectable.length ? new Set() : new Set(selectable.map((s) => s.id)))} />
      <span class="ellipsis" title={selIds.length > 0 ? `${selIds.length} selected` : 'Select all'}>{selIds.length > 0 ? selIds.length : 'All'}</span>
    </label>
    <button type="button" class="btn small ghost" disabled={liveSel.length === 0} data-testid="agents-archive-selected" onclick={() => void archiveSelected()}>Archive</button>
    <button type="button" class="icon-btn" aria-haspopup="menu" aria-label="More for the selected sessions" title="More" data-testid="agents-select-more" onclick={selectionMenu}><Icon name="more" size={14} /></button>
    <button type="button" class="btn small ghost" data-testid="agents-select-done" onclick={() => setSelectMode(false)}>Done</button>
  </div>
{/snippet}

<ListPane
  name="Sessions"
  count={ptyRows.length + threadRows.length}
  primary={{ icon: 'plus', label: 'New session (⌘T)', onclick: () => (ui.newSessionOpen = true), testid: 'agents-inbox-new' }}
  menu={paneMenu}
  selection={selectMode ? selectionBar : undefined}
  loading={ws.sessionsLoading && ws.sessions.length === 0}
  rows=".tr-main"
  testid="agents-inbox"
>
  {#snippet searchRow()}
    <div class="ai-search">
      <Icon name="search" size={14} />
      <input bind:this={searchEl} type="search" dir="auto" bind:value={query} placeholder="Filter sessions…" aria-label="Search sessions, branches, workspaces" data-testid="agents-search" />
      {#if query}
        <button type="button" class="icon-btn ai-tool" aria-label="Clear search" title="Clear search" onclick={clearSearch}><Icon name="x" size={12} /></button>
      {/if}
      {#if selectMode || selectable.length > 0}
        <button type="button" class="icon-btn ai-tool" class:on={selectMode} aria-pressed={selectMode} aria-label="Select sessions" title={selectMode ? 'Done selecting' : 'Select sessions to tile, archive or delete'} data-testid="agents-select-toggle" onclick={() => setSelectMode(!selectMode)}>
          <Icon name={selectMode ? 'check' : 'square'} size={14} />
        </button>
      {/if}
      <button type="button" class="icon-btn ai-tool" class:on={ws.allWorkspaces || sessionOrder.mode !== 'recent' || ws.needsYouFilter} aria-haspopup="menu" aria-label="Show and sort sessions" title="Showing {ws.allWorkspaces ? 'all workspaces' : 'this workspace'} · {sortLabel}" data-testid="agents-sort-toggle" onclick={sortMenu}>
        <Icon name="filter" size={14} />
      </button>
    </div>
  {/snippet}
  {#snippet filterRow()}
    {#if ws.current}
      <button type="button" class="ai-ws" use:longPress aria-haspopup="menu" title="{ws.current.name}\n{ws.current.root_path}\nClick to switch workspace" aria-label="Current workspace: {ws.current.name}. Switch workspace" data-testid="agents-current-ws" onclick={wsSwitcher} oncontextmenu={(e) => { e.preventDefault(); if (ws.current) ctxMenu.show(e, workspaceMenuItems(ws.current, () => (removedOpen = true))); }}>
        <Icon name="folder" size={13} />
        <span class="ellipsis">{ws.allWorkspaces ? `All workspaces · ${ws.current.name}` : ws.current.name}</span>
        <Icon name="chevronDown" size={12} />
      </button>
    {/if}
  {/snippet}
  {#snippet footer()}
    {#if meterReady}
      <LazyMount lazy={UsageMeter} what="usage" variant="compact" quiet props={{ provider: ws.getSession(ws.activeSessionId)?.provider ?? null, onopen: () => router.go('usage'), quiet: true }} />
    {/if}
  {/snippet}

  <div class="ai-lists" class:grouped>
    <div class="ai-list" data-testid="agents-list">
      {#each inbox as group (group.kind + ':' + group.key)}
        {#if group.kind === 'workspace'}
          {@const name = wsName(group.key)}
          {@const expanded = !isCollapsed(group)}
          <InboxFold
            label={name}
            count={group.sessions.length}
            {expanded}
            sticky
            testid={`agents-workspace-${group.key}`}
            ontoggle={isFiltering() ? undefined : () => toggleWorkspace(group.key)}
          >
            {#snippet leading()}<ProjectBadge {name} />{/snippet}
            {#if expanded}
              {#each group.sessions as s (s.id)}{@render row(s, false)}{/each}
            {/if}
          </InboxFold>
        {:else if group.kind === 'status'}
          <InboxFold label={SESSION_LIST_STATUS_LABELS[group.key]} count={group.sessions.length} sticky>
            {#each group.sessions as s (s.id)}{@render row(s)}{/each}
          </InboxFold>
        {:else}
          {#each group.sessions as s (s.id)}{@render row(s)}{/each}
        {/if}
      {:else}
        {#if isFiltering()}
          <div class="ai-empty">
            <p>No sessions match</p>
            <button type="button" class="btn small ghost" onclick={clearFilters}>Clear filter</button>
          </div>
        {/if}
      {/each}
    </div>
    {#if telegram.length > 0}
      <InboxFold label="Telegram" count={telegram.length} expanded={telegramOpen} ontoggle={() => (telegramOpen = !telegramOpen)}>
        {#if telegramOpen}{#each telegram as s (s.id)}{@render row(s)}{/each}{/if}
      </InboxFold>
    {/if}
    {#if slack.length > 0}
      <InboxFold label="Slack" count={slack.length} expanded={slackOpen} ontoggle={() => (slackOpen = !slackOpen)}>
        {#if slackOpen}{#each slack as s (s.id)}{@render row(s)}{/each}{/if}
      </InboxFold>
    {/if}
    {#if ws.hasArchived || historyRows.length > 0}
      <InboxFold label="History" ariaLabel="Archived sessions" count={ws.archivedLoaded ? historyRows.length : undefined} expanded={historyOpen} testid="archived-toggle" ontoggle={toggleHistory}>
        {#if historyOpen}
          <div class="ai-list" data-testid="agents-history-list">
            {#each historyRows as s (s.id)}
              {@render row(s)}
            {/each}
            {#if archivedError}
              <LoadState what="archived sessions" variant="compact" error={archivedError} empty onretry={() => void loadArchived(ws.archivedLoaded)} />
            {:else if ws.archivedLoading}
              <Skeleton rows={2} height={48} label="archived sessions" />
            {:else if ws.archivedLoaded && historyRows.length === 0}
              <p class="ai-empty">Sessions you archive appear here.</p>
            {:else if ws.archivedHasMore}
              <button type="button" class="btn small ghost ai-more" data-testid="archived-load-more" onclick={() => void loadArchived(true)}>Load more</button>
            {/if}
          </div>
        {/if}
      </InboxFold>
    {/if}
  </div>
</ListPane>

{#if removedOpen}<LazyMount lazy={RemovedWorkspacesChunk} what="removed workspaces" quiet onerror={() => (removedOpen = false)} props={{ onclose: () => (removedOpen = false) }} />{/if}
{#if shareId}
  {@const id = shareId}
  <LazyMount lazy={ShareModalChunk} what="the share dialog" quiet onerror={() => (shareId = null)} props={{ sessionId: id, onclose: () => (shareId = null) }} />
{/if}

<style>
  /* The pane's one search row (as Threads'): the field, Select and Show / sort at its end. */
  .ai-search {
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 28px;
    padding-inline: 8px 2px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
  }
  .ai-search:focus-within {
    color: var(--text);
    border-color: var(--accent-text);
  }
  /* One clear button: ours, not WebKit's as well. */
  .ai-search input::-webkit-search-cancel-button {
    display: none;
  }
  .ai-search input {
    flex: 1;
    min-width: 0;
    block-size: 100%;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    outline: none;
  }
  .ai-tool {
    flex: none;
  }
  .on {
    color: var(--accent-text);
  }
  .ai-ws {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    inline-size: 100%;
    padding: var(--sp-2) var(--sp-3);
    min-block-size: 28px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .ai-ws:hover {
    background: var(--hover);
    color: var(--text);
  }
  /* The selection bar replaces the pane header (count · one action · ⋯ · Done). */
  .ai-tools {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .ai-count {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
    flex: 1;
    padding-inline-start: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  /* Just the count beside the box (its title says "N selected"). */
  .ai-count span {
    min-width: 0;
  }
  .ai-tools :global(.btn) {
    flex: none;
  }
  .ai-lists {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }
  .ai-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ai-empty {
    margin: 0;
    padding: 20px 8px;
    text-align: center;
    font-size: var(--fs-s);
    color: var(--text-dim);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }
  .ai-empty p {
    margin: 0;
  }
  .ai-more {
    margin: 6px 8px;
  }
  /* Touch: finger-sized Select / Show-sort / Clear (44 px, design review H8). */
  @media (pointer: coarse) {
    .ai-search {
      block-size: 46px;
      padding-inline-end: 0;
    }
    .ai-tool {
      inline-size: 44px;
      block-size: 44px;
    }
    .ai-ws {
      min-block-size: 44px;
    }
  }
</style>
