<script lang="ts">
  // Sheet-style modal: centered card, dimmed backdrop, Esc / backdrop click to
  // close.
  import type { Snippet } from 'svelte';
  import { untrack } from 'svelte';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';
  import { dialogReturnTarget } from '../dialogFocus';

  interface Props {
    title: string;
    width?: number;
    onclose: () => void;
    children: Snippet;
    footer?: Snippet;
  }
  let { title, width = 460, onclose, children, footer }: Props = $props();

  let sheetEl = $state<HTMLElement | null>(null);
  // Text-only confirmations need a keyboard stop for their scrolling body.
  // Keep ordinary short sheets out of the Tab order.
  function scrollableBody(node: HTMLElement) {
    const update = () => {
      if (node.scrollHeight > node.clientHeight + 1) node.tabIndex = 0;
      else node.removeAttribute('tabindex');
    };
    const resize = new ResizeObserver(update);
    const content = new MutationObserver(update);
    resize.observe(node);
    content.observe(node, { childList: true, subtree: true, characterData: true });
    update();
    return { destroy() { resize.disconnect(); content.disconnect(); } };
  }
  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function focusables(): HTMLElement[] {
    if (!sheetEl) return [];
    // offsetParent filters display:none/collapsed elements (a fixed-position
    // sheet still gives its children an offsetParent).
    return Array.from(sheetEl.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => el.tabIndex >= 0 && (el.offsetParent !== null || el === document.activeElement),
    );
  }

  // Register as an open overlay so the native browser webview (which paints
  // above the HTML) hides while this modal is up.
  //
  // `untrack` is essential: `pushModal()` does `modalCount += 1`, which READS
  // modalCount. Without untrack the effect would depend on the very state it
  // mutates → it re-runs on every change → infinite push/pop loop
  // (`effect_update_depth_exceeded`), which wedges Svelte's flush and makes the
  // whole UI unclickable (Cancel/Save/✕ all dead). We only want push-on-mount
  // / pop-on-unmount, so the effect must have no reactive dependencies.
  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });

  // Move focus into the sheet on open and hand it back to whatever had it
  // when the modal closes. An explicit `data-autofocus` / `autofocus` control
  // wins (the default button of a confirm, a prompt's field), else the first
  // body control, else the close button.
  $effect(() => {
    const prev = dialogReturnTarget();
    const els = untrack(focusables);
    (
      els.find((el) => el.hasAttribute('data-autofocus') || el.hasAttribute('autofocus')) ??
      els.find((el) => !el.closest('header') && !el.classList.contains('sheet-body')) ??
      els[0]
    )?.focus();
    return () => prev?.focus();
  });

  function onKeydown(e: KeyboardEvent) {
    // Nested pickers are separate sheets. Only the top sheet may trap keys;
    // otherwise an underlying New Session modal steals focus on every Tab.
    const sheets = document.querySelectorAll<HTMLElement>('.sheet[role="dialog"][aria-modal="true"]');
    if (sheets[sheets.length - 1] !== sheetEl) return;
    if (e.key === 'Escape') {
      e.stopPropagation();
      onclose();
    } else if (e.key === 'Tab') {
      // Trap Tab inside the sheet: wrap at the ends, and pull focus back in if
      // it somehow escaped (e.g. a click on the backdrop).
      const els = focusables();
      if (els.length === 0) {
        e.preventDefault();
        return;
      }
      const active = document.activeElement;
      const inside = active instanceof HTMLElement && sheetEl?.contains(active);
      const idx = inside ? els.indexOf(active as HTMLElement) : -1;
      if (e.shiftKey) {
        if (idx <= 0) {
          e.preventDefault();
          els[els.length - 1].focus();
        }
      } else if (idx === -1 || idx === els.length - 1) {
        e.preventDefault();
        els[0].focus();
      }
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div
  class="backdrop"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) onclose();
  }}
>
  <div
    bind:this={sheetEl}
    class="sheet"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    style="width: min({width}px, calc(100vw - 24px))"
  >
    <header>
      <h2>{title}</h2>
      <button class="icon-btn" onclick={onclose} aria-label="Close" title="Close (Esc)">
        <Icon name="x" size={14} />
      </button>
    </header>
    <div class="sheet-body" use:scrollableBody>{@render children()}</div>
    {#if footer}
      <footer>{@render footer()}</footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    background: rgba(0, 0, 0, 0.35);
    display: grid;
    place-items: center;
    animation: fade-in 140ms ease-out;
  }
  .sheet {
    /* Size relative to the backdrop (which is `inset:0` → window height), NOT
       100vh — in the transparent overlay-titlebar WKWebView, 100vh resolves to
       the full SCREEN height, making tall modals overflow the window and clip
       the footer/bottom off-screen. */
    max-width: calc(100% - 48px);
    max-height: calc(100% - 64px);
    display: flex;
    flex-direction: column;
    min-height: 0;
    /* The sheet holds forms, so it is OPAQUE (never glass — foundations §7);
       it shares the floating-glass edge + elevation tokens so every floating
       layer reads as one family. */
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-l);
    box-shadow: var(--glass-shadow);
    animation: sheet-in 160ms ease-out;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 16px 10px;
  }
  h2 {
    margin: 0;
    min-width: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    /* A title carrying a path / branch / session name wraps instead of
       pushing the close button out of the sheet. */
    overflow-wrap: anywhere;
  }
  .sheet-body {
    padding: 4px 16px 16px;
    overflow-y: auto;
    flex: 1 1 auto;
    min-height: 0;
  }
  footer {
    display: flex;
    flex-wrap: wrap;
    flex-shrink: 0;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid var(--separator);
  }
  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }
  @keyframes sheet-in {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.985);
    }
  }
</style>
