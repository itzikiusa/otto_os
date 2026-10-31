<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { loadPaneWidth } from '../../lib/paneResizer';
  import { router } from '../../lib/router.svelte';
  import { openFile } from '../../lib/stores/openfile.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import StartRoomModal from '../rooms/StartRoomModal.svelte'; // ui-guards: allow — shared session room modal
  import NewThreadStage from './NewThreadStage.svelte';
  import ThreadCanvas from './ThreadCanvas.svelte';
  import ThreadInspector from './ThreadInspector.svelte';
  import ThreadSidebar from './ThreadSidebar.svelte';
  import { isThreadSession } from './threadsHelpers';
  import { threads, type ThreadsLayoutMode } from './threadsStore.svelte';

  const SIDEBAR_W_KEY = 'threads.sidebarW';
  const INSPECTOR_W_KEY = 'threads.inspectorW';

  let sidebarW = $state(loadPaneWidth(SIDEBAR_W_KEY, 272, 240, 380));
  let inspectorW = $state(loadPaneWidth(INSPECTOR_W_KEY, 380, 300, 560));
  let stageWidth = $state(1280);
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
    const known = new Set(ws.sessions.filter((s) => !s.archived).map((s) => s.id));
    if ((!ws.listSettled || ws.sessionsLoading) && known.size === 0) return;
    untrack(() => {
      const valid = threads.paneIds.filter((id) => known.has(id));
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
        title: 'Toggle Threads inspector',
        group: 'Threads',
        run: () => threads.toggleInspector(),
      },
    ]),
  );

  const showWelcomeStage = $derived(threads.newDraftMode || threads.paneIds.length === 0);

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
  <PageHeader title="Threads" icon="threads">
    {#snippet badge()}
      <Badge tone="info" variant="outline" label="Experimental" testid="threads-experimental-badge" />
    {/snippet}

    {#snippet actions()}
      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={compact ? mobileDrawer === 'sidebar' : threads.sidebarOpen}
        aria-pressed={compact ? mobileDrawer === 'sidebar' : threads.sidebarOpen}
        aria-label="Toggle threads inbox sidebar"
        title="Toggle threads inbox"
        data-testid="threads-toggle-sidebar"
        onclick={toggleLeftBar}
      >
        <Icon name="sidebar" size={14} />
      </button>

      <div class="segmented" role="group" aria-label="Multi-session layout" data-testid="threads-layout-switch">
        <button
          type="button"
          class:active={threads.layoutMode === 'single' && !threads.newDraftMode}
          aria-pressed={threads.layoutMode === 'single' && !threads.newDraftMode}
          onkeydown={onTabKey}
          title="Single thread view (1-up)"
          data-testid="threads-layout-single"
          onclick={() => pickLayout('single')}
        >
          <span dir="ltr">1-up</span>
        </button>
        <button
          type="button"
          class:active={threads.layoutMode === 'split' && !threads.newDraftMode}
          aria-pressed={threads.layoutMode === 'split' && !threads.newDraftMode}
          onkeydown={onTabKey}
          title="Side-by-side split view (2 panes)"
          data-testid="threads-layout-split"
          onclick={() => pickLayout('split')}
        >
          Split
        </button>
        <button
          type="button"
          class:active={threads.layoutMode === 'grid' && !threads.newDraftMode}
          aria-pressed={threads.layoutMode === 'grid' && !threads.newDraftMode}
          onkeydown={onTabKey}
          title="2×2 multi-session grid (up to 4 panes)"
          data-testid="threads-layout-grid"
          onclick={() => pickLayout('grid')}
        >
          Grid
        </button>
      </div>

      <button
        type="button"
        class="icon-btn"
        class:th-btn-on={compact ? mobileDrawer === 'inspector' : threads.inspectorOpen}
        aria-pressed={compact ? mobileDrawer === 'inspector' : threads.inspectorOpen}
        aria-label="Toggle thread inspector"
        title="Toggle inspector (Files, Changes, Connections, Peers)"
        data-testid="threads-toggle-inspector"
        onclick={toggleRightBar}
      >
        <Icon name="panel" size={14} />
      </button>

      <button
        type="button"
        class="btn small primary"
        data-testid="threads-new-btn"
        onclick={() => threads.startNewThread({ workspaceId: ws.currentId })}
      >
        <Icon name="plus" size={12} />
        New Thread
      </button>
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

      <div class="th-center-col">
        {#if showWelcomeStage}
          <NewThreadStage />
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
