<script lang="ts">
  import LazyMount from '../lib/components/LazyMount.svelte';
  import { rowMenu } from '../lib/rowMenu';
  import { newScratchSession, workspaceMenuItems as wsMenu } from './workspaceActions';
  import { RemovedWorkspacesChunk } from './lazyDialogs';
  import { LS_WS_RECENT, activityText, parseRecent, switcherItems, workspaceActivity } from './workspaceSwitch';
  import { lsGet } from '../lib/storage';
  // Expanded 240px navigator: modules in foldable macOS source-list sections
  // (Agents with its nested session lists in Work), workspaces section,
  // user/settings at the bottom.
  import { navBadge } from '../lib/navBadge';
  import Icon from '../lib/components/Icon.svelte';
  import NotificationBell from './NotificationBell.svelte';
  import { router } from '../lib/router.svelte';
  import { navPending } from '../lib/navPending.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { RAIL_MIN, RAIL_MAX } from '../lib/stores/ui.svelte';
  import { paneResizer, pxWide, RESIZE_TITLE } from '../lib/paneResizer';
  import { startMouseDrag } from '../lib/dragCursor';
  import { ws } from '../lib/stores/workspace.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
  import { sidePane, splitMenuItems, navClick, SPLIT_HINT } from '../lib/stores/sidePane.svelte';
  import { viewport } from '../lib/stores/viewport.svelte';
  import NavLightToggle from './NavLightToggle.svelte';
  import type { WorkspaceWithRole } from '../lib/api/types';
  import { tick, untrack } from 'svelte';
  import {
    FAVORITES_ID,
    activeNavId,
    availableModules,
    moveAmong,
    moveWithinGroup,
    reorderAmong,
    moduleHome, resolveGroupOrder,
    resolveOrder,
    sidebarSections, agentsLast,
    visibleOrder,
    type SidebarModule,
    type SidebarPluginEntry,
    type SidebarSection,
  } from '../lib/sidebar';

  // Drag-to-resize the navigator from its inline-end edge (widens the session area).
  let resizing = $state(false);
  // Overlay cursor + one width write per frame + one localStorage write on
  // release (lib/dragCursor.ts) — no body.style restyle of the whole app.
  function startResize(e: MouseEvent): void {
    resizing = true;
    const startX = e.clientX;
    const startW = ui.railWidth;
    const rtl = typeof document !== 'undefined' && getComputedStyle(document.documentElement).direction === 'rtl';
    startMouseDrag(e, {
      onMove: (ev) => ui.setRailWidth(startW + (rtl ? startX - ev.clientX : ev.clientX - startX), false),
      onEnd: () => {
        resizing = false;
        ui.setRailWidth(ui.railWidth);
      },
    });
  }

  // ── Workspace quick filter ──────────────────────────────────────────────────
  // Name or root-path substring (case-insensitive). The box only appears once
  // the list is long enough to need it (or while a query is still set, so a
  // workspace removed mid-filter can't strand a hidden query).
  const WS_FILTER_MIN = 5;
  let wsQuery = $state('');
  const wq = $derived(wsQuery.trim().toLowerCase());
  const showWsFilter = $derived(ws.workspaces.length >= WS_FILTER_MIN || wsQuery !== '');
  const fWorkspaces = $derived(
    wq === ''
      ? ws.workspaces
      : ws.workspaces.filter((w) => w.name.toLowerCase().includes(wq) || w.root_path.toLowerCase().includes(wq)),
  );
  /** Esc clears the query (a second Esc leaves the box); Enter switches to the
   *  first match and clears, so "type a few letters ⏎" is a workspace switcher. */
  function wsFilterKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (wsQuery) wsQuery = '';
      else (e.currentTarget as HTMLInputElement).blur();
    } else if (e.key === 'Enter' && wq !== '' && fWorkspaces.length > 0) {
      e.preventDefault();
      void ws.select(fWorkspaces[0].id);
      wsQuery = '';
    }
  }

  /** The current-workspace chip under Agents: a switcher menu (filterable once
   *  the list is long, like the Workspaces section's quick filter). */
  function openWsSwitcher(e: MouseEvent | KeyboardEvent): void {
    const items = switcherItems(ws.workspaces, parseRecent(lsGet(LS_WS_RECENT)), wsActivity, ws.currentId, (id) => void ws.select(id));
    if (items.length > 0) items.push({ separator: true });
    items.push({ label: 'Add workspace…', icon: 'plus', pinned: true, action: () => (ui.newWorkspaceOpen = true) });
    items.push({ label: 'Removed workspaces…', icon: 'archive', pinned: true, action: () => (removedOpen = true) });
    ctxMenu.showAt(
      e.currentTarget as HTMLElement,
      items,
      ws.workspaces.length >= WS_FILTER_MIN ? { filter: true, filterPlaceholder: 'Switch workspace…', maxVisible: 12 } : undefined,
    );
  }

  // ── Workspace management: workspaceActions.ts; live counts per workspace row ──
  const wsActivity = $derived(workspaceActivity([...ws.foregroundActive, ...ws.otherWsSessions], ws.statusMap, ws.needsYou));
  let removedOpen = $state(false);

  const workspaceMenuItems = (w: WorkspaceWithRole): MenuItem[] => wsMenu(w, () => (removedOpen = true));

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
    return ui.sidebarEditMode || sec.group.id === activeGroup;
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
    // Plain clicks resume the module where it was left; this is the way home.
    const home: MenuItem = { label: `Open ${m.label} main page`, icon: 'home', action: () => router.go(moduleHome(m.id)) };
    ctxMenu.show(e, [home, { separator: true }, ...splitItems(m), ...favoriteMenuItems(m), { separator: true }, customizeItem()]);
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

