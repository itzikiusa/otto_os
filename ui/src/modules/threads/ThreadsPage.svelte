<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { phoneDevice } from '../../lib/stores/phoneDevice.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Drawer from '../../shell/Drawer.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { keyContext } from '../../lib/keys';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadPaneWidth } from '../../lib/paneResizer';
  import { router } from '../../lib/router.svelte';
  import { openFile } from '../../lib/stores/openfile.svelte';
  import { pageChrome } from '../../lib/stores/pageChrome.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { lightMode } from '../../lib/lightMode.svelte';
  import { keyboard } from '../../lib/stores/keyboard.svelte';
  import { trackComposerFocus, typingChrome } from './threadsTyping'; import { consumeThreadDraftHandoff } from './recapThreadConsumer';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import NewThreadStage from './NewThreadStage.svelte';
  import ThreadCanvas from './ThreadCanvas.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  const ThreadInspector = lazyComponent(() => import('./ThreadInspector.svelte'));
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { DetailsCardView, RelatedListView, PullRequestsView, StartRoomView, TerminalDock, ThreadSheetsView } from './threadLazy.svelte';
  import { threadRepos } from './threadRepos.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import ThreadSidebar from './ThreadSidebar.svelte';
  import { buildThreadMenu } from './threadMenu';
  import ThreadParentChip from './runtime/ThreadParentChip.svelte';
  import ProjectBadge from './ProjectBadge.svelte';
  import ThreadPopover from './ThreadPopover.svelte';
  import { CARD_RESERVE, cardFloats } from './threadDetails';
  import { relatedPopover } from './relatedPopover.svelte';
  import { threadsPageMenu } from './threadsPageMenu';
  import { threadPrefs } from './runtime/threadPrefs.svelte';
  import { fitRightPanel } from './threadsLayout';
  import { prepareDeepLink } from '../../lib/deepLinkOpen';
  import { runtimeThreadMenu } from './runtime/runtimeMenu';
  import { asThreadSession, isRuntimeThread, runtimeThreads } from './runtime/runtimeThreads.svelte';
  import { isThreadSession, projectNameFromCwd } from './threadsHelpers';
  import { threadOrigin } from './threadOrigins';
  import { personalAgentName } from '../../lib/personalAgentBadges';
  import { threads, type ThreadsLayoutMode } from './threadsStore.svelte';
  import { initialThreadId } from './threadNav.svelte';

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
  // The thread list is a column wherever it leaves the conversation room (an
  // iPad in portrait once the navigator folds), a drawer below that — and on
  // a phone on its side, where the conversation needs the width more.
  const listDrawer = $derived(viewport.isPhone || viewport.isShort || (stageWidth > 0 && stageWidth < 760));
  // At 1280 with both panels open the right panel gives way (never below its
  // minimum) so the conversation keeps ~640 px; a manual width comes back
  // whenever there is room for it.
  const inspectorShownW = $derived(fitRightPanel(inspectorW, stageWidth, !listDrawer && threads.sidebarOpen ? sidebarW : 0));

  $effect(() => threads.setInspectorCompact(compact));

  // Typing with the on-screen keyboard up: what folds away (threadsTyping.ts).
  // `focusIn` is where the caret is — the composer, the attention dock above
  // it (a question's answer, a secret), or elsewhere (the terminal drawer).
  let pageEl = $state<HTMLDivElement | null>(null);
  let focusIn = $state<'composer' | 'dock' | null>(null);
  $effect(() => (pageEl ? trackComposerFocus(pageEl, (f) => (focusIn = f)) : undefined));
  const typing = $derived(typingChrome(keyboard.open, focusIn !== null, keyboard.visibleHeight));
  // The keyboard opening/closing resizes every timeline at once; one that was
  // following the latest message keeps it in view (RuntimeFollow marks it
  // `data-at-bottom`; Chromium's resize scroll event can beat its observer).
  $effect(() => {
    void [keyboard.visibleHeight, typing];
    const el = untrack(() => pageEl);
    const frame = requestAnimationFrame(() => {
      for (const s of el?.querySelectorAll<HTMLElement>('.rt-scroll[data-at-bottom="1"]') ?? []) s.scrollTop = s.scrollHeight;
    });
    return () => cancelAnimationFrame(frame);
  });
  onMount(() => () => threads.setInspectorCompact(false));

  // Newest activity first, so the page opens on the most recent thread (not
  // an agent's routine run or another module's helper).
  const liveThreadIds = $derived(
    [...ws.sessions.filter((s) => !s.archived && isThreadSession(s)), ...runtimeThreads.rows()]
      .filter((s) => threadOrigin(s) === 'main')
      .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at))
      .map((s) => s.id),
  );
  // The focused thread is read: clear a runtime thread's unread dot.
  $effect(() => {
    const id = threads.focusedSessionId;
    if (id) untrack(() => runtimeThreads.markRead(id));
  });

  // ⌘K opens on these threads, what needs you first (threadSwitcherSource;
  // its own chunk, so the page does not carry it).
  onMount(() => {
    let off: (() => void) | null = null;
    let gone = false;
    void import('./threadSwitcherSource.svelte').then((m) => (gone ? undefined : (off = m.installThreadSwitcher())));
    return () => {
      gone = true;
      off?.();
    };
  });
  // Subscribe to live cross-session messages + MCP DB activity while mounted.
  onMount(() => threads.mountLive());
  // The header's phone button used to learn this at mount; with the handoff
  // only in ⋯ now, a paired phone must know before the first menu opens.
  onMount(() => void phoneDevice.ensure());
  // Structured runtime threads: their rows, and live events for open ones.
  onMount(() => runtimeThreads.mount());
  // …and every workspace's terminal threads, whatever the sidebar toggle says.
  onMount(() => {
    ws.otherWsWanted++;
    void ws.refreshOtherSessions();
    return () => void ws.otherWsWanted--;
  });
  $effect(() => {
    for (const wid of new Set([ws.currentId, ws.scratch ? SCRATCH_WORKSPACE_ID : null])) {
      if (wid) untrack(() => runtimeThreads.load(wid)).catch(() => {});
    }
    // …and the user's threads of every other workspace (one started on the
    // phone lands in whatever workspace the phone had open).
    untrack(() => runtimeThreads.loadAll()).catch(() => {});
  });

  // Like T3, no side-panel rail and no status bar while Threads is open (the
  // sidebar's status dots carry the working count). Not persisted: a saved
  // open Side panel is shown closed here (transient) and comes back on
  // leaving; opening it on purpose (shortcut / ⌘K) shows it.
  onMount(() => {
    // foldNav: an iPad folds Otto's navigator to its rail, so the thread list
    // gets the left column (App.svelte).
    const restoreChrome = pageChrome.set({ hideSideRail: true, hideStatusBar: true, foldNav: true });
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
    if (viewport.isPhone) return;
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

  $effect(() => {
    if (router.module !== 'threads') return;
    const handoff = untrack(() => consumeThreadDraftHandoff());
    if (handoff) untrack(() => threads.startNewThread({ workspaceId: handoff.workspaceId ?? ws.currentId, draft: handoff.draft }));
    if (!openFile.request) return;
    const focused = threads.focusedSessionId ? ws.getSession(threads.focusedSessionId) : null;
    const wsKey = focused?.workspace_id ?? ws.currentId ?? 'scratch';
    untrack(() => threads.consumeOpenFileRequest(wsKey));
  });

  // Route sync: `#/threads`, `#/threads/<sessionId>` and `#/threads/new`
  // (the composer — the New session sheet's "Start a thread" link).
  let lastRoutedId: string | null = null;
  $effect(() => {
    if (router.module !== 'threads') return;
    const routeSessionId = router.parts[1] ?? null;
    if (routeSessionId === 'new') {
      lastRoutedId = null;
      untrack(() => {
        threads.startNewThread({ workspaceId: ws.currentId });
        router.replace('threads');
      });
    } else if (routeSessionId && routeSessionId !== lastRoutedId) {
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
          threads.selectThread(initialThreadId(liveThreadIds) ?? liveThreadIds[0], { syncRoute: false });
        } else {
          threads.newDraftMode = true;
        }
      }
    });
  });

  // ⌘T (and its browser-tab alias ⌃⇧T) starts a thread here, not the
  // terminal New session sheet; an open dialog keeps the chord.
  $effect(() => {
    const claim = (e: KeyboardEvent): boolean => {
      if (router.module !== 'threads' || !(e.metaKey || e.ctrlKey) || e.shiftKey || ui.overlayOpen) return false;
      if (e.key.toLowerCase() !== 't') return false;
      e.preventDefault();
      threads.startNewThread({ workspaceId: ws.currentId });
      return true;
    };
    keyContext.pageChords = claim;
    return () => {
      if (keyContext.pageChords === claim) keyContext.pageChords = null;
    };
  });

  // Register ⌘K commands while Threads is open.
  $effect(() =>
    registry.register('threads', [
      {
        id: 'threads.new',
        title: 'New thread',
        group: 'Threads',
        shortcut: '⌘T',
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
  const threadListError = $derived((failedListId ? runtimeThreads.loadErrors[failedListId] : null) ?? ws.sessionsError ?? null);
  function retryThreads(): void {
    if (failedListId) void runtimeThreads.load(failedListId).catch(() => {});
    if (ws.sessionsError) void ws.retrySessions();
  }
  const showWelcomeStage = $derived(threads.newDraftMode || threads.paneIds.length === 0);
  const focused = $derived.by(() => {
    const id = showWelcomeStage ? null : threads.focusedSessionId;
    if (!id) return null;
    const rt = runtimeThreads.get(id);
    return ws.getSession(id) ?? (rt ? asThreadSession(rt) : null);
  });
  const crumbProject = $derived.by(() => {
    // A personal agent's thread: its agent's name, never its folder's id.
    const agent = focused ? personalAgentName(focused.meta as Record<string, unknown> | undefined, focused.title) : null;
    if (agent && !showWelcomeStage) return agent;
    const cwd = focused?.cwd ?? (showWelcomeStage ? threads.stageCwd : null) ?? ws.current?.root_path ?? null;
    return cwd ? projectNameFromCwd(cwd, ws.current?.name ?? 'Workspace') : (ws.current?.name ?? 'Scratch');
  });
  const crumbTitle = $derived(showWelcomeStage ? 'New thread' : (focused?.title ?? 'Thread'));
  async function renameFocusedThread(): Promise<void> {
    if (!focused) return;
    const id = focused.id;
    const next = await confirmer.promptText('Thread name', { title: 'Rename thread', initial: focused.title, confirmLabel: 'Rename' });
    if (next == null || !next.trim() || next.trim() === focused.title) return;
    try {
      if (runtimeThreads.get(id)) await runtimeThreads.rename(id, next.trim());
      else await ws.renameSession(id, next.trim());
    } catch (e) { toastError('Couldn’t rename thread', e); }
  }
  const rightOpen = $derived(threads.inspectorVisible);
  const contextCwd = $derived(
    focused?.cwd ?? (showWelcomeStage ? threads.stageCwd : null) ?? ws.current?.root_path ?? null,
  );
  const contextRepo = $derived(threadRepos.repoFor(contextCwd, focused?.workspace_id ?? ws.currentId));
  const maximized = $derived(!compact && threads.inspectorOpen && threadsChrome.maximized);
  // Thread details (ThreadDetailsCard): float top-right beside ONE
  // conversation while the right panel is closed and a whole reading column
  // is left over (threadDetails.ts `cardFloats`) — the column then centres in
  // the space left of it (`--thread-card-reserve` on .th-stage), so the card
  // never covers a message. Anywhere else the card heads the right panel.
  // Hidden with its ×, remembered on this device; ⋯ › View brings it back.
  const cardRoom = $derived(
    cardFloats({
      centerWidth,
      compact,
      rightPanel: threads.inspectorOpen,
      panes: showWelcomeStage ? 1 : threads.paneIds.length,
      altView: !!threadsChrome.stageView,
    }),
  );
  const cardShown = $derived(cardRoom && threadsChrome.cardVisible && !!contextCwd);
  let menuBtn = $state<HTMLButtonElement | null>(null);

  // The toolbar's one ⋯: the focused thread's actions, then the view toggles
  // that used to be their own toolbar buttons (threadsPageMenu.ts).
  function openThreadMenu(e: MouseEvent): void {
    const threadItems: MenuItem[] = !focused ? []
      : isRuntimeThread(focused) ? runtimeThreadMenu(focused.id)
      : buildThreadMenu(focused);
    ctxMenu.show(e, threadsPageMenu(
      {
        threadItems,
        layout: threads.layoutMode,
        terminalOpen: threadsChrome.terminalOpen,
        cardOn: cardRoom ? threadsChrome.cardVisible : null,
        threadId: focused?.id ?? null,
        compact,
        // A phone on its side is tablet-wide but still the phone: no hand-off to itself.
        phone: viewport.isPhone || viewport.isShort,
        sidebarVisible: !listDrawer && threads.sidebarOpen,
        midTurn: threadPrefs.midTurn,
        tools: threadPrefs.tools,
      },
      {
        newThread: () => threads.startNewThread({ workspaceId: ws.currentId }),
        related: (id) => relatedPopover.open(id),
        toggleCard: () => threadsChrome.toggleCard(),
        layout: pickLayout,
        toggleTerminal: () => threadsChrome.setTerminal(!threadsChrome.terminalOpen),
        setMidTurn: (m) => threadPrefs.setMidTurn(m),
        setTools: (t) => threadPrefs.setTools(t),
      },
    ));
  }

  function toggleLeftBar(): void {
    if (listDrawer) {
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

<div
  class="th-page"
  class:kb-nohead={!typing.header}
  class:kb-nobanners={!typing.banners}
  class:kb-noterm={!typing.terminal}
  class:kb-tight={typing.tight}
  class:kb-dockhead={typing.tight && focusIn === 'composer'}
  data-testid="threads-page"
  bind:this={pageEl}
>
  <PageHeader title="Threads">
    {#snippet leading()}
      {#if viewport.isShort || (viewport.isPhone && !lightMode.active)}
        <!-- Phone landscape, or Phone portrait in Full mode (where the shell
             top bar steps aside): keep Otto's Navigator, workspace switcher and
             notification bell one tap away. -->
        <button
          type="button"
          class="icon-btn"
          aria-label="Open navigator"
          title="Open navigator"
          aria-haspopup="dialog"
          aria-expanded={ui.navDrawerOpen}
          data-testid="threads-open-navigator"
          onclick={() => (ui.navDrawerOpen = !ui.navDrawerOpen)}
        >
          <Icon name="sidebar" size={14} />
        </button>
      {/if}
      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={listDrawer ? mobileSidebarOpen : threads.sidebarOpen}
        aria-pressed={listDrawer ? mobileSidebarOpen : threads.sidebarOpen}
        aria-label="Thread list"
        title={(listDrawer ? mobileSidebarOpen : threads.sidebarOpen) ? 'Hide the thread list' : 'Show the thread list'}
        data-testid="threads-toggle-sidebar"
        onclick={toggleLeftBar}
      >
        <!-- The threads icon, not a sidebar glyph: the app rail's own sidebar
             button sits right beside it and toggles the navigator. -->
        <Icon name="threads" size={14} />
      </button>
    {/snippet}

    {#snippet titleContent()}
      <span class="th-crumbs" data-testid="threads-breadcrumb">
        <ProjectBadge name={crumbProject} />
        <span class="th-crumb-project ellipsis" dir="auto">{crumbProject}</span>
        <span class="th-crumb-sep" aria-hidden="true">/</span>
        {#if focused}
          <button type="button" class="th-crumb-title th-crumb-btn ellipsis" dir="auto" title="{crumbTitle} — double-click to rename" ondblclick={() => void renameFocusedThread()} onkeydown={(e) => { if (e.key === 'F2') { e.preventDefault(); void renameFocusedThread(); } }}>{crumbTitle}</button>
        {:else}
          <span class="th-crumb-title ellipsis" dir="auto">{crumbTitle}</span>
        {/if}
      </span>
    {/snippet}

    {#snippet badge()}
      {#if focused}<span class="th-header-chips"><ThreadParentChip threadId={focused.id} /></span>{/if}
    {/snippet}

    {#snippet actions()}
      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={rightOpen}
        aria-pressed={rightOpen}
        aria-label="Right panel"
        title={rightOpen ? 'Hide the right panel' : 'Show the right panel: files, changes, pull request, side chat'}
        data-icon="panel"
        data-testid="threads-toggle-inspector"
        onclick={toggleRightBar}
      >
        <Icon name="panel" size={14} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-haspopup="menu"
        aria-label="Thread and view actions"
        title="More: thread actions, project card, terminal, layout"
        data-icon="more"
        data-testid="threads-thread-menu"
        bind:this={menuBtn}
        onpointerenter={() => { TerminalDock.prefetch(); RelatedListView.prefetch(); }}
        onclick={openThreadMenu}
      >
        <Icon name="more" size={14} />
      </button>
    {/snippet}
  </PageHeader>

  <PageBody width="full" padded={false} fill>
    <div class="th-workspace" class:compact bind:clientWidth={stageWidth}>
      {#if !listDrawer && threads.sidebarOpen}
        <div class="th-left-col" style:inline-size="{compact ? Math.min(sidebarW, 240) : sidebarW}px">
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
        {#if threadListError && !(liveThreadIds.length === 0 && showWelcomeStage)}
          <LoadState what="threads" variant="compact" error={threadListError} onretry={retryThreads} />
        {/if}
        <div class="th-stage" style:--thread-card-reserve={cardShown ? `${CARD_RESERVE}px` : '0px'}>
          {#if threadListError && liveThreadIds.length === 0 && showWelcomeStage}
            <LoadState what="threads" error={threadListError} empty onretry={retryThreads} />
          {:else if threadsChrome.stageView === 'prs'}
            <LazyMount lazy={PullRequestsView} what="pull requests" variant="panel" props={{ repo: contextRepo }} />
          {:else if showWelcomeStage}
            <NewThreadStage />
          {:else}
            <ThreadCanvas onstartroom={(sid) => (roomSessionId = sid)} />
          {/if}
          {#if cardShown && contextCwd && DetailsCardView.component}
            <div class="th-card-float">
              <DetailsCardView.component floating cwd={contextCwd} threadId={focused?.id ?? null} ondismiss={() => threadsChrome.toggleCard()} />
            </div>
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
        <div class="th-right-col" class:th-max={maximized} style:inline-size={maximized ? null : `${inspectorShownW}px`}>
          <LazyMount lazy={ThreadInspector} what="the thread details" variant="panel" props={{}} />
        </div>
      {/if}

      {#if listDrawer}
        <Drawer bind:open={mobileSidebarOpen} side="left" title="Threads" width="min(360px, 100vw)">
          <ThreadSidebar
            onselect={() => (mobileSidebarOpen = false)}
            onstartroom={(sid) => { mobileSidebarOpen = false; roomSessionId = sid; }}
          />
        </Drawer>
      {/if}
      {#if compact}
        <Drawer bind:open={threads.inspectorOverlayOpen} side="right" title="Thread details" width="min(420px, 100vw)">
          <LazyMount lazy={ThreadInspector} what="the thread details" variant="panel" props={{}} />
        </Drawer>
      {/if}
    </div>
  </PageBody>
</div>

{#if threadsChrome.sheet && ThreadSheetsView.component}
  <ThreadSheetsView.component />
{/if}

<!-- ⋯ › Send to another thread…: message a thread, open one beside, start a
     room — under the toolbar's ⋯ (the details card lists the subagents). -->
{#if relatedPopover.openFor && menuBtn && RelatedListView.component}
  {@const relatedId = relatedPopover.openFor}
  <ThreadPopover anchor={menuBtn} label="Send to another thread" testid="threads-related-popover" onclose={() => relatedPopover.close()}>
    <RelatedListView.component threadId={relatedId} onstartroom={(sid: string) => (roomSessionId = sid)} ondone={() => relatedPopover.close()} />
  </ThreadPopover>
{/if}

{#if roomSessionId && StartRoomView.component}
  <StartRoomView.component sessionId={roomSessionId} onclose={() => (roomSessionId = null)} />
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
    gap: var(--sp-4);
    min-width: 0;
    font-size: var(--fs-l);
    font-weight: 400;
  }
  .th-header-chips { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
  .th-crumb-project {
    color: var(--text-dim);
    max-width: 200px;
  }
  .th-crumb-sep {
    color: var(--text-dim);
  }
  .th-crumb-title { color: var(--text); font-weight: 600; min-width: 0; }
  .th-crumb-btn { border: 0; background: none; padding: 0 2px; border-radius: var(--radius-s); font: inherit; font-weight: 600; color: var(--text); cursor: default; }
  .th-crumb-btn:hover { background: var(--hover); }
  @media (max-width: 640px), (max-height: 500px) and (pointer: coarse) {
    .th-crumbs > :global(.pb), .th-crumb-project, .th-crumb-sep { display: none; }
  }
  /* A phone on its side: the one header row is a little lower. */
  @media (max-height: 500px) and (pointer: coarse) {
    .th-page :global(.page-header-bar) { --ph-h: 40px; }
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
  /* While the details card floats, the conversation keeps out of its way. */
  .th-stage {
    padding-inline-end: var(--thread-card-reserve, 0px);
  }
  .th-card-float {
    position: absolute;
    inset-block-start: var(--sp-6);
    inset-inline-end: 16px;
    inline-size: calc(var(--thread-card-reserve) - 32px);
    max-block-size: calc(100% - 2 * var(--sp-6));
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
  /* The conversation's height chain (RuntimeThread's .rt-scroll / .rt-dock):
     the composer never leaves the screen. When height runs short the notices
     and the attention dock above the composer shrink (and scroll inside),
     the composer keeps its size, and the timeline keeps a one-line peek. */
  .th-page :global(.rt-scroll) {
    min-block-size: 22px;
  }
  .th-page :global(.rt-dock) {
    min-block-size: 0;
  }
  .th-page :global(.rt-dock > :not(.rt-composer)) {
    min-block-size: 0;
    overflow: auto;
  }
  .th-page :global(.rt-composer) {
    flex-shrink: 0;
  }
  /* Typing with the on-screen keyboard up (threadsTyping.ts): fold what the
     reply does not need, the least needed first. */
  .th-page.kb-nohead > :global(.page-header-bar) {
    display: none;
  }
  .th-page.kb-nobanners :global(.rt-dock [data-banner='plan']),
  .th-page.kb-nobanners :global(.rt-dock [data-banner='queue']),
  .th-page.kb-nobanners :global(.rt-dock [data-testid='runtime-banners-more']) {
    display: none;
  }
  .th-page.kb-noterm :global(:is([data-testid='threads-terminal-dock'], [data-testid^='thread-terminal-drawer-'])) {
    display: none;
  }
  /* Tight (a phone on its side, an iPhone SE): the composer is one row —
     text, attach, send/stop — with the model and access chips back on blur;
     the attention dock shows its one-line head while the caret is in the
     composer (its answers return when you leave it). */
  .th-page.kb-tight :global(.rt-dock) {
    padding-block-end: 4px;
  }
  .th-page.kb-tight :global(.rt-composer) {
    flex-direction: row;
    flex-wrap: wrap;
    align-items: flex-end;
    column-gap: 6px;
    padding-block: 4px;
    padding-inline: 10px 6px;
  }
  .th-page.kb-tight :global(.rt-composer > :not(textarea, .rt-foot)) {
    flex-basis: 100%;
  }
  .th-page.kb-tight :global(.rt-composer textarea) {
    flex: 1 1 0;
    min-inline-size: 0;
    min-block-size: 0;
    max-block-size: calc(3lh + 4px);
    align-self: center;
  }
  .th-page.kb-tight :global(.rt-foot) {
    flex: none;
    gap: 4px;
  }
  .th-page.kb-tight :global(.rt-foot :is(.rt-grow, [data-testid='runtime-ask-agent'])) {
    display: none;
  }
  .th-page.kb-dockhead :global(.rt-dock :is(.ad-body, .ad-jump)) {
    display: none;
  }
  /* A narrow phone (iPhone SE): the composer's foot stays ONE row — send /
     stop, attach and the model chip (truncated, never below ~5 letters); the
     access mode shows as its icon (its name stays in the label and the
     menu) instead of wrapping both chips onto a row of their own. */
  @media (pointer: coarse) {
    @container (max-inline-size: 420px) {
      .th-page :global(.rt-foot) { flex-wrap: nowrap; }
      .th-page :global(.rt-foot .rt-grow) { order: 0; flex-basis: auto; flex-wrap: nowrap; }
      .th-page :global(.rt-foot .rt-agent) { min-inline-size: 72px; }
      .th-page :global(.rt-foot .ac:not(.static) > .ellipsis) { display: none; }
    }
    /* 16px stops iOS zoom-on-focus that leaves the Threads header cut off. */
    .th-page :global(:is(textarea, select, input:not([type='checkbox'], [type='radio'], [type='range'], [type='file']))) {
      font-size: 16px !important; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
    }
  }
</style>
