<script lang="ts">
  // Removable `@file` workspace mention items (inside the composer card) and
  // staged image upload thumbnails (below the composer card).
  import Icon from '../../../lib/components/Icon.svelte';

  export interface ComposerAttachment {
    path: string;
    name: string;
    url: string;
  }

  interface Props {
    mentionedFiles?: string[];
    onremovemention?: (rel: string) => void;
    attachments?: ComposerAttachment[];
    onremoveattachment?: (a: ComposerAttachment) => void;
  }

  let {
    mentionedFiles = [],
    onremovemention,
    attachments = [],
    onremoveattachment,
  }: Props = $props();
</script>

{#if mentionedFiles.length > 0}
  <div class="mention-strip" data-file-mentions={mentionedFiles.length}>
    {#each mentionedFiles as rel (rel)}
      <span class="mention-item" dir="ltr">
        <Icon name="file" size={12} />
        <span class="mention-path mono">{rel}</span>
        <button
          type="button"
          class="mention-rm"
          onclick={() => onremovemention?.(rel)}
          aria-label="Remove @{rel}"
          title="Remove @{rel}"
        >
          <Icon name="x" size={12} />
        </button>
      </span>
    {/each}
  </div>
{/if}

{#if attachments.length > 0}
  <div class="thumbs" data-attachments={attachments.length}>
    {#each attachments as a (a.path)}
      <div class="thumb" title={a.path}>
        <img src={a.url} alt={a.name} />
        <button
          type="button"
          class="thumb-x"
          onclick={() => onremoveattachment?.(a)}
          title="Remove {a.name}"
          aria-label="Remove {a.name}"
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .mention-strip {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-block: 0 4px;
    padding-inline-end: 6px;
  }
  .mention-item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent-line);
    background: var(--accent-faint);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    unicode-bidi: isolate;
  }
  .mention-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 240px;
  }
  .mention-rm {
    display: inline-grid;
    place-items: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-s);
    background: none;
    color: currentColor;
    cursor: pointer;
    opacity: 0.75;
  }
  .mention-rm:hover {
    opacity: 1;
    background: var(--accent-soft);
  }
  .thumbs {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .thumb {
    position: relative;
    width: 84px;
    height: 84px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface-2);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .thumb-x {
    position: absolute;
    inset-block-start: 4px;
    inset-inline-end: 4px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 0;
    background: var(--scrim-media);
    color: var(--on-scrim);
    display: grid;
    place-items: center;
    padding: 0;
    cursor: pointer;
  }
  @media (max-width: 640px), (pointer: coarse) {
    .thumb-x {
      width: var(--hit-min);
      height: var(--hit-min);
    }
  }
</style>
