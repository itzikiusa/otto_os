<script lang="ts">
  // One transcript image: authed fetch → blob URL (revoked on unmount), click
  // → lightbox. On-disk history transcripts have no session to serve images
  // from, so they get a labelled placeholder instead.
  import { getContext } from 'svelte';
  import { authedBlobUrl } from '../../../lib/api/client';
  import Icon from '../../../lib/components/Icon.svelte';
  import Lightbox from './Lightbox.svelte';
  import { fetchImageUrl } from './api';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    id: string;
    alt: string | null;
    mediaType?: string | null;
    /** Thumbnail height for images inside tool results. */
    small?: boolean;
  }
  let { id, alt, mediaType = null, small = false }: Props = $props();

  const ctx = getContext<ConvContext>(CONV_CTX);

  const localPath = $derived.by(() => {
    if (id.startsWith('file://')) {
      const raw = id.replace(/^file:\/\/(?:localhost)?/i, '');
      try {
        return decodeURIComponent(raw);
      } catch {
        return raw;
      }
    }
    return id.startsWith('/') ? id : null;
  });

  let url = $state<string | null>(null);
  let failed = $state(false);
  let open = $state(false);

  $effect(() => {
    const sid = ctx?.sessionId;
    const lp = localPath;
    if (!sid && !lp) return;
    const primary = sid
      ? (ctx.imageUrl ?? ((img: string) => fetchImageUrl(sid, img)))(id)
      : Promise.reject(new Error('no session'));
    const task = lp
      ? primary.catch(() => authedBlobUrl(`/fs/raw?path=${encodeURIComponent(lp)}`))
      : primary;
    let alive = true;
    let got: string | null = null;
    url = null;
    failed = false;
    task
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
</script>

{#if (!ctx?.sessionId && !localPath) || failed}
  <span class="img-ph" class:small title={mediaType ?? 'image'}>
    <Icon name="image" size={12} /> {alt ?? 'Image'}{failed ? ' (unavailable)' : ' (not served for on-disk transcripts)'}
  </span>
{:else if url}
  <button type="button" class="img-btn" class:small onclick={() => (open = true)} aria-label="Open image {alt ?? 'preview'}" title="Click to zoom">
    <img src={url} alt={alt ?? 'Image'} loading="lazy" />
  </button>
  {#if open}
    <Lightbox src={url} alt={alt ?? 'Image'} path={localPath} onclose={() => (open = false)} />
  {/if}
{:else}
  <span class="img-ph loading" class:small><Icon name="image" size={12} /> loading…</span>
{/if}

<style>
  .img-btn {
    display: block;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
    cursor: zoom-in;
    max-width: min(100%, 520px);
    margin: 6px 0;
  }
  .img-btn.small {
    max-width: 240px;
    margin: 4px 0;
  }
  .img-btn img {
    display: block;
    max-width: 100%;
    max-height: 360px;
    object-fit: contain;
  }
  .img-btn.small img {
    max-height: 140px;
  }
  .img-ph {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    padding: 2px 8px;
    margin: 4px 0;
  }
  .img-ph.loading {
    border-style: solid;
  }
</style>
