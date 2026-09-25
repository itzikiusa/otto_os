<script lang="ts">
  // Expanded 240px navigator: modules in foldable macOS source-list sections
  // (Agents with its nested session lists in Work), workspaces section,
  // user/settings at the bottom.
  import Icon from '../lib/components/Icon.svelte';
  import NotificationBell from './NotificationBell.svelte';
  import StatusDot from '../lib/components/StatusDot.svelte';
  import ProviderIcon, { hasProviderIcon } from '../lib/components/ProviderIcon.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../lib/stores/workspace.svelte';
  import { assistant } from '../lib/stores/assistant.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import { activity } from '../lib/stores/activity.svelte';
  import { proof } from '../lib/stores/proof.svelte';
  import ProofStatusChip from '../lib/components/ProofStatusChip.svelte';
  import ShareModal from '../modules/agents/ShareModal.svelte';
  import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
  import { sidePane, splitMenuItems, navClick, SPLIT_HINT } from '../lib/stores/sidePane.svelte';
  import { popoutItems } from '../lib/popoutMenu';
  import { sessionOrder, applyOrder } from '../lib/stores/sessionOrder.svelte';
  import { viewport } from '../lib/stores/viewport.svelte';
  import { confirmer } from '../lib/confirm.svelte';
  import { toasts } from '../lib/toast.svelte';
  import type { WorkspaceWithRole } from '../lib/api/types';
  import { tick, untrack } from 'svelte';
  import {
    FAVORITES_ID,
    activeNavId,
    availableModules,
    moveAmong,
    moveWithinGroup,
    reorderAmong,
    resolveGroupOrder,
    resolveOrder,
    sidebarSections,
    visibleOrder,
    type SidebarModule,
    type SidebarPluginEntry,
    type SidebarSection,
  } from '../lib/sidebar';
  import type { Session } from '../lib/api/types';
  import { events } from '../lib/events.svelte';
  import { sessionState, type SessionStateInfo } from '../lib/status';

  // Load the per-session task roll-up for the current workspace (sidebar chips);
  // it then stays fresh from the events WS (tasks_updated / trail_appended).
  $effect(() => {
    const w = ws.currentId;
    if (w) void activity.loadSummary(w);
  });

  // Load the per-work-item proof roll-up so each session row can show an inline
  // proof chip; kept fresh from the events WS (proof_pack_updated).
  $effect(() => {
    const w = ws.currentId;
    if (w) void proof.loadSummary(w);
  });

  // One session vocabulary (lib/status.ts): a row's dot, tooltip and resume
  // affordance all come from `sessionState`. While the events socket is down
  // the live claims are stale — "Reconnecting…", no pulse (patterns.md §1).
  const staleEvents = $derived(events.state !== 'connected');
  /** Session-row tooltip: a long title made one very wide native tooltip that
   *  WKWebView pinned against the window edge and clipped over the page. Keep
   *  it short and put the state + secondary signals on their own lines. */
  function rowTip(title: string, st: SessionStateInfo, tasks: { done: number; total: number; in_progress?: string | null } | null): string {
    const t = title.length > 80 ? `${title.slice(0, 79).trimEnd()}…` : title;
    const lines = [t, st.hint ?? st.label];
    if (tasks && tasks.total > 0) {
      lines.push(tasks.in_progress ? `Now: ${tasks.in_progress} · ${tasks.done}/${tasks.total} tasks` : `${tasks.done}/${tasks.total} tasks done`);
    }
    if (!st.resumable) lines.push('Double-click to rename');
    return lines.join('\n');
  }

  let agentsOpen = $state(true);
  // Channel groups (ticket/chat sessions) start collapsed — at ticketing volume
  // they can number in the dozens, so the header + count is shown by default and
  // the user expands on demand.
  let telegramOpen = $state(false);
  let slackOpen = $state(false);
  let archivedOpen = $state(false);
  // Multi-select in the Archived list: checked ids → one "Delete selected"
  // confirm instead of N single-row deletes.
  let archSel = $state<Set<string>>(new Set());
  // Select mode on the Agents list: header toggle → per-row checkboxes → one
  // Archive / Delete for the whole selection (delete confirms once).
  let agentSelMode = $state(false);
  let agentSel = $state<Set<string>>(new Set());
  function toggleAgentSel(id: string): void {
    const next = new Set(agentSel);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    agentSel = next;
  }
  function agentSelectAll(): void {
    agentSel = agentSelIds.length === selectable.length ? new Set() : new Set(selectable.map((s) => s.id));
  }
  function setAgentSelMode(on: boolean): void {
    agentSelMode = on;
    if (!on) agentSel = new Set();
  }
  async function archiveSelectedAgents(): Promise<void> {
    const ids = agentSelIds;
    if (ids.length === 0) return;
    await ws.archiveSessions(ids);
    setAgentSelMode(false);
  }
  async function deleteSelectedAgents(): Promise<void> {
    const ids = agentSelIds;
    if (ids.length === 0) return;
    const ok = await confirmer.ask(
      `Delete ${ids.length} session${ids.length === 1 ? '' : 's'} and their entire history? This cannot be undone.`,
      { title: `Delete ${ids.length} sessions`, confirmLabel: 'Delete' },
    );
    if (!ok) return;
    await ws.killSessions(ids);
    setAgentSelMode(false);
  }
  const archSelCount = $derived([...archSel].filter((id) => ws.archivedSessions.some((s) => s.id === id)).length);
  function toggleArchSel(id: string): void {
    const next = new Set(archSel);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    archSel = next;
  }
  function archSelectAll(): void {
    archSel = archSelCount === ws.archivedSessions.length ? new Set() : new Set(ws.archivedSessions.map((s) => s.id));
  }
  async function deleteSelectedArchived(): Promise<void> {
    const ids = ws.archivedSessions.filter((s) => archSel.has(s.id)).map((s) => s.id);
    if (ids.length === 0) return;
    const ok = await confirmer.ask(
      `Delete ${ids.length} archived session${ids.length === 1 ? '' : 's'} and their entire history? This cannot be undone.`,
      { title: `Delete ${ids.length} sessions`, confirmLabel: 'Delete' },
    );
    if (!ok) return;
    await ws.killSessions(ids);
    archSel = new Set();
  }
  let renamingId: string | null = $state(null);
  let draft = $state('');
  /** The session whose Share sheet is open (same sheet as the tab / pane menus). */
  let shareSessionId = $state<string | null>(null);

  // When a channel group is expanded, show only the most recent N sessions with
  // a "show more" expander, so a busy day's worth of tickets doesn't flood the
  // sidebar. Bypassed while searching (the user is actively filtering).
  const CHANNEL_CAP = 20;
  let telegramShowAll = $state(false);
  let slackShowAll = $state(false);

  // Session search: filters every group by title (case-insensitive), plus an
  // optional "Needs you" filter that narrows to sessions blocked on the operator.
  let sessionQuery = $state('');
  const q = $derived(sessionQuery.trim().toLowerCase());
  const matches = (s: Session): boolean => {
    if (ws.needsYouFilter && ws.needsYou[s.id] !== true) return false;
    return q === '' || s.title.toLowerCase().includes(q);
  };
  // ── Manual sidebar order (C3b) ───────────────────────────────────────────
  // "Recent" is the daemon's order, rendered unchanged. "Manual" applies the
  // persisted id list, with sessions it has never seen on TOP by recency.
  $effect(() => {
    sessionOrder.load(ws.currentId ?? SCRATCH_WORKSPACE_ID);
  });
  const orderedAgents = $derived(
    sessionOrder.mode === 'manual' ? applyOrder(ws.plainAgentSessions, sessionOrder.order) : ws.plainAgentSessions,
  );
  const fAgents = $derived(orderedAgents.filter(matches));
  // Rows drag only in the flat Agents list: never while searching, selecting,
  // needs-you filtering, or on phones (HTML5 DnD is inert on iOS Safari).
  const rowsDraggable = $derived(q === '' && !agentSelMode && !ws.needsYouFilter && !viewport.isPhone);
  let rowDragId = $state<string | null>(null);
  let rowDragOverId = $state<string | null>(null);
  function onRowDragStart(e: DragEvent, id: string): void {
    rowDragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onRowDragOver(e: DragEvent, id: string): void {
    if (!rowDragId || id === rowDragId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    rowDragOverId = id;
  }
  function onRowDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    if (rowDragId && rowDragId !== id) sessionOrder.dragTo(fAgents.map((x) => x.id), rowDragId, id);
    rowDragId = null;
    rowDragOverId = null;
  }
  function onRowDragEnd(): void {
    rowDragId = null;
    rowDragOverId = null;
  }
  function openSortMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, [
      { label: 'Sort: Recent', checked: sessionOrder.mode === 'recent', action: () => sessionOrder.setMode('recent') },
      {
        label: 'Sort: Manual (drag rows)',
        checked: sessionOrder.mode === 'manual',
        action: () => sessionOrder.setMode('manual', fAgents.map((x) => x.id)),
      },
      { separator: true },
      { label: 'Reset to recent', icon: 'refresh', disabled: sessionOrder.mode === 'recent', action: () => sessionOrder.reset() },
    ]);
  }
  // Workspace-less sessions (the "No workspace" group below the flat list).
  const fScratch = $derived(ws.scratchSessions.filter(matches));
  // Select mode covers the flat list AND the "No workspace" group.
  const selectable = $derived([...fAgents, ...fScratch]);
  const agentSelIds = $derived(selectable.filter((s) => agentSel.has(s.id)).map((s) => s.id));
  const fTelegram = $derived(ws.telegramSessions.filter(matches));
  const fSlack = $derived(ws.slackSessions.filter(matches));
  // Capped views (full list when searching or "show all" toggled).
  const visTelegram = $derived(q || telegramShowAll ? fTelegram : fTelegram.slice(0, CHANNEL_CAP));
  const visSlack = $derived(q || slackShowAll ? fSlack : fSlack.slice(0, CHANNEL_CAP));

  // Drag-to-resize the navigator from its right edge (widens the session area).
  let resizing = $state(false);
  function startResize(e: MouseEvent): void {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = ui.railWidth;
    const onMove = (ev: MouseEvent) => ui.setRailWidth(startW + (ev.clientX - startX));
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

  function openSession(id: string): void {
    ws.navigateToSession(id);
  }

  /** Respawn a stuck in-progress agent's PTY (the terminal reconnects itself).
   *  Asks first when it is working, same as the pane header. */
  function restartAgent(id: string): Promise<void> {
    return ws.requestRestart(id);
  }

  /** Delete = PTY killed, row + full history gone. Asks first unless the
   *  user chose "Always delete" (see ws.requestDeleteSession). */
  function deleteSession(id: string): Promise<void> {
    return ws.requestDeleteSession(id);
  }

  // ── Workspace management (context menu on the Workspaces rows) ──────────────
  async function renameWorkspace(w: WorkspaceWithRole): Promise<void> {
    const name = await confirmer.promptText('New workspace name', {
      title: `Rename “${w.name}”`,
      confirmLabel: 'Rename',
      initial: w.name,
    });
    if (!name || name === w.name) return;
    try {
      await ws.updateWorkspace(w.id, { name });
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function changeWorkspaceDir(w: WorkspaceWithRole): Promise<void> {
    const root = await confirmer.promptText('Working directory (absolute path, ~ ok)', {
      title: `Change folder of “${w.name}”`,
      browseFolder: true,
      confirmLabel: 'Change',
      initial: w.root_path,
      placeholder: '~/projects/my-repo',
    });
    if (!root || root === w.root_path) return;
    try {
      await ws.updateWorkspace(w.id, { root_path: root });
      toasts.success('Folder changed', `${w.name} → ${root}`);
    } catch (e) {
      toasts.error('Change failed', e instanceof Error ? e.message : String(e));
    }
  }

  /** "Remove", not "Delete": the workspace is archived (off the sidebar) and
   *  its folder and files are never touched (patterns.md §7 — Remove detaches,
   *  the thing still exists). */
  async function removeWorkspace(w: WorkspaceWithRole): Promise<void> {
    const ok = await confirmer.ask(
      `Remove “${w.name}” from Otto? It leaves the sidebar with its sessions; the folder and its files on disk are not touched.`,
      { title: 'Remove workspace', confirmLabel: 'Remove' },
    );
    if (!ok) return;
    try {
      await ws.archiveWorkspace(w.id);
      toasts.info('Workspace removed', w.name);
    } catch (e) {
      toasts.error('Couldn’t remove the workspace', e instanceof Error ? e.message : String(e));
    }
  }

  function startRename(id: string, current: string): void {
    const s = ws.sessions.find((x) => x.id === id);
    if (!s || !ws.canEditSession(s)) return;
    renamingId = id;
    draft = current;
  }

  /** Open the New Session sheet pre-set to "No workspace". */
  function newScratchSession(): void {
    ui.newSessionScratch = true;
    ui.newSessionOpen = true;
  }

  async function commitRename(): Promise<void> {
    const id = renamingId;
    renamingId = null;
    if (!id) return;
    const next = draft.trim();
    if (next) await ws.renameSession(id, next);
  }

  // ── Module list (shared registry → RBAC + plugins → user order/visibility) ──
  // `resolved` is the full ordered list (incl. hidden); `visible` drops hidden
  // modules; `navList` is what we actually render — everything while editing (so
  // hidden rows can be toggled back on), only the visible ones otherwise.
  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p): SidebarPluginEntry => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const resolved = $derived(
    resolveOrder(availableModules((f) => auth.can(f, 'view'), pluginEntries), ui.sidebarOrder),
  );
  const visible = $derived(visibleOrder(resolved, ui.sidebarHidden));
  const navList = $derived(ui.sidebarEditMode ? resolved : visible);
  // Sections: Favorites first (only while it holds something), then Work /
  // Automate / Build / … in the user's section order; the saved module order
  // applies within each. A favorited module shows ONLY under Favorites. A
  // section whose modules are all hidden (or all favorited) drops out.
  const sections = $derived(sidebarSections(navList, ui.sidebarFavorites, ui.sidebarGroupOrder));
  /** The favorites as rendered right now (RBAC-filtered; hidden ones only
   *  while editing), in order — what the move/drag bounds are measured on. */
  const favIds = $derived(sections[0]?.group.id === FAVORITES_ID ? sections[0].modules.map((m) => m.id) : []);
  function isFav(id: string): boolean {
    return favIds.includes(id);
  }

  // The sidebar id the current route highlights (plugin slug / default route /
  // database+brokers → Connections are resolved in activeNavId).
  const activeId = $derived(activeNavId(router.parts));
  function isActive(id: string): boolean {
    return activeId === id;
  }
  // The section holding the current page — Favorites when it's favorited.
  const activeGroup = $derived(
    ui.sidebarFavorites.includes(activeId) && resolved.some((m) => m.id === activeId)
      ? FAVORITES_ID
      : (resolved.find((m) => m.id === activeId)?.group ?? null),
  );

  function isHidden(id: string): boolean {
    return ui.sidebarHidden.includes(id);
  }

  // ── Section fold state ──────────────────────────────────────────────────
  // The section holding the current page is never shown folded: navigating
  // into a folded section unfolds it (and persists that, so the state the
  // user sees is the state that's saved). While a session search / Needs-you
  // filter is on, Agents' section is forced open so its results show; edit
  // mode opens everything so every row can be reordered / re-shown.
  $effect(() => {
    const g = activeGroup;
    if (g) untrack(() => ui.setSidebarGroupCollapsed(g, false));
  });
  /** Held open regardless of the saved fold state (so its header can't fold it). */
  function sectionPinned(sec: SidebarSection): boolean {
    if (ui.sidebarEditMode || sec.group.id === activeGroup) return true;
    return (q !== '' || ws.needsYouFilter) && sec.modules.some((m) => m.id === 'agents');
  }
  function sectionOpen(sec: SidebarSection): boolean {
    return sectionPinned(sec) || !ui.sidebarCollapsedGroups.includes(sec.group.id);
  }

  // Keep the current row on screen: on every route / focused-session change,
  // scroll the active row (the focused session's row when there is one, else
  // the module row) into view — at laptop heights the list overflows and the
  // active entry would otherwise sit below the fold.
  let scrollEl = $state<HTMLDivElement>();
  $effect(() => {
    void router.parts.join('/');
    void ws.activeSessionId;
    const el = scrollEl;
    if (!el) return;
    void tick().then(() => {
      const row =
        el.querySelector<HTMLElement>('.nav-item.nested-item.active') ??
        el.querySelector<HTMLElement>('.nav-item.active');
      row?.scrollIntoView({ block: 'nearest' });
    });
  });

  // ── Reordering ─────────────────────────────────────────────────────────
  // Edit mode: every row drags (HTML5 DnD, like TabBar) and has up/down
  // buttons (touch-reliable + keyboard-accessible). Moves stay inside a
  // section; the one crossing is a drop ONTO a favorite, which favorites the
  // dragged module at that slot. Favorites also drag outside edit mode and
  // move with ⌥↑ / ⌥↓ (Finder's sidebar Favorites are always reorderable),
  // except on phones where HTML5 DnD is inert. Section headers get up/down
  // (and drag) while editing, and "Move section up/down" in their menu.
  let dragId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);
  /** Where the dragged row would land relative to the hovered one. */
  let dropSide = $state<'before' | 'after'>('before');

  /** Can the dragged module `from` be dropped on the row `to`? */
  function canDrop(from: string | null, to: string): boolean {
    if (!from || from === to) return false;
    if (isFav(to)) return true; // reorder inside Favorites, or favorite-at-slot
    return !isFav(from) && sameGroup(from, to);
  }
  function onDragStart(e: DragEvent, id: string): void {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onDragOver(e: DragEvent, id: string): void {
    if (!canDrop(dragId, id)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverId = id;
    dropSide = dropSideFor(dragId!, id);
  }
  function onDragLeave(id: string): void {
    if (dragOverId === id) dragOverId = null;
  }
  function onDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    const from = dragId;
    dragId = null;
    dragOverId = null;
    if (!from || !canDrop(from, id)) return;
    if (isFav(id)) {
      if (isFav(from)) {
        const next = reorderAmong(ui.sidebarFavorites, from, id);
        if (next) ui.setSidebarFavorites(next);
      } else ui.addSidebarFavorite(from, id);
    } else ui.reorderSidebar(resolved.map((m) => m.id), from, id);
  }
  function onDragEnd(): void {
    dragId = null;
    dragOverId = null;
  }
  /** A drop lands before the target when dragging up (or in from another
   *  section), after it when dragging down — the indicator says which. */
  function dropSideFor(from: string, to: string): 'before' | 'after' {
    const list = isFav(to) ? favIds : resolved.map((m) => m.id);
    const a = list.indexOf(from);
    return a >= 0 && a < list.indexOf(to) ? 'after' : 'before';
  }
  /** Drag + keyboard reorder props for a Favorites row OUTSIDE edit mode
   *  (spread onto the row's button; nothing for other rows or on phones). */
  function favRowProps(id: string): Record<string, unknown> {
    if (ui.sidebarEditMode || viewport.isPhone || !isFav(id)) return {};
    return {
      draggable: true,
      'aria-keyshortcuts': 'Alt+ArrowUp Alt+ArrowDown',
      ondragstart: (e: DragEvent) => onDragStart(e, id),
      ondragover: (e: DragEvent) => onDragOver(e, id),
      ondragleave: () => onDragLeave(id),
      ondrop: (e: DragEvent) => onDrop(e, id),
      ondragend: onDragEnd,
      onkeydown: (e: KeyboardEvent) => {
        if (!e.altKey || e.metaKey || e.ctrlKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
        e.preventDefault();
        moveFavorite(id, e.key === 'ArrowUp' ? -1 : 1, `[data-nav-id="${CSS.escape(id)}"]`);
      },
    };
  }

  /** After a move re-renders the list, put focus back where it was: a keyed
   *  row that the DOM moved loses focus otherwise. Falls back to the sibling
   *  control when the moved-to end disables the one that was pressed. */
  function refocus(selector: string, fallback?: string): void {
    const root = scrollEl;
    if (!root) return;
    void tick().then(() => {
      const el = root.querySelector<HTMLElement>(selector);
      if (el && !(el as HTMLButtonElement).disabled) el.focus();
      else if (fallback) root.querySelector<HTMLElement>(fallback)?.focus();
    });
  }
  /** Selector for one of an edit row's move buttons (by its aria-label). */
  const moveBtn = (label: string, dir: 'up' | 'down') => `[aria-label="${CSS.escape(`Move ${label} ${dir}`)}"]`;

  // Reordering stays inside a section: up/down swap with the nearest
  // same-section neighbour, and a drag only targets rows of its own section.
  function move(m: SidebarModule, delta: -1 | 1): void {
    if (isFav(m.id)) {
      moveFavorite(m.id, delta, moveBtn(m.label, delta < 0 ? 'up' : 'down'), moveBtn(m.label, delta < 0 ? 'down' : 'up'));
      return;
    }
    const next = moveWithinGroup(resolved, m.id, delta);
    if (next) ui.setSidebarOrder(next);
    refocus(moveBtn(m.label, delta < 0 ? 'up' : 'down'), moveBtn(m.label, delta < 0 ? 'down' : 'up'));
  }
  /** Star toggle (edit mode): the row jumps between Favorites and its own
   *  section, so keep keyboard focus on its star. */
  function toggleFavorite(id: string): void {
    ui.toggleSidebarFavorite(id);
    refocus(`[data-testid="sidebar-fav-${CSS.escape(id)}"]`);
  }
  /** Move a favorite among the RENDERED favorites (saved ids the user can't
   *  see keep their slots and are hopped over). */
  function moveFavorite(id: string, delta: -1 | 1, focusSel?: string, fallback?: string): void {
    const next = moveAmong(ui.sidebarFavorites, id, delta, isFav);
    if (next) ui.setSidebarFavorites(next);
    if (focusSel) refocus(focusSel, fallback);
  }
  function sameGroup(a: string | null, b: string): boolean {
    const ga = resolved.find((m) => m.id === a)?.group;
    return ga != null && ga === resolved.find((m) => m.id === b)?.group;
  }

  // ── Sections: order ─────────────────────────────────────────────────────
  // Favorites is always first and never moves. The others swap with their
  // nearest RENDERED neighbour (an empty section — no plugins, all hidden —
  // isn't a visible slot), persisted as the full resolved section order.
  const movableSections = $derived(sections.filter((s) => s.group.id !== FAVORITES_ID).map((s) => s.group.id as string));
  function canMoveSection(id: string, delta: -1 | 1): boolean {
    const i = movableSections.indexOf(id);
    return i >= 0 && i + delta >= 0 && i + delta < movableSections.length;
  }
  function moveSection(id: string, delta: -1 | 1, focusSel?: string, fallback?: string): void {
    const all = resolveGroupOrder(ui.sidebarGroupOrder).map((g) => g.id as string);
    const next = moveAmong(all, id, delta, (g) => movableSections.includes(g));
    if (next) ui.setSidebarGroupOrder(next);
    if (focusSel) refocus(focusSel, fallback);
  }
  const sectionBtn = (label: string, dir: 'up' | 'down') =>
    `[aria-label="${CSS.escape(`Move ${label} section ${dir}`)}"]`;
  // Section drag (edit mode): the header row is the handle; any other
  // movable section's block is a target.
  let secDragId = $state<string | null>(null);
  let secDragOverId = $state<string | null>(null);
  let secDropSide = $state<'before' | 'after'>('before');
  function onSecDragStart(e: DragEvent, id: string): void {
    secDragId = id;
    e.dataTransfer?.setData('text/plain', `section:${id}`);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onSecDragOver(e: DragEvent, id: string): void {
    if (!secDragId || secDragId === id || !movableSections.includes(id)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    secDragOverId = id;
    secDropSide = movableSections.indexOf(secDragId) < movableSections.indexOf(id) ? 'after' : 'before';
  }
  function onSecDrop(e: DragEvent, id: string): void {
    if (!secDragId) return;
    e.preventDefault();
    const from = secDragId;
    secDragId = null;
    secDragOverId = null;
    if (from === id || !movableSections.includes(id)) return;
    const next = reorderAmong(resolveGroupOrder(ui.sidebarGroupOrder).map((g) => g.id as string), from, id);
    if (next) ui.setSidebarGroupOrder(next);
  }
  function onSecDragEnd(): void {
    secDragId = null;
    secDragOverId = null;
  }

  // ── Context menus ───────────────────────────────────────────────────────
  /** A module row's Favorites entries (+ in-Favorites moves). Shared by the
   *  plain rows and the Agents row, which prepends its own session verbs. */
  function favoriteMenuItems(m: SidebarModule): MenuItem[] {
    if (!isFav(m.id)) {
      return [{ label: 'Add to Favorites', icon: 'star', action: () => ui.addSidebarFavorite(m.id) }];
    }
    const i = favIds.indexOf(m.id);
    return [
      { label: 'Remove from Favorites', icon: 'star', action: () => ui.removeSidebarFavorite(m.id) },
      { label: 'Move up', icon: 'arrowUp', hint: '⌥↑', disabled: i <= 0, action: () => moveFavorite(m.id, -1) },
      { label: 'Move down', icon: 'arrowDown', hint: '⌥↓', disabled: i >= favIds.length - 1, action: () => moveFavorite(m.id, 1) },
    ];
  }
  const customizeItem = (): MenuItem => ({
    label: 'Customize sidebar',
    icon: 'edit',
    action: () => (ui.sidebarEditMode = true),
  });
  /** "Open side by side" & co. (stores/sidePane.svelte.ts), then a separator. */
  function splitItems(m: SidebarModule): MenuItem[] {
    const items = splitMenuItems(m.id, m.label);
    return items.length ? [...items, { separator: true }] : [];
  }
  function moduleMenu(e: MouseEvent, m: SidebarModule): void {
    ctxMenu.show(e, [...splitItems(m), ...favoriteMenuItems(m), { separator: true }, customizeItem()]);
  }
  function sectionMenu(e: MouseEvent, sec: SidebarSection): void {
    const id = sec.group.id;
    const open = sectionOpen(sec);
    const items: MenuItem[] = [
      {
        label: open ? 'Collapse section' : 'Expand section',
        icon: open ? 'chevronRight' : 'chevronDown',
        disabled: sectionPinned(sec),
        action: () => ui.toggleSidebarGroup(id),
      },
    ];
    if (id !== FAVORITES_ID) {
      items.push(
        { separator: true },
        { label: 'Move section up', icon: 'arrowUp', disabled: !canMoveSection(id, -1), action: () => moveSection(id, -1) },
        { label: 'Move section down', icon: 'arrowDown', disabled: !canMoveSection(id, 1), action: () => moveSection(id, 1) },
      );
    }
    items.push({ separator: true }, customizeItem());
    ctxMenu.show(e, items);
  }
</script>

<nav class="navigator sidebar-material" class:resizing aria-label="Navigator" style="width:{ui.railWidth}px">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="rail-resize"
    onmousedown={startResize}
    ondblclick={() => ui.setRailWidth(240)}
    title="Drag to resize · double-click to reset"
  ></div>
  <div class="nav-head" class:tauri-pad={false}>
    <img class="nav-logo" src="/otto-mark-64.png" alt="" width="20" height="20" />
    <span class="nav-title">Otto</span>
    <span class="grow"></span>
    <!-- Phone / tablet: the top bar carries Back / Forward (NavButtons) — a
         second pair here was duplicate chrome. -->
    {#if viewport.isDesktop}
    <button
      class="icon-btn nav-back"
      onclick={() => router.back()}
      disabled={!router.canBack}
      title="Back (⌘⇧←)"
      aria-label="Back"
    >
      <Icon name="chevronRight" size={14} />
    </button>
    <button
      class="icon-btn"
      onclick={() => router.forward()}
      disabled={!router.canForward}
      title="Forward (⌘⇧→)"
      aria-label="Forward"
    >
      <Icon name="chevronRight" size={14} />
    </button>
    {/if}
    <!-- The one notification bell on desktop/tablet: same spot on every page,
         so no module has to reserve room for a floating one. It sits at the
         header's inline-end, next to the sidebar edge its panel opens beside,
         so the panel's caret points straight back at it. -->
    <NotificationBell />
    <!-- On phone the Navigator is the off-canvas drawer: this closes it (the
         desktop collapse-to-Rail preference means nothing there). -->
    <!-- Tablet: the Navigator is a fixed column (no Rail to collapse to), so
         the button would do nothing there. -->
    {#if !viewport.isTablet}
    <button
      class="icon-btn"
      onclick={() => (viewport.isPhone ? (ui.navDrawerOpen = false) : ui.toggleRail())}
      title={viewport.isPhone ? 'Close sidebar' : 'Collapse sidebar (⌘1)'}
      aria-label={viewport.isPhone ? 'Close sidebar' : 'Collapse sidebar'}
    >
      <Icon name="sidebar" size={14} />
    </button>
    {/if}
  </div>

  <div class="nav-scroll" bind:this={scrollEl}>
    <!-- Global session search: filters every group below (Agents / Telegram /
         Slack). Hidden while editing the sidebar (no session lists shown then). -->
    {#if !ui.sidebarEditMode}
      <div class="nav-search">
        <Icon name="search" size={12} />
        <input
          class="nav-search-input"
          placeholder="Search all sessions…"
          aria-label="Search all sessions"
          bind:value={sessionQuery}
        />
        {#if sessionQuery}
          <button class="search-clear" onclick={() => (sessionQuery = '')} aria-label="Clear search" title="Clear search">
            <Icon name="x" size={11} />
          </button>
        {/if}
      </div>

      <!-- "Needs you" filter: narrows every group to sessions blocked on input.
           Only shown once at least one session is flagged (or while already on). -->
      {#if ws.needsYouCount > 0 || ws.needsYouFilter}
        <button
          class="needs-you-filter"
          class:active={ws.needsYouFilter}
          onclick={() => (ws.needsYouFilter = !ws.needsYouFilter)}
          title="Show only sessions waiting on you"
        >
          <Icon name="bell" size={11} />
          <span class="grow">Needs you</span>
          {#if ws.needsYouCount > 0}
            <span class="needs-you-count">{ws.needsYouCount}</span>
          {/if}
        </button>
      {/if}
    {/if}

    <div class="nav-section modules" data-testid="sidebar-modules">
      {#if ui.sidebarEditMode}
        <p class="edit-hint">Drag rows and section headers to reorder (or ⌥↑ / ⌥↓). Star to add to Favorites, eye to hide.</p>
      {/if}

      <!-- Modules render section by section (macOS source list), each section
           in the user's saved order (shared registry). A section header folds
           its rows away (persisted per device). While editing, every resolved
           module shows as a compact draggable row (incl. hidden ones, so they
           can be toggled back on); otherwise the special Agents block (with its
           nested session list) and plain rows render normally. Connections is a
           plain row — open connections live as tabs on the Agents view, so the
           sidebar doesn't repeat them. -->
      {#each sections as sec (sec.group.id)}
        {@const open = sectionOpen(sec)}
        {@const pinned = sectionPinned(sec)}
        {@const fav = sec.group.id === FAVORITES_ID}
        {@const secMovable = ui.sidebarEditMode && !fav}
        <div
          class="nav-group"
          class:favorites={fav}
          class:sec-dragging={secDragId === sec.group.id}
          class:drop-before={secDragOverId === sec.group.id && secDropSide === 'before'}
          class:drop-after={secDragOverId === sec.group.id && secDropSide === 'after'}
          role="group"
          aria-label={sec.group.label}
          data-testid={`sidebar-group-${sec.group.id}`}
          data-open={open}
          ondragover={secMovable ? (e) => onSecDragOver(e, sec.group.id) : undefined}
          ondragleave={secMovable ? () => { if (secDragOverId === sec.group.id) secDragOverId = null; } : undefined}
          ondrop={secMovable ? (e) => onSecDrop(e, sec.group.id) : undefined}
        >
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="group-head-row"
            draggable={secMovable}
            ondragstart={secMovable ? (e) => onSecDragStart(e, sec.group.id) : undefined}
            ondragend={secMovable ? onSecDragEnd : undefined}
          >
            {#if secMovable}
              <span class="grip sec-grip" title="Drag to reorder sections" aria-hidden="true"><Icon name="grip" size={12} /></span>
            {/if}
            <button
              class="group-head"
              class:pinned
              aria-expanded={open}
              onclick={() => !pinned && ui.toggleSidebarGroup(sec.group.id)}
              oncontextmenu={(e) => sectionMenu(e, sec)}
              title={pinned ? undefined : open ? `Hide ${sec.group.label}` : `Show ${sec.group.label}`}
              data-testid={`sidebar-group-head-${sec.group.id}`}
            >
              <span class="group-label">{sec.group.label}</span>
              {#if fav}<span class="group-star" aria-hidden="true"><Icon name="star" size={11} /></span>{/if}
              {#if !open && sec.modules.some((m) => m.id === 'agents') && ws.workingCount > 0}
                <span class="count-chip working" title="working sessions">{ws.workingCount}</span>
              {/if}
              {#if !pinned}
                <span class="group-chev"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} /></span>
              {/if}
            </button>
            {#if secMovable}
              <button
                class="row-action mv"
                onclick={() => moveSection(sec.group.id, -1, sectionBtn(sec.group.label, 'up'), sectionBtn(sec.group.label, 'down'))}
                disabled={!canMoveSection(sec.group.id, -1)}
                title="Move section up"
                aria-label={`Move ${sec.group.label} section up`}
              >
                <Icon name="arrowUp" size={12} />
              </button>
              <button
                class="row-action mv"
                onclick={() => moveSection(sec.group.id, 1, sectionBtn(sec.group.label, 'down'), sectionBtn(sec.group.label, 'up'))}
                disabled={!canMoveSection(sec.group.id, 1)}
                title="Move section down"
                aria-label={`Move ${sec.group.label} section down`}
              >
                <Icon name="arrowDown" size={12} />
              </button>
            {/if}
          </div>
          {#if open}
            {#each sec.modules as m, i (m.id)}
              {#if ui.sidebarEditMode}
                {@render editRow(m, i === 0, i === sec.modules.length - 1)}
              {:else if m.id === 'agents'}
                {@render agentsBlock(m)}
              {:else}
                {@render simpleRow(m)}
              {/if}
            {/each}
          {/if}
        </div>
      {/each}
    </div>

    <div class="nav-section">
      <div class="nav-label-row">
        <span class="nav-label">Workspaces</span>
        <button
          class="icon-btn add-ws"
          onclick={() => (ui.newWorkspaceOpen = true)}
          title="Add workspace"
          aria-label="Add workspace"
        >
          <Icon name="plus" size={14} />
        </button>
      </div>
      {#each ws.workspaces as w (w.id)}
        <button
          class="nav-item"
          class:active-ws={ws.currentId === w.id}
          onclick={() => ws.select(w.id)}
          oncontextmenu={(e) => ctxMenu.show(e, [
            { label: 'Switch to this workspace', icon: 'check', action: () => ws.select(w.id) },
            { separator: true },
            ...(w.my_role === 'admin' ? [
              { label: 'Rename…', icon: 'edit', action: () => void renameWorkspace(w) },
              { label: 'Change folder…', icon: 'folder', action: () => void changeWorkspaceDir(w) },
              { label: 'Remove workspace…', icon: 'trash', danger: true as const, action: () => void removeWorkspace(w) },
              { separator: true as const },
            ] : []),
            { label: 'Add workspace…', icon: 'plus', action: () => (ui.newWorkspaceOpen = true) },
            { label: 'Workspace context', icon: 'note', action: async () => { await ws.select(w.id); router.go('settings/context-soul'); } },
          ])}
          title={w.root_path}
        >
          <Icon name="folder" size={14} />
          <span class="grow ellipsis">{w.name}</span>
          {#if ws.currentId === w.id}<Icon name="check" size={12} />{/if}
        </button>
      {/each}
    </div>
  </div>

  <div class="nav-foot">
    {#if ui.sidebarEditMode}
      <!-- Customizing: a button row, not a nav row — "Done" drawn as the
           selected page read as if it were one. -->
      <div class="edit-foot">
        <button class="btn small" onclick={() => ui.resetSidebar()} data-testid="sidebar-reset" title="Restore the default order, sections and visibility, and clear Favorites">
          <Icon name="refresh" size={12} /> Reset to default
        </button>
        <span class="grow"></span>
        <button
          class="btn small primary"
          onclick={() => ui.toggleSidebarEdit()}
          title="Finish customizing the sidebar"
          data-testid="sidebar-edit-toggle"
        >
          Done
        </button>
      </div>
    {:else}
      <button
        class="nav-item subtle"
        onclick={() => ui.toggleSidebarEdit()}
        title="Show, hide and reorder sidebar items"
        data-testid="sidebar-edit-toggle"
      >
        <Icon name="edit" size={14} />
        <span class="grow">Customize sidebar</span>
      </button>
    {/if}
    <button
      class="nav-item"
      class:active={router.module === 'walkthroughs'}
      onclick={() => router.go('walkthroughs')}
    >
      <Icon name="info" size={14} />
      <span class="grow">Help</span>
    </button>
    <button
      class="nav-item"
      class:active={router.module === 'settings'}
      onclick={() => router.go('settings/appearance')}
    >
      <Icon name="gear" size={14} />
      <span class="grow">Settings</span>
    </button>
    <div class="nav-user">
      <span class="avatar">{(auth.me?.display_name ?? '?').slice(0, 1).toUpperCase()}</span>
      <div class="grow" title={auth.me?.display_name}>
        <div class="user-name ellipsis">{auth.me?.display_name}</div>
        <div class="user-sub ellipsis">{auth.isRoot ? 'root' : auth.me?.username}</div>
      </div>
      <button class="icon-btn" onclick={() => auth.logout()} title="Sign out" aria-label="Sign out">
        <Icon name="logout" size={14} />
      </button>
    </div>
  </div>
</nav>

{#if shareSessionId}
  <ShareModal sessionId={shareSessionId} onclose={() => (shareSessionId = null)} />
{/if}

<!-- The module the side-by-side pane shows: a quiet trailing glyph. -->
{#snippet sideMark(id: string)}
  {#if sidePane.showing && sidePane.key === id}
    <span class="side-mark" role="img" title="Open in the side pane" aria-label="Open in the side pane" data-testid={`side-mark-${id}`}>
      <Icon name="columns" size={12} />
    </span>
  {/if}
{/snippet}

{#snippet simpleRow(m: SidebarModule)}
  <button
    class="nav-item"
    class:active={isActive(m.id)}
    class:drop-before={dragOverId === m.id && dropSide === 'before'}
    class:drop-after={dragOverId === m.id && dropSide === 'after'}
    class:dragging={dragId === m.id}
    data-nav-id={m.id}
    title={sidePane.supported ? SPLIT_HINT : undefined}
    onclick={(e) => navClick(e, m.id, m.label)}
    oncontextmenu={(e) => moduleMenu(e, m)}
    {...favRowProps(m.id)}
  >
    <Icon name={m.icon} size={14} />
    <span class="grow">{m.label}</span>
    {@render sideMark(m.id)}
    {#if m.id === 'workflows' && ws.activeWorkflowRuns.length > 0}
      <span class="count-chip working" title="running workflows">{ws.activeWorkflowRuns.length}</span>
    {/if}
    {#if m.id === 'assistant' && assistant.needsYouCount > 0}
      <span class="count-chip needs" title={`${assistant.needsYouCount} waiting on you`} data-testid="assistant-needs-badge">{assistant.needsYouCount}</span>
    {/if}
  </button>
{/snippet}

{#snippet editRow(m: SidebarModule, first: boolean, last: boolean)}
  {@const fav = isFav(m.id)}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="edit-row"
    class:hidden-row={isHidden(m.id)}
    class:dragging={dragId === m.id}
    class:drop-before={dragOverId === m.id && dropSide === 'before'}
    class:drop-after={dragOverId === m.id && dropSide === 'after'}
    draggable={true}
    ondragstart={(e) => onDragStart(e, m.id)}
    ondragover={(e) => onDragOver(e, m.id)}
    ondragleave={() => onDragLeave(m.id)}
    ondrop={(e) => onDrop(e, m.id)}
    ondragend={onDragEnd}
    oncontextmenu={(e) => ctxMenu.show(e, favoriteMenuItems(m))}
    onkeydown={(e) => {
      // ⌥↑ / ⌥↓ from any control in the row (the arrows show on hover /
      // focus, so the label keeps its room).
      if (!e.altKey || e.metaKey || e.ctrlKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
      if ((e.key === 'ArrowUp' && first) || (e.key === 'ArrowDown' && last)) return;
      e.preventDefault();
      move(m, e.key === 'ArrowUp' ? -1 : 1);
    }}
    data-testid={`sidebar-edit-row-${m.id}`}
  >
    <span class="grip" title="Drag to reorder" aria-hidden="true"><Icon name="grip" size={14} /></span>
    <Icon name={m.icon} size={14} />
    <span class="grow ellipsis">{m.label}</span>
    <span class="edit-actions">
      <button
        class="row-action star-toggle"
        class:on={fav}
        onclick={() => toggleFavorite(m.id)}
        title={fav ? 'Remove from Favorites' : 'Add to Favorites'}
        aria-label={`Favorite ${m.label}`}
        aria-pressed={fav}
        data-testid={`sidebar-fav-${m.id}`}
      >
        <Icon name="star" size={13} />
      </button>
      <button
        class="row-action mv"
        onclick={() => move(m, -1)}
        disabled={first}
        title="Move up (⌥↑)"
        aria-label={`Move ${m.label} up`}
      >
        <Icon name="arrowUp" size={12} />
      </button>
      <button
        class="row-action mv"
        onclick={() => move(m, 1)}
        disabled={last}
        title="Move down (⌥↓)"
        aria-label={`Move ${m.label} down`}
      >
        <Icon name="arrowDown" size={12} />
      </button>
      <button
        class="row-action"
        onclick={() => ui.toggleSidebarHidden(m.id)}
        title={isHidden(m.id) ? 'Show' : 'Hide'}
        aria-label={isHidden(m.id) ? `Show ${m.label}` : `Hide ${m.label}`}
        data-testid={`sidebar-hide-${m.id}`}
      >
        <Icon name={isHidden(m.id) ? 'eyeOff' : 'eye'} size={13} />
      </button>
    </span>
  </div>
{/snippet}

{#snippet agentsBlock(m: SidebarModule)}
  <div
    class="nav-item-row agents-row"
    class:drop-before={dragOverId === m.id && dropSide === 'before'}
    class:drop-after={dragOverId === m.id && dropSide === 'after'}
    class:dragging={dragId === m.id}
  >
    <button
      class="nav-item"
      class:active={router.module === 'agents' || router.module === ''}
      data-nav-id={m.id}
      title={sidePane.supported ? SPLIT_HINT : undefined}
      onclick={(e) => navClick(e, 'agents', m.label)}
      oncontextmenu={(e) => ctxMenu.show(e, [
        { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
        { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
        { label: 'Add workspace…', icon: 'folder', action: () => (ui.newWorkspaceOpen = true) },
        { separator: true },
        ...splitItems(m),
        ...favoriteMenuItems(m),
        { separator: true },
        customizeItem(),
      ])}
      {...favRowProps(m.id)}
    >
      <Icon name="terminal" size={14} />
      <span class="grow">Agents</span>
      {@render sideMark('agents')}
      {#if ws.workingCount > 0}
        <span class="count-chip working">{ws.workingCount}</span>
      {/if}
    </button>
    {#if selectable.some((s) => ws.canEditSession(s))}
      <button
        class="icon-btn twisty sel-toggle"
        class:on={agentSelMode}
        onclick={() => setAgentSelMode(!agentSelMode)}
        title={agentSelMode ? 'Done selecting' : 'Select sessions to archive or delete'}
        aria-label="Select sessions"
        aria-pressed={agentSelMode}
        data-testid="agents-select-toggle"
      >
        <Icon name={agentSelMode ? 'check' : 'square'} size={12} />
      </button>
    {/if}
    {#if fAgents.length > 1}
      <button
        class="icon-btn twisty sort-toggle"
        class:on={sessionOrder.mode === 'manual'}
        onclick={openSortMenu}
        onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && openSortMenu(e)}
        title={sessionOrder.mode === 'manual' ? 'Manual order — click to change' : 'Sort sessions'}
        aria-label="Sort sessions"
        data-testid="agents-sort-toggle"
      >
        <Icon name={sessionOrder.mode === 'manual' ? 'grip' : 'arrowDown'} size={12} />
      </button>
    {/if}
    <button
      class="icon-btn twisty all-ws-toggle"
      class:on={ws.allWorkspaces}
      onclick={() => ws.setAllWorkspaces(!ws.allWorkspaces)}
      title={ws.allWorkspaces
        ? 'Showing sessions from all workspaces — click for current only'
        : 'Show sessions from all workspaces'}
      aria-label="Toggle all-workspaces session list"
      aria-pressed={ws.allWorkspaces}
    >
      <Icon name="globe" size={12} />
    </button>
    <button
      class="icon-btn twisty"
      onclick={() => (agentsOpen = !agentsOpen)}
      aria-label="Toggle session list"
      aria-expanded={agentsOpen}
      title={agentsOpen ? 'Hide sessions' : 'Show sessions'}
    >
      <Icon name={agentsOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  </div>

  <!-- Everything Agents owns (its session lists, channel groups, archive)
       hangs off its row on one outline guide, so it reads as Agents' content
       rather than more sections. -->
  <div class="agents-sub">
    {#if q ? fAgents.length > 0 : agentsOpen}
      <div class="nested" data-testid="agents-list">
        {#if agentSelMode}
          <div class="arch-tools" data-testid="agents-select-tools">
            <label class="arch-all" title="Select all sessions">
              <input type="checkbox" aria-label="Select all sessions" checked={agentSelIds.length > 0 && agentSelIds.length === selectable.length} indeterminate={agentSelIds.length > 0 && agentSelIds.length < selectable.length} onchange={agentSelectAll} />
              <span>{agentSelIds.length > 0 ? `${agentSelIds.length} selected` : 'Select all'}</span>
            </label>
            <button class="row-action arch-del-sel" disabled={agentSelIds.length === 0} title="Archive selected sessions" aria-label="Archive selected sessions" data-testid="agents-archive-selected" onclick={() => void archiveSelectedAgents()}>
              <Icon name="archive" size={11} /><span>Archive</span>
            </button>
            <button class="row-action danger arch-del-sel" disabled={agentSelIds.length === 0} title="Delete selected sessions" aria-label="Delete selected sessions" data-testid="agents-delete-selected" onclick={() => void deleteSelectedAgents()}>
              <Icon name="trash" size={11} /><span>Delete</span>
            </button>
          </div>
        {/if}
        {#each fAgents as s (s.id)}
          {@render sessionRow(s, undefined, true)}
        {:else}
          <div class="nested-empty">No sessions — ⌘T to start one</div>
        {/each}
      </div>
    {/if}

    <!-- Workspace-less sessions (the daemon's hidden scratch workspace): one
         group in every workspace and with none. Plain `sessionRow`s — they are
         already in `ws.sessions`, so open / rename / archive work as above. -->
    {#if q ? fScratch.length > 0 : agentsOpen && (ws.scratchSessions.length > 0 || ws.current === null)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="ws-group-label"
        title="Sessions not tied to any workspace"
        data-testid="scratch-group"
        oncontextmenu={(e) => ctxMenu.show(e, [
          { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
        ])}
      >
        <Icon name="home" size={11} />
        <span class="ellipsis">No workspace</span>
      </div>
      <div class="nested">
        {#each fScratch as s (s.id)}
          {@render sessionRow(s)}
        {:else}
          <div class="nested-empty">No sessions — ⌘T, then “No workspace”</div>
        {/each}
      </div>
    {/if}

    <!-- All-workspaces view: sessions living in OTHER workspaces, grouped by
         workspace (the current one keeps its flat list above). Clicking a row
         switches to that workspace and focuses the session. -->
    {#if ws.allWorkspaces && (agentsOpen || q)}
      {#each ws.otherWsGroups as g (g.ws.id)}
        {@const rows = g.sessions.filter(matches)}
        {#if rows.length > 0}
          <div class="ws-group-label" title="Sessions in workspace “{g.ws.name}”">
            <Icon name="folder" size={11} />
            <span class="ellipsis">{g.ws.name}</span>
          </div>
          <div class="nested">
            {#each rows as s (s.id)}
              {@render sessionRow(s, g.ws.id)}
            {/each}
          </div>
        {/if}
      {/each}
    {/if}

    {#if q ? fTelegram.length > 0 : ws.telegramSessions.length > 0}
      <div class="nav-item-row">
        <button class="nav-item" onclick={() => (telegramOpen = !telegramOpen)}>
          <Icon name="send" size={14} />
          <span class="grow">Telegram</span>
          <span class="count-chip">{ws.telegramSessions.length}</span>
        </button>
        <button
          class="icon-btn twisty"
          onclick={() => (telegramOpen = !telegramOpen)}
          aria-label="Toggle Telegram list"
          aria-expanded={telegramOpen}
          title={telegramOpen ? 'Hide Telegram sessions' : 'Show Telegram sessions'}
        >
          <Icon name={telegramOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        </button>
      </div>
      {#if telegramOpen || q}
        <div class="nested">
          {#each visTelegram as s (s.id)}
            {@render sessionRow(s)}
          {:else}
            <div class="nested-empty">No matching</div>
          {/each}
          {#if !q && fTelegram.length > CHANNEL_CAP}
            <button class="show-more" onclick={() => (telegramShowAll = !telegramShowAll)}>
              {telegramShowAll ? 'Show less' : `Show ${fTelegram.length - CHANNEL_CAP} more`}
            </button>
          {/if}
        </div>
      {/if}
    {/if}

    {#if q ? fSlack.length > 0 : ws.slackSessions.length > 0}
      <div class="nav-item-row">
        <button class="nav-item" onclick={() => (slackOpen = !slackOpen)}>
          <Icon name="slack" size={14} />
          <span class="grow">Slack</span>
          <span class="count-chip">{ws.slackSessions.length}</span>
        </button>
        <button
          class="icon-btn twisty"
          onclick={() => (slackOpen = !slackOpen)}
          aria-label="Toggle Slack list"
          aria-expanded={slackOpen}
          title={slackOpen ? 'Hide Slack sessions' : 'Show Slack sessions'}
        >
          <Icon name={slackOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        </button>
      </div>
      {#if slackOpen || q}
        <div class="nested">
          {#each visSlack as s (s.id)}
            {@render sessionRow(s)}
          {:else}
            <div class="nested-empty">No matching</div>
          {/each}
          {#if !q && fSlack.length > CHANNEL_CAP}
            <button class="show-more" onclick={() => (slackShowAll = !slackShowAll)}>
              {slackShowAll ? 'Show less' : `Show ${fSlack.length - CHANNEL_CAP} more`}
            </button>
          {/if}
        </div>
      {/if}
    {/if}

    <!-- Archived sessions: parked rows under the Agents lists (restore /
         delete), folded by default. -->
    {#if ws.archivedSessions.length > 0}
      <button class="nav-item subtle" onclick={() => (archivedOpen = !archivedOpen)}>
        <Icon name="archive" size={14} />
        <span class="grow">Archived</span>
        <span class="count-chip">{ws.archivedSessions.length}</span>
        <Icon name={archivedOpen ? 'chevronDown' : 'chevronRight'} size={11} />
      </button>
      {#if archivedOpen}
        <div class="nested">
          {#if ws.myRole !== 'viewer' && ws.archivedSessions.length > 1}
            <div class="arch-tools">
              <label class="arch-all" title="Select all archived sessions">
                <input type="checkbox" aria-label="Select all archived sessions" checked={archSelCount > 0 && archSelCount === ws.archivedSessions.length} indeterminate={archSelCount > 0 && archSelCount < ws.archivedSessions.length} onchange={archSelectAll} />
                <span>{archSelCount > 0 ? `${archSelCount} selected` : 'Select all'}</span>
              </label>
              <button class="row-action danger arch-del-sel" disabled={archSelCount === 0} title="Delete selected sessions" aria-label="Delete selected sessions" data-testid="archived-delete-selected" onclick={() => void deleteSelectedArchived()}>
                <Icon name="trash" size={11} /><span>Delete{archSelCount > 0 ? ` (${archSelCount})` : ''}</span>
              </button>
            </div>
          {/if}
          {#each ws.archivedSessions as s (s.id)}
            <div class="nested-row" class:selected={archSel.has(s.id)}>
              {#if ws.canEditSession(s)}
                <input type="checkbox" class="arch-check" checked={archSel.has(s.id)} onchange={() => toggleArchSel(s.id)} aria-label="Select {s.title}" />
              {/if}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="nav-item nested-item archived"
                title={s.title}
                oncontextmenu={(e) => ctxMenu.show(e, [
                  ...(ws.canEditSession(s) ? [
                    { label: 'Unarchive', icon: 'refresh', action: () => ws.unarchiveSession(s.id) },
                    { label: 'Delete', icon: 'trash', danger: true as const, action: () => void deleteSession(s.id) },
                  ] : []),
                  { separator: true },
                  { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
                  { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
                ])}
              >
                <StatusDot status="exited" />
                <span class="grow ellipsis">{s.title}</span>
                {#if hasProviderIcon(s.provider)}
                  <span class="provider-ico" title={s.provider}><ProviderIcon provider={s.provider} size={13} /></span>
                {:else}
                  <span class="provider">{s.provider}</span>
                {/if}
              </div>
              {#if ws.canEditSession(s)}
                <button class="row-action" title="Restore" aria-label="Restore session" onclick={() => ws.unarchiveSession(s.id)}>
                  <Icon name="refresh" size={11} />
                </button>
                <button class="row-action danger" title="Delete" aria-label="Delete session" onclick={() => void deleteSession(s.id)}>
                  <Icon name="trash" size={11} />
                </button>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
{/snippet}

{#snippet sessionRow(s: Session, otherWs?: string, reorderable = false)}
  {@const status = ws.statusMap[s.id] ?? s.status}
  {@const sum = activity.summary(s.id)}
  {@const proofRow = proof.summaryFor('session', s.id)}
  {@const needsYou = ws.needsYou[s.id] === true}
  {@const st = sessionState(s, status, needsYou, { stale: staleEvents })}
  {@const resumable = st.resumable}
  {@const dnd = rowsDraggable && reorderable}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="nested-row"
    class:needs-you={st.key === 'needs-you'}
    class:selected={agentSelMode && agentSel.has(s.id)}
    class:drag-over={rowDragOverId === s.id}
    draggable={dnd}
    ondragstart={dnd ? (e) => onRowDragStart(e, s.id) : undefined}
    ondragover={dnd ? (e) => onRowDragOver(e, s.id) : undefined}
    ondragleave={() => { if (rowDragOverId === s.id) rowDragOverId = null; }}
    ondrop={dnd ? (e) => onRowDrop(e, s.id) : undefined}
    ondragend={onRowDragEnd}
  >
    {#if agentSelMode && !otherWs}
      <input type="checkbox" class="arch-check" checked={agentSel.has(s.id)} onchange={() => toggleAgentSel(s.id)} aria-label="Select {s.title}" />
    {/if}
    {#if renamingId === s.id}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="nav-rename"
        bind:value={draft}
        autofocus
        onblur={commitRename}
        onkeydown={(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renamingId = null;
        }}
      />
    {:else}
      <button
        class="nav-item nested-item"
        class:active={!otherWs && router.module === 'agents' && ws.activeSessionId === s.id}
        class:resumable
        onclick={() => (otherWs ? void ws.openInWorkspace(otherWs, s.id) : openSession(s.id))}
        ondblclick={() => startRename(s.id, s.title)}
        oncontextmenu={(e) => ctxMenu.show(e, [
          // Same order as the tab and pane menus: Rename · Share… · Open in
          // new window, then Restart · Archive · Delete, then New session.
          ...(ws.canEditSession(s) ? [{ label: 'Rename', icon: 'edit', action: () => startRename(s.id, s.title) }] : []),
          ...(otherWs ? [] : [{ label: 'Share…', icon: 'share', action: () => (shareSessionId = s.id) }]),
          ...(otherWs ? [] : popoutItems(`agents/${s.id}`, s.title)),
          ...(reorderable && fAgents.length > 1
            ? [{ separator: true }, { label: 'Move to top', icon: 'arrowUp', action: () => { const ids = fAgents.map((x) => x.id); sessionOrder.dragTo(ids, s.id, ids[0]); } }]
            : []),
          { separator: true },
          ...(ws.canEditSession(s) ? [
            // In-progress agent only: respawn a stuck PTY (provider resume when
            // possible). Idle/exited/reconnectable sessions have their own paths.
            ...(s.kind === 'agent' && (status === 'running' || status === 'working')
              ? [{ label: 'Restart session', icon: 'refresh', action: () => void restartAgent(s.id) }]
              : []),
            { label: 'Archive', icon: 'archive', action: () => ws.archiveSession(s.id) },
            { label: 'Delete', icon: 'trash', danger: true as const, action: () => void deleteSession(s.id) },
          ] : []),
          { separator: true },
          { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
          { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
        ])}
        title={rowTip(s.title, st, sum)}
        data-state={st.key}
      >
        <!-- Row = state dot · title · (needs-you bell) · provider. Task and
             proof roll-ups are secondary: in the tooltip, and revealed on
             hover / the active row so the list stays scannable. -->
        {#if resumable}
          <span class="susp-dot" role="img" aria-label={st.label} title={st.hint}>
            <Icon name="refresh" size={10} />
          </span>
        {:else}
          <StatusDot state={st} />
        {/if}
        <span class="grow ellipsis">{s.title}</span>
        {#if st.key === 'needs-you'}
          <span class="needs-you-dot" role="img" title="Waiting on you" aria-label="Needs you">
            <Icon name="bell" size={10} />
          </span>
        {/if}
        {#if (sum && sum.total > 0) || proofRow}
          <span class="row-secondary">
            {#if sum && sum.total > 0}
              <span
                class="task-chip"
                class:done={sum.done === sum.total}
                class:active={sum.in_progress != null}
                title={sum.in_progress ? `Now: ${sum.in_progress}` : `${sum.done}/${sum.total} tasks done`}
              >{sum.done}/{sum.total}</span>
            {/if}
            {#if proofRow}
              <ProofStatusChip status={proofRow.status} risk={proofRow.risk_score} compact />
            {/if}
          </span>
        {/if}
        {#if hasProviderIcon(s.provider)}
          <span class="provider-ico" title={s.provider}><ProviderIcon provider={s.provider} size={13} /></span>
        {:else}
          <span class="provider">{s.provider}</span>
        {/if}
      </button>
      {#if ws.canEditSession(s)}
        <button
          class="row-action"
          title={ws.closeTabTitle(s.id, 'session')}
          aria-label="Close session"
          onclick={() => void ws.requestCloseTab(s.id)}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    {/if}
  </div>
{/snippet}

<style>
  .nested-row.drag-over {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .nested-row[draggable='true'] .nested-item {
    cursor: grab;
  }
  .navigator {
    /* width is set inline from ui.railWidth (drag-resizable) */
    height: 100%;
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--separator);
    position: relative;
  }
  .rail-resize {
    position: absolute;
    inset-inline-end: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: var(--z-sticky);
  }
  .rail-resize:hover,
  .navigator.resizing .rail-resize {
    background: linear-gradient(
      to right,
      transparent 0,
      color-mix(in srgb, var(--accent) 40%, transparent) 45%,
      color-mix(in srgb, var(--accent) 40%, transparent) 55%,
      transparent 100%
    );
  }
  .nav-head {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: 10px 6px;
    padding-inline: 14px 10px;
  }
  .nav-head .nav-title {
    margin-inline-start: 6px;
  }
  .nav-logo {
    border-radius: 5px;
    display: block;
    flex-shrink: 0;
  }
  .nav-back :global(svg) {
    transform: scaleX(-1);
  }
  .nav-head :global(.icon-btn:disabled) {
    opacity: 0.3;
    cursor: default;
  }
  .nav-title {
    font-size: var(--fs-m);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .nav-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 8px;
  }
  .nav-section {
    margin-bottom: 14px;
  }
  .nav-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
    padding: 6px 8px 4px;
  }
  .nav-label-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-inline-end: 4px;
  }
  .nav-label-row .nav-label {
    flex: 1;
  }
  .add-ws {
    width: 22px;
    height: 22px;
    color: var(--text-dim);
  }
  .add-ws:hover {
    color: var(--text);
  }
  .nav-item-row {
    display: flex;
    align-items: center;
  }
  .nav-item-row .nav-item {
    flex: 1;
  }
  .twisty {
    width: 20px;
    height: 20px;
  }
  .nav-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-m);
    cursor: pointer;
    text-align: start;
    transition: background 120ms ease-out;
  }
  .nav-item:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  /* Module / group labels ellipsize in a narrow (resized) sidebar instead of
     running under the count chips and toggles beside them. */
  .nav-item > .grow {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .nav-item > .count-chip {
    flex-shrink: 0;
  }
  /* Selection = the theme accent as a TINT (never a solid fill: --text on a
     solid --accent is unreadable in several themes) + a short accent bar at the
     inline-start edge and an accent icon. The label stays --text, which clears
     contrast on the tint in every theme × scheme. */
  .nav-item.active {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 600;
  }
  /* The module shown in the side-by-side pane (not the page itself). */
  .side-mark {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .nav-item.active::before {
    content: '';
    position: absolute;
    inset-inline-start: 0;
    inset-block: 6px;
    width: 3px;
    border-radius: 0 2px 2px 0;
    background: var(--accent);
  }
  :global([dir='rtl']) .nav-item.active::before {
    border-radius: 2px 0 0 2px;
  }
  .nav-item.active > :global(svg) {
    color: var(--accent-text);
  }
  /* Agents' row carries trailing toggles (select / sort / all-workspaces /
     fold): tint the WHOLE row so the selection doesn't stop short of them. */
  .nav-item-row:has(> .nav-item.active) {
    background: var(--accent-soft);
    border-radius: var(--radius-s);
  }
  .nav-item-row > .nav-item.active {
    background: transparent;
  }
  /* A focused session row is the quieter variant: a lighter tint, no bar —
     the Agents row above it already carries the strong marker. */
  .nav-item.nested-item.active {
    background: color-mix(in srgb, var(--accent) 11%, transparent);
    font-weight: 500;
  }
  .nav-item.nested-item.active::before {
    content: none;
  }
  /* ── Sections (macOS source-list headers) ───────────────────────────── */
  .nav-group + .nav-group {
    margin-top: 10px;
  }
  .group-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 22px;
    padding: 0 6px 0 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    cursor: pointer;
    text-align: start;
  }
  .group-head.pinned {
    cursor: default;
  }
  /* Header row: the fold button, plus (while editing) a grip and the section
     up/down controls. */
  .group-head-row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius-s);
  }
  .group-head-row > .group-head {
    flex: 1;
    min-width: 0;
  }
  .group-head-row[draggable='true'] {
    cursor: grab;
  }
  .group-head-row[draggable='true']:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  /* Section arrows line up with the rows' up/down columns: the last one
     skips the rows' eye column (22px) + their inline-end padding + border. */
  .group-head-row .row-action {
    width: 22px;
    height: 20px;
    opacity: 1;
  }
  .group-head-row .row-action:last-child {
    margin-inline-end: 27px;
  }
  /* The module list is a size container so edit mode can make room for the
     label in a narrow (resized) sidebar: the module glyph (the grip + label
     identify the row) goes, and the controls tighten. */
  .nav-section.modules {
    container: navmods / inline-size;
  }
  @container navmods (max-width: 220px) {
    .edit-row > :global(svg) {
      display: none;
    }
    .edit-actions .row-action,
    .group-head-row .row-action {
      width: 20px;
    }
    .group-head-row .row-action:last-child {
      margin-inline-end: 25px;
    }
  }
  .group-head-row .row-action:disabled {
    opacity: 0.25;
    cursor: default;
  }
  .sec-grip {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    margin-inline-start: 4px;
    color: var(--text-dim);
  }
  .group-head-row .sec-grip + .group-head {
    padding-inline-start: 2px;
  }
  /* Favorites' star trails its label, so every section label starts on the
     same x (a leading star pushed "FAVORITES" 18px in from "WORK"). */
  .group-star {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .group-label {
    min-width: 0;
    margin-inline-end: auto;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The label (or Favorites' trailing star) pushes the count + chevron to
     the inline end. */
  .group-label:has(+ .group-star) {
    margin-inline-end: 0;
  }
  .group-star {
    margin-inline-end: auto;
  }
  /* Chevron appears on hover / keyboard focus (like Finder's "Hide"), and
     stays visible while the section is folded so the state reads at a glance. */
  .group-chev {
    display: grid;
    place-items: center;
    opacity: 0;
    transition: opacity 120ms ease-out;
  }
  .group-head:hover .group-chev,
  .group-head:focus-visible .group-chev,
  .group-head[aria-expanded='false'] .group-chev {
    opacity: 1;
  }
  .group-head:not(.pinned):hover {
    color: var(--text);
  }
  .nav-item.active-ws {
    font-weight: 600;
  }
  /* ── Sidebar edit mode ──────────────────────────────────────────────── */
  .edit-hint {
    margin: 2px 6px 8px;
    padding: 6px 8px;
    font-size: var(--fs-xs);
    line-height: 1.4;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border-radius: var(--radius-s);
  }
  .edit-row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 4px 0 6px;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-m);
    cursor: grab;
    border: 1px solid transparent;
  }
  /* The up/down arrows show on hover / keyboard focus (⌥↑ / ⌥↓ work from
     any control in the row), so the label isn't squeezed to "Connectio…" by
     four always-on buttons. Touch screens have no hover: always shown. */
  /* Collapsed, not display:none: they stay in the accessibility tree and in
     the Tab order (focusing one opens the row up). */
  /* `.row-action.mv` out-specifies the generic `.row-action` sizes/opacity
     below (and the narrow-sidebar container query) — those used to win,
     leaving a 0-width button whose arrow was squashed into a sliver. */
  .edit-row .row-action.mv,
  .group-head-row .row-action.mv {
    width: 0;
    min-width: 0;
    padding: 0;
    opacity: 0;
    overflow: hidden;
  }
  .edit-row:hover .row-action.mv,
  .edit-row:focus-within .row-action.mv,
  .group-head-row:hover .row-action.mv,
  .group-head-row:focus-within .row-action.mv {
    width: 22px;
    opacity: 1;
  }
  .edit-row:hover .row-action.mv:disabled,
  .edit-row:focus-within .row-action.mv:disabled,
  .group-head-row:hover .row-action.mv:disabled,
  .group-head-row:focus-within .row-action.mv:disabled {
    opacity: 0.25;
  }
  @media (hover: none) {
    .edit-row .row-action.mv,
    .group-head-row .row-action.mv {
      width: 22px;
      opacity: 1;
    }
    .edit-row .row-action.mv:disabled,
    .group-head-row .row-action.mv:disabled {
      opacity: 0.25;
    }
  }
  .edit-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  /* Drop indicator: an accent line on the edge the dragged row will land
     on (before when dragging up / in from another section, after when
     dragging down); the row being dragged dims. Shared by module rows, the
     Agents row and whole sections. */
  .drop-before {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .drop-after {
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .edit-row.dragging,
  .nav-item.dragging,
  .nav-item-row.dragging,
  .nav-group.sec-dragging {
    opacity: 0.5;
  }
  .edit-row .grow {
    min-width: 0;
  }
  /* The row's controls sit tight together so the label keeps its room in a
     narrow (resized) sidebar. */
  .edit-actions {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
  /* Favorites toggle: an outline star, filled in the accent when on (the
     state is also aria-pressed + the tooltip, never colour alone). */
  .star-toggle.on {
    color: var(--accent-text);
  }
  .star-toggle.on :global(svg path),
  .group-star :global(svg path) {
    fill: currentColor;
  }
  /* Hidden modules stay listed while editing, dimmed, so they can be re-shown. */
  .edit-row.hidden-row {
    opacity: 0.45;
  }
  .edit-row .grip {
    display: grid;
    place-items: center;
    color: var(--text-dim);
    cursor: grab;
    flex-shrink: 0;
  }
  /* Edit-row action buttons are always visible (unlike the hover-revealed
     session row-actions). */
  .edit-row .row-action {
    opacity: 1;
  }
  .edit-row .row-action:disabled {
    opacity: 0.25;
    cursor: default;
  }
  .nested {
    margin: 2px 0 6px;
    padding-inline-start: 10px;
  }
  /* Agents' sub-content hangs off one hairline guide under the Agents icon
     (8px row padding + half the 14px icon), outline-view style. */
  .agents-sub {
    margin-inline-start: 15px;
    padding-inline-start: 4px;
    border-inline-start: 1px solid color-mix(in srgb, var(--text-dim) 22%, transparent);
  }
  .agents-sub .nested {
    padding-inline-start: 0;
    margin: 1px 0 4px;
  }
  /* A workspace-name sub-heading inside Agents (No workspace / other
     workspaces). Sentence case, so it never reads as a sidebar section. */
  .ws-group-label {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px 2px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    min-width: 0;
  }
  .all-ws-toggle.on {
    color: var(--accent-text);
  }
  /* Agents' row tools (select / sort / all workspaces) show on hover or
     keyboard focus, like Finder's row accessories — ON states stay visible,
     since they change what the list below shows. The fold chevron stays. */
  .agents-row > .twisty:not(.on):not([aria-expanded]) {
    opacity: 0;
    transition: opacity 120ms ease-out;
  }
  .agents-row:hover > .twisty:not(.on):not([aria-expanded]),
  .agents-row:focus-within > .twisty:not(.on):not([aria-expanded]) {
    opacity: 1;
  }
  @media (hover: none) {
    .agents-row > .twisty:not(.on):not([aria-expanded]) {
      opacity: 1;
    }
  }

  .nested-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .nested-row .nested-item {
    flex: 1;
    min-width: 0;
  }
  .nested-item {
    height: 26px;
    font-size: var(--fs-s);
  }
  .nested-item.archived {
    opacity: 0.65;
    cursor: default;
  }
  /* Suspended / resumable: parked to save memory, instantly reopenable.
     Dimmed (calm, not an error) but kept fully listed + clickable. */
  .nested-item.resumable:not(.active) {
    opacity: 0.72;
  }
  .nested-item.resumable:not(.active):hover {
    opacity: 1;
  }
  /* ↻ = suspended, resumes on open. Calm (dim), never amber — amber is
     reserved for "needs you". */
  .susp-dot {
    display: grid;
    place-items: center;
    width: 10px;
    height: 10px;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  /* Secondary roll-ups (tasks, proof): revealed on hover / focus / the active
     row; always summarised in the row tooltip. */
  .row-secondary {
    display: none;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .nested-item:hover .row-secondary,
  .nested-item:focus-visible .row-secondary,
  .nested-item.active .row-secondary {
    display: inline-flex;
  }
  .arch-tools { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 6px; padding: 2px 6px 4px 8px; font-size: var(--fs-xs); color: var(--text-dim); }
  .arch-all { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; cursor: pointer; }
  .arch-all input, .arch-check { margin: 0; accent-color: var(--accent); }
  .arch-check { flex-shrink: 0; }
  .sel-toggle.on { color: var(--accent-text); }
  .nested-row.selected .nested-item { background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .row-action {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease-out;
  }
  .nested-row:hover .row-action {
    opacity: 1;
  }
  /* Hover-revealed, but a keyboard user tabbing onto one must see it (and
     its focus ring) — it was an invisible focused button. */
  .row-action:focus-visible {
    opacity: 1;
  }
  .row-action:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .row-action.danger:hover {
    color: var(--danger);
  }
  /* Text-bearing toolbar actions (bulk Archive / Delete): declared AFTER
     .row-action so they beat its fixed 22px grid square — otherwise the label
     wrapped under the icon and spilled past the sidebar edge. */
  .arch-tools .row-action {
    display: inline-flex; align-items: center; gap: 4px; width: auto; height: 22px;
    padding: 0 7px; flex-shrink: 0; opacity: 1;
    border: 1px solid var(--border); border-radius: var(--radius-s, 4px); white-space: nowrap;
  }
  .arch-tools .row-action:disabled { opacity: 0.4; cursor: default; }
  .arch-tools .row-action.danger:not(:disabled) { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 40%, transparent); }
  .nav-item.subtle {
    color: var(--text-dim);
  }
  .nav-rename {
    flex: 1;
    height: 24px;
    font-size: var(--fs-s);
    background: var(--surface-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 0 6px;
    margin: 1px 0;
    outline: none;
  }
  .nav-search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 2px 4px 4px;
    padding: 0 8px;
    height: 26px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text-dim);
  }
  .nav-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .search-clear {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }
  .search-clear:hover {
    color: var(--text);
  }
  .nested-empty {
    padding: 4px 10px 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .show-more {
    display: block;
    width: 100%;
    text-align: start;
    padding: 4px 10px 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: none;
    border: none;
    cursor: pointer;
  }
  .show-more:hover {
    color: var(--text);
  }
  .provider {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .provider-ico {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
  }
  /* Per-session task roll-up: "done/total". Accent while a task is in progress,
     green when all complete. */
  .task-chip {
    flex-shrink: 0;
    padding: 0 5px;
    height: 16px;
    line-height: 16px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    background: var(--surface-2);
  }
  .task-chip.active {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .task-chip.done {
    color: var(--success);
    background: var(--success-soft);
  }
  /* "Needs you" — sticky flag for a session blocked on operator input. Amber to
     stand out from the calmer status colors, without shouting. */
  .needs-you-dot {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    border-radius: 99px;
    color: var(--warning);
    background: color-mix(in srgb, var(--status-warn) 18%, transparent);
  }
  .nested-row.needs-you .nested-item:not(.active) {
    box-shadow: inset 2px 0 0 var(--status-warn);
  }
  .needs-you-filter {
    display: flex;
    align-items: center;
    gap: 8px;
    width: calc(100% - 8px);
    margin: 0 4px 6px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid color-mix(in srgb, var(--status-warn) 40%, transparent);
    background: color-mix(in srgb, var(--status-warn) 8%, transparent);
    border-radius: var(--radius-s);
    color: var(--warning);
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .needs-you-filter:hover {
    background: color-mix(in srgb, var(--status-warn) 14%, transparent);
  }
  .needs-you-filter.active {
    background: color-mix(in srgb, var(--status-warn) 22%, transparent);
  }
  .needs-you-count {
    min-width: 16px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 700;
    display: grid;
    place-items: center;
    /* The Navigator's count-chip language (tint + semantic text), same as
       the Assistant's needs-you chip. */
    color: var(--warning);
    background: var(--warning-soft);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count-chip {
    min-width: 16px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .count-chip.working {
    background: var(--success-soft);
    color: var(--success);
  }
  /* Needs you (the Assistant's approvals/questions): the one attention tone. */
  .count-chip.needs {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .nav-foot {
    border-top: 1px solid var(--border);
    padding: 6px 8px 8px;
  }
  .edit-foot {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px 6px;
  }
  .nav-user {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 2px;
  }
  .avatar {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 28%, transparent);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .user-name {
    font-size: var(--fs-s);
    font-weight: 500;
    line-height: 1.2;
  }
  .user-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
