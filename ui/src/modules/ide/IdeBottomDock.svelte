<script lang="ts">
  // Collapsible, height-resizable (120px..85vh), maximizable IDE Bottom Tool Dock
  // hosting Terminal, Problems, Git and Run, plus Debug, Usages and DB Watch while they
  // are in use (`ideDockTabs.svelte.ts`).
  // `stack` (the phone layout's Tools pane): always open, fills the pane, no
  // resizer, Terminal first, and picking a tab never writes the workbench's
  // persisted "dock open" preference.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import {
    loadPaneWidth,
    paneResizer,
    RESIZE_TITLE_VERTICAL,
    savePaneWidth,
  } from '../../lib/paneResizer';
  import { ideDockPrefs } from './debug/ideDockPrefs.svelte';
  import { ideDbWatch } from './dbwatch/ideDbWatchState.svelte';
  import { ideGit } from './git/ideGitController.svelte';
  import { IDE_DOCK_ON_DEMAND, ideDockOrder, ideDockTabMemory, ideDockTabVisible } from './ideDockTabs.svelte';
  import { ide, type IdeBottomTab } from './ideStore.svelte';
  import { dockMaxFor } from './shell/ideLayout.svelte';

  type DockTab = IdeBottomTab | 'git';

  const IdeDbWatchPanelLazy = lazyComponent(() => import('./dbwatch/IdeDbWatchPanel.svelte'));
  const IdeDebuggerPanelLazy = lazyComponent(() => import('./IdeDebuggerPanel.svelte'));
  const IdeGitToolPanelLazy = lazyComponent(() => import('./git/IdeGitToolPanel.svelte'));
  const IdeProblemsPanelLazy = lazyComponent(() => import('./IdeProblemsPanel.svelte'));
  const IdeRunPanelLazy = lazyComponent(() => import('./IdeRunPanel.svelte'));
  const IdeTerminalPanelLazy = lazyComponent(() => import('./IdeTerminalPanel.svelte'));
  const IdeUsagesPanelLazy = lazyComponent(() => import('./debug/IdeUsagesPanel.svelte'));

  interface Props {
    defaultHeight?: number;
    stack?: boolean;
    /** Stacked and the Tools pane is the one showing. */
    stackShowing?: boolean;
  }
  let { defaultHeight = 300, stack = false, stackShowing = false }: Props = $props();
  const open = $derived(stack || ide.bottomDockOpen);

  const LS_DOCK_H = 'otto_ide_dock_h';
  const DOCK_MIN = 120;

  let winH = $state(typeof window !== 'undefined' ? window.innerHeight : 900);
  // The column the dock shares with the editor: the dock never takes more
  // than it minus room for a few editor lines (a short window — a phone on
  // its side — used to get a 300 px dock in a 230 px column, no editor left).
  let colH = $state(0);
  let shellEl: HTMLElement | undefined = $state();
  $effect(() => {
    const col = shellEl?.parentElement;
    if (!col || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => (colH = col.clientHeight));
    ro.observe(col);
    return () => ro.disconnect();
  });
  const dockMax = $derived(dockMaxFor(colH, winH, DOCK_MIN));
  const maxTargetH = $derived(Math.min(dockMax, Math.max(460, Math.round(winH * 0.78))));

  let dockHeight = $state(loadPaneWidth(LS_DOCK_H, 300, DOCK_MIN, 760));
  let maximized = $state(false);

  const effectiveHeight = $derived(
    Math.max(DOCK_MIN, Math.min(dockMax, maximized ? maxTargetH : dockHeight)),
  );
  const activeDockTab = $derived(ide.activeBottomTab as DockTab);

  $effect(() => {
    void ide.target.root;
    void ide.projectRoots.length;
    untrack(() => ideGit.syncVisibleRoots({ probeAll: false }));
  });

  $effect(() => {
    const cur = activeDockTab;
    if (ideDockPrefs.isDockTabHidden(cur)) {
      ideDockPrefs.ensureDockTabVisible(cur);
    }
    // Opened from ⌘K, ⋯, a shortcut or a debug run: an on-demand tab stays for the session.
    untrack(() => ideDockTabMemory.markUsed(cur));
  });

  function setDockHeight(next: number): void {
    maximized = false;
    dockHeight = Math.max(DOCK_MIN, Math.min(dockMax, Math.round(next)));
    savePaneWidth(LS_DOCK_H, dockHeight);
  }

  function resetDockHeight(): void {
    maximized = false;
    dockHeight = Math.max(DOCK_MIN, Math.min(dockMax, defaultHeight));
    savePaneWidth(LS_DOCK_H, dockHeight);
  }

  function startDockDrag(e: MouseEvent): void {
    const startY = e.clientY;
    const startH = effectiveHeight;
    maximized = false;
    startMouseDrag(e, {
      cursor: 'row-resize',
      onMove: (ev) => {
        const delta = startY - ev.clientY;
        dockHeight = Math.max(DOCK_MIN, Math.min(dockMax, Math.round(startH + delta)));
      },
      onEnd: () => savePaneWidth(LS_DOCK_H, dockHeight),
    });
  }

  const debugCount = $derived.by((): string | number | undefined => {
    const st = ide.activeDebug?.state;
    if (st === 'paused') return 'paused';
    if (st === 'running' || st === 'starting') return 'live';
    if (ide.allBreakpoints.length > 0) return ide.allBreakpoints.length;
    return undefined;
  });

  const runCount = $derived.by((): string | number | undefined => {
    const r = ide.activeRun;
    if (!r) return undefined;
    if (r.status === 'running') return 'live';
    if (r.tests.length > 0) {
      const failed = r.tests.filter((t) => t.status === 'failed').length;
      const passed = r.tests.filter((t) => t.status === 'passed').length;
      return failed > 0 ? `${failed} failed` : `${passed}/${r.tests.length}`;
    }
    return r.status;
  });

  const allDockTabs = $derived<readonly TabItem<DockTab>[]>(ideDockOrder<TabItem<DockTab>>([
    { id: 'debug', label: 'Debug', icon: 'bug', count: debugCount, testid: 'ide-tab-debug' },
    { id: 'run', label: stack ? 'Run' : 'Run & tests', icon: 'play', count: runCount, testid: 'ide-tab-run' },
    {
      id: 'git',
      label: 'Git',
      icon: 'branch',
      count: ideGit.totalChangeCount > 0 ? ideGit.totalChangeCount : undefined,
      testid: 'ide-dock-tab-git',
    },
    {
      id: 'problems',
      label: 'Problems',
      icon: 'warning',
      count: ide.problems.length > 0 ? ide.problems.length : undefined,
      testid: 'ide-tab-problems',
    },
    {
      id: 'usages',
      label: 'Usages',
      icon: 'search',
      count: ide.usages.length > 0 ? ide.usages.length : undefined,
      testid: 'ide-tab-usages',
    },
    {
      id: 'dbwatch',
      label: 'DB Watch',
      icon: 'db',
      count: ideDbWatch.changeBadgeCount,
      testid: 'ide-tab-dbwatch',
    },
    { id: 'terminal', label: 'Terminal', icon: 'terminal', testid: 'ide-tab-terminal' },
  ]));

  /** Does an on-demand tool hold something right now (a session, a result, an armed watch)? */
  function tabHasContent(id: DockTab): boolean {
    if (id === 'debug') return Boolean(ide.activeDebug);
    if (id === 'usages') return ide.usages.length > 0 || ide.usagesLoading;
    if (id === 'dbwatch') return Boolean(ideDbWatch.baselineSnapshot || ideDbWatch.liveWatch);
    return true;
  }

  function isTabShown(id: DockTab): boolean {
    return ideDockTabVisible(id, {
      hidden: ideDockPrefs.hiddenDockTabs,
      shown: ideDockTabMemory.shown,
      used: ideDockTabMemory.used,
      hasContent: tabHasContent(id),
      active: false,
    });
  }

  const dockTabs = $derived(
    allDockTabs.filter(
      // A live-session guest follows the host's debugger; its other tools have no guest routes.
      (t) => (!ide.collabIsGuest || t.id === 'debug') && (t.id === activeDockTab || isTabShown(t.id)),
    ),
  );

  function onTabSelect(tab: DockTab): void {
    if (stack) ide.activeBottomTab = tab as IdeBottomTab;
    else ide.openBottomTab(tab as IdeBottomTab);
  }

  // The stacked Tools pane opens on the terminal unless a run, debug session
  // or problem list is what the person was last looking at.
  let stackTabChosen = false;
  $effect(() => {
    if (!stackShowing || stackTabChosen) return;
    stackTabChosen = true;
    untrack(() => {
      if (!ide.activeRun && !ide.activeDebug && ide.activeBottomTab === 'run') ide.activeBottomTab = 'terminal';
    });
  });

  /** The dock's one ⋯: which tools sit on the strip, plus height. An on-demand tool ticked
   *  here stays; unticking any tool hides it until it is opened again. */
  function openDockViewMenu(e: MouseEvent): void {
    const allIds = allDockTabs.map((t) => t.id);
    const items: MenuItem[] = allDockTabs.map((t) => {
      const visible = t.id === activeDockTab || isTabShown(t.id);
      return {
        label: t.label,
        checked: visible,
        action: () => {
          const onDemand = IDE_DOCK_ON_DEMAND.includes(t.id);
          if (visible) {
            if (onDemand) ideDockTabMemory.setShown(t.id, false);
            if (!ideDockPrefs.isDockTabHidden(t.id)) ideDockPrefs.toggleDockTabHidden(t.id, allIds);
            if (activeDockTab === t.id && ideDockPrefs.isDockTabHidden(t.id)) {
              const next = allDockTabs.find((x) => x.id !== t.id && isTabShown(x.id));
              if (next) onTabSelect(next.id);
            }
          } else {
            ideDockPrefs.ensureDockTabVisible(t.id);
            if (onDemand) ideDockTabMemory.setShown(t.id, true);
          }
        },
      };
    });
    items.push({ separator: true });
    if (!stack) {
      items.push({
        label: maximized ? 'Restore dock height' : 'Maximize dock height',
        icon: maximized ? 'minimize' : 'maximize',
        action: () => {
          ide.toggleBottomDock(true);
          maximized = !maximized;
        },
      });
    }
    items.push({
      label: 'Reset visible tabs',
      icon: 'refresh',
      disabled: ideDockPrefs.hiddenDockTabs.length === 0 && ideDockTabMemory.shown.length === 0,
      action: () => {
        ideDockPrefs.resetDockTabs();
        ideDockTabMemory.reset();
      },
    });
    ctxMenu.show(e, items);
  }

  function onHeaderClickCapture(e: MouseEvent): void {
    if (open) return;
    const hitTab = (e.target as HTMLElement | null)?.closest('[role="tab"]');
    if (hitTab) ide.toggleBottomDock(true);
  }
