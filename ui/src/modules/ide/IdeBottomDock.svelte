<script lang="ts">
  // Collapsible, height-resizable (120px..85vh), maximizable IDE Bottom Tool Dock
  // hosting Debug, Run & tests, Git, Problems, Usages, and Terminal.
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
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ideDockPrefs } from './debug/ideDockPrefs.svelte';
  import { ideGit } from './git/ideGitController.svelte';
  import { ide, type IdeBottomTab } from './ideStore.svelte';

  type DockTab = IdeBottomTab | 'git';

  const IdeDebuggerPanelLazy = lazyComponent(() => import('./IdeDebuggerPanel.svelte'));
  const IdeGitToolPanelLazy = lazyComponent(() => import('./git/IdeGitToolPanel.svelte'));
  const IdeProblemsPanelLazy = lazyComponent(() => import('./IdeProblemsPanel.svelte'));
  const IdeRunPanelLazy = lazyComponent(() => import('./IdeRunPanel.svelte'));
  const IdeTerminalPanelLazy = lazyComponent(() => import('./IdeTerminalPanel.svelte'));
  const IdeUsagesPanelLazy = lazyComponent(() => import('./debug/IdeUsagesPanel.svelte'));

  interface Props {
    defaultHeight?: number;
  }
  let { defaultHeight = 300 }: Props = $props();

  const LS_DOCK_H = 'otto_ide_dock_h';
  const DOCK_MIN = 120;

  let winH = $state(typeof window !== 'undefined' ? window.innerHeight : 900);
  const dockMax = $derived(Math.max(520, Math.round(winH * 0.85)));
  const maxTargetH = $derived(Math.max(460, Math.round(winH * 0.78)));

  let dockHeight = $state(loadPaneWidth(LS_DOCK_H, 300, DOCK_MIN, 760));
  let maximized = $state(false);

  const effectiveHeight = $derived(
    Math.max(DOCK_MIN, Math.min(dockMax, maximized ? maxTargetH : dockHeight)),
  );
  const activeDockTab = $derived(ide.activeBottomTab as DockTab);

  $effect(() => {
    void ide.target.root;
    void ide.projectRoots.length;
    ideGit.syncVisibleRoots({ probeAll: false });
  });

  $effect(() => {
    const cur = activeDockTab;
    if (ideDockPrefs.isDockTabHidden(cur)) {
      ideDockPrefs.ensureDockTabVisible(cur);
    }
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

  const allDockTabs = $derived<readonly TabItem<DockTab>[]>([
    { id: 'debug', label: 'Debug', icon: 'bug', count: debugCount, testid: 'ide-tab-debug' },
    { id: 'run', label: 'Run & tests', icon: 'play', count: runCount, testid: 'ide-tab-run' },
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
    { id: 'terminal', label: 'Terminal', icon: 'terminal', testid: 'ide-tab-terminal' },
  ]);

  const dockTabs = $derived(
    allDockTabs.filter((t) => !ideDockPrefs.isDockTabHidden(t.id) || t.id === activeDockTab),
  );

  function onTabSelect(tab: DockTab): void {
    ide.openBottomTab(tab as IdeBottomTab);
  }

  function openDockViewMenu(e: MouseEvent): void {
    const allIds = allDockTabs.map((t) => t.id);
    const items: MenuItem[] = allDockTabs.map((t) => {
      const visible = !ideDockPrefs.isDockTabHidden(t.id);
      return {
        label: t.label,
        checked: visible,
        action: () => {
          const wasActive = activeDockTab === t.id;
          ideDockPrefs.toggleDockTabHidden(t.id, allIds);
          if (wasActive && ideDockPrefs.isDockTabHidden(t.id)) {
            const next = allDockTabs.find((x) => !ideDockPrefs.isDockTabHidden(x.id));
            if (next) onTabSelect(next.id);
          }
        },
      };
    });
    items.push(
      { separator: true },
      {
        label: maximized ? 'Restore dock height' : 'Maximize dock height (78%)',
        icon: maximized ? 'minimize' : 'maximize',
        action: () => {
          ide.toggleBottomDock(true);
          maximized = !maximized;
        },
      },
      {
        label: 'Reset visible tabs',
        icon: 'refresh',
        disabled: ideDockPrefs.hiddenDockTabs.length === 0,
        action: () => ideDockPrefs.resetDockTabs(),
      },
    );
    ctxMenu.show(e, items);
  }

  function onHeaderClickCapture(e: MouseEvent): void {
    if (ide.bottomDockOpen) return;
    const hitTab = (e.target as HTMLElement | null)?.closest('[role="tab"]');
    if (hitTab) ide.toggleBottomDock(true);
  }
</script>

<svelte:window onresize={() => (winH = window.innerHeight)} />

<section
  class="dock-shell"
  class:is-open={ide.bottomDockOpen}
  aria-label="IDE bottom tool dock"
  data-testid="ide-bottom-dock"
  style:block-size={ide.bottomDockOpen && !viewport.isPhone ? `${effectiveHeight}px` : undefined}
>
  {#if ide.bottomDockOpen && !viewport.isPhone}
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
      idBase={ide.bottomDockOpen ? 'ide-dock' : undefined}
      size="s"
      testid="ide-dock-tabs"
    >
      {#snippet trailing()}
        <button
          type="button"
          class="icon-btn dock-act"
          aria-label="Dock view options"
          title="Show or hide dock tabs"
          onclick={openDockViewMenu}
          data-testid="ide-dock-view-menu"
        >
          <Icon name="more" size={12} />
        </button>
        {#if ide.bottomDockOpen}
          <button
            type="button"
            class="icon-btn dock-act"
            aria-label={maximized ? 'Restore dock height' : 'Maximize dock height'}
            title={maximized ? 'Restore dock height' : 'Maximize dock height (78%)'}
            onclick={() => (maximized = !maximized)}
            data-testid="ide-dock-maximize"
          >
            <Icon name={maximized ? 'minimize' : 'maximize'} size={12} />
          </button>
        {/if}
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
      {/snippet}
    </Tabs>
  </div>

  {#if ide.bottomDockOpen}
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
  .dock-resizer { position: absolute; inset-block-start: -4px; inset-inline: 0; block-size: 8px; cursor: row-resize; z-index: var(--z-sticky); }
  .dock-resizer::after { content: ''; position: absolute; inset-block-start: 4px; inset-inline: 0; block-size: 2px; background: transparent; transition: background var(--dur-fast) ease-out; }
  .dock-resizer:hover::after, .dock-resizer:focus-visible::after { background: var(--accent-text); }
  .dock-header { flex-shrink: 0; min-inline-size: 0; block-size: 32px; min-block-size: 32px; max-block-size: 32px; overflow: hidden; }
  .dock-header :global(.otabs) { block-size: 32px; min-block-size: 32px; }
  .dock-act { block-size: 22px; inline-size: 22px; }
  .dock-body { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; flex-direction: column; overflow: hidden; }
  @media (max-width: 640px) {
    .dock-shell.is-open { flex: 1; block-size: 100%; max-block-size: none; }
    .dock-resizer { display: none; }
  }
</style>
