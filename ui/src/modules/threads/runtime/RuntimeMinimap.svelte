<script lang="ts">
  // A long thread's minimap (T3 Code's TimelineMinimap): a slim rail at the
  // reading column's inline-end edge, one tick per message you sent (a failed
  // turn and a turn that asked for an approval marked apart), the current
  // position lit. Hover / focus shows the message's first line; a click
  // jumps. ⌥↑ / ⌥↓ step between your messages from anywhere in the pane but
  // a text field (as in the transcript view). Hidden below MINIMAP_MIN turns
  // and on a phone, where the toolbar ⋯ › "Jump to…" opens the same list as
  // a sheet (the `otto:thread-jump` event names the thread).
  // Cost: the ticks are re-derived only when a message is added or a turn's
  // status changes, never per streamed delta; the position is read once per
  // animation frame while scrolling, over the ticks only.
  import { tick as flush, untrack } from 'svelte';
  import type { ProviderTurn, TurnItem } from '../../../lib/api/types';
  import Modal from '../../../lib/components/Modal.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { currentTick, MINIMAP_MIN, minimapTicks, type MinimapTick } from './minimap';

  interface Props {
    sessionId: string;
    items: readonly TurnItem[];
    turns: readonly ProviderTurn[];
    /** The timeline's message ids (memoized: changes only when one is added). */
    messageIds: readonly string[];
    scroller: HTMLElement | null;
    /** Render every row and stop following the bottom, before a jump. */
    onexpand: () => void;
    /** This pane is the focused one (⌥↑ / ⌥↓ with focus on the page body). */
    focused?: boolean;
  }
  let { sessionId, items, turns, messageIds, scroller, onexpand, focused = false }: Props = $props();

  const turnsKey = $derived(turns.map((t) => t.status).join());
  const ticks = $derived.by((): MinimapTick[] => {
    void messageIds;
    void turnsKey;
    return untrack(() => minimapTicks(items, turns));
  });
  const shown = $derived(ticks.length >= MINIMAP_MIN && !viewport.isPhone && !viewport.isShort);
  let cur = $state(0);
  let hover = $state<number | null>(null);
  let sheet = $state(false);

  const el = (id: string): HTMLElement | null => scroller?.querySelector<HTMLElement>(`[data-rt-msg="${CSS.escape(id)}"]`) ?? null;

  function measure(): void {
    if (!scroller) return;
    const atEnd = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 4;
    // Following the bottom (a streaming answer scrolls every frame): the last
    // tick, without querying and measuring every message.
    if (atEnd) {
      cur = Math.max(0, ticks.length - 1);
      return;
    }
    const top = scroller.getBoundingClientRect().top;
    cur = currentTick(ticks.map((t) => (el(t.id)?.getBoundingClientRect().top ?? NaN) - top), scroller.clientHeight, atEnd);
  }

  // The lit tick follows the scroll, once a frame (only while the rail shows).
  $effect(() => {
    const s = scroller;
    if (!s || !shown) return;
    let raf = 0;
    const on = (): void => {
      if (!raf) raf = requestAnimationFrame(() => ((raf = 0), measure()));
    };
    s.addEventListener('scroll', on, { passive: true });
    untrack(measure);
    return () => {
      s.removeEventListener('scroll', on);
      cancelAnimationFrame(raf);
    };
  });

  async function jump(i: number): Promise<void> {
    const t = ticks[i];
    if (!t) return;
    onexpand();
    await flush();
    el(t.id)?.scrollIntoView({ block: 'start' });
    cur = i;
  }

  // ⌥↑ / ⌥↓ in this pane (not while typing) and the phone's ⋯ › Jump to….
  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (!e.altKey || e.metaKey || e.ctrlKey || e.shiftKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
      const pane = scroller?.parentElement;
      const t = e.target as Element | null;
      // Inside this pane (not while typing), or outside every thread pane —
      // the page after clicking the conversation text — while it is the focused one.
      const here = !!t && (pane?.contains(t) || (untrack(() => focused) && !t.closest('section.rt')));
      if (!pane || !here || t instanceof HTMLTextAreaElement || t instanceof HTMLInputElement) return;
      if (!untrack(() => ticks.length)) return;
      e.preventDefault();
      untrack(measure);
      const next = untrack(() => Math.min(ticks.length - 1, Math.max(0, cur + (e.key === 'ArrowUp' ? -1 : 1))));
      void jump(next);
    };
    const onOpen = (e: Event): void => {
      if ((e as CustomEvent<{ sessionId: string }>).detail?.sessionId === sessionId) sheet = true;
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('otto:thread-jump', onOpen);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('otto:thread-jump', onOpen);
    };
  });

  function railKey(e: KeyboardEvent, i: number): void {
    const d = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : e.key === 'Home' ? -i : e.key === 'End' ? ticks.length - 1 - i : 0;
    if (!d) return;
    e.preventDefault();
    const n = Math.min(ticks.length - 1, Math.max(0, i + d));
    hover = n;
    ((e.currentTarget as HTMLElement).parentElement?.children[n] as HTMLElement | undefined)?.focus();
  }
