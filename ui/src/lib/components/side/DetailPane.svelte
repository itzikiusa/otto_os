<script lang="ts">
  // The ONE right-hand pane about the thing selected on this page (design.md
  // §3 "Detail pane"; grown from DockedDrawer): a 36 px header — the title, or
  // the pane's views as a segmented control (`views`), at most ONE action,
  // ⋯ when `menu` is given, Close — a scrolling body and an optional footer
  // (row navigation "‹ Previous · 3 of 50 · Next ›").
  //
  //   inline   while the content beside it keeps `minContent` (480 px):
  //            resizable (300–560, default 380), persisted as
  //            otto.pane.<module>.<pane>.w; Esc closes, focus returns.
  //   sheet    otherwise (and always on a phone): a right sheet over the
  //            content — full width on a phone — with the shell Drawer's
  //            backdrop, focus trap and Esc.
  //
  // `ontoggle` makes ⌥⌘B open / close it. Put it at the end of a ListDetail's
  // content row (its `detail` snippet) or of any flex row.
  //
  //   <DetailPane module="aws" pane="instance" open={!!sel} title="Instance" onclose={() => (sel = null)}>…</DetailPane>
  import type { Snippet } from 'svelte';
  import Drawer from '../../../shell/Drawer.svelte';
  import Icon from '../Icon.svelte';
  import PaneDivider from '../PaneDivider.svelte';
  import { ctxMenu, type MenuItem } from '../../contextmenu.svelte';
  import { dialogReturnTarget } from '../../dialogFocus';
  import { viewport } from '../../stores/viewport.svelte';
  import { pageNav } from './pageNav.svelte';
  import { DETAIL_PANE, MIN_CONTENT, detailFit, fitPaneMax, loadPaneW, paneKey, paneStorage, type PaneRange } from './paneKeys';

  interface Props {
    open: boolean;
    /** Heading and accessible name ("Thread details"). */
    title: string;
    onclose: () => void;
    /** ⌥⌘B: open / close from the keyboard. */
    ontoggle?: () => void;
    module: string;
    pane?: string;
    /** Inside the shared range; a pane that must grow further (a Threads surface) passes its own. */
    range?: PaneRange;
    /** What the content keeps before this pane becomes a sheet. */
    minContent?: number;
    /** Take all the room the content leaves. */
    maximized?: boolean;
    /** Bound: the pane is presented as a sheet right now. */
    sheet?: boolean;
    /** The pane's views in place of the title (Details | Changes | Files). */
    views?: Snippet;
    /** The ONE header action. */
    action?: Snippet;
    menu?: () => MenuItem[];
    footer?: Snippet;
    testid?: string;
    children: Snippet;
  }
  let {
    open,
    title,
    onclose,
    ontoggle,
    module,
    pane = 'detail',
    range = DETAIL_PANE,
    minContent = MIN_CONTENT,
    maximized = false,
    sheet = $bindable(false),
    views,
    action,
    menu,
    footer,
    testid,
    children,
  }: Props = $props();

  const store = paneStorage();
  // svelte-ignore state_referenced_locally
  let width = $state(loadPaneW(store, module, pane, range));
  // A pane that changes key (another surface) reads that key's width.
  // svelte-ignore state_referenced_locally
  let loadedKey = paneKey(module, pane, 'w');
  $effect.pre(() => {
    const key = paneKey(module, pane, 'w');
    if (key === loadedKey) return;
    loadedKey = key;
    width = loadPaneW(store, module, pane, range);
  });

  let anchor = $state<HTMLElement | null>(null);
  let rowW = $state(0);
  $effect(() => {
    const row = anchor?.parentElement;
    if (!row) return;
    const ro = new ResizeObserver(() => (rowW = row.clientWidth));
    ro.observe(row);
    rowW = row.clientWidth;
    return () => ro.disconnect();
  });
  const fit = $derived(detailFit(rowW, width, range, minContent, maximized));
  $effect.pre(() => {
    sheet = viewport.isPhone || fit.sheet;
  });

  $effect(() => (ontoggle ? pageNav.claimDetail(ontoggle) : undefined));

  // Inline focus return (the sheet's Drawer owns its own): closing unmounts
  // the focused control; hand focus back to what opened the pane.
  let column = $state<HTMLElement | null>(null);
  $effect(() => {
    if (!open || sheet) return;
    let opener = dialogReturnTarget();
    const onFocusIn = (e: FocusEvent): void => {
      const t = e.target;
      if (t instanceof HTMLElement && t !== document.body && !column?.contains(t)) opener = t;
    };
    document.addEventListener('focusin', onFocusIn);
    return () => {
      document.removeEventListener('focusin', onFocusIn);
      const target = opener;
      queueMicrotask(() => {
        const now = document.activeElement;
        if (target?.isConnected && (!now || now === document.body)) target.focus();
      });
    };
  });

  function onKey(e: KeyboardEvent): void {
    if (!open || sheet || e.key !== 'Escape' || e.defaultPrevented) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
    if (!column?.contains(t)) return;
    if (document.querySelector('[aria-modal="true"]')) return;
    e.stopPropagation();
    onclose();
  }

  function openMenu(e: MouseEvent): void {
    if (menu) ctxMenu.showAt(e.currentTarget as HTMLElement, menu(), { align: 'end' });
  }
