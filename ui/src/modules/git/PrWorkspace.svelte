<script lang="ts">
  // A repo's Pull requests tab (ListDetail): the PR list is the list pane
  // (wide preset; ⋯ › Hide Pull requests ⌘B), the open PRs the content — as
  // tabs once two or more are open (several can be open together). The open
  // PR's Files tree sits under the list while its Code view shows. The Detail
  // pane holds the PR's details card or its agent thread (views; ⓘ / ✦ in the
  // PR header, ⌥⌘B). A phone pushes: the list is the page, an opened PR is
  // pushed with "‹ Pull requests". Open tabs are persisted per repo
  // (prWorkspace store). Only the PR in front is mounted: switching tabs
  // repaints from the SWR caches instantly, and nothing heavy (a big diff's
  // row model) stays alive for tabs nobody is looking at.
  import { untrack, type Snippet } from 'svelte';
  import type { PrSummary } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
  import { pageNav } from '../../lib/components/side/pageNav.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { paneResizer } from '../../lib/paneResizer';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { prWorkspace } from './prWorkspaceStore.svelte';
  import { invalidatePr, prDetailCache, prKey } from './pr-cache';
  import PrList from './PrList.svelte';
  import PrDetail from './PrDetail.svelte';
  import PrFilesPanel from './PrFilesPanel.svelte';
  import PrAgentPanel from './PrAgentPanel.svelte';
  import PrAgentLinePick from './PrAgentLinePick.svelte';
  import CreatePr from './CreatePr.svelte';
  import { prAgentBridge } from './pr-agent-bridge.svelte';
  import { prAgentOriginId } from './pr-agent-thread';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { RESIZE_TITLE_VERTICAL } from '../../lib/paneResizer';
  import { nextTabAfterClose } from './gitUiHelpers';
  import { firstNeedingPr } from './prStatus';

  interface Props {
    repoId: string;
  }
  let { repoId }: Props = $props();

  const tabs = $derived(prWorkspace.tabs(repoId));
  /** Freshest copy of each PR the detail pane has read (list row patches). */
  let patch = $state<PrSummary | null>(null);
  /** Titles for the tab strip (from the list rows or the detail cache). */
  let titles = $state<Record<number, string>>({});

  const readNum = (k: string, d: number): number => {
    try { return Number(localStorage.getItem(k)) || d; } catch { return d; }
  };
  function persist(k: string, v: string): void {
    try { localStorage.setItem(k, v); } catch { /* not persisted */ }
  }
  /** The open PR's view (PrDetail): the Files tree shows beside 'code' only. */
  let prView = $state('summary');
  /** The tab strip earns its row with two or more open PRs. */
  const strip = $derived(tabs.open.length >= 2);
  /** Phone: an opened PR is the page (pushed from the list; a deep link lands on it); Back returns. */
  let phoneOpen = $state(false);
  $effect(() => {
    if (prWorkspace.revealed !== repoId) return;
    untrack(() => {
      phoneOpen = true;
      prWorkspace.revealed = null;
    });
  });
  /** The repository has no pull request at all: no list pane, the EmptyState owns the page. */
  let listEmpty = $state(false);
  let createOpen = $state(false);
  // The Files panel under the list (the open PR's files): its height and
  // whether it is folded to its header, both persisted.
  const FILES_H_KEY = 'otto_pr_files_h';
  const FILES_FOLDED_KEY = 'otto_pr_files_folded';
  const clampH = (h: number): number => Math.max(120, Math.min(900, Math.round(h)));
  let filesH = $state(clampH(readNum(FILES_H_KEY, 320)));
  let filesFolded = $state((() => { try { return localStorage.getItem(FILES_FOLDED_KEY) === '1'; } catch { return false; } })());
  function setFilesH(h: number): void {
    filesH = clampH(h);
    persist(FILES_H_KEY, String(filesH));
  }
  function toggleFilesFolded(): void {
    filesFolded = !filesFolded;
    persist(FILES_FOLDED_KEY, filesFolded ? '1' : '0');
  }
  function startFilesResize(e: MouseEvent): void {
    const startY = e.clientY;
    const startH = filesH;
    startMouseDrag(e, {
      cursor: 'row-resize',
      // The bar sits on the panel's top edge: dragging up grows it.
      onMove: (ev) => { filesH = clampH(startH - (ev.clientY - startY)); },
      onEnd: () => persist(FILES_H_KEY, String(filesH)),
    });
  }

  // The Detail pane: the open PR's details card or its agent thread. Which
  // one is remembered (an inline pane stays open across PRs); a sheet — a
  // phone, or no room beside the PR — opens only when asked.
  type PaneView = 'details' | 'agent';
  const VIEW_KEY = 'otto.pane.git.pr.view';
  function readView(): PaneView | null {
    try {
      const v = localStorage.getItem(VIEW_KEY);
      if (v !== null) return v === 'details' || v === 'agent' ? v : null;
      // rc.5 kept the agent thread unfolded under the list: start on it.
      return localStorage.getItem('otto_pr_agent_folded') === '0' ? 'agent' : 'details';
    } catch { return 'details'; }
  }
  let paneView = $state<PaneView | null>(readView());
  let sheet = $state(false);
  let asked = $state(false);
  let agentMax = $state(false);
  const paneOpen = $derived(tabs.active !== null && paneView !== null && (!sheet || asked));
  const PANE_VIEWS = [{ id: 'details', label: 'Details' }, { id: 'agent', label: 'Agent' }];
  function showPane(v: PaneView): void {
    paneView = v;
    persist(VIEW_KEY, v);
    asked = true;
  }
  function closePane(): void {
    if (sheet) asked = false;
    else {
      paneView = null;
      persist(VIEW_KEY, '');
    }
  }
  const togglePane = (v: PaneView): void => (paneOpen && paneView === v ? closePane() : showPane(v));
  /** The open PR's details card (PrDetail hands it over). */
  let detailsCard = $state.raw<Snippet | null>(null);

  // The agent thread's live state: the ✦ button's dot while the thread is not
  // on screen (a hidden status-only panel keeps the embed reporting it).
  const agentLive = $derived(tabs.active === null ? null : (prAgentBridge.status[prAgentOriginId(repoId, tabs.active)] ?? null));
  const agentLabel = $derived(agentLive === 'working' ? 'Agent thread: the agent is working' : agentLive === 'needs-you' ? 'Agent thread: the agent needs you' : 'Agent thread');
  const agentShown = $derived(paneOpen && paneView === 'agent');
  // An ask (⋯ menu, Files panel, diff, the thread's own box) brings the
  // thread forward; its panel mounts with the pane and takes the waiting ask.
  $effect(() => {
    const p = prAgentBridge.pending;
    const n = tabs.active;
    if (!p || n === null || p.key !== prAgentOriginId(repoId, n)) return;
    untrack(() => showPane('agent'));
  });

  function titleOf(n: number): string {
    return titles[n] ?? prDetailCache.get(prKey(repoId, n))?.value.title ?? '';
  }
  function remember(pr: PrSummary): void {
    if (titles[pr.number] !== pr.title) titles = { ...titles, [pr.number]: pr.title };
  }

  function openPr(pr: PrSummary, opts: { background: boolean }): void {
    remember(pr);
    prWorkspace.open(repoId, pr.number, opts.background);
    if (!opts.background) phoneOpen = true;
  }
  /** Open on an item: with nothing open, the list opens on what needs you, else its first row. */
  function openFirst(prs: PrSummary[]): void {
    if (tabs.active !== null) return;
    const pr = firstNeedingPr(prs, (p) => p.is_mine ?? false) ?? prs[0];
    if (!pr) return;
    remember(pr);
    prWorkspace.open(repoId, pr.number);
  }
  function openMany(prs: PrSummary[]): void {
    for (const p of prs) remember(p);
    prWorkspace.openMany(repoId, prs.map((p) => p.number));
    phoneOpen = true;
  }

  let prwEl = $state<HTMLDivElement | null>(null);
  let tablistEl = $state<HTMLElement | null>(null);
  let findPrBtnEl = $state<HTMLButtonElement | null>(null);

  function closeTab(n: number): void {
    const nextFocus = nextTabAfterClose(tabs.open, n);
    prWorkspace.close(repoId, n);
    requestAnimationFrame(() => {
      if (nextFocus !== null) {
        tablistEl?.querySelector<HTMLElement>(`[data-pr-tab="${nextFocus}"]`)?.focus();
      } else if (findPrBtnEl?.isConnected) {
        findPrBtnEl.focus();
      } else {
        prwEl?.querySelector<HTMLInputElement>('.pr-search')?.focus();
      }
    });
  }

  function tabMenu(e: MouseEvent, n: number): void {
    ctxMenu.show(e, [
      { label: 'Close', icon: 'x', action: () => closeTab(n) },
      { label: 'Close others', disabled: tabs.open.length < 2, action: () => prWorkspace.closeOthers(repoId, n) },
      { label: 'Close all', action: () => { prWorkspace.closeAll(repoId); requestAnimationFrame(() => findPrBtnEl?.focus()); } },
    ]);
  }
  function onTabKeydown(e: KeyboardEvent, n: number): void {
    const i = tabs.open.indexOf(n);
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      const d = (e.key === 'ArrowRight') !== (getComputedStyle(e.currentTarget as Element).direction === 'rtl') ? 1 : -1;
      const next = tabs.open[(i + d + tabs.open.length) % tabs.open.length];
      prWorkspace.activate(repoId, next);
      requestAnimationFrame(() => tablistEl?.querySelector<HTMLElement>(`[data-pr-tab="${next}"]`)?.focus());
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      closeTab(n);
    }
  }
  /** "+": bring the list back (a phone: go back to it) and put the cursor in its search box. */
  function focusSearch(): void {
    if (viewport.navDrawer) phoneOpen = false;
    else pageNav.show?.onclick();
    requestAnimationFrame(() => prwEl?.querySelector<HTMLInputElement>('.pr-search')?.focus());
  }
