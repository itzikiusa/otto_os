<script lang="ts" module>
  // Sanitized markdown for prose + tool text. Goes through the vault renderer
  // (marked + allowlist sanitizer) — NOT lib/md.ts, which is unsanitized.
  // Transcript prose has no vault to resolve wikilinks/embeds against, so the
  // resolver returns null (they render as plain text).
  import { renderNote } from '../../vault/mdRender';
  import { ensureHljs } from '../../../lib/hl';
  import { createMdCache } from './mdCache';
  import { decorateCodeBlocks, runCodeAction } from './codeBlocks';

  const ctx = { resolve: () => null, assetUrl: () => null };
  // Shared across every mounted block: a re-derived block with an unchanged
  // string is a Map lookup, not marked + hljs + DOMParser (mdCache.ts).
  // Code blocks get their toolbar (label · Wrap · Copy · Show all) here, so it
  // is part of the cached string (codeBlocks.ts).
  const cache = createMdCache((md) => decorateCodeBlocks(renderNote(md, ctx)), {
    // Room for the whole mounted window (≤300 turns, a few prose blocks each).
    maxEntries: 1000,
    maxChars: 4_000_000,
    maxEntryChars: 512 * 1024,
  });
  // Perf-spec probe (e2e/desktop-conversation-perf.spec.ts): opt-in via a
  // `window.__ottoMdProbe` object installed before load; nothing otherwise.
  if (typeof window !== 'undefined') {
    const probe = (window as unknown as { __ottoMdProbe?: { cache?: unknown } }).__ottoMdProbe;
    if (probe) probe.cache = cache;
  }
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

<!-- Code-block buttons are real <button>s inside the html; one delegated
     handler drives them (keyboard activation bubbles as a click too). -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="md" class:small dir="auto" onclick={(e) => runCodeAction(e.target)}>{@html html}</div>

<style>
  .md {
    font-size: var(--fs-m);
    line-height: 1.6;
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
    line-height: 1.5;
    direction: ltr;
    text-align: start;
    margin: 0 0 0.6em;
  }
  /* ── Code block chrome (codeBlocks.ts): a quiet header strip, capped height
     for long blocks, Wrap toggle. The header is part of the block's frame. */
  .md :global(.code-block) {
    position: relative;
    margin: 0.4em 0 0.8em;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
    direction: ltr;
  }
  .md :global(.code-block pre) {
    margin: 0;
    border: 0;
    border-radius: 0;
    background: none;
    padding: 8px 12px 10px;
  }
  .md :global(.code-head) {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 26px;
    padding-inline: 10px 4px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .md :global(.code-lang) {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .md :global(.code-btn),
  .md :global(.code-more) {
    background: none;
    border: 0;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    padding: 2px 7px;
    cursor: pointer;
  }
  .md :global(.code-btn:hover),
  .md :global(.code-more:hover) {
    color: var(--text);
    background: var(--hover);
  }
  .md :global(.code-btn[aria-pressed='true']) {
    color: var(--accent-text);
  }
  .md :global(.code-btn:focus-visible),
  .md :global(.code-more:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .md :global(.code-block.wrap pre) {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  /* ~18 lines (1.5 × fs-s) + padding; the rest behind "Show all N lines". */
  .md :global(.code-block.capped:not(.expanded) pre) {
    max-height: calc(18 * 1.5em + 18px);
    overflow-y: hidden;
    -webkit-mask-image: linear-gradient(to bottom, rgba(0, 0, 0, 1) 80%, rgba(0, 0, 0, 0));
    mask-image: linear-gradient(to bottom, rgba(0, 0, 0, 1) 80%, rgba(0, 0, 0, 0));
  }
  .md :global(.code-more) {
    display: block;
    width: 100%;
    border-top: 1px solid var(--border);
    border-radius: 0;
    padding: 5px 10px;
    text-align: center;
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