</script>

<svelte:window onresize={() => (winH = window.innerHeight)} />

<section
  bind:this={shellEl}
  class="dock-shell"
  class:is-open={open}
  class:stack
  class:stack-hidden={stack && !stackShowing}
  aria-label="IDE bottom tool dock"
  data-testid="ide-bottom-dock"
  style:block-size={open && !stack ? `${effectiveHeight}px` : undefined}
>
  {#if open && !stack}
    <div
      class="dock-resizer"
      role="separator"
      aria-label="Resize IDE bottom dock"
      title={RESIZE_TITLE_VERTICAL}
      data-testid="ide-dock-resizer"
      use:paneResizer={{
        value: effectiveHeight,
        min: DOCK_MIN,
        max: dockMax,
        orientation: 'horizontal',
        invert: true,
        text: (v) => `${Math.round(v)} pixels tall`,
        onChange: setDockHeight,
        onReset: resetDockHeight,
        onDragStart: startDockDrag,
      }}
    ></div>
  {/if}

  <div class="dock-header" role="presentation" onclickcapture={onHeaderClickCapture}>
    <Tabs
      tabs={dockTabs}
      value={activeDockTab}
      onchange={onTabSelect}
      label="IDE tool panels"
      idBase={open ? 'ide-dock' : undefined}
      size="s"
      testid="ide-dock-tabs"
    >
      {#snippet trailing()}
        <button
          type="button"
          class="icon-btn dock-act"
          aria-label="Dock options"
          title="Tool windows and dock height"
          onclick={openDockViewMenu}
          data-testid="ide-dock-view-menu"
        >
          <Icon name="more" size={12} />
        </button>
        {#if !stack}
          <button
            type="button"
            class="icon-btn dock-act"
            aria-expanded={ide.bottomDockOpen}
            aria-label={ide.bottomDockOpen ? 'Collapse bottom dock' : 'Expand bottom dock'}
            title={ide.bottomDockOpen ? 'Collapse bottom dock' : 'Expand bottom dock'}
            onclick={() => ide.toggleBottomDock()}
          >
            <Icon name={ide.bottomDockOpen ? 'chevronDown' : 'chevronUp'} size={13} />
          </button>
        {/if}
      {/snippet}
    </Tabs>
  </div>

  {#if open}
    <div
      class="dock-body"
      role="tabpanel"
      id="ide-dock-panel-{activeDockTab}"
      aria-labelledby="ide-dock-tab-{activeDockTab}"
    >
      {#if activeDockTab === 'debug'}
        <LazyMount lazy={IdeDebuggerPanelLazy} what="the debugger" variant="panel" />
      {:else if activeDockTab === 'run'}
        <LazyMount lazy={IdeRunPanelLazy} what="run & tests" variant="panel" />
      {:else if activeDockTab === 'git'}
        <LazyMount lazy={IdeGitToolPanelLazy} what="the Git tool window" variant="panel" />
      {:else if activeDockTab === 'problems'}
        <LazyMount lazy={IdeProblemsPanelLazy} what="the problems list" variant="panel" />
      {:else if activeDockTab === 'usages'}
        <LazyMount lazy={IdeUsagesPanelLazy} what="the usages panel" variant="panel" />
      {:else if activeDockTab === 'dbwatch'}
        <LazyMount lazy={IdeDbWatchPanelLazy} what="the database watch panel" variant="panel" />
      {:else if activeDockTab === 'terminal'}
        <LazyMount lazy={IdeTerminalPanelLazy} what="the terminal" variant="panel" />
      {/if}
    </div>
  {/if}
</section>

<style>
  .dock-shell {
    position: relative; display: flex; flex-direction: column; flex-shrink: 0;
    min-inline-size: 0; border-block-start: 1px solid var(--border); background: var(--surface);
  }
  .dock-shell.is-open { max-block-size: 85%; min-block-size: 120px; }
  .dock-resizer { position: absolute; inset-block-start: -3px; inset-inline-start: 0; inset-inline-end: 72px; block-size: 5px; cursor: row-resize; z-index: var(--z-sticky); }
  .dock-resizer::after { content: ''; position: absolute; inset-block-start: 3px; inset-inline: 0; block-size: 2px; background: transparent; transition: background var(--dur-fast) ease-out; }
  .dock-resizer:hover::after, .dock-resizer:focus-visible::after { background: var(--accent-text); }
  .dock-header { flex-shrink: 0; min-inline-size: 0; block-size: 32px; min-block-size: 32px; max-block-size: 32px; overflow: hidden; }
  .dock-header :global(.otabs) { block-size: 32px; min-block-size: 32px; }
  .dock-act { block-size: 22px; inline-size: 22px; }
  .dock-body { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; overflow: hidden; }
  /* Stacked Tools pane: the dock IS the pane (hidden, still mounted, on the others). */
  .dock-shell.stack-hidden { display: none; }
  .dock-shell.stack.is-open { flex: 1; block-size: 100%; max-block-size: none; }
  /* Stacked: 44 px touch tabs whose underline is the row's bottom edge (no dead band under
     it); the header grows instead of clipping when the ⋯ wraps under a strip that overflows
     (the strip itself fades at the edge that hides a tab). */
  .dock-shell.stack .dock-header { block-size: auto; min-block-size: 44px; max-block-size: none; }
  .dock-shell.stack .dock-header :global(.otabs) { block-size: auto; min-block-size: 44px; }
  .dock-shell.stack .dock-header :global(.otab) { min-block-size: 44px; }
  .dock-shell.stack .dock-act { block-size: 36px; inline-size: 36px; }
</style>
