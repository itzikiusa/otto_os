<script lang="ts">
  // A repo's Pull requests tab: the PR list on the start side, the open PRs
  // as tabs on the other (several can be open together). The list pane can be
  // resized, hidden (⇥ "Hide list") or the PR pane maximized. Open tabs are
  // persisted per repo (prWorkspace store). Only the PR in front is mounted:
  // switching tabs repaints from the SWR caches instantly, and nothing heavy
  // (a big diff's row model) stays alive for tabs nobody is looking at.
  import { untrack } from 'svelte';
  import type { PrSummary } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { paneResizer } from '../../lib/paneResizer';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { prWorkspace } from './prWorkspaceStore.svelte';
  import { prDetailCache, prKey } from './pr-cache';
  import PrList from './PrList.svelte';
  import PrDetail from './PrDetail.svelte';

  interface Props {
    repoId: string;
  }
  let { repoId }: Props = $props();

  const tabs = $derived(prWorkspace.tabs(repoId));
  /** Freshest copy of each PR the detail pane has read (list row patches). */
  let patch = $state<PrSummary | null>(null);
  /** Titles for the tab strip (from the list rows or the detail cache). */
  let titles = $state<Record<number, string>>({});

  const LIST_W_KEY = 'otto_pr_list_w';
  const LIST_HIDDEN_KEY = 'otto_pr_list_hidden';
  const readNum = (k: string, d: number): number => {
    try { return Number(localStorage.getItem(k)) || d; } catch { return d; }
  };
  let listW = $state(readNum(LIST_W_KEY, 420));
  /** The PR pane fills the workspace (list out of the way, not closed). */
  let maximized = $state(false);
  /** ≤1024px shows one pane at a time: true = the list. */
  let mobileList = $state(untrack(() => prWorkspace.tabs(repoId).active === null));
  let listHidden = $state((() => { try { return localStorage.getItem(LIST_HIDDEN_KEY) === '1'; } catch { return false; } })());
  const clampW = (w: number): number => Math.max(280, Math.min(720, Math.round(w)));
  function persist(k: string, v: string): void {
    try { localStorage.setItem(k, v); } catch { /* not persisted */ }
  }
  function setListW(w: number): void {
    listW = clampW(w);
    persist(LIST_W_KEY, String(listW));
  }
  function toggleList(): void {
    listHidden = !listHidden;
    persist(LIST_HIDDEN_KEY, listHidden ? '1' : '0');
  }
  function startResize(e: MouseEvent): void {
    const startX = e.clientX;
    const startW = listW;
    const rtl = getComputedStyle(e.currentTarget as Element).direction === 'rtl';
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => { listW = clampW(startW + (rtl ? startX - ev.clientX : ev.clientX - startX)); },
      onEnd: () => persist(LIST_W_KEY, String(listW)),
    });
  }

  function titleOf(n: number): string {
    return titles[n] ?? prDetailCache.get(prKey(repoId, n))?.value.title ?? '';
  }
  function remember(pr: PrSummary): void {
    if (titles[pr.number] !== pr.title) titles = { ...titles, [pr.number]: pr.title };
  }

  function openPr(pr: PrSummary, opts: { background: boolean }): void {
    remember(pr);
    prWorkspace.open(repoId, pr.number, opts.background);
    if (!opts.background) mobileList = false;
  }
  function openMany(prs: PrSummary[]): void {
    for (const p of prs) remember(p);
    prWorkspace.openMany(repoId, prs.map((p) => p.number));
    mobileList = false;
  }

  function tabMenu(e: MouseEvent, n: number): void {
    ctxMenu.show(e, [
      { label: 'Close', icon: 'x', action: () => prWorkspace.close(repoId, n) },
      { label: 'Close others', disabled: tabs.open.length < 2, action: () => prWorkspace.closeOthers(repoId, n) },
      { label: 'Close all', action: () => prWorkspace.closeAll(repoId) },
    ]);
  }
  function onTabKeydown(e: KeyboardEvent, n: number): void {
    const i = tabs.open.indexOf(n);
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      const d = (e.key === 'ArrowRight') !== (getComputedStyle(e.currentTarget as Element).direction === 'rtl') ? 1 : -1;
      const next = tabs.open[(i + d + tabs.open.length) % tabs.open.length];
      prWorkspace.activate(repoId, next);
      requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-pr-tab="${next}"]`)?.focus());
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      prWorkspace.close(repoId, n);
    }
  }
  /** "+": bring the list back and put the cursor in its search box. */
  function focusSearch(): void {
    if (listHidden) toggleList();
    requestAnimationFrame(() => document.querySelector<HTMLInputElement>('.prw .pr-search')?.focus());
  }
</script>

<div class="prw" class:m-list={mobileList || tabs.active === null} class:list-hidden={listHidden || (tabs.active !== null && maximized)} style="--listw:{listW}px">
  {#if !(listHidden || (tabs.active !== null && maximized))}
    <section class="prw-list" aria-label="Pull requests">
      <PrList
        {repoId}
        activeNumber={tabs.active}
        openNumbers={tabs.open}
        onopen={openPr}
        onopenmany={openMany}
        {patch}
      />
      <div
        class="prw-resize"
        role="separator"
        aria-label="Resize the pull request list"
        title="Drag or use ←/→ to resize · double-click or Enter to reset"
        use:paneResizer={{ value: listW, min: 280, max: 720, onChange: setListW, onReset: () => setListW(420), onDragStart: startResize, text: (v) => `${Math.round(v)} pixels wide` }}
      ></div>
    </section>
  {/if}

  <section class="prw-detail" aria-label="Open pull requests">
    <div class="prw-tabs">
      <button class="prw-icon prw-back" aria-label="Back to the list" title="Back to the list" onclick={() => (mobileList = true)}><Icon name="chevronLeft" size={14} /></button>
      <nav class="prw-tablist" aria-label="Open pull requests">
        {#each tabs.open as n (n)}
          {@const title = titleOf(n)}
          <div class="prw-tab" class:active={tabs.active === n}>
            <button
              class="prw-tab-btn"
              data-pr-tab={n}
              aria-current={tabs.active === n ? 'page' : undefined}
              title={title ? `#${n} ${title}` : `#${n}`}
              onclick={() => prWorkspace.activate(repoId, n)}
              onauxclick={(e) => { if (e.button === 1) prWorkspace.close(repoId, n); }}
              oncontextmenu={(e) => tabMenu(e, n)}
              onkeydown={(e) => onTabKeydown(e, n)}
            >
              <Icon name="pr" size={12} />
              <span class="prw-tab-num">#{n}</span>
              {#if title}<span class="prw-tab-title">{title}</span>{/if}
            </button>
            <button class="prw-tab-x" aria-label="Close #{n}" title="Close #{n}" onclick={() => prWorkspace.close(repoId, n)}>
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </nav>
      <button class="prw-icon" aria-label="Find a pull request" title="Find a pull request" onclick={focusSearch}><Icon name="plus" size={13} /></button>
      <span class="grow"></span>
      {#if tabs.active !== null}
        <button class="prw-icon prw-desk" aria-pressed={maximized} aria-label={maximized ? 'Restore the list' : 'Maximize the pull request'}
          title={maximized ? 'Restore the list' : 'Maximize the pull request'} onclick={() => (maximized = !maximized)}>
          <Icon name={maximized ? 'minimize' : 'maximize'} size={13} />
        </button>
      {/if}
      <button class="prw-icon prw-desk" aria-pressed={!listHidden} aria-label={listHidden ? 'Show the list' : 'Hide the list'}
        title={listHidden ? 'Show the list' : 'Hide the list'} onclick={toggleList}>
        <Icon name="sidebar" size={13} />
      </button>
    </div>

    <div class="prw-pane">
      {#if tabs.active !== null}
        {#key `${repoId}:${tabs.active}`}
          <PrDetail
            {repoId}
            number={tabs.active}
            onloaded={(p) => { patch = p; remember(p); }}
          />
        {/key}
      {:else}
        <EmptyState
          icon="pr"
          title="No pull request open"
          body="Pick one from the list — ⌘-click opens it in the background, and you can select several and open them together."
          actionLabel={listHidden ? 'Show the list' : undefined}
          onaction={listHidden ? toggleList : undefined}
        />
      {/if}
    </div>
  </section>
</div>

<style>
  .prw {
    display: grid;
    grid-template-columns: var(--listw) minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .prw.list-hidden { grid-template-columns: minmax(0, 1fr); }
  .prw-list {
    position: relative;
    min-width: 0;
    min-height: 0;
    border-inline-end: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
  }
  .prw-resize {
    position: absolute;
    inset-block: 0;
    inset-inline-end: -3px;
    width: 6px;
    cursor: col-resize;
    z-index: 2;
  }
  .prw-resize:hover, .prw-resize:focus-visible { background: var(--accent-line); }
  .prw-detail { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
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
  .prw-icon[aria-pressed='true'] { color: var(--text); }
  .prw-pane { flex: 1; min-height: 0; min-width: 0; }
  .prw-back { display: none; }

  /* Phone/tablet: one pane at a time — the list, or the open PR. */
  @media (max-width: 1024px) {
    .prw { grid-template-columns: minmax(0, 1fr); }
    .prw-list { border-inline-end: none; }
    .prw-resize { display: none; }
    .prw:not(.m-list) .prw-list { display: none; }
    .prw.m-list .prw-detail { display: none; }
    .prw-back { display: inline-flex; }
    .prw-desk { display: none; }
    .prw-icon, .prw-tab-x { width: 32px; height: 32px; }
  }
</style>
