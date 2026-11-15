<script lang="ts">
  // Progressive history windowing + "Load earlier" control for RuntimeThread.
  // Keeps scroll offset anchored to `[data-runtime-stream]`'s height delta so
  // prepending older turns shifts visible messages 0 px.
  import { tick } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { plural } from '../../../lib/plural';
  import { runtimeViews } from './runtimeView.svelte';

  const INITIAL_ROW_CAP = 200;
  const ROW_STEP = 200;
  const AUTO_LOAD_TOP_PX = 240;

  interface Props {
    sessionId: string;
    hasOlder: boolean;
    olderTurns: number;
    loadingOlder: boolean;
    totalRows: number;
    scroller: HTMLDivElement | null;
    streamEl: HTMLDivElement | null;
    onreleasebottom?: () => void;
    rowCap?: number;
  }

  let {
    sessionId,
    hasOlder,
    olderTurns,
    loadingOlder,
    totalRows,
    scroller,
    streamEl,
    onreleasebottom,
    rowCap = $bindable(INITIAL_ROW_CAP),
  }: Props = $props();

  $effect(() => {
    void sessionId;
    rowCap = INITIAL_ROW_CAP;
  });

  const hiddenLocalRows = $derived(Math.max(0, totalRows - rowCap));
  const olderError = $derived(runtimeViews.olderError[sessionId] ?? null);
  let barEl = $state<HTMLDivElement | null>(null);
  let busy = $state(false);

  async function loadMore(): Promise<void> {
    if (busy || loadingOlder || (hiddenLocalRows === 0 && !hasOlder)) return;
    busy = true;
    onreleasebottom?.();
    const hadFocusInBar = Boolean(
      barEl && typeof document !== 'undefined' && document.activeElement && barEl.contains(document.activeElement),
    );
    const el = scroller;
    const content = streamEl;
    const anchor = (barEl?.nextElementSibling ?? null) as HTMLElement | null;
    const beforeAnchorDocY =
      anchor && el ? anchor.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop : null;
    const beforeH = content?.offsetHeight ?? el?.scrollHeight ?? 0;
    try {
      if (hiddenLocalRows > 0) {
        rowCap += ROW_STEP;
      } else {
        rowCap = Math.max(rowCap + ROW_STEP, totalRows + ROW_STEP);
        const loaded = await runtimeViews.loadOlder(sessionId);
        if (!loaded) return;
      }
      await tick();
      if (el && el.isConnected) {
        if (anchor && anchor.isConnected && beforeAnchorDocY !== null) {
          const afterAnchorDocY =
            anchor.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop;
          const delta = afterAnchorDocY - beforeAnchorDocY;
          if (Math.abs(delta) >= 0.5) el.scrollTop += delta;
        } else {
          const afterH = content?.offsetHeight ?? el.scrollHeight;
          const delta = afterH - beforeH;
          if (delta > 0) el.scrollTop += delta;
        }
      }
      if (hadFocusInBar && !barEl?.isConnected) {
        const nextFocus =
          content?.querySelector<HTMLElement>('button:not(:disabled), [href], [tabindex]:not([tabindex="-1"])') ?? el;
        if (nextFocus) {
          if (nextFocus === el && !nextFocus.hasAttribute('tabindex')) {
            nextFocus.setAttribute('tabindex', '-1');
          }
          nextFocus.focus({ preventScroll: true });
        }
      }
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    const el = scroller;
    void sessionId;
    if (!el) return;
    let prevTop = el.scrollTop;
    let prevHeight = el.scrollHeight;
    const maybeAutoLoad = (forceUpward: boolean) => {
      const top = el.scrollTop;
      const height = el.scrollHeight;
      const scrolledUp = forceUpward || (top < prevTop && height >= prevHeight - 2);
      prevTop = top;
      prevHeight = height;
      const unpinned = el.dataset.atBottom === '0';
      if (
        (scrolledUp || unpinned) &&
        top <= AUTO_LOAD_TOP_PX &&
        (hiddenLocalRows > 0 || hasOlder) &&
        !olderError &&
        !busy &&
        !loadingOlder
      ) {
        void loadMore();
      }
    };
    const onScroll = () => maybeAutoLoad(false);
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY < 0 && el.scrollTop <= AUTO_LOAD_TOP_PX) maybeAutoLoad(true);
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    el.addEventListener('wheel', onWheel, { passive: true });
    return () => {
      el.removeEventListener('scroll', onScroll);
      el.removeEventListener('wheel', onWheel);
    };
  });
</script>

{#if hiddenLocalRows > 0 || hasOlder || olderError}
  <div class="rt-older" bind:this={barEl}>
    {#if olderError}
      <LoadState
        variant="compact"
        what="earlier turns"
        loading={busy || loadingOlder}
        error={olderError}
        empty
        onretry={() => void loadMore()}
      />
    {:else}
      <button
        type="button"
        class="btn small ghost"
        data-testid="runtime-load-older"
        disabled={busy || loadingOlder}
        onclick={() => void loadMore()}
      >
        <Icon name="arrowUp" size={12} />
        {#if loadingOlder}
          Loading earlier…
        {:else if hasOlder && olderTurns > 0}
          Load earlier ({plural(olderTurns, 'turn')})
        {:else}
          Show earlier ({hiddenLocalRows > 0 ? hiddenLocalRows : 'more'})
        {/if}
      </button>
    {/if}
  </div>
{/if}

<style>
  .rt-older {
    display: flex;
    justify-content: center;
    padding-block-end: 12px;
  }
</style>
