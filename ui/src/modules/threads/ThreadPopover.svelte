<script lang="ts">
  // A small anchored popover for Threads' header chips: placed with the
  // shared placeAnchored (clamped into the viewport, height capped, the list
  // scrolls), closed by Escape, an outside click or the trigger. Focus moves
  // in on open and back to the trigger on Escape. It lives under <body>: the
  // toolbar chip that opens it sits inside the page title's <h1>.
  import { onMount, tick, type Snippet } from 'svelte';
  import { placeAnchored } from '../../lib/anchorPlace';

  interface Props {
    anchor: HTMLElement;
    label: string;
    onclose: () => void;
    testid?: string;
    children: Snippet;
  }
  let { anchor, label, onclose, testid, children }: Props = $props();
  let panel = $state<HTMLDivElement | null>(null);
  let pos = $state({ top: -9999, left: -9999, maxH: 400 });

  function place(): void {
    if (panel) pos = placeAnchored(anchor, panel, { prefer: 'below', align: 'start' });
  }
  onMount(() => {
    void tick().then(() => {
      place();
      panel?.focus();
    });
    const onDown = (e: PointerEvent): void => {
      const t = e.target as Node | null;
      // Menus and dialogs the popover opens live outside it.
      if (t && !panel?.contains(t) && !anchor.contains(t) && !(t as Element).closest?.('.ctx-menu, [role="dialog"]')) onclose();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('resize', place);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('resize', place);
    };
  });
  function toBody(node: HTMLElement): { destroy(): void } {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    onclose();
    anchor.focus();
  }
</script>

<div
  bind:this={panel}
  class="tpop"
  role="dialog"
  aria-label={label}
  tabindex="-1"
  style:top="{pos.top}px"
  style:left="{pos.left}px"
  style:max-height="{pos.maxH}px"
  data-testid={testid}
  onkeydown={onKey}
  use:toBody
>
  {@render children()}
</div>

<style>
  .tpop {
    position: fixed;
    z-index: var(--z-popover);
    inline-size: min(360px, calc(100vw - 16px));
    overflow-y: auto;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    box-shadow: var(--glass-shadow);
    color: var(--text);
    font-family: var(--font-ui);
    font-size: var(--fs-m);
    font-weight: 400;
    text-align: start;
  }
  .tpop:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
</style>
