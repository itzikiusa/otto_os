<script lang="ts">
  // Sanitized markdown for prose + tool text. Goes through the vault renderer
  // (marked + allowlist sanitizer) — NOT lib/md.ts, which is unsanitized.
  // Transcript prose has no vault to resolve wikilinks/embeds against, so the
  // resolver returns null (they render as plain text).
  import { renderNote } from '../../vault/mdRender';

  interface Props {
    md: string;
    /** Compact variant for tool-result text / notes. */
    small?: boolean;
  }
  let { md, small = false }: Props = $props();

  const ctx = { resolve: () => null, assetUrl: () => null };
  const html = $derived(renderNote(md, ctx));
</script>

<div class="md" class:small dir="auto">{@html html}</div>

<style>
  .md {
    font-size: 13.5px;
    line-height: 1.55;
    color: var(--text);
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .md.small {
    font-size: 12.5px;
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
    font-size: 12px;
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
    font-size: 12.5px;
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
    color: var(--accent);
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