</script>

{#if shown}
  <nav class="mm" style:--n={ticks.length} aria-label="Your messages in this thread" data-testid="runtime-minimap" onmouseleave={() => (hover = null)}>
    {#each ticks as t, i (t.id)}
      <button
        type="button"
        class="mm-tick"
        class:on={i === cur}
        class:failed={t.mark === 'failed'}
        class:approval={t.mark === 'approval'}
        aria-label="{i + 1}. {t.text}{t.mark === 'failed' ? ' (failed)' : t.mark === 'approval' ? ' (asked for approval)' : ''}"
        aria-current={i === cur ? 'location' : undefined}
        tabindex={i === cur ? 0 : -1}
        data-testid="runtime-minimap-tick"
        onmouseenter={() => (hover = i)}
        onfocus={() => (hover = i)}
        onblur={() => (hover = null)}
        onkeydown={(e) => railKey(e, i)}
        onclick={() => void jump(i)}
      ></button>
    {/each}
    {#if hover !== null && ticks[hover]}
      <span class="mm-label" style:--at="{(hover + 0.5) / ticks.length}" aria-hidden="true" data-testid="runtime-minimap-label">{ticks[hover].text}</span>
    {/if}
  </nav>
{/if}

{#if sheet}
  <Modal title="Jump to…" onclose={() => (sheet = false)}>
    <ol class="mm-list" data-testid="runtime-jump-list">
      {#each ticks as t, i (t.id)}
        <li>
          <button type="button" class="mm-row" class:on={i === cur} onclick={() => { sheet = false; void jump(i); }}>
            <span class="mm-n">{i + 1}</span><span class="ellipsis" dir="auto">{t.text}</span>
            {#if t.mark === 'failed'}<span class="mm-mark failed">Failed</span>{:else if t.mark === 'approval'}<span class="mm-mark approval">Approval</span>{/if}
          </button>
        </li>
      {:else}
        <li class="mm-empty">No messages yet.</li>
      {/each}
    </ol>
  </Modal>
{/if}

<style>
  /* Just outside the 760 px reading column when the pane has room, else at
     the pane's edge; never more than half the pane tall. */
  .mm {
    position: absolute;
    inset-block-start: 50%;
    inset-inline-end: max(4px, calc((100cqi - var(--read-col) - 2 * var(--sp-6)) / 2 - 20px));
    z-index: 2;
    display: flex;
    flex-direction: column;
    justify-content: center;
    inline-size: 16px;
    block-size: min(50cqb, calc(var(--n, 1) * 12px));
    max-block-size: 50cqb;
    translate: 0 -60%;
  }
  .mm-tick {
    flex: 1;
    min-block-size: 3px;
    max-block-size: 12px;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: pointer;
    position: relative;
  }
  .mm-tick::after {
    content: '';
    position: absolute;
    inset-inline: 3px;
    inset-block: calc(50% - 1px);
    border-radius: 1px;
    background: var(--border-strong);
  }
  .mm-tick:hover::after,
  .mm-tick:focus-visible::after {
    background: var(--text-dim);
  }
  .mm-tick.on::after {
    inset-inline: 0;
    background: var(--accent-solid);
  }
  .mm-tick.failed::after {
    background: var(--danger);
  }
  .mm-tick.approval::after {
    background: var(--warning);
  }
  .mm-tick:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .mm-label {
    position: absolute;
    inset-inline-end: 22px;
    inset-block-start: calc(var(--at) * 100%);
    translate: 0 -50%;
    max-inline-size: 280px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow-card);
    color: var(--text);
    font-size: var(--fs-s);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    pointer-events: none;
  }
  .mm-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .mm-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    inline-size: 100%;
    min-block-size: calc(var(--hit-min) + 8px);
    padding-inline: var(--sp-2);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .mm-row.on {
    background: var(--surface-2);
  }
  .mm-row:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .mm-row .ellipsis {
    flex: 1;
    min-inline-size: 0;
  }
  .mm-n {
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .mm-mark {
    font-size: var(--fs-s);
  }
  .mm-mark.failed {
    color: var(--danger);
  }
  .mm-mark.approval {
    color: var(--warning);
  }
  .mm-empty {
    color: var(--text-dim);
  }
</style>
