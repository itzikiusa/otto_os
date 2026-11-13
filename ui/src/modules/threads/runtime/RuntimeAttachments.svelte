<script lang="ts">
  // What a sent message carried: image thumbnails and file chips, fetched
  // only once they scroll into view — readable size, aspect kept, never
  // cropped; a click opens the full image (Lightbox: fit / 1:1) or file.
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/TimelineUserMessageRow.tsx (attachments over
  // the bubble) and ExpandedImageDialog.tsx (click to open full).
  import { onDestroy } from 'svelte';
  import { authedBlobUrl } from '../../../lib/api/client';
  import type { MessageAttachment } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Lightbox from '../../../lib/components/Lightbox.svelte';
  import { toastError } from '../../../lib/toastError';

  interface Props {
    sessionId: string;
    attachments: MessageAttachment[];
  }
  let { sessionId, attachments }: Props = $props();

  let urls = $state<Record<string, string>>({});
  let opened = $state<MessageAttachment | null>(null);
  const pending = new Map<string, Promise<string | null>>();

  function load(a: MessageAttachment): Promise<string | null> {
    if (urls[a.id]) return Promise.resolve(urls[a.id]);
    let p = pending.get(a.id);
    if (!p) {
      p = authedBlobUrl(`/runtime-sessions/${encodeURIComponent(sessionId)}/attachments/${encodeURIComponent(a.id)}`)
        .then((u) => {
          urls = { ...urls, [a.id]: u };
          return u;
        })
        .catch(() => {
          pending.delete(a.id);
          return null;
        });
      pending.set(a.id, p);
    }
    return p;
  }

  /** Load a thumbnail once it is (nearly) on screen. */
  function lazy(node: HTMLElement, a: MessageAttachment) {
    const io = new IntersectionObserver(
      (es) => {
        if (es.some((e) => e.isIntersecting)) {
          io.disconnect();
          void load(a);
        }
      },
      { rootMargin: '200px' },
    );
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  async function open(a: MessageAttachment): Promise<void> {
    if (await load(a)) opened = a;
    else toastError('Couldn’t open the attachment', new Error(a.name ?? a.mime));
  }

  onDestroy(() => {
    for (const u of Object.values(urls)) URL.revokeObjectURL(u);
  });
</script>

<div class="ratt" data-testid="runtime-attachments">
  {#each attachments as a (a.id)}
    {#if a.kind === 'image'}
      <button type="button" class="ratt-thumb" class:loading={!urls[a.id]} use:lazy={a} aria-label="Open {a.name ?? 'image'}" title={a.name ?? 'Image'} onclick={() => void open(a)}>
        {#if urls[a.id]}<img src={urls[a.id]} alt={a.name ?? ''} />{:else}<Icon name="image" size={16} />{/if}
      </button>
    {:else}
      <button type="button" class="ratt-file" title="Open {a.name ?? a.mime}" onclick={() => void open(a)}>
        <Icon name="file" size={13} /><span class="ellipsis" dir="auto">{a.name ?? a.mime}</span>
      </button>
    {/if}
  {/each}
</div>

{#if opened && urls[opened.id] && opened.kind === 'image'}
  <Lightbox src={urls[opened.id]} alt={opened.name ?? 'Image'} testid="runtime-attachment-full" onclose={() => (opened = null)} />
{:else if opened && urls[opened.id]}
  <Modal title={opened.name ?? 'File'} width={900} onclose={() => (opened = null)}>
    <iframe class="ratt-doc" src={urls[opened.id]} title={opened.name ?? 'File'}></iframe>
  </Modal>
{/if}

<style>
  .ratt {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
    max-inline-size: 80%;
  }
  /* Readable, uncropped: 160px tall, width from the aspect (64–240px);
     a placeholder box until the bytes load. */
  .ratt-thumb {
    display: grid;
    place-items: center;
    min-inline-size: 64px;
    min-block-size: 64px;
    max-inline-size: 100%;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    cursor: zoom-in;
  }
  .ratt-thumb.loading {
    inline-size: 160px;
    block-size: 160px;
  }
  .ratt-thumb img {
    display: block;
    block-size: 160px;
    inline-size: auto;
    min-inline-size: 64px;
    max-inline-size: min(240px, 100%);
    object-fit: contain;
  }
  .ratt-thumb:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .ratt-file {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 220px;
    block-size: 28px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .ratt-doc {
    inline-size: 100%;
    block-size: 70vh;
    border: 0;
  }
</style>
