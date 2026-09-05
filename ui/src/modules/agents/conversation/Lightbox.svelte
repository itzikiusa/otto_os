<script lang="ts">
  // Full-window image viewer for transcript images. Esc / click-outside closes.
  interface Props {
    src: string;
    alt: string;
    onclose: () => void;
  }
  let { src, alt, onclose }: Props = $props();

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
  <button class="lb-close icon-btn" aria-label="Close" onclick={onclose}>✕</button>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
  <img {src} {alt} onclick={(e) => e.stopPropagation()} />
</div>

<style>
  .lb {
    position: fixed;
    inset: 0;
    z-index: 1000;
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
    color: #fff;
    font-size: 16px;
  }
</style>
