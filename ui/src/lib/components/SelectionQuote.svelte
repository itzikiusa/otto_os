<script lang="ts">
  // A "Quote" button over text selected inside `root` (only within an element
  // matching `within`, e.g. an assistant message): it hands the selection to
  // `onquote` and clears it. Fixed-positioned above the selection and clamped
  // into the viewport (RTL-aware, tracks scroll/resize).
  import Icon from './Icon.svelte';
  import { placeSelectionQuote } from '../quote/quote';

  interface Props {
    root: HTMLElement | null;
    within: string;
    onquote: (text: string) => void;
  }
  let { root, within, onquote }: Props = $props();

  let pick = $state<{ text: string; top: number; inlineStart: number } | null>(null);

  function read(): void {
    const sel = typeof window === 'undefined' ? null : window.getSelection();
    const text = sel?.toString().trim() ?? '';
    if (!sel || !text || sel.rangeCount === 0 || !root) {
      pick = null;
      return;
    }
    const range = sel.getRangeAt(0);
    const node = range.commonAncestorContainer;
    const el = node instanceof Element ? node : node.parentElement;
    if (!el || !root.contains(el) || !el.closest(within)) {
      pick = null;
      return;
    }
    const r = range.getBoundingClientRect();
    if ((r.width === 0 && r.height === 0) || r.bottom < 0 || r.top > window.innerHeight) {
      pick = null;
      return;
    }
    const rtl = document.documentElement.dir === 'rtl';
    const pos = placeSelectionQuote(r, { width: window.innerWidth, height: window.innerHeight }, rtl);
    pick = { text, top: pos.top, inlineStart: pos.inlineStart };
  }

  $effect(() => {
    let frame = 0;
    const onChange = (): void => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(read);
    };
    document.addEventListener('selectionchange', onChange);
    document.addEventListener('scroll', onChange, { capture: true, passive: true });
    window.addEventListener('resize', onChange);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener('selectionchange', onChange);
      document.removeEventListener('scroll', onChange, { capture: true });
      window.removeEventListener('resize', onChange);
    };
  });

  function quote(): void {
    if (!pick) return;
    onquote(pick.text);
    window.getSelection()?.removeAllRanges();
    pick = null;
  }
</script>

{#if pick}
  <button
    type="button"
    class="sq"
    style:top="{pick.top}px"
    style:inset-inline-start="{pick.inlineStart}px"
    onmousedown={(e) => e.preventDefault()}
    onclick={quote}
    title="Quote in your reply"
    aria-label="Quote in your reply"
    data-testid="selection-quote"
  >
    <Icon name="comment" size={12} />Quote
  </button>
{/if}

<style>
  .sq {
    position: fixed;
    z-index: var(--z-floating-bar);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding-block: 4px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-s);
    box-shadow: var(--shadow-card);
    cursor: pointer;
  }
  .sq:hover {
    background: var(--surface-2);
  }
</style>
