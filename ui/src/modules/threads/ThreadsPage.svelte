<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
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
  import { ws } from '../../lib/stores/workspace.svelte';
  import { sessionState } from '../../lib/status';
  import StartRoomModal from '../rooms/StartRoomModal.svelte'; // ui-guards: allow — shared session room modal
  import NewThreadStage from './NewThreadStage.svelte';
  import ThreadCanvas from './ThreadCanvas.svelte';
  import ThreadInspector from './ThreadInspector.svelte';
  import ThreadSidebar from './ThreadSidebar.svelte';
  import { buildThreadMenu } from './threadMenu';
  import { isPlainShellSession, isThreadSession, projectNameFromCwd } from './threadsHelpers';
  import { threads, type ThreadsLayoutMode } from './threadsStore.svelte';

  const SIDEBAR_W_KEY = 'threads.sidebarW';
  const INSPECTOR_W_KEY = 'threads.inspectorW';

  let sidebarW = $state(loadPaneWidth(SIDEBAR_W_KEY, 272, 240, 380));
  let inspectorW = $state(loadPaneWidth(INSPECTOR_W_KEY, 380, 300, 560));
  let stageWidth = $state(1280);
  let centerWidth = $state(1000);
  let roomSessionId = $state<string | null>(null);
  let mobileDrawer = $state<'none' | 'sidebar' | 'inspector'>('none');

  const compact = $derived(
    viewport.isPhone || viewport.isTablet || (stageWidth > 0 && stageWidth < 860),
  );

  const liveThreadIds = $derived(
    ws.sessions.filter((s) => !s.archived && isThreadSession(s)).map((s) => s.id),
  );

  // Subscribe to live cross-session messages + MCP DB activity while mounted.
  onMount(() => threads.mountLive());

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
        void ws.ensureSession(routeSessionId).then((s) => {
          if (s) threads.selectThread(s.id, { syncRoute: false });
        });
      });
    } else if (!routeSessionId) {
      lastRoutedId = null;
    }
  });

  // Keep paneIds clean when sessions are loaded; pick an initial thread or show NewThreadStage.
  $effect(() => {
    const knownThreads = new Set(liveThreadIds);
    const anyLoaded = ws.sessions.some((s) => !s.archived);
    if ((!ws.listSettled || ws.sessionsLoading) && !anyLoaded) return;
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
        run: () => threads.toggleTerminal(threads.newDraftMode ? null : threads.focusedSessionId),
      },
    ]),
  );

  const showWelcomeStage = $derived(threads.newDraftMode || threads.paneIds.length === 0);
  const focused = $derived(
    !showWelcomeStage && threads.focusedSessionId ? (ws.getSession(threads.focusedSessionId) ?? null) : null,
  );
  const focusedState = $derived(
    focused
      ? sessionState(focused, ws.statusMap[focused.id] ?? focused.status, ws.needsYou[focused.id] === true, {
          stale: events.state !== 'connected',
        })
      : null,
  );
  const crumbProject = $derived.by(() => {
    const cwd = focused?.cwd ?? (showWelcomeStage ? threads.stageCwd : null) ?? ws.current?.root_path ?? null;
    return cwd ? projectNameFromCwd(cwd, ws.current?.name ?? 'Workspace') : (ws.current?.name ?? 'Scratch');
  });
  const crumbTitle = $derived(showWelcomeStage ? 'New thread' : (focused?.title ?? 'Thread'));
  const rightOpen = $derived(compact ? mobileDrawer === 'inspector' : threads.inspectorOpen);
  const changesOpen = $derived(rightOpen && threads.inspectorTab === 'changes');
  const terminalId = $derived(focused && !isPlainShellSession(focused) ? focused.id : null);
  // T3's floating project card: only where it can't crowd the composer.
  const showStageCard = $derived(!compact && !threads.inspectorOpen && centerWidth >= 1100);

  function toggleChanges(): void {
    if (compact) {
      threads.setInspectorTab('changes');
      mobileDrawer = changesOpen ? 'none' : 'inspector';
    } else threads.toggleInspector('changes');
  }

  function openLayoutMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Single thread', hint: '1-up', checked: threads.layoutMode === 'single', action: () => pickLayout('single') },
      { label: 'Split', hint: '2-up', checked: threads.layoutMode === 'split', action: () => pickLayout('split') },
      { label: 'Grid', hint: '4-up', checked: threads.layoutMode === 'grid', action: () => pickLayout('grid') },
    ];
    ctxMenu.show(e, items);
  }

  function openThreadMenu(e: MouseEvent): void {
    if (focused) ctxMenu.show(e, buildThreadMenu(focused, { onstartroom: (sid) => (roomSessionId = sid) }));
  }

  function toggleLeftBar(): void {
    if (compact) mobileDrawer = mobileDrawer === 'sidebar' ? 'none' : 'sidebar';
    else threads.toggleSidebar();
  }

  function toggleRightBar(): void {
    if (compact) mobileDrawer = mobileDrawer === 'inspector' ? 'none' : 'inspector';
    else threads.toggleInspector();
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
        class:th-btn-on={compact ? mobileDrawer === 'sidebar' : threads.sidebarOpen}
        aria-pressed={compact ? mobileDrawer === 'sidebar' : threads.sidebarOpen}
        aria-label="Toggle threads sidebar"
        title="Toggle sidebar"
        data-testid="threads-toggle-sidebar"
        onclick={toggleLeftBar}
      >
        <Icon name="sidebar" size={14} />
      </button>
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
        class:th-btn-on={changesOpen}
        data-overflow="2"
        aria-pressed={changesOpen}
        aria-label="Toggle changes"
        title="Changes (diff summary)"
        data-icon="file"
        data-testid="threads-toggle-changes"
        onclick={toggleChanges}
      >
        <Icon name="file" size={14} />
      </button>
      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={threads.isTerminalOpen(terminalId)}
        data-overflow="2"
        aria-pressed={threads.isTerminalOpen(terminalId)}
        aria-label="Toggle terminal drawer"
        title={terminalId ? 'Terminal drawer' : 'Open a thread to use its terminal'}
        data-icon="terminal"
        data-testid="threads-toggle-terminal"
        disabled={!terminalId}
        onclick={() => threads.toggleTerminal(terminalId)}
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

      <div class="th-center-col" bind:clientWidth={centerWidth}>
        {#if showWelcomeStage}
          <NewThreadStage showCard={showStageCard} />
        {:else}
          <ThreadCanvas onstartroom={(sid) => (roomSessionId = sid)} />
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
        <div class="th-right-col" style:inline-size="{inspectorW}px">
          <ThreadInspector onstartroom={(sid) => (roomSessionId = sid)} />
        </div>
      {/if}

      {#if compact && mobileDrawer !== 'none'}
        <button
          type="button"
          class="th-scrim"
          aria-label="Close drawer"
          title="Close drawer"
          onclick={() => (mobileDrawer = 'none')}
        ></button>
        {#if mobileDrawer === 'sidebar'}
          <div class="th-drawer start">
            <ThreadSidebar
              onselect={() => (mobileDrawer = 'none')}
              onstartroom={(sid) => {
                mobileDrawer = 'none';
                roomSessionId = sid;
              }}
            />
          </div>
        {:else if mobileDrawer === 'inspector'}
          <div class="th-drawer end">
            <ThreadInspector
              onstartroom={(sid) => {
                mobileDrawer = 'none';
                roomSessionId = sid;
              }}
            />
          </div>
        {/if}
      {/if}
    </div>
  </PageBody>
</div>

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
  .th-center-col {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .th-scrim {
    position: absolute;
    inset: 0;
    z-index: var(--z-drawer);
    border: 0;
    padding: 0;
    background: var(--scrim);
    cursor: pointer;
  }
  .th-drawer {
    position: absolute;
    inset-block: 0;
    z-index: var(--z-drawer);
    inline-size: min(360px, 88vw);
    max-inline-size: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    box-shadow: var(--glass-shadow);
  }
  .th-drawer.start {
    inset-inline-start: 0;
  }
  .th-drawer.end {
    inset-inline-end: 0;
  }
</style>