<nav class="navigator sidebar-material" class:resizing aria-label="Navigator" style:width={viewport.isDesktop ? `${ui.railWidth}px` : undefined}>
  <!-- The window splitter (paneResizer: focus, ←/→, Home/End, Enter, drag, double-click reset). Desktop only: tablet & mobile drawers own their own width. -->
  {#if viewport.isDesktop}
  <div
    class="rail-resize"
    role="separator"
    aria-label="Resize the navigator"
    title={RESIZE_TITLE}
    use:paneResizer={{ value: ui.railWidth, min: RAIL_MIN, max: RAIL_MAX, onChange: (w) => ui.setRailWidth(w), onReset: () => ui.setRailWidth(240), onDragStart: startResize, text: pxWide }}
  ></div>
  {/if}
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
      title="Go back"
      aria-label="Go back"
      aria-keyshortcuts="Meta+Shift+ArrowLeft"
    >
      <Icon name="chevronLeft" size={14} />
    </button>
    <button
      class="icon-btn"
      onclick={() => router.forward()}
      disabled={!router.canForward}
      title="Go forward"
      aria-label="Go forward"
      aria-keyshortcuts="Meta+Shift+ArrowRight"
    >
      <Icon name="chevronRight" size={14} />
    </button>
    {/if}
    <!-- The one notification bell on desktop/tablet: same spot on every page,
         so no module has to reserve room for a floating one. It sits at the
         header's inline-end, next to the sidebar edge its panel opens beside,
         so the panel's caret points straight back at it. -->
    <NotificationBell />
    <!-- Phone / short landscape closes drawer; tablet folds to Rail; desktop collapses sidebar. -->
    <button
      class="icon-btn"
      onclick={() => (viewport.navDrawer ? (ui.navDrawerOpen = false) : viewport.isTablet ? viewport.toggleTabletRail(false) : ui.toggleRail())}
      title={viewport.navDrawer ? 'Close sidebar' : 'Collapse sidebar'}
      aria-label={viewport.navDrawer ? 'Close sidebar' : 'Collapse sidebar'}
      aria-keyshortcuts={viewport.navDrawer ? undefined : 'Meta+1'}
    >
      <Icon name="sidebar" size={14} />
    </button>
  </div>

  <div class="nav-scroll" bind:this={scrollEl}>
    {#if !ui.sidebarEditMode}
      <!-- "Needs you": opens the Agents inbox narrowed to the sessions blocked
           on input. Only shown once at least one session is flagged (or while on). -->
      {#if ws.needsYouCount > 0 || ws.needsYouFilter}
        <button
          class="needs-you-filter"
          class:active={ws.needsYouFilter}
          aria-pressed={ws.needsYouFilter}
          onclick={() => {
            ws.needsYouFilter = !ws.needsYouFilter;
            if (ws.needsYouFilter && router.module !== 'agents') router.go('agents');
          }}
          title="Show only the sessions waiting on you"
        >
          <Icon name="bell" size={12} />
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
          <!-- Drag to reorder is the pointer path (presentation); the keyboard
               path is the header menu’s "Move section up/down". -->
          <div
            role="presentation"
            class="group-head-row"
            draggable={secMovable}
            ondragstart={secMovable ? (e) => onSecDragStart(e, sec.group.id) : undefined}
            ondragend={secMovable ? onSecDragEnd : undefined}
          >
            {#if secMovable}
              <span class="grip sec-grip" title="Drag to reorder sections" aria-hidden="true"><Icon name="grip" size={12} /></span>
            {/if}
            <button use:rowMenu
              class="group-head"
              class:pinned
              aria-expanded={open}
              onclick={() => !pinned && ui.toggleSidebarGroup(sec.group.id)}
              oncontextmenu={(e) => sectionMenu(e, sec)}
              title={pinned ? undefined : open ? `Hide ${sec.group.label}` : `Show ${sec.group.label}`}
              data-testid={`sidebar-group-head-${sec.group.id}`}
            >
              <span class="group-label">{sec.group.label}</span>
              {#if fav}<span class="group-star" aria-hidden="true"><Icon name="star" size={12} /></span>{/if}
              {#if !open && sec.modules.some((m) => m.id === 'agents') && ws.workingCount > 0}
                <span class="count-chip working" title="Working sessions">{ws.workingCount}</span>
              {/if}
              {#if !pinned}
                <span class="group-chev"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
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
            {#each ui.sidebarEditMode || fav ? sec.modules : agentsLast(sec.modules) as m, i (m.id)}
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
      {#if showWsFilter}
        <div class="nav-search ws-filter">
          <Icon name="search" size={12} />
          <input dir="ltr"
            class="nav-search-input"
            placeholder="Filter workspaces…"
            aria-label="Filter workspaces"
            data-testid="ws-filter"
            bind:value={wsQuery}
            onkeydown={wsFilterKey}
          />
          {#if wsQuery}
            <button class="search-clear" onclick={() => (wsQuery = '')} aria-label="Clear workspace filter" title="Clear workspace filter">
              <Icon name="x" size={12} />
            </button>
          {/if}
        </div>
      {/if}
      {#if wq !== '' && fWorkspaces.length === 0}
        <div class="nested-empty">No matching workspaces</div>
      {/if}
      {#each fWorkspaces as w (w.id)}
        {@const act = wsActivity.get(w.id)}
        <button use:rowMenu
          class="nav-item"
          class:active-ws={ws.currentId === w.id}
          onclick={() => ws.select(w.id)}
          oncontextmenu={(e) => ctxMenu.show(e, workspaceMenuItems(w))}
          title={w.root_path}
        >
          <Icon name="folder" size={14} />
          <span class="grow ellipsis">{w.name}</span>
          {#if act}<span class="sr-only">{activityText(act)}</span>{/if}
          {#if act?.needs}<span class="count-chip needs" aria-hidden="true" title={activityText(act)}>{act.needs}</span>{/if}
          {#if act?.working}<span class="count-chip working" aria-hidden="true" title={activityText(act)}>{act.working}</span>{/if}
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
      aria-current={router.module === 'walkthroughs' ? 'page' : undefined}
      onclick={() => router.go('walkthroughs')}
      title="Help"
    >
      <Icon name="info" size={14} />
      <span class="grow">Help</span>
    </button>
    <NavLightToggle />
    <button
      class="nav-item"
      class:active={router.module === 'settings'}
      aria-current={router.module === 'settings' ? 'page' : undefined}
      onclick={() => router.go('settings/appearance')}
      title="Settings"
      aria-keyshortcuts="Meta+,"
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

{#if removedOpen}<LazyMount lazy={RemovedWorkspacesChunk} what="removed workspaces" quiet onerror={() => (removedOpen = false)} props={{ onclose: () => (removedOpen = false) }} />{/if}

<!-- The module the side-by-side pane shows: a quiet trailing glyph. -->
{#snippet sideMark(id: string)}
  {#if sidePane.active && sidePane.key === id}
    <span class="side-mark" role="img" title="Open in the side pane" aria-label="Open in the side pane" data-testid={`side-mark-${id}`}>
      <Icon name="columns" size={12} />
    </span>
  {/if}
{/snippet}

{#snippet simpleRow(m: SidebarModule)}
  <button use:rowMenu
    class="nav-item"
    class:active={isActive(m.id)}
    aria-current={isActive(m.id) ? 'page' : undefined}
    class:drop-before={dragOverId === m.id && dropSide === 'before'}
    class:drop-after={dragOverId === m.id && dropSide === 'after'}
    class:dragging={dragId === m.id}
    data-nav-id={m.id}
    aria-busy={navPending.id === m.id || undefined}
    title={sidePane.supported ? SPLIT_HINT : undefined}
    onclick={(e) => navClick(e, m.id, m.label)}
    oncontextmenu={(e) => moduleMenu(e, m)}
    {...favRowProps(m.id)}
  >
    <Icon name={m.icon} size={14} />
    <span class="grow">{m.label}</span>
    {@render sideMark(m.id)}
    {#if navBadge(m.id)}
      {@const b = navBadge(m.id)!}
      <span class="sr-only">{b.spoken}</span><span class="count-chip {b.tone}" aria-hidden="true" title={b.title}>{b.count}</span>
    {/if}
  </button>
{/snippet}

{#snippet editRow(m: SidebarModule, first: boolean, last: boolean)}
  {@const fav = isFav(m.id)}
  <!-- Drag is the pointer path (presentation); the row’s ↑/↓ buttons and ⌥↑/⌥↓ reorder from the keyboard. -->
  <div use:rowMenu
    role="presentation"
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
        title={`Move ${m.label} up`}
        aria-label={`Move ${m.label} up`}
        aria-keyshortcuts="Alt+ArrowUp"
      >
        <Icon name="arrowUp" size={12} />
      </button>
      <button
        class="row-action mv"
        onclick={() => move(m, 1)}
        disabled={last}
        title={`Move ${m.label} down`}
        aria-label={`Move ${m.label} down`}
        aria-keyshortcuts="Alt+ArrowDown"
      >
        <Icon name="arrowDown" size={12} />
      </button>
      <button
        class="row-action"
        onclick={() => ui.toggleSidebarHidden(m.id)}
        title={isHidden(m.id) ? `Show ${m.label}` : `Hide ${m.label}`}
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
    <button use:rowMenu
      class="nav-item"
      class:active={router.module === 'agents' || router.module === ''}
      aria-current={router.module === 'agents' || router.module === '' ? 'page' : undefined}
      data-nav-id={m.id}
      aria-busy={navPending.id === m.id || undefined}
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
  </div>
{/snippet}

<style>
  .navigator { width: 100%; max-width: 100%; min-width: 0; height: 100%; display: flex; flex-direction: column; border-inline-end: 1px solid var(--separator); position: relative; }
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
      var(--accent-line) 45%,
      var(--accent-line) 55%,
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
    border-radius: var(--radius-s);
    display: block;
    flex-shrink: 0;
  }
  .nav-head :global(.icon-btn:disabled) {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .nav-title {
    font-size: var(--fs-m);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .nav-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    contain: paint;
    padding: 4px 8px;
  }
  /* Touch (see tokens.css): scrolled rows paint on an opaque layer, never see-through tiles. */
  @media (pointer: coarse) { .nav-scroll { background: var(--bg-sidebar); } }
  .nav-section {
    margin-bottom: 14px;
  }
  .nav-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
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
    transition: background var(--dur-fast) ease-out;
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
    background: var(--accent-soft);
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
    padding-block: 0; padding-inline: 8px 6px;
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
    margin-inline-end: 28px;
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
      margin-inline-end: 24px;
    }
  }
  .group-head-row .row-action:disabled {
    opacity: var(--disabled-opacity);
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
    letter-spacing: .06em;
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
    transition: opacity var(--dur-fast) ease-out;
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
    padding-block: 0; padding-inline: 6px 4px;
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
    opacity: var(--disabled-opacity);
  }
  @media (hover: none) {
    .edit-row .row-action.mv,
    .group-head-row .row-action.mv {
      width: 22px;
      opacity: 1;
    }
    .edit-row .row-action.mv:disabled,
    .group-head-row .row-action.mv:disabled {
      opacity: var(--disabled-opacity);
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
    opacity: var(--disabled-opacity);
    cursor: default;
  }
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
    transition: opacity var(--dur-fast) ease-out;
  }
  /* Hover-revealed, but a keyboard user tabbing onto one must see it (and
     its focus ring) — it was an invisible focused button. */
  .row-action:focus-visible {
    opacity: 1;
  }
  .row-action:hover {
    background: var(--hover);
    color: var(--text);
  }
  .row-action.danger:hover {
    color: var(--danger);
  }
  .nav-item.subtle {
    color: var(--text-dim);
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
  /* The input drops its own outline, so the field it sits in shows focus. */
  .nav-search:focus-within {
    outline: 2px solid var(--accent-text);
    outline-offset: -1px;
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
    transition: background var(--dur-fast) ease-out;
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
    font-weight: 600;
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
    font-weight: 600;
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
    background: var(--accent-soft-strong);
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
  /* Touch: every navigation row and its disclosure twisty is a 40px target. */
  @media (pointer: coarse), (max-width: 640px) {
    .nav-item,
    .nested-item,
    .nested-row .nested-item {
      height: auto;
      min-block-size: 40px;
    }
    .twisty {
      width: 40px;
      height: 40px;
    }
  }
</style>
