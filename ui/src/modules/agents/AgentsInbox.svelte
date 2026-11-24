<script lang="ts">
  // The Agents inbox (patterns.md §10 — the Threads concept, where the owner
  // works): terminal sessions AND structured threads in ONE list, needs you
  // first, then the sort mode's order (agentsInbox.ts); working sessions in the
  // pinned "Working" fold, closed ones in "History". Top: search · select ·
  // sort/scope menu, then the workspace this list belongs to (click → switch).
  // Foot: New session · the usage meter. It replaced the session tree that
  // hung under "Agents" in the app sidebar; everything that tree did is here
  // (scope by workspace, multi-select → Tile / Archive / Delete, drag to
  // reorder in Manual, the row menu, rename, delete with undo).
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import InboxFold from '../../lib/components/InboxFold.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
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
  import { lsGet } from '../../lib/storage';
  import type { Session } from '../../lib/api/types';
  import { ShareModalChunk } from '../../shell/overlays.svelte';
  import { RemovedWorkspacesChunk } from '../../shell/lazyDialogs';
  import { closeConfirm } from '../../shell/closeConfirm';
  import { checkOpenTabs } from '../../shell/openTabsCheck';
  import { LS_WS_RECENT, parseRecent, switcherItems, workspaceActivity } from '../../shell/workspaceSwitch';
  import { newScratchSession, newSessionIn, workspaceMenuItems } from '../../shell/workspaceActions';
  import AgentsInboxRow from './AgentsInboxRow.svelte';
  import { agentsInbox, byRecent, matchesQuery } from './agentsInbox';

  // The inbox lists the same structured threads as Threads (spec A: one set of sessions, two views).
  type RuntimeStore = typeof import('../threads/runtime/runtimeThreads.svelte').runtimeThreads; // ui-guards: allow — one store
  // The Threads footer's meter (the focused session's agent): after first paint.
  const UsageMeter = lazyComponent(() => import('../threads/ThreadUsageMeter.svelte')); // ui-guards: allow — the Threads footer meter, one implementation

  interface Props {
    /** A row was opened (the phone list pushes the session). */
    onopen?: () => void;
    /** Hide the inbox column (desktop). */
    onhide?: () => void;
  }
  let { onopen, onhide }: Props = $props();

  let query = $state('');
  let workingOpen = $state(true);
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
  let meterReady = $state(false);
  onMount(() => {
    let gone = false;
    let off: (() => void) | null = null;
    const cancel = whenIdle(() => {
      meterReady = true;
      void import('../threads/runtime/runtimeThreads.svelte').then((m) => { // ui-guards: allow — see RuntimeStore
        if (gone) return;
        rt = m.runtimeThreads;
        off = m.runtimeThreads.mount();
        void m.runtimeThreads.loadAll().catch(() => {});
      });
    });
    return () => {
      gone = true;
      cancel();
      off?.();
    };
  });

  // ── Sort (Recent · Manual · Tokens), persisted per workspace ──────────────
  $effect(() => sessionOrder.load(ws.currentId ?? SCRATCH_WORKSPACE_ID));
  const canUsage = $derived(auth.can('usage', 'view'));
  const usageWs = $derived(ws.currentId ?? SCRATCH_WORKSPACE_ID);
  const tokensSort = $derived(sessionOrder.mode === 'tokens' && canUsage && sessionUsage.available);
  // The 30-day rollup feeds the Tokens sort only; polled while it is on.
  $effect(() => {
    if (!tokensSort) return;
    const wid = usageWs;
    const p = pollWhileVisible(() => sessionUsage.loadWorkspace(wid), { ms: 60_000 });
    return () => p.stop();
  });

  const stale = $derived(events.state !== 'connected');
  const wsName = (id: string | null | undefined): string =>
    id === SCRATCH_WORKSPACE_ID || !id ? 'No workspace' : (ws.workspaces.find((w) => w.id === id)?.name ?? 'Workspace');
  const isThread = (s: Session): boolean => !!rt && !!rt.get(s.id);

  // Every row this scope shows: this workspace's terminals (+ "No workspace"),
  // other workspaces' while "All workspaces" is on, and the structured threads.
  const ptyRows = $derived.by(() => {
    const own = sessionOrder.mode === 'manual'
      ? applyOrder(ws.plainAgentSessions, sessionOrder.order)
      : tokensSort
        ? applyTokenOrder(ws.plainAgentSessions, sessionUsage.tokensFor(usageWs))
        : byRecent(ws.plainAgentSessions);
    return [...own, ...ws.scratchSessions, ...(ws.allWorkspaces ? ws.otherWsGroups.flatMap((g) => g.sessions) : [])];
  });
  const scope = (s: Session): boolean =>
    ws.allWorkspaces || s.workspace_id === ws.currentId || s.workspace_id === SCRATCH_WORKSPACE_ID;
  const threadRows = $derived(rt ? byRecent(rt.rows().filter((s) => isForeground(s) && scope(s))) : []);

  function stateOf(s: Session) {
    return sessionState(s, ws.statusMap[s.id] ?? s.status, ws.needsYou[s.id] === true, { stale });
  }
  const unread = (s: Session): boolean => ws.unread[s.id] === true && ws.activeSessionId !== s.id;
  const classOf = (s: Session) => inboxClass(stateOf(s).key, unread(s));
  const branchOf = (s: Session): string | null => {
    const m = s.meta as Record<string, unknown> | null | undefined;
    const b = m?.git_branch ?? m?.branch ?? m?.worktree_branch;
    if (typeof b === 'string' && b.trim()) return b.trim();
    const repo = s.cwd ? git.repos.find((r) => s.cwd === r.path || s.cwd!.startsWith(r.path.replace(/\/+$/, '') + '/')) : null;
    return repo ? (git.statusById[repo.id]?.branch ?? null) : null;
  };
  const matches = (s: Session): boolean =>
    (!ws.needsYouFilter || ws.needsYou[s.id] === true) && matchesQuery(s, query, [branchOf(s), wsName(s.workspace_id)]);

  const inbox = $derived(agentsInbox([...ptyRows, ...threadRows].filter(matches), classOf));
  const historyRows = $derived(
    [...ws.archivedAgentSessions, ...(rt ? rt.rows(true).filter((s) => isForeground(s) && scope(s)) : [])].filter(matches),
  );
  const telegram = $derived(ws.telegramSessions.filter(matches));
  const slack = $derived(ws.slackSessions.filter(matches));

  // ── Select mode: Tile · Archive · Delete for the checked rows ────────────
  // This workspace's rows (and "No workspace"), as the old list's select mode.
  const own = (s: Session): boolean => s.workspace_id === ws.currentId || s.workspace_id === SCRATCH_WORKSPACE_ID;
  const selectable = $derived(
    [...inbox.main, ...inbox.working, ...(historyOpen ? historyRows : [])].filter((s) => !isThread(s) && own(s) && ws.canEditSession(s)),
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
  const dragOn = $derived(!query.trim() && !selectMode && !ws.needsYouFilter && !viewport.isPhone);
  const shownOwn = (): string[] =>
    (sessionOrder.mode === 'manual' ? applyOrder(ws.plainAgentSessions, sessionOrder.order) : tokensSort ? applyTokenOrder(ws.plainAgentSessions, sessionUsage.tokensFor(usageWs)) : byRecent(ws.plainAgentSessions)).map((x) => x.id);
  let dragId = $state<string | null>(null);
  let dragOver = $state<string | null>(null);
  const ownIds = $derived(ws.plainAgentSessions.map((s) => s.id));
  function dragFor(s: Session) {
    if (!dragOn || !ownIds.includes(s.id)) return null;
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
    if (isThread(s)) router.go(`threads/${s.id}`);
    else if (s.archived) void ws.unarchiveAndOpen(s.id).catch((err) => toastError('Couldn’t reopen the session', err));
    else if (s.workspace_id !== ws.currentId && s.workspace_id !== SCRATCH_WORKSPACE_ID) void ws.openInWorkspace(s.workspace_id, s.id);
    else if (e.metaKey || e.ctrlKey) ws.openInSplit(s.id);
    else ws.navigateToSession(s.id);
    onopen?.();
  }
  async function rename(s: Session): Promise<void> {
    if (isThread(s) || !ws.canEditSession(s)) return;
    const next = await confirmer.promptText('Session name', { title: 'Rename session', initial: s.title, confirmLabel: 'Rename' });
    if (next?.trim() && next.trim() !== s.title) await ws.renameSession(s.id, next.trim()).catch((e) => toastError('Couldn’t rename the session', e));
  }
  function rowMenu(e: MouseEvent, s: Session): void {
    e.preventDefault();
    if (isThread(s)) {
      void import('../threads/runtime/runtimeMenu').then((m) => ctxMenu.show(e, m.runtimeThreadMenu(s.id))); // ui-guards: allow — a thread's own menu
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
          ...(sessionOrder.mode === 'manual' && ownIds.includes(s.id)
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
    );
    ctxMenu.show(e, items);
  }

  function sortMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'This workspace', checked: !ws.allWorkspaces, action: () => ws.setAllWorkspaces(false) },
      { label: 'All workspaces', checked: ws.allWorkspaces, action: () => ws.setAllWorkspaces(true) },
      { label: 'Only what needs you', checked: ws.needsYouFilter, action: () => (ws.needsYouFilter = !ws.needsYouFilter) },
      { separator: true },
      { label: 'Sort: Recent', checked: sessionOrder.mode === 'recent', action: () => sessionOrder.setMode('recent') },
      { label: 'Sort: Manual (drag rows)', checked: sessionOrder.mode === 'manual', action: () => sessionOrder.setMode('manual', byRecent(ws.plainAgentSessions).map((x) => x.id)) },
      ...(canUsage ? [{ label: 'Sort: Tokens (most first, 30 days)', checked: sessionOrder.mode === 'tokens', action: () => sessionOrder.setMode('tokens') }] : []),
      { label: 'Reset to recent', icon: 'refresh', disabled: sessionOrder.mode === 'recent', action: () => sessionOrder.reset() },
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

{#snippet row(s: Session)}
  {@const thread = isThread(s)}
  {@const st = stateOf(s)}
  {@const word = s.archived ? { word: 'Stopped', tone: '' as const } : sessionWord(st, { unread: unread(s) })}
  {@const cls = s.archived ? 'rest' : inboxClass(st.key, unread(s))}
  <AgentsInboxRow
    session={s}
    project={wsName(s.workspace_id)}
    status={word}
    strong={cls === 'needs' || cls === 'done'}
    kind={s.archived ? 'archived' : thread ? 'chat' : 'terminal'}
    branch={s.archived ? null : branchOf(s)}
    selected={!thread && ws.activeSessionId === s.id && router.module !== 'threads'}
    inPane={!thread && ws.panes.includes(s.id)}
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

<aside class="ai" aria-label="Sessions" data-testid="agents-inbox">
  <div class="ai-top">
    <label class="ai-search">
      <Icon name="search" size={14} />
      <input type="search" dir="auto" bind:value={query} placeholder="Search" aria-label="Search sessions, branches, workspaces" data-testid="agents-search" />
    </label>
    {#if selectable.length > 0}
      <button type="button" class="icon-btn" class:on={selectMode} aria-pressed={selectMode} aria-label="Select sessions" title={selectMode ? 'Done selecting' : 'Select sessions to tile, archive or delete'} data-testid="agents-select-toggle" onclick={() => setSelectMode(!selectMode)}>
        <Icon name={selectMode ? 'check' : 'square'} size={14} />
      </button>
    {/if}
    <button type="button" class="icon-btn" class:on={ws.allWorkspaces || sessionOrder.mode !== 'recent' || ws.needsYouFilter} aria-haspopup="menu" aria-label="Show and sort sessions" title="Showing {ws.allWorkspaces ? 'all workspaces' : 'this workspace'} · {sessionOrder.mode === 'manual' ? 'Manual order' : sessionOrder.mode === 'tokens' ? 'Sorted by tokens' : 'Recent first'}" data-testid="agents-sort-toggle" onclick={sortMenu}>
      <Icon name="filter" size={14} />
    </button>
    {#if onhide}
      <button type="button" class="icon-btn" aria-label="Hide the session list" title="Hide the session list" data-testid="agents-inbox-hide" onclick={onhide}>
        <Icon name="sidebar" size={14} />
      </button>
    {/if}
  </div>
  {#if ws.current}
    <button type="button" class="ai-ws" use:longPress aria-haspopup="menu" title="{ws.current.name}\n{ws.current.root_path}\nClick to switch workspace" aria-label="Current workspace: {ws.current.name}. Switch workspace" data-testid="agents-current-ws" onclick={wsSwitcher} oncontextmenu={(e) => { e.preventDefault(); if (ws.current) ctxMenu.show(e, workspaceMenuItems(ws.current, () => (removedOpen = true))); }}>
      <Icon name="folder" size={13} />
      <span class="ellipsis">{ws.allWorkspaces ? `All workspaces · ${ws.current.name}` : ws.current.name}</span>
      <Icon name="chevronDown" size={12} />
    </button>
  {/if}
  {#if selectMode}
    <div class="ai-tools" data-testid="agents-select-tools">
      <label class="ai-count">
        <input type="checkbox" aria-label="Select all sessions" checked={selIds.length > 0 && selIds.length === selectable.length} indeterminate={selIds.length > 0 && selIds.length < selectable.length} onchange={() => (sel = selIds.length === selectable.length ? new Set() : new Set(selectable.map((s) => s.id)))} />
        <span>{selIds.length > 0 ? `${selIds.length} selected` : 'Select all'}</span>
      </label>
      <button type="button" class="btn small ghost" disabled={liveSel.length < 2} data-testid="agents-tile-selected" onclick={tileSelected}>Tile</button>
      <button type="button" class="btn small ghost" disabled={liveSel.length === 0} data-testid="agents-archive-selected" onclick={() => void archiveSelected()}>Archive</button>
      <button type="button" class="btn small ghost danger" disabled={selIds.length === 0} data-testid="agents-delete-selected" onclick={() => void deleteSelected()}>Delete{selIds.length > 0 ? ` (${selIds.length})` : ''}</button>
    </div>
  {/if}

  <div class="ai-body">
    <div class="ai-scroll">
      <div class="ai-list" data-testid="agents-list">
        {#each inbox.main as s (s.id)}
          {@render row(s)}
        {:else}
          {#if ws.sessionsLoading && ws.sessions.length === 0}
            <Skeleton rows={3} height={56} label="sessions" />
          {:else if query.trim() || ws.needsYouFilter}
            <p class="ai-empty">No sessions match</p>
          {:else if inbox.working.length === 0}
            <div class="ai-empty">
              <p>No sessions yet</p>
              <button type="button" class="btn small ghost" onclick={() => (ui.newSessionOpen = true)}>New session</button>
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
    </div>
    <div class="ai-folds">
      {#if inbox.working.length > 0}
        <InboxFold label="Working" ariaLabel="Working sessions" count={inbox.working.length} expanded={workingOpen} testid="agents-working-toggle" ontoggle={() => (workingOpen = !workingOpen)}>
          <div class="ai-list" data-testid="agents-working-list">
            {#each workingOpen ? inbox.working : inbox.working.filter((s) => s.id === ws.activeSessionId) as s (s.id)}
              {@render row(s)}
            {/each}
          </div>
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
  </div>

  <footer class="ai-foot">
    <button type="button" class="btn small ghost ai-new" data-testid="agents-inbox-new" aria-keyshortcuts="Meta+T" title="New session (⌘T)" onclick={() => (ui.newSessionOpen = true)}>
      <Icon name="plus" size={14} /> New session
    </button>
    {#if meterReady}
      <LazyMount lazy={UsageMeter} what="usage" variant="compact" quiet props={{ provider: ws.getSession(ws.activeSessionId)?.provider ?? null, onopen: () => router.go('usage') }} />
    {/if}
  </footer>
</aside>

{#if removedOpen}<LazyMount lazy={RemovedWorkspacesChunk} what="removed workspaces" quiet onerror={() => (removedOpen = false)} props={{ onclose: () => (removedOpen = false) }} />{/if}
{#if shareId}
  {@const id = shareId}
  <LazyMount lazy={ShareModalChunk} what="the share dialog" quiet onerror={() => (shareId = null)} props={{ sessionId: id, onclose: () => (shareId = null) }} />
{/if}

<style>
  .ai {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-width: 0;
    background: var(--bg-sidebar);
    border-inline-end: 1px solid var(--border);
  }
  .ai-top {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: var(--sp-5) var(--sp-2);
    padding-inline: var(--sp-6) var(--sp-4);
    flex-shrink: 0;
  }
  .ai-search {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
  }
  .ai-search input {
    flex: 1;
    min-width: 0;
    block-size: 28px;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-l);
    outline: none;
  }
  .ai-search:focus-within {
    color: var(--text);
  }
  .ai-search input:focus-visible {
    box-shadow: 0 2px 0 -1px var(--accent-text);
  }
  .on {
    color: var(--accent-text);
  }
  .ai-ws {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    margin-inline: var(--sp-4);
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
    flex-shrink: 0;
  }
  .ai-ws:hover {
    background: var(--hover);
    color: var(--text);
  }
  .ai-tools {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-5);
    flex-shrink: 0;
  }
  .ai-count {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
    flex: 1;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .ai-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .ai-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--sp-2) var(--sp-4) var(--sp-4);
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }
  /* Working and History stay pinned above the foot (≤ 40 %); an open fold scrolls inside. */
  .ai-folds {
    flex-shrink: 0;
    max-block-size: 40%;
    display: flex;
    flex-direction: column;
    padding: var(--sp-2) var(--sp-4);
    border-block-start: 1px solid var(--separator);
  }
  .ai-folds :global(.if-group) {
    flex: 0 1 auto;
  }
  .ai-folds .ai-list {
    min-height: 0;
    overflow-y: auto;
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
  .ai-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    border-block-start: 1px solid var(--separator);
    flex-shrink: 0;
  }
  @media (hover: none), (pointer: coarse) {
    .ai-top :global(.icon-btn),
    .ai-ws,
    .ai-new {
      min-block-size: var(--hit-min);
      min-inline-size: var(--hit-min);
    }
  }
</style>