</script>

{#snippet paneToggles()}
  <button class="prw-icon" aria-pressed={paneOpen && paneView === 'details'} aria-label="Details" title="Details: reviewers, checks, labels (⌥⌘B)"
    onclick={() => togglePane('details')} data-testid="pr-details-toggle">
    <Icon name="info" size={13} />
  </button>
  <button class="prw-icon prw-agent-open" aria-pressed={agentShown} aria-label={agentLabel} title={agentLabel} onclick={() => togglePane('agent')}
    data-testid="pr-agent-open" data-status={agentLive ?? 'idle'}>
    <Icon name="sparkle" size={13} />
    {#if !agentShown && (agentLive === 'working' || agentLive === 'needs-you')}<span class="prw-agent-dot" class:needs={agentLive === 'needs-you'} aria-hidden="true"></span>{/if}
  </button>
{/snippet}

<div class="prw" bind:this={prwEl}>
  <ListDetail
    module="git"
    pane="prs"
    name="Pull requests"
    wide
    open={phoneOpen}
    onback={() => (phoneOpen = false)}
    empty={listEmpty}
    legacy={{ w: 'otto_pr_list_w', hidden: () => { try { const v = localStorage.getItem('otto_pr_list_hidden'); return v === null ? null : v === '1'; } catch { return null; } } }}
  >
    {#snippet list()}
      <PrList
        {repoId}
        activeNumber={tabs.active}
        openNumbers={tabs.open}
        onopen={openPr}
        onopenmany={openMany}
        {patch}
        onfirst={openFirst}
        onempty={() => (listEmpty = true)}
      />
      <!-- The open PR's Files under the list (a phone: the diff's Files picker). -->
      {#if tabs.active !== null && prView === 'code' && !viewport.navDrawer}
        {#if !filesFolded}
          <div
            class="prw-files-resize"
            role="separator"
            aria-label="Resize the files panel"
            title={RESIZE_TITLE_VERTICAL}
            use:paneResizer={{ value: filesH, min: 120, max: 900, orientation: 'horizontal', invert: true, onChange: setFilesH, onReset: () => setFilesH(320), onDragStart: startFilesResize, text: (v) => `${Math.round(v)} pixels tall` }}
          ></div>
        {/if}
        <div class="prw-files" style={filesFolded ? undefined : `block-size:${filesH}px`}>
          <PrFilesPanel {repoId} number={tabs.active} collapsed={filesFolded} ontogglecollapsed={toggleFilesFolded} />
        </div>
      {/if}
    {/snippet}

    {#snippet detail()}
      <DetailPane
        module="git"
        pane="pr"
        title={paneView === 'agent' ? `Agent · PR #${tabs.active}` : 'Pull request details'}
        open={paneOpen}
        onclose={closePane}
        ontoggle={() => (paneOpen ? closePane() : showPane(paneView ?? 'details'))}
        maximized={agentMax && paneView === 'agent'}
        bind:sheet
        menu={paneView === 'agent' ? () => [{ label: agentMax ? 'Narrow' : 'Widen', icon: agentMax ? 'minimize' : 'maximize', action: () => (agentMax = !agentMax) }] : undefined}
        testid="pr-pane"
      >
        {#snippet views()}
          <Tabs label="Pull request panes" size="s" idBase="pr-pane" testid="pr-pane-tabs" tabs={PANE_VIEWS} value={paneView ?? 'details'} onchange={(id) => showPane(id as PaneView)} />
        {/snippet}
        <div class="prw-pane-body" role="tabpanel" id="pr-pane-panel-{paneView}" aria-labelledby="pr-pane-tab-{paneView}">
          {#if tabs.active !== null && paneView === 'agent'}
            {#key `${repoId}:${tabs.active}`}
              <PrAgentPanel {repoId} number={tabs.active} title={titleOf(tabs.active)} onmaximize={() => (agentMax = !agentMax)} onasked={() => showPane('agent')} />
            {/key}
          {:else}
            <div class="prw-card">{@render detailsCard?.()}</div>
          {/if}
        </div>
      </DetailPane>
    {/snippet}

    {#if listEmpty}
      <EmptyState variant="page" icon="pr" title="No pull requests" body="Open one from your current branch." actionLabel="New pull request" actionIcon="plus" onaction={() => (createOpen = true)} />
    {:else}
    <section class="prw-detail" aria-label="Open pull requests">
      {#if strip}
      <div class="prw-tabs">
        <nav class="prw-tablist" aria-label="Open pull requests" bind:this={tablistEl}>
          {#each tabs.open as n (n)}
            {@const title = titleOf(n)}
            <div class="prw-tab" class:active={tabs.active === n}>
              <button
                class="prw-tab-btn"
                data-pr-tab={n}
                aria-current={tabs.active === n ? 'page' : undefined}
                title={title ? `#${n} ${title}` : `#${n}`}
                onclick={() => prWorkspace.activate(repoId, n)}
                onauxclick={(e) => { if (e.button === 1) closeTab(n); }}
                oncontextmenu={(e) => tabMenu(e, n)}
                onkeydown={(e) => onTabKeydown(e, n)}
              >
                <Icon name="pr" size={12} />
                <span class="prw-tab-num">#{n}</span>
                {#if title}<span class="prw-tab-title">{title}</span>{/if}
              </button>
              <button class="prw-tab-x" aria-label="Close #{n}" title="Close #{n}" onclick={() => closeTab(n)}>
                <Icon name="x" size={12} />
              </button>
            </div>
          {/each}
        </nav>
        <button class="prw-icon" bind:this={findPrBtnEl} aria-label="Find a pull request" title="Find a pull request" onclick={focusSearch}><Icon name="plus" size={13} /></button>
        <span class="grow"></span>
        {@render paneToggles()}
      </div>
      {/if}

      <div class="prw-pane">
        {#if tabs.active !== null}
          {#key `${repoId}:${tabs.active}`}
            <PrDetail
              {repoId}
              number={tabs.active}
              onloaded={(p) => { patch = p; remember(p); }}
              onview={(v) => (prView = v)}
              ondetails={(card) => (detailsCard = card)}
              lead={strip ? undefined : paneToggles}
            />
          {/key}
        {:else}
          <EmptyState
            icon="pr"
            title="No pull request open"
            body="Pick one from the list — ⌘-click opens it in the background, and you can select several and open them together."
          />
        {/if}
      </div>
    </section>
    {/if}
  </ListDetail>
</div>

{#if tabs.active !== null}
  <PrAgentLinePick onexplain={(n, pick) => prAgentBridge.ask(repoId, n, { kind: 'lines', pick })} />
  {#if !agentShown}
    <div hidden>
      {#key `${repoId}:${tabs.active}`}<PrAgentPanel {repoId} number={tabs.active} title={titleOf(tabs.active)} statusOnly />{/key}
    </div>
  {/if}
{/if}

{#if createOpen}
  <CreatePr
    {repoId}
    onclose={() => (createOpen = false)}
    oncreated={(pr) => {
      createOpen = false;
      invalidatePr(repoId);
      listEmpty = false;
      openPr(pr, { background: false });
    }}
  />
{/if}

<style>
  .prw {
    display: flex;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .prw-files { flex: none; min-height: 0; max-block-size: 70%; display: flex; flex-direction: column; background: var(--bg-sidebar); }
  .prw-files-resize {
    flex: none;
    height: 6px;
    margin-block: -4px -2px;
    position: relative;
    z-index: 2;
    cursor: row-resize;
  }
  .prw-files-resize:hover, .prw-files-resize:focus-visible { background: var(--accent-line); }
  .prw-detail { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .prw-tabs {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 8px;
    border-block-end: 1px solid var(--separator);
    min-width: 0;
  }
  .prw-tablist { display: flex; gap: 4px; overflow-x: auto; min-width: 0; scrollbar-width: none; }
  .prw-tab {
    display: inline-flex;
    align-items: center;
    flex: none;
    max-width: 220px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .prw-tab:hover { background: var(--hover); }
  .prw-tab.active { background: var(--surface-2); border-color: var(--border); color: var(--text); }
  .prw-tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding-block: 4px;
    padding-inline: 8px 4px;
    background: none;
    border: none;
    color: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .prw-tab-num { font-variant-numeric: tabular-nums; font-weight: 600; }
  .prw-tab-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 130px; }
  .prw-tab-x, .prw-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .prw-tab-x { width: 18px; height: 18px; margin-inline-end: 4px; opacity: 0.6; }
  .prw-tab-x:hover, .prw-tab.active .prw-tab-x { opacity: 1; }
  .prw-tab-x:hover { background: var(--hover); color: var(--text); }
  .prw-icon { width: 26px; height: 26px; flex: none; }
  .prw-icon:hover { background: var(--hover); color: var(--text); }
  .prw-icon[aria-pressed='true'] { color: var(--accent-text); background: var(--accent-soft); }
  .prw-agent-open { position: relative; }
  .prw-agent-dot {
    position: absolute;
    inset-block-start: 4px;
    inset-inline-end: 4px;
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    background: var(--accent-solid);
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  .prw-agent-dot.needs { background: var(--warning); animation: none; }
  @media (prefers-reduced-motion: reduce) { .prw-agent-dot { animation: none; } }
  .prw-pane { flex: 1; min-height: 0; min-width: 0; }
  .prw-pane-body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .prw-card { padding: 12px; }
  /* Touch: the strip's controls take a finger. */
  @media (pointer: coarse) {
    .prw-icon, .prw-tab-x { width: 44px; height: 44px; }
  }
</style>
