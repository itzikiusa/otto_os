<script lang="ts">
  // Full-window image viewer for transcript images. Esc / click-outside closes.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  interface Props {
    src: string;
    alt: string;
    onclose: () => void;
  }
  let { src, alt, onclose }: Props = $props();

  // A full-window overlay on the Modal layer: register it so the native
  // browser webview hides under it (untracked: pushModal reads the counter it
  // bumps — see Modal.svelte).
  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onclose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="lb" role="dialog" tabindex="-1" aria-modal="true" aria-label={alt || 'Image'} onclick={onclose}>
  <button class="lb-close icon-btn" aria-label="Close" title="Close (Esc)" onclick={onclose}><Icon name="x" size={16} /></button>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
  <img {src} {alt} onclick={(e) => e.stopPropagation()} />
</div>

<style>
  .lb {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    background: color-mix(in srgb, #000 78%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    cursor: zoom-out;
  }
  .lb img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    cursor: default;
  }
  .lb-close {
    position: absolute;
    top: 12px;
    inset-inline-end: 12px;
    color: #fff; /* on the fixed dark scrim in every theme */
  }
</style>
