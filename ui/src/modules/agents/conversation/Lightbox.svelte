<script lang="ts">
  // Full-window image viewer for transcript images. Esc / click-outside closes;
  // dialogFocus traps Tab inside and returns focus to the thumbnail on close.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { dialogFocus } from '../../../lib/dialogFocus';
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
</script>

<div class="lb" role="dialog" aria-modal="true" aria-label={alt || 'Image'} use:dialogFocus={onclose}>
  <div class="lb-backdrop" role="presentation" onclick={onclose}></div>
  <button class="lb-close icon-btn" aria-label="Close" title="Close" aria-keyshortcuts="Escape" onclick={onclose}><Icon name="x" size={16} /></button>
  <img {src} {alt} />
</div>

<style>
  .lb {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    background: var(--scrim-media);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  /* The click-outside target: behind the image and the close button. */
  .lb-backdrop {
    position: absolute;
    inset: 0;
    cursor: zoom-out;
  }
  .lb img {
    position: relative;
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
  }
  .lb-close {
    position: absolute;
    top: 12px;
    inset-inline-end: 12px;
    color: var(--on-scrim);
  }
</style>
