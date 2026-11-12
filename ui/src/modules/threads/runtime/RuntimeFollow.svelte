<script lang="ts">
  // Follow the bottom of a runtime thread's timeline while the reader is
  // there (T3's scroll anchoring); scrolled up, new messages are counted
  // into a "N new messages" pill that jumps back down — the terminal
  // conversation view's pill (agents/conversation/ConversationView.svelte),
  // ported for the runtime timeline. Rendered between the scroller and the
  // composer: the pill floats over the scroller's bottom edge.
  import { tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { followPillText, unreadSince } from './follow';

  interface Props {
    scroller: HTMLDivElement | null;
    /** The projection's last applied sequence: any change is new activity. */
    seq: number | undefined;
    /** The timeline's message ids, oldest first. */
    messageIds: string[];
  }
  let { scroller, seq, messageIds }: Props = $props();

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

  $effect(() => {
    const el = scroller;
    if (!el) return;
    const onScroll = (): void => {
      const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
      atBottom = gap < 40;
      farUp = gap > el.clientHeight;
      if (atBottom) unseen = 0;
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  });

  // New events: follow when at the bottom, else count them for the pill
  // (read untracked — `unseen += 1` here would re-trigger this effect).
  $effect(() => {
    void seq;
    if (untrack(() => atBottom)) void tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
    else unseen = untrack(() => unseen) + 1;
  });

  function jump(): void {
    scroller?.scrollTo({ top: scroller.scrollHeight });
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
