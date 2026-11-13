<script lang="ts">
  // Collapsible, height-resizable IDE Bottom Tool Dock hosting Debug, Run &
  // Tests, Problems, Usages (LSP references), and Terminal (local or SSH PTY).
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { startMouseDrag } from '../../lib/dragCursor';
  import {
    loadPaneWidth,
    paneResizer,
    RESIZE_TITLE_VERTICAL,
    savePaneWidth,
  } from '../../lib/paneResizer';
  import { plural } from '../../lib/plural';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import IdeDebuggerPanel from './IdeDebuggerPanel.svelte';
  import IdeProblemsPanel from './IdeProblemsPanel.svelte';
  import IdeRunPanel from './IdeRunPanel.svelte';
  import IdeTerminalPanel from './IdeTerminalPanel.svelte';
  import {
    ide,
    ideBasename,
    type IdeBottomTab,
    type IdeReferenceLocation,
  } from './ideStore.svelte';

  interface Props {
    defaultHeight?: number;
  }
  let { defaultHeight = 280 }: Props = $props();

  const LS_DOCK_H = 'otto_ide_dock_h';
  const DOCK_MIN = 160;
  const DOCK_MAX = 640;

  let dockHeight = $state(loadPaneWidth(LS_DOCK_H, 280, DOCK_MIN, DOCK_MAX));
  let tall = $state(false);
  let usagesQuery = $state('');

  const effectiveHeight = $derived(tall ? 480 : dockHeight);

  function setDockHeight(next: number): void {
    tall = false;
    dockHeight = Math.max(DOCK_MIN, Math.min(DOCK_MAX, Math.round(next)));
    savePaneWidth(LS_DOCK_H, dockHeight);
  }

  function resetDockHeight(): void {
    tall = false;
    dockHeight = Math.max(DOCK_MIN, Math.min(DOCK_MAX, defaultHeight));
    savePaneWidth(LS_DOCK_H, dockHeight);
  }

  function startDockDrag(e: MouseEvent): void {
    const startY = e.clientY;
    const startH = effectiveHeight;
    tall = false;
    startMouseDrag(e, {
      cursor: 'row-resize',
      onMove: (ev) => {
        const delta = startY - ev.clientY;
        dockHeight = Math.max(DOCK_MIN, Math.min(DOCK_MAX, Math.round(startH + delta)));
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

  const dockTabs = $derived<readonly TabItem<IdeBottomTab>[]>([
    {
      id: 'debug',
      label: 'Debug',
      icon: 'bug',
      count: debugCount,
      testid: 'ide-tab-debug',
    },
    {
      id: 'run',
      label: 'Run & tests',
      icon: 'play',
      count: runCount,
      testid: 'ide-tab-run',
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
      id: 'terminal',
      label: 'Terminal',
      icon: 'terminal',
      testid: 'ide-tab-terminal',
    },
  ]);

  function onTabSelect(tab: IdeBottomTab): void {
    ide.openBottomTab(tab);
  }

  function onHeaderClickCapture(e: MouseEvent): void {
    if (ide.bottomDockOpen) return;
    const hitTab = (e.target as HTMLElement | null)?.closest('[role="tab"]');
    if (hitTab) ide.toggleBottomDock(true);
  }

  // Group LSP find-references items by file for the Usages tab.
  interface UsageGroup {
    path: string;
    relPath: string;
    items: IdeReferenceLocation[];
  }

  const filteredUsages = $derived.by((): IdeReferenceLocation[] => {
    const q = usagesQuery.trim().toLowerCase();
    if (!q) return ide.usages;
    return ide.usages.filter(
      (u) => u.path.toLowerCase().includes(q) || u.preview.toLowerCase().includes(q),
    );
  });

  const usageGroups = $derived.by((): UsageGroup[] => {
    const map = new Map<string, IdeReferenceLocation[]>();
    for (const u of filteredUsages) {
      const arr = map.get(u.path);
      if (arr) arr.push(u);
      else map.set(u.path, [u]);
    }
    const groups: UsageGroup[] = [];
    for (const [path, items] of map.entries()) {
      groups.push({
        path,
        relPath: ide.relativePath(path) || ideBasename(path),
        items,
      });
    }
    return groups;
  });

  function findAtCursor(): void {
    if (!ide.activePath) return;
    void ide.findReferences(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
  }
</script>

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
      use:paneResizer={{
        value: effectiveHeight,
        min: DOCK_MIN,
        max: DOCK_MAX,
        orientation: 'horizontal',
        invert: true,
        text: (v) => `${Math.round(v)} pixels tall`,
        onChange: setDockHeight,
        onReset: resetDockHeight,
        onDragStart: startDockDrag,
      }}
    ></div>
  {/if}

  <!-- Clicking any tab while the dock is collapsed opens the dock on that tab. -->
  <div class="dock-header" role="presentation" onclickcapture={onHeaderClickCapture}>
    <Tabs
      tabs={dockTabs}
      value={ide.activeBottomTab}
      onchange={onTabSelect}
      label="IDE tool panels"
      idBase={ide.bottomDockOpen ? 'ide-dock' : undefined}
      size="s"
      testid="ide-dock-tabs"
    >
      {#snippet trailing()}
        {#if ide.bottomDockOpen}
          <button
            type="button"
            class="icon-btn"
            aria-label={tall ? 'Restore dock height' : 'Expand dock height'}
            title={tall ? 'Restore dock height' : 'Expand dock height'}
            onclick={() => (tall = !tall)}
          >
            <Icon name={tall ? 'minimize' : 'maximize'} size={12} />
          </button>
        {/if}
        <button
          type="button"
          class="icon-btn"
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
      id="ide-dock-panel-{ide.activeBottomTab}"
      aria-labelledby="ide-dock-tab-{ide.activeBottomTab}"
    >
      {#if ide.activeBottomTab === 'debug'}
        <IdeDebuggerPanel />
      {:else if ide.activeBottomTab === 'run'}
        <IdeRunPanel />
      {:else if ide.activeBottomTab === 'problems'}
        <IdeProblemsPanel />
      {:else if ide.activeBottomTab === 'usages'}
        <div class="usg-root" data-testid="ide-usages-panel">
          {#if ide.usages.length > 0 || ide.usagesSymbol}
            <header class="usg-bar">
              {#if ide.usagesSymbol}
                <Badge tone="accent" label={ide.usagesSymbol} />
              {/if}
              <span class="usg-summary dim">
                {plural(ide.usages.length, 'reference')} across {plural(usageGroups.length, 'file')}
              </span>
              <span class="grow"></span>
              {#if ide.usages.length > 0}
                <div class="input-group usg-search">
                  <Icon name="search" size={12} />
                  <input
                    type="search"
                    dir="ltr"
                    aria-label="Filter usages"
                    placeholder="Filter usages…"
                    bind:value={usagesQuery}
                  />
                </div>
              {/if}
              {#if ide.activePath}
                <button type="button" class="btn small ghost" onclick={findAtCursor}>
                  <Icon name="refresh" size={12} />
                  <span>Find at cursor</span>
                </button>
              {/if}
            </header>
          {/if}

          <div class="usg-body">
            <LoadState
              what="symbol usages"
              loading={ide.usagesLoading}
              empty={ide.usages.length === 0}
              variant="panel"
              rows={3}
            >
              {#snippet emptyView()}
                <EmptyState
                  icon="search"
                  title="No symbol usages yet"
                  body="Place the caret on any symbol in the editor and run Find usages to list every reference across the project."
                  actionLabel={ide.activePath ? 'Find usages at cursor' : undefined}
                  actionIcon="search"
                  actionKind="secondary"
                  onaction={ide.activePath ? findAtCursor : undefined}
                  headingLevel={2}
                />
              {/snippet}

              {#if usageGroups.length === 0}
                <EmptyState
                  icon="filter"
                  title="No matching usages"
                  body="No reference locations match the active filter."
                  actionLabel="Clear filter"
                  actionKind="secondary"
                  onaction={() => (usagesQuery = '')}
                  headingLevel={2}
                />
              {:else}
                {#each usageGroups as group (group.path)}
                  <section class="usg-group" aria-label={`References in ${group.relPath}`}>
                    <div class="usg-file-head">
                      <Icon name="file" size={12} />
                      <span class="usg-file-path mono ellipsis" dir="ltr" title={group.path}>
                        {group.relPath}
                      </span>
                      <Badge tone="neutral" variant="outline" label={String(group.items.length)} />
                    </div>
                    {#each group.items as u, idx (`${u.path}:${u.line}:${u.column}:${idx}`)}
                      <div class="usg-row-wrap reveal-host">
                        <button
                          type="button"
                          class="usg-row"
                          onclick={() => void ide.openFile(u.path, { line: u.line, col: u.column })}
                        >
                          <span class="usg-loc mono dim" dir="ltr">:{u.line}:{u.column}</span>
                          <span class="usg-preview mono ellipsis" dir="ltr" title={u.preview.trim()}>
                            {u.preview.trim() || `${ideBasename(u.path)}:${u.line}`}
                          </span>
                        </button>
                        <button
                          type="button"
                          class="icon-btn reveal-on-hover"
                          aria-label={`Copy ${group.relPath}:${u.line}`}
                          title={`Copy ${group.relPath}:${u.line}`}
                          onclick={() => void copyWithToast(`${group.relPath}:${u.line}:${u.column}`, 'location')}
                        >
                          <Icon name="copy" size={12} />
                        </button>
                      </div>
                    {/each}
                  </section>
                {/each}
              {/if}
            </LoadState>
          </div>
        </div>
      {:else if ide.activeBottomTab === 'terminal'}
        <IdeTerminalPanel />
      {/if}
    </div>
  {/if}
</section>

<style>
  .dock-shell {
    position: relative;
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    min-inline-size: 0;
    border-block-start: 1px solid var(--border);
    background: var(--surface);
  }
  .dock-shell.is-open {
    max-block-size: 65%;
    min-block-size: 160px;
  }
  .dock-resizer {
    position: absolute;
    inset-block-start: -4px;
    inset-inline: 0;
    block-size: 8px;
    cursor: row-resize;
    z-index: var(--z-sticky);
  }
  .dock-resizer::after {
    content: '';
    position: absolute;
    inset-block-start: 4px;
    inset-inline: 0;
    block-size: 2px;
    background: transparent;
    transition: background var(--dur-fast) ease-out;
  }
  .dock-resizer:hover::after,
  .dock-resizer:focus-visible::after {
    background: var(--accent-text);
  }
  .dock-header {
    flex-shrink: 0;
    min-inline-size: 0;
  }
  .dock-body {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .usg-root {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    min-inline-size: 0;
    background: var(--surface);
  }
  .usg-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-block: 4px;
    padding-inline: 10px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .usg-summary {
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .usg-search {
    block-size: 24px;
    inline-size: 180px;
    font-size: var(--fs-xs);
  }
  .usg-body {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
  }
  .usg-group {
    border-block-end: 1px solid var(--separator);
  }
  .usg-file-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 6px;
    padding-inline: 10px;
    background: var(--surface-2);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .usg-file-path {
    flex: 1;
    font-size: var(--fs-xs);
  }
  .usg-row-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline-end: 8px;
    border-block-start: 1px solid var(--separator);
  }
  .usg-row-wrap:hover {
    background: var(--hover);
  }
  .usg-row {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding-block: 4px;
    padding-inline: 22px 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .usg-loc {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .usg-preview {
    flex: 1;
    font-size: var(--fs-xs);
  }
  @media (max-width: 640px) {
    .dock-shell.is-open {
      flex: 1;
      block-size: 100%;
      max-block-size: none;
    }
    .dock-resizer {
      display: none;
    }
    .usg-search {
      inline-size: 100%;
    }
  }
</style>
