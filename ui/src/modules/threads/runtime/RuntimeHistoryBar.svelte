<script lang="ts">
  // Progressive history windowing + "Load earlier" control for RuntimeThread.
  // Keeps scroll offset anchored to `[data-runtime-stream]`'s height delta so
  // prepending older turns shifts visible messages 0 px.
  import { tick } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { runtimeViews } from './runtimeView.svelte';

  const INITIAL_ROW_CAP = 80;
  const ROW_STEP = 80;

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
  let barEl = $state<HTMLDivElement | null>(null);
  let busy = $state(false);

  async function loadMore(): Promise<void> {
    if (busy || loadingOlder || (hiddenLocalRows === 0 && !hasOlder)) return;
    busy = true;
    onreleasebottom?.();
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
    } finally {
      busy = false;
    }
  }
</script>

{#if hiddenLocalRows > 0 || hasOlder}
  <div class="rt-older" bind:this={barEl}>
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
        Load earlier ({olderTurns} {olderTurns === 1 ? 'turn' : 'turns'})
      {:else}
        Show earlier ({hiddenLocalRows > 0 ? hiddenLocalRows : 'more'})
      {/if}
    </button>
  </div>
{/if}

<style>
  .rt-older {
    display: flex;
    justify-content: center;
    padding-block-end: 12px;
  }
</style>
