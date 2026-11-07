<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Drawer from '../../shell/Drawer.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { events } from '../../lib/events.svelte';
  import { loadPaneWidth } from '../../lib/paneResizer';
  import { router } from '../../lib/router.svelte';
  import { openFile } from '../../lib/stores/openfile.svelte';
  import { pageChrome } from '../../lib/stores/pageChrome.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { sessionState } from '../../lib/status';
  import StartRoomModal from '../rooms/StartRoomModal.svelte'; // ui-guards: allow — shared session room modal
  import NewThreadStage from './NewThreadStage.svelte';
  import ThreadCanvas from './ThreadCanvas.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  const ThreadInspector = lazyComponent(() => import('./ThreadInspector.svelte'));
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import ThreadProjectCard from './ThreadProjectCard.svelte';
  import ThreadSheets from './ThreadSheets.svelte';
  import { PullRequestsView, TerminalDock } from './threadLazy.svelte';
  import { threadRepos } from './threadRepos.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import ThreadSidebar from './ThreadSidebar.svelte';
  import { buildThreadMenu } from './threadMenu';
  import PhoneHandoffButton from '../../lib/components/PhoneHandoffButton.svelte';
  import { prepareDeepLink } from '../../lib/deepLinkOpen';
  import { runtimeThreadMenu } from './runtime/runtimeMenu';
  import { asThreadSession, isRuntimeThread, runtimeThreads } from './runtime/runtimeThreads.svelte';
  import { isThreadSession, projectNameFromCwd } from './threadsHelpers';
  import { personalAgentName } from '../../lib/personalAgentBadges';
  import { threads, type ThreadsLayoutMode } from './threadsStore.svelte';

  const SIDEBAR_W_KEY = 'threads.sidebarW';
  const INSPECTOR_W_KEY = 'threads.inspectorW';

  let sidebarW = $state(loadPaneWidth(SIDEBAR_W_KEY, 272, 240, 380));
  let inspectorW = $state(loadPaneWidth(INSPECTOR_W_KEY, 380, 300, 560));
  let stageWidth = $state(1280);
  let centerWidth = $state(1000);
  let roomSessionId = $state<string | null>(null);
  let mobileSidebarOpen = $state(false);
  $effect(() => { if (threads.inspectorOverlayOpen) mobileSidebarOpen = false; });

  const compact = $derived(
    viewport.isPhone || viewport.isTablet || (stageWidth > 0 && stageWidth < 860),
  );

  $effect(() => threads.setInspectorCompact(compact));
  onMount(() => () => threads.setInspectorCompact(false));

  const liveThreadIds = $derived([
    ...ws.sessions.filter((s) => !s.archived && isThreadSession(s)).map((s) => s.id),
    ...runtimeThreads.live().map((s) => s.id),
  ]);

  // Subscribe to live cross-session messages + MCP DB activity while mounted.
  onMount(() => threads.mountLive());
  // Structured runtime threads: their rows, and live events for open ones.
  onMount(() => runtimeThreads.mount());
  $effect(() => {
    for (const wid of new Set([ws.currentId, ws.scratch ? SCRATCH_WORKSPACE_ID : null])) {
      if (wid) untrack(() => runtimeThreads.load(wid)).catch(() => {});
    }
  });

  // Like T3, no side-panel rail and no status bar while Threads is open (the
  // sidebar's status dots carry the working count). Not persisted: a saved
  // open Side panel is shown closed here (transient) and comes back on
  // leaving; opening it on purpose (shortcut / ⌘K) shows it.
  onMount(() => {
    const restoreChrome = pageChrome.set({ hideSideRail: true, hideStatusBar: true });
    const hadTransient = ui.rightTransient !== null;
    if (!hadTransient && ui.rightOpen) ui.rightTransient = { open: false, tab: ui.rightTab };
    return () => {
      restoreChrome();
      if (!hadTransient && ui.rightTransient && !ui.rightTransient.open) ui.rightTransient = null;
    };
  });

  // One sidebar, like T3: while Threads is open Otto's navigator folds to its
  // icon rail (not persisted — the shell's own preference is untouched), and
  // comes back on leaving. Expanding it here is remembered as "don't fold".
  onMount(() => {
    if (viewport.isPhone || viewport.isTablet) return;
    let folded = false;
    if (threads.collapseNav && ui.railExpanded) {
      ui.railExpanded = false;
      folded = true;
    }
    const stop = $effect.root(() => {
      $effect(() => {
        const expanded = ui.railExpanded;
        untrack(() => {
          if (expanded && folded) {
            folded = false;
            threads.setCollapseNav(false);
          } else if (!expanded && !folded && !threads.collapseNav) {
            threads.setCollapseNav(true);
          }
        });
      });
    });
    return () => {
      stop();
      if (folded && !ui.railExpanded) ui.railExpanded = true;
    };
  });

  // Seed recent cross-session messages from workspace trail.
  $effect(() => {
    const wid = ws.currentId;
    if (wid) void threads.seedWorkspaceTrail(wid);
  });

  // Consume global "Open in Files" requests (`file:line` clicks in conversation turns).
  $effect(() => {
    if (router.module !== 'threads' || !openFile.request) return;
    const focused = threads.focusedSessionId ? ws.getSession(threads.focusedSessionId) : null;
    const wsKey = focused?.workspace_id ?? ws.currentId ?? 'scratch';
    untrack(() => threads.consumeOpenFileRequest(wsKey));
  });

  // Route sync: `#/threads` and `#/threads/<sessionId>`.
  let lastRoutedId: string | null = null;
  $effect(() => {
    if (router.module !== 'threads') return;
    const routeSessionId = router.parts[1] ?? null;
    if (routeSessionId && routeSessionId !== lastRoutedId) {
      lastRoutedId = routeSessionId;
      untrack(() => {
        // Resolve the thread's workspace first: a link from another workspace
        // (a phone QR) switches there instead of opening nothing.
        void (async () => {
          if (!runtimeThreads.get(routeSessionId) && !(await prepareDeepLink(ws, routeSessionId, 'thread'))) return;
          const s = await ws.ensureSession(routeSessionId);
          if (!s && !runtimeThreads.get(routeSessionId) && ws.currentId) await runtimeThreads.load(ws.currentId).catch(() => {});
          const id = s?.id ?? runtimeThreads.get(routeSessionId)?.id;
          if (id && lastRoutedId === routeSessionId) threads.selectThread(id, { syncRoute: false });
        })();
      });
    } else if (!routeSessionId) {
      lastRoutedId = null;
    }
  });

  // Keep paneIds clean when sessions are loaded; pick an initial thread or show NewThreadStage.
  $effect(() => {
    // Closed runtime threads stay open read-only (their History row opens them).
    const knownThreads = new Set([...liveThreadIds, ...Object.keys(runtimeThreads.byId)]);
    const anyLoaded = ws.sessions.some((s) => !s.archived);
    if ((!ws.listSettled || ws.sessionsLoading) && !anyLoaded) return;
    if (ws.currentId && !runtimeThreads.loaded[ws.currentId]) return;
    untrack(() => {
      const valid = threads.paneIds.filter((id) => knownThreads.has(id));
      if (valid.length !== threads.paneIds.length) {
        threads.paneIds = valid;
        if (valid.length === 0) {
          threads.focusedSessionId = null;
        } else if (!threads.focusedSessionId || !valid.includes(threads.focusedSessionId)) {
          threads.focusedSessionId = valid[0];
        }
      }
      if (threads.paneIds.length === 0 && !threads.newDraftMode) {
        if (liveThreadIds.length > 0 && !router.parts[1]) {
          threads.selectThread(liveThreadIds[0], { syncRoute: false });
        } else {
          threads.newDraftMode = true;
        }
      }
    });
  });

  // Register ⌘K commands while Threads is open.
  $effect(() =>
    registry.register('threads', [
      {
        id: 'threads.new',
        title: 'New thread',
        group: 'Threads',
        keywords: 'start create thread claude codex antigravity',
        run: () => threads.startNewThread({ workspaceId: ws.currentId }),
      },
      {
        id: 'threads.layout.single',
        title: 'Threads layout: 1-up single view',
        group: 'Threads',
        run: () => threads.setLayout('single', liveThreadIds),
      },
      {
        id: 'threads.layout.split',
        title: 'Threads layout: 2-up Split view',
        group: 'Threads',
        run: () => threads.setLayout('split', liveThreadIds),
      },
      {
        id: 'threads.layout.grid',
        title: 'Threads layout: 4-up Grid view',
        group: 'Threads',
        run: () => threads.setLayout('grid', liveThreadIds),
      },
      {
        id: 'threads.toggle.inspector',
        title: 'Toggle Threads right panel',
        group: 'Threads',
        run: () => threads.toggleInspector(),
      },
      {
        id: 'threads.toggle.changes',
        title: 'Threads: show changes',
        group: 'Threads',
        keywords: 'diff summary git',
        run: () => threads.toggleInspector('changes'),
      },
      {
        id: 'threads.toggle.terminal',
        title: 'Threads: toggle terminal drawer',
        group: 'Threads',
        run: () => threadsChrome.setTerminal(!threadsChrome.terminalOpen),
      },
    ]),
  );

  const failedListId = $derived([ws.currentId, ws.scratch ? SCRATCH_WORKSPACE_ID : null].find((id) => id && runtimeThreads.loadErrors[id]) ?? null);
  const threadListError = $derived(failedListId ? runtimeThreads.loadErrors[failedListId] : null);
  function retryThreads(): void {
    if (failedListId) void runtimeThreads.load(failedListId).catch(() => {});
  }
  const showWelcomeStage = $derived(threads.newDraftMode || threads.paneIds.length === 0);
  const focused = $derived.by(() => {
    const id = showWelcomeStage ? null : threads.focusedSessionId;
    if (!id) return null;
    const rt = runtimeThreads.get(id);
    return ws.getSession(id) ?? (rt ? asThreadSession(rt) : null);
  });
  const focusedState = $derived(
    focused
      ? sessionState(focused, ws.statusMap[focused.id] ?? focused.status, ws.needsYou[focused.id] === true, {
          stale: events.state !== 'connected',
        })
      : null,
  );
  const crumbProject = $derived.by(() => {
    // A personal agent's thread: its agent's name, never its folder's id.
    const agent = focused ? personalAgentName(focused.meta as Record<string, unknown> | undefined, focused.title) : null;
    if (agent && !showWelcomeStage) return agent;
    const cwd = focused?.cwd ?? (showWelcomeStage ? threads.stageCwd : null) ?? ws.current?.root_path ?? null;
    return cwd ? projectNameFromCwd(cwd, ws.current?.name ?? 'Workspace') : (ws.current?.name ?? 'Scratch');
  });
  const crumbTitle = $derived(showWelcomeStage ? 'New thread' : (focused?.title ?? 'Thread'));
  const rightOpen = $derived(threads.inspectorVisible);
  const contextCwd = $derived(
    focused?.cwd ?? (showWelcomeStage ? threads.stageCwd : null) ?? ws.current?.root_path ?? null,
  );
  const contextRepo = $derived(threadRepos.repoFor(contextCwd, focused?.workspace_id ?? ws.currentId));
  const maximized = $derived(!compact && threads.inspectorOpen && threadsChrome.maximized);
  // T3's floating project card: remembered toggle, only where it can't crowd
  // the column, and out of the way of the terminal drawer and the right panel.
  const cardRoom = $derived(
    !compact && !threads.inspectorOpen && !threadsChrome.terminalOpen && !threadsChrome.stageView && centerWidth >= 1100,
  );
  const showStageCard = $derived(cardRoom && threadsChrome.cardVisible);
  // Over a conversation the card is T3's header popover (ThreadDetailsCard's
  // popover mode): opened from the toolbar, closed by an outside click or
  // Esc, never left pinned over the timeline.
  let cardOpen = $state(false);
  const cardPopover = $derived(cardOpen && !compact && !threads.inspectorOpen && !showWelcomeStage && !threadsChrome.stageView);
  // T3's inline placement: where the column can spare it, the conversation
  // makes room beside the open card; where it can't, the card docks above
  // the timeline and pushes it down. It never runs over a message.
  const cardReserve = $derived(cardPopover && centerWidth >= 840);
  $effect(() => {
    if (!cardPopover) return;
    const onDown = (e: PointerEvent): void => {
      const t = e.target as Element | null;
      // Menus and sheets the card opens live outside it.
      if (!t?.closest('.th-card-pop, [data-testid="threads-toggle-card"], .ctx-menu, [role="dialog"]')) cardOpen = false;
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape' && !document.querySelector('.ctx-menu, [role="dialog"]')) cardOpen = false;
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey);
    };
  });

  function openLayoutMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Single thread', hint: '1-up', checked: threads.layoutMode === 'single', action: () => pickLayout('single') },
      { label: 'Split', hint: '2-up', checked: threads.layoutMode === 'split', action: () => pickLayout('split') },
      { label: 'Grid', hint: '4-up', checked: threads.layoutMode === 'grid', action: () => pickLayout('grid') },
    ];
    ctxMenu.show(e, items);
  }

  function openThreadMenu(e: MouseEvent): void {
    if (focused && isRuntimeThread(focused)) ctxMenu.show(e, runtimeThreadMenu(focused.id));
    else if (focused) ctxMenu.show(e, buildThreadMenu(focused, { onstartroom: (sid) => (roomSessionId = sid) }));
  }

  function toggleLeftBar(): void {
    if (compact) {
      mobileSidebarOpen = !mobileSidebarOpen;
      if (mobileSidebarOpen) threads.setInspectorOpen(false);
    } else threads.toggleSidebar();
  }

  function toggleRightBar(): void {
    threads.toggleInspector();
  }

  function pickLayout(mode: ThreadsLayoutMode): void {
    threads.setLayout(mode, liveThreadIds);
  }