</script>

<svelte:window onkeydown={onKey} />

<span class="dp-anchor" bind:this={anchor} aria-hidden="true"></span>

{#snippet head()}
  <div class="dp-head" class:views={!!views}>
    {#if views}<div class="dp-views">{@render views()}</div>{:else}<h2 class="dp-title">{title}</h2>{/if}
    {@render action?.()}
    {#if menu}
      <button type="button" class="icon-btn" aria-haspopup="menu" aria-label="{title} actions" title="More" onclick={openMenu}><Icon name="more" size={14} /></button>
    {/if}
    <button type="button" class="icon-btn" onclick={onclose} aria-label="Close {title}" title="Close {title}" aria-keyshortcuts="Escape" data-testid={testid ? `${testid}-close` : undefined}>
      <Icon name="x" size={14} />
    </button>
  </div>
{/snippet}

{#if sheet}
  <Drawer {open} side="right" label={title} width={viewport.isPhone ? '100vw' : 'min(92vw, 420px)'} bare {onclose}>
    <div class="detail-pane sheet" data-testid={testid}>
      {@render head()}
      <div class="dp-body">{@render children()}</div>
      {#if footer}<footer class="dp-foot">{@render footer()}</footer>{/if}
    </div>
  </Drawer>
{:else if open}
  <PaneDivider bind:width storageKey={paneKey(module, pane, 'w')} min={range.min} max={fitPaneMax(rowW, range, minContent)} defaultWidth={range.default} invert label="Resize {title}" />
  <aside bind:this={column} class="detail-pane" style:inline-size="{fit.shown}px" aria-label={title} data-testid={testid}>
    {@render head()}
    <div class="dp-body">{@render children()}</div>
    {#if footer}<footer class="dp-foot">{@render footer()}</footer>{/if}
  </aside>
{/if}

<style>
  .dp-anchor {
    flex: none;
    inline-size: 0;
  }
  .detail-pane {
    flex: none;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    block-size: 100%;
    background: var(--bg-sidebar);
    border-inline-start: 1px solid var(--separator);
  }
  .detail-pane.sheet {
    flex: 1;
    inline-size: 100%;
    border: 0;
  }
  .dp-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    block-size: 36px;
    padding-inline: 16px 6px;
    border-block-end: 1px solid var(--separator);
  }
  .dp-head.views {
    padding-inline-start: 8px;
    gap: 4px;
  }
  .dp-title {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dp-views {
    flex: 1;
    min-width: 0;
    display: flex;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .dp-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .dp-foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    min-block-size: 36px;
    padding-inline: 16px 8px;
    border-block-start: 1px solid var(--separator);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  /* A phone sheet: the same 46 px row as the page header, finger-sized. */
  @media (max-width: 640px) {
    .dp-head {
      block-size: 46px;
    }
    .dp-head :global(.icon-btn) {
      inline-size: 38px;
      block-size: 44px;
    }
  }
</style>
