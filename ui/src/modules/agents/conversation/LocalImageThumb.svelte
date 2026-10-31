<script lang="ts">
  // Inline local/workspace image thumbnail fetched via `/fs/raw?path=...`.
  // Used for `[Image: /abs/path]` attachments in user prompts and `file://`
  // image references; falls back to a clean path chip if `/fs/raw` is unavailable.
  import { getContext } from 'svelte';
  import { authedBlobUrl } from '../../../lib/api/client';
  import { copyTextOrThrow } from '../../../lib/clipboard';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import Icon from '../../../lib/components/Icon.svelte';
  import Lightbox from './Lightbox.svelte';
  import { resolvePath } from './format';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    path: string;
    alt?: string | null;
    small?: boolean;
  }
  let { path, alt = null, small = false }: Props = $props();
  const ctx = getContext<ConvContext | undefined>(CONV_CTX);

  const absPath = $derived(resolvePath(path, ctx?.cwd ?? null));
  const fileName = $derived(absPath.split('/').pop() || absPath);
  let url = $state<string | null>(null);
  let failed = $state(false);
  let open = $state(false);

  $effect(() => {
    const p = absPath;
    if (!p) return;
    let alive = true;
    let got: string | null = null;
    url = null;
    failed = false;
    authedBlobUrl(`/fs/raw?path=${encodeURIComponent(p)}`)
      .then((u) => {
        if (!alive) {
          URL.revokeObjectURL(u);
          return;
        }
        got = u;
        url = u;
      })
      .catch(() => {
        if (alive) failed = true;
      });
    return () => {
      alive = false;
      if (got) URL.revokeObjectURL(got);
    };
  });

  async function copyPath(): Promise<void> {
    try {
      await copyTextOrThrow(absPath);
      toasts.info('Path copied', absPath);
    } catch (e) {
      toastError('Couldn’t copy to the clipboard', e);
    }
  }
</script>

{#if url}
  <button
    type="button"
    class="local-img-card"
    class:small
    onclick={() => (open = true)}
    aria-label="Open image {alt ?? fileName}"
    title="Click to zoom · {absPath}"
  >
    <img src={url} alt={alt ?? fileName} loading="lazy" />
    <span class="local-img-bar">
      <Icon name="image" size={12} />
      <span class="local-img-name mono" dir="ltr">{fileName}</span>
      <Icon name="maximize" size={12} />
    </span>
  </button>
  {#if open}
    <Lightbox src={url} alt={alt ?? fileName} path={absPath} onclose={() => (open = false)} />
  {/if}
{:else if failed}
  <button
    type="button"
    class="attach-fallback"
    dir="ltr"
    onclick={() => void copyPath()}
    title="Attached image — {absPath} (click to copy path)"
  >
    <Icon name="image" size={12} />
    <span class="fallback-name mono">{fileName}</span>
  </button>
{:else}
  <span class="local-img-loading" class:small dir="ltr">
    <Icon name="image" size={12} />
    <span class="mono">{fileName}</span>
  </span>
{/if}

<style>
  .local-img-card {
    display: inline-flex;
    flex-direction: column;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
    cursor: zoom-in;
    max-width: min(100%, 320px);
    text-align: start;
    transition: border-color var(--dur-fast);
  }
  .local-img-card.small {
    max-width: 200px;
  }
  .local-img-card:hover {
    border-color: var(--border-strong);
  }
  .local-img-card:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .local-img-card img {
    display: block;
    max-width: 100%;
    max-height: 220px;
    object-fit: contain;
    background: var(--surface);
  }
  .local-img-card.small img {
    max-height: 120px;
  }
  .local-img-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    box-sizing: border-box;
    padding: 4px 8px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    min-width: 0;
  }
  .local-img-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    unicode-bidi: isolate;
  }
  .attach-fallback {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    padding: 2px 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    unicode-bidi: isolate;
  }
  .attach-fallback:hover {
    border-color: var(--border-strong);
  }
  .fallback-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .local-img-loading {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 4px 8px;
    background: var(--surface-2);
    unicode-bidi: isolate;
  }
</style>
