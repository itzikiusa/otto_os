<script lang="ts">
  // Follow the bottom of a runtime thread's timeline while the reader is
  // there (T3's scroll anchoring); scrolled up, new messages are counted
  // into a "N new messages" pill that jumps back down — the terminal
  // conversation view's pill (agents/conversation/ConversationView.svelte),
  // ported for the runtime timeline. Rendered between the scroller and the
  // composer: the pill floats over the scroller's bottom edge.
  import { tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { END_BAND, followAfterScroll, followPillText, unreadSince } from './follow';

  interface Props {
    scroller: HTMLDivElement | null;
    /** The projection's last applied sequence: any change is new activity. */
    seq: number | undefined;
    /** The timeline's message ids, oldest first. */
    messageIds: string[];
  }
  let { scroller, seq, messageIds }: Props = $props();

  /** Pinned to the end. Only the reader's own scroll lets go (follow.ts). */
  let atBottom = $state(true);
  /** Scrolled up by more than a screen: the pill offers the way back even with nothing new. */
  let farUp = $state(false);
  let unseen = $state(0);
  let lastSeenId = $state<string | null>(null);

  $effect(() => {
    if (atBottom) lastSeenId = messageIds[messageIds.length - 1] ?? null;
  });
  const unread = $derived(atBottom ? 0 : unreadSince(messageIds, lastSeenId));
  const text = $derived(atBottom ? null : followPillText(unread, unseen, farUp));

  function pin(): void {
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  $effect(() => {
    const el = scroller;
    if (!el) return;
    // The reader's input; a scroll within this window of it is theirs
    // (iOS momentum keeps scrolling after the finger lifts).
    let gestureAt = -Infinity;
    const gesture = (): void => {
      gestureAt = performance.now();
    };
    let lastTop = el.scrollTop;
    const onScroll = (): void => {
      const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
      const up = el.scrollTop < lastTop;
      lastTop = el.scrollTop;
      atBottom = followAfterScroll(atBottom, gap, performance.now() - gestureAt < 600, up);
      farUp = gap > el.clientHeight;
      if (gap < END_BAND) unseen = 0;
    };
    // Content that grows on its own (a diagram or an image finishing)
    // keeps the end in view while following, not only on the next event.
    // Growth the reader caused (opening a fold) moves the end away instead
    // of dragging what they opened out of view.
    const grown = new ResizeObserver(() => {
      if (performance.now() - gestureAt < 600) onScroll();
      else if (untrack(() => atBottom)) pin();
    });
    for (const child of el.children) grown.observe(child);
    const keys = (e: KeyboardEvent): void => {
      if (['PageUp', 'PageDown', 'Home', 'End', 'ArrowUp', 'ArrowDown', ' '].includes(e.key)) gesture();
    };
    const opts = { passive: true } as const;
    el.addEventListener('scroll', onScroll, opts);
    el.addEventListener('wheel', gesture, opts);
    el.addEventListener('touchmove', gesture, opts);
    el.addEventListener('pointerdown', gesture, opts);
    el.addEventListener('keydown', keys);
    return () => {
      grown.disconnect();
      el.removeEventListener('scroll', onScroll);
      el.removeEventListener('wheel', gesture);
      el.removeEventListener('touchmove', gesture);
      el.removeEventListener('pointerdown', gesture);
      el.removeEventListener('keydown', keys);
    };
  });

  // New events: follow when pinned, else count them for the pill
  // (read untracked — `unseen += 1` here would re-trigger this effect).
  $effect(() => {
    void seq;
    if (untrack(() => atBottom)) void tick().then(pin);
    else unseen = untrack(() => unseen) + 1;
  });

  function jump(): void {
    pin();
    atBottom = true;
    farUp = false;
    unseen = 0;
  }
</script>

{#if text}
  <div class="rt-follow">
    <button type="button" class="rt-pill" data-testid="runtime-new-pill" data-unread={unread} title="Jump to the latest message" onclick={jump}>
      <Icon name="arrowDown" size={12} />
      {text}
    </button>
  </div>
{/if}

<style>
  .rt-follow {
    position: relative;
    block-size: 0;
    flex: none;
  }
  .rt-pill {
    position: absolute;
    inset-block-end: 10px;
    inset-inline: 0;
    margin-inline: auto;
    inline-size: max-content;
    max-inline-size: calc(100% - 24px);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding-block: 4px;
    padding-inline: 10px 12px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    font-weight: 600;
    white-space: nowrap;
    box-shadow: var(--glass-shadow);
    cursor: pointer;
    z-index: 1;
  }
  .rt-pill:hover {
    background: var(--hover);
  }
  .rt-pill:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
</style>
