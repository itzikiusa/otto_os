<script lang="ts" module>
  // Sanitized markdown for prose + tool text. Goes through the vault renderer
  // (marked + allowlist sanitizer) — NOT lib/md.ts, which is unsanitized.
  // Transcript prose has no vault to resolve wikilinks/embeds against, so the
  // resolver returns null (they render as plain text).
  import { renderNote } from '../../vault/mdRender';
  import { ensureHljs } from '../../../lib/hl';
  import { createMdCache } from './mdCache';

  const ctx = { resolve: () => null, assetUrl: () => null };
  // Shared across every mounted block: a re-derived block with an unchanged
  // string is a Map lookup, not marked + hljs + DOMParser (mdCache.ts).
  const cache = createMdCache((md) => renderNote(md, ctx), {
    // Room for the whole mounted window (≤300 turns, a few prose blocks each).
    maxEntries: 1000,
    maxChars: 4_000_000,
    maxEntryChars: 512 * 1024,
  });
  // hljs loads lazily; until it lands, fenced code renders escaped. Such
  // output is not cached, and the block re-renders once when hljs arrives
  // (this used to ride on the next live delta re-rendering everything).
  let hlLoaded = false;
  const FENCE = /```|~~~/;
</script>

<script lang="ts">
  interface Props {
    md: string;
    /** Compact variant for tool-result text / notes. */
    small?: boolean;
  }
  let { md, small = false }: Props = $props();

  let hlReady = $state(hlLoaded);
  if (!hlLoaded) {
    void ensureHljs().then(() => {
      hlLoaded = true;
      hlReady = true;
    });
  }
  const html = $derived.by(() => {
    const fenced = FENCE.test(md);
    // Read hlReady only when it matters, so fence-free blocks never re-render.
    return cache.get(md, !fenced || hlReady);
  });
</script>

<div class="md" class:small dir="auto">{@html html}</div>

<style>
  .md {
    font-size: var(--fs-m);
    line-height: 1.55;
    color: var(--text);
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .md.small {
    font-size: var(--fs-s);
  }
  .md :global(p) {
    margin: 0 0 0.6em;
  }
  .md :global(p:last-child) {
    margin-bottom: 0;
  }
  .md :global(pre) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 10px;
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.45;
    direction: ltr;
    text-align: start;
  }
  .md :global(code) {
    font-family: var(--font-mono);
    font-size: 0.92em;
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    padding: 0 4px;
    border-radius: 3px;
  }
  .md :global(pre code) {
    background: none;
    padding: 0;
  }
  .md :global(h1),
  .md :global(h2),
  .md :global(h3),
  .md :global(h4) {
    margin: 0.9em 0 0.4em;
    line-height: 1.3;
    font-weight: 600;
  }
  .md :global(h1) {
    font-size: 1.25em;
  }
  .md :global(h2) {
    font-size: 1.15em;
  }
  .md :global(h3) {
    font-size: 1.05em;
  }
  .md :global(ul),
  .md :global(ol) {
    margin: 0.3em 0 0.6em;
    padding-inline-start: 1.5em;
  }
  .md :global(li) {
    margin: 0.15em 0;
  }
  .md :global(blockquote) {
    margin: 0.5em 0;
    padding-inline-start: 10px;
    border-inline-start: 3px solid var(--border);
    color: var(--text-dim);
  }
  .md :global(table) {
    border-collapse: collapse;
    display: block;
    max-width: 100%;
    overflow-x: auto;
    font-size: var(--fs-s);
    margin: 0.5em 0;
  }
  .md :global(th),
  .md :global(td) {
    border: 1px solid var(--border);
    padding: 3px 8px;
    text-align: start;
  }
  .md :global(th) {
    background: var(--surface-2);
  }
  .md :global(a) {
    color: var(--accent-text);
  }
  .md :global(img) {
    max-width: 100%;
    border-radius: var(--radius-s);
  }
  .md :global(hr) {
    border: 0;
    border-top: 1px solid var(--border);
    margin: 0.8em 0;
  }
</style>