</script>

<div class="th-page" data-testid="threads-page">
  <PageHeader title="Threads">
    {#snippet leading()}
      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={compact ? mobileSidebarOpen : threads.sidebarOpen}
        aria-pressed={compact ? mobileSidebarOpen : threads.sidebarOpen}
        aria-label="Toggle threads sidebar"
        title="Toggle sidebar"
        data-testid="threads-toggle-sidebar"
        onclick={toggleLeftBar}
      >
        <Icon name="sidebar" size={14} />
      </button>
      {#if !viewport.isPhone}
        <button
          type="button"
          class="icon-btn"
          aria-label="Back"
          title="Back (⌘[)"
          disabled={!router.canBack}
          data-testid="threads-back"
          onclick={() => router.back()}
        >
          <Icon name="chevronLeft" size={14} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Forward"
          title="Forward (⌘])"
          disabled={!router.canForward}
          data-testid="threads-forward"
          onclick={() => router.forward()}
        >
          <Icon name="chevronRight" size={14} />
        </button>
      {/if}
      {#if compact || !threads.sidebarOpen}
        <button
          type="button"
          class="icon-btn"
          aria-label="New thread"
          title="New thread"
          data-testid="threads-new-btn-top"
          onclick={() => threads.startNewThread({ workspaceId: ws.currentId })}
        >
          <Icon name="edit" size={14} />
        </button>
      {/if}
    {/snippet}

    {#snippet titleContent()}
      <span class="th-crumbs" data-testid="threads-breadcrumb">
        <span class="th-crumb-project ellipsis" dir="auto">{crumbProject}</span>
        <span class="th-crumb-sep" aria-hidden="true">/</span>
        {#if focusedState}<StatusDot state={focusedState} />{/if}
        <span class="th-crumb-title ellipsis" dir="auto">{crumbTitle}</span>
      </span>
    {/snippet}

    {#snippet actions()}
      {#if !compact}
        <button
          type="button"
          class="icon-btn"
          class:th-btn-on={showWelcomeStage ? threadsChrome.cardVisible : cardPopover}
          data-overflow="2"
          aria-pressed={showWelcomeStage ? threadsChrome.cardVisible : undefined}
          aria-expanded={showWelcomeStage ? undefined : cardPopover}
          aria-label="Toggle project card"
          title="Project card (folder, branch, PR, changes)"
          data-icon="note"
          data-testid="threads-toggle-card"
          onclick={() => (showWelcomeStage ? threadsChrome.toggleCard() : (cardOpen = !cardPopover))}
        >
          <Icon name="note" size={14} />
        </button>
      {/if}
      <button
        type="button"
        class="icon-btn"
        data-overflow="1"
        aria-haspopup="menu"
        aria-label="Layout: single, split or grid"
        title="Layout (single · split · grid)"
        data-icon="grid"
        data-testid="threads-layout-switch"
        onclick={openLayoutMenu}
      >
        <Icon name="grid" size={14} />
      </button>
      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={threadsChrome.terminalOpen}
        data-overflow="2"
        aria-pressed={threadsChrome.terminalOpen}
        aria-label="Toggle terminal drawer"
        title="Terminal (project shell under the stage)"
        data-icon="terminal"
        data-testid="threads-toggle-terminal"
        onpointerenter={() => TerminalDock.prefetch()}
        onclick={() => threadsChrome.setTerminal(!threadsChrome.terminalOpen)}
      >
        <Icon name="terminal" size={14} />
      </button>
      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={rightOpen}
        data-overflow="3"
        aria-pressed={rightOpen}
        aria-label="Toggle right panel"
        title="Right panel (project, files, changes, peers)"
        data-icon="panel"
        data-testid="threads-toggle-inspector"
        onclick={toggleRightBar}
      >
        <Icon name="panel" size={14} />
      </button>
      {#if focused}
        <PhoneHandoffButton route={`threads/${focused.id}`} what="this thread" size={14} overflow={4} testid="threads-continue-phone" />
        <button
          type="button"
          class="icon-btn"
          aria-haspopup="menu"
          aria-label="Thread actions"
          title="Thread actions"
          data-icon="more"
          data-testid="threads-thread-menu"
          onclick={openThreadMenu}
        >
          <Icon name="more" size={14} />
        </button>
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody width="full" padded={false} fill>
    <div class="th-workspace" class:compact bind:clientWidth={stageWidth}>
      {#if !compact && threads.sidebarOpen}
        <div class="th-left-col" style:inline-size="{sidebarW}px">
          <ThreadSidebar onstartroom={(sid) => (roomSessionId = sid)} />
        </div>
        <PaneDivider
          bind:width={sidebarW}
          storageKey={SIDEBAR_W_KEY}
          min={240}
          max={380}
          defaultWidth={272}
          label="Resize threads inbox"
        />
      {/if}

      <div class="th-center-col" class:th-hidden={maximized} bind:clientWidth={centerWidth}>
        {#if threadListError && liveThreadIds.length > 0}
          <LoadState what="threads" error={threadListError} onretry={retryThreads} />
        {/if}
        <div class="th-stage" class:th-card-reserve={cardReserve}>
          {#if threadListError && liveThreadIds.length === 0 && showWelcomeStage}
            <LoadState what="threads" error={threadListError} empty onretry={retryThreads} />
          {:else if threadsChrome.stageView === 'prs'}
            <LazyMount lazy={PullRequestsView} what="pull requests" variant="panel" props={{ repo: contextRepo }} />
          {:else if showWelcomeStage}
            <NewThreadStage showCard={showStageCard} />
          {:else}
            <ThreadCanvas onstartroom={(sid) => (roomSessionId = sid)} />
            {#if cardPopover && contextCwd}
              <div class="th-card-pop" class:docked={!cardReserve}><ThreadProjectCard floating cwd={contextCwd} /></div>
            {/if}
          {/if}
        </div>
        {#if threadsChrome.terminalOpen}
          <LazyMount lazy={TerminalDock} what="the terminal" variant="compact" props={{ cwd: contextCwd }} />
        {/if}
      </div>

      {#if !compact && threads.inspectorOpen}
        <PaneDivider
          bind:width={inspectorW}
          storageKey={INSPECTOR_W_KEY}
          min={300}
          max={560}
          defaultWidth={380}
          invert
          label="Resize thread inspector"
        />
        <div class="th-right-col" class:th-max={maximized} style:inline-size={maximized ? null : `${inspectorW}px`}>
          <LazyMount lazy={ThreadInspector} what="the thread details" variant="panel" props={{ onstartroom: (sid: string) => (roomSessionId = sid) }} />
        </div>
      {/if}

      {#if compact}
        <Drawer bind:open={mobileSidebarOpen} side="left" title="Threads" width="min(360px, 100vw)">
          <ThreadSidebar
            onselect={() => (mobileSidebarOpen = false)}
            onstartroom={(sid) => { mobileSidebarOpen = false; roomSessionId = sid; }}
          />
        </Drawer>
        <Drawer bind:open={threads.inspectorOverlayOpen} side="right" title="Thread details" width="min(420px, 100vw)">
          <LazyMount lazy={ThreadInspector} what="the thread details" variant="panel" props={{
            showCard: true,
            onstartroom: (sid: string) => { threads.setInspectorOpen(false); roomSessionId = sid; },
          }} />
        </Drawer>
      {/if}
    </div>
  </PageBody>
</div>

<ThreadSheets />

{#if roomSessionId}
  <StartRoomModal sessionId={roomSessionId} onclose={() => (roomSessionId = null)} />
{/if}

<style>
  .th-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .th-btn-on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .th-crumbs {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-size: var(--fs-m);
    font-weight: 400;
  }
  .th-crumb-project {
    color: var(--text-dim);
    max-width: 200px;
  }
  .th-crumb-sep {
    color: var(--text-dim);
  }
  .th-crumb-title {
    color: var(--text);
    font-weight: 500;
    min-width: 0;
  }
  @media (max-width: 640px) {
    .th-crumb-project, .th-crumb-sep { display: none; }
  }
  .th-workspace {
    position: relative;
    display: flex;
    align-items: stretch;
    flex: 1;
    width: 100%;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .th-left-col,
  .th-right-col {
    flex-shrink: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .th-stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .th-card-pop.docked {
    position: static;
    order: -1;
    flex-shrink: 0;
    max-block-size: 50%;
    margin-block: 8px 0;
    margin-inline: auto 12px;
  }
  .th-stage.th-card-reserve {
    padding-inline-end: 324px;
  }
  .th-card-pop {
    position: absolute;
    inset-block-start: 8px;
    inset-inline-end: 12px;
    inline-size: min(300px, calc(100% - 24px));
    max-block-size: calc(100% - 16px);
    overflow-y: auto;
    border-radius: var(--radius-l);
    z-index: var(--z-sticky);
  }
  .th-center-col.th-hidden {
    display: none;
  }
  .th-right-col.th-max {
    flex: 1;
  }
  .th-center-col {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
</style>
