<script lang="ts">
  // Modal image lightbox for transcript, inbox, and workspace images:
  // shared Modal chrome, 1:1 / Fit zoom toggle, and path caption with copy.
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { copyTextOrThrow } from '../../../lib/clipboard';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';

  interface Props {
    src: string;
    alt: string;
    path?: string | null;
    onclose: () => void;
  }
  let { src, alt, path = null, onclose }: Props = $props();
  let zoomed = $state(false);

  const title = $derived(
    alt && alt !== 'Image'
      ? alt
      : path
        ? (path.split('/').pop() || 'Image preview')
        : 'Image preview',
  );

  async function copyCaptionPath(): Promise<void> {
    if (!path) return;
    try {
      await copyTextOrThrow(path);
      toasts.info('Path copied', path);
    } catch (e) {
      toastError('Couldn’t copy to the clipboard', e);
    }
  }
</script>

<Modal {title} width={960} {onclose}>
  <div class="lb-stage" class:zoomed data-lightbox>
    <button
      type="button"
      class="lb-canvas"
      class:zoomed
      onclick={() => (zoomed = !zoomed)}
      aria-label={zoomed ? 'Fit image to window' : 'Zoom image to actual size'}
      title={zoomed ? 'Click to fit to window' : 'Click to zoom (1:1)'}
    >
      <img {src} alt={alt || 'Image'} />
    </button>
  </div>

  {#snippet footer()}
    <div class="lb-foot">
      {#if path}
        <span class="lb-path mono" dir="ltr" title={path}>{path}</span>
        <button
          type="button"
          class="icon-btn"
          onclick={() => void copyCaptionPath()}
          aria-label="Copy image path"
          title="Copy image path"
        >
          <Icon name="copy" size={12} />
        </button>
      {/if}
      <span class="lb-grow"></span>
      <button
        type="button"
        class="btn small"
        onclick={() => (zoomed = !zoomed)}
        title={zoomed ? 'Fit image inside dialog' : 'Zoom image to actual size'}
      >
        <Icon name={zoomed ? 'minimize' : 'maximize'} size={12} />
        {zoomed ? 'Fit' : 'Zoom 1:1'}
      </button>
      <button type="button" class="btn small primary" onclick={onclose}>Done</button>
    </div>
  {/snippet}
</Modal>

<style>
  .lb-stage {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 220px;
    max-height: min(72vh, 720px);
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 12px;
  }
  .lb-stage.zoomed {
    align-items: flex-start;
    justify-content: flex-start;
  }
  .lb-canvas {
    display: flex;
    align-items: center;
    justify-content: center;
    margin: auto;
    padding: 0;
    border: 0;
    background: none;
    cursor: zoom-in;
    max-width: 100%;
  }
  .lb-canvas.zoomed {
    cursor: zoom-out;
    max-width: none;
  }
  .lb-canvas:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
    border-radius: var(--radius-s);
  }
  .lb-canvas img {
    display: block;
    max-width: 100%;
    max-height: calc(min(72vh, 720px) - 28px);
    object-fit: contain;
    border-radius: var(--radius-s);
  }
  .lb-canvas.zoomed img {
    max-width: none;
    max-height: none;
  }
  .lb-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }
  .lb-path {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    unicode-bidi: isolate;
  }
  .lb-grow {
    flex: 1;
  }
</style>
