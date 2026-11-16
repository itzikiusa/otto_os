<script lang="ts">
  // A "Quote" button over text selected inside `root` (only within an element
  // matching `within`, e.g. an assistant message): it hands the selection to
  // `onquote` and clears it. Fixed-positioned above the selection and clamped
  // into the viewport.
  import Icon from './Icon.svelte';

  interface Props {
    root: HTMLElement | null;
    within: string;
    onquote: (text: string) => void;
  }
  let { root, within, onquote }: Props = $props();

  let pick = $state<{ text: string; top: number; left: number } | null>(null);

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
    const W = 84;
    pick = {
      text,
      top: Math.min(Math.max(8, r.top - 34), window.innerHeight - 40),
      left: Math.min(Math.max(8, r.left + r.width / 2 - W / 2), window.innerWidth - W - 8),
    };
  }

  $effect(() => {
    let frame = 0;
    const onChange = (): void => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(read);
    };
    document.addEventListener('selectionchange', onChange);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener('selectionchange', onChange);
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
    style:inset-inline-start="{pick.left}px"
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
    z-index: 50;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding-block: 3px;
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
