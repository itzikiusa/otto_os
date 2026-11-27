<script lang="ts">
  // The list / detail page layout (design.md §2–§4): a list pane, the content,
  // and optionally a Detail pane at the end of the content row.
  //
  //   desktop / tablet  [list pane | divider | content | detail]
  //                     the list is resizable (LIST_PANE, or `wide`), clamped
  //                     so the content keeps 480 px (the divider's range says
  //                     so), and can be hidden: ⋯ › "Hide <name>" or ⌘B; while
  //                     hidden PageHeader shows "Show <name>" in its leading slot.
  //   phone             push: the list IS the page until `open`; then the
  //                     content is, with "‹ <name>" in PageHeader (onback).
  //                     Both stay mounted, so going back and forth keeps the
  //                     list's scroll and the content's state.
  //   empty             no pane at all: the content (the page's EmptyState)
  //                     owns the page.
  //
  // Width and hidden persist as otto.pane.<module>.<pane>.{w,hidden}; a page
  // that used other keys passes them once as `legacy`. Open on an item (the
  // last, else the first): lib/lastSelection `initialSelection`.
  //
  //   <ListDetail module="insights" name="Reports" open={!!openId} onback={() => (openId = null)} empty={reports.length === 0}>
  //     {#snippet list()}<ListPane count={reports.length} …>…</ListPane>{/snippet}
  //     <ReportView id={selectedId} />
  //   </ListDetail>
  import { getContext, setContext, type Snippet } from 'svelte';
  import PaneDivider from '../PaneDivider.svelte';
  import { viewport } from '../../stores/viewport.svelte';
  import { pageNav } from './pageNav.svelte';
  import { LIST_PANE, WIDE_LIST_PANE, fitPaneMax, loadPaneHidden, loadPaneW, paneKey, paneStorage, savePaneHidden, type PaneLegacy } from './paneKeys';
  import { LIST_DETAIL, type ListDetailCtx } from './context';

  interface Props {
    /** Key scheme: otto.pane.<module>.<pane>.{w,hidden}. */
    module: string;
    pane?: string;
    /** The list's name: the phone Back says it, so do Hide / Show. */
    name: string;
    /** Phone: an item's content is the page (pushed). Ignored on desktop / tablet. */
    open: boolean;
    /** Phone Back: return to the list (keep the selection; it stays marked). */
    onback: () => void;
    /** The list has nothing: no pane; the content (an EmptyState) owns the page. */
    empty?: boolean;
    /** Rich rows (pull requests): 420 · 280–720 instead of 280 · 220–420. */
    wide?: boolean;
    /** A default inside the range (Settings 240). */
    defaultWidth?: number;
    /** Old storage keys, read once. */
    legacy?: PaneLegacy;
    list: Snippet;
    /** The Detail pane (DetailPane), drawn at the end of the content row. */
    detail?: Snippet;
    testid?: string;
    children: Snippet;
  }
  let { module, pane = 'list', name, open, onback, empty = false, wide = false, defaultWidth, legacy, list, detail, testid, children }: Props = $props();

  // svelte-ignore state_referenced_locally
  const range = wide ? WIDE_LIST_PANE : LIST_PANE;
  const store = paneStorage();
  // svelte-ignore state_referenced_locally
  const fallback = defaultWidth ?? range.default;
  // svelte-ignore state_referenced_locally
  let width = $state(loadPaneW(store, module, pane, range, legacy, fallback));
  // svelte-ignore state_referenced_locally
  let hidden = $state(loadPaneHidden(store, module, pane, legacy));
  let rowW = $state(0);

  const phone = $derived(viewport.isPhone);
  const max = $derived(fitPaneMax(rowW, range));
  const listShown = $derived(!empty && (phone ? !open : !hidden));
  const contentShown = $derived(empty || !phone || open);

  function setHidden(h: boolean): void {
    hidden = h;
    savePaneHidden(store, module, pane, h);
  }

  const parent = getContext<ListDetailCtx | undefined>(LIST_DETAIL);
  const visible = (): boolean => (parent ? parent.contentVisible() : true);
  setContext<ListDetailCtx>(LIST_DETAIL, {
    get name() {
      return name;
    },
    get canHide() {
      return !phone && !empty;
    },
    hide: () => setHidden(true),
    contentVisible: () => visible() && contentShown,
  });

  $effect(() =>
    pageNav.claim({
      active: visible,
      back: () => (phone && open && !empty ? { label: name, onclick: onback } : null),
      show: () => (!phone && hidden && !empty ? { label: name, onclick: () => setHidden(false) } : null),
      toggle: () => (!phone && !empty ? () => setHidden(!hidden) : null),
    }),
  );
</script>

<div class="ld" class:phone bind:clientWidth={rowW} data-testid={testid}>
  {#if !empty}
    <div class="ld-list" hidden={!listShown} style:inline-size={phone ? undefined : `${Math.min(width, max)}px`}>
      {@render list()}
    </div>
    {#if listShown && !phone}
      <PaneDivider bind:width storageKey={paneKey(module, pane, 'w')} min={range.min} {max} defaultWidth={fallback} label="Resize {name}" />
    {/if}
  {/if}
  <div class="ld-main" hidden={!contentShown}>
    <div class="ld-content">{@render children()}</div>
    {@render detail?.()}
  </div>
</div>

<style>
  .ld {
    flex: 1;
    display: flex;
    min-width: 0;
    min-height: 0;
    block-size: 100%;
  }
  .ld-list {
    flex: none;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .ld.phone .ld-list {
    flex: 1;
    inline-size: 100%;
  }
  .ld-main,
  .ld-content {
    flex: 1;
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .ld-content {
    flex-direction: column;
  }
  .ld-list[hidden],
  .ld-main[hidden] {
    display: none;
  }
</style>
