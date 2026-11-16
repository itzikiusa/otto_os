<script lang="ts">
  // Workbench preview pane: picks the renderer for the (already effective)
  // language and feeds it a DEBOUNCED copy of the editor content (~250 ms), so
  // typing never re-renders a diagram or a big table per keystroke. Async
  // renderers (Mermaid / D2 / images) carry their own latest-wins token.
  import { untrack } from 'svelte';
  import EmptyState from '../../../../lib/components/EmptyState.svelte';
  import { previewKind } from './kinds';
  import MarkdownPreview from './MarkdownPreview.svelte';
  import HtmlPreview from './HtmlPreview.svelte';
  import ImagePreview from './ImagePreview.svelte';
  import DiagramPreview from './DiagramPreview.svelte';
  import JsonPreview from './JsonPreview.svelte';

  interface Props {
    /** A preview language id or file extension (`md`, `mmd`, `json`, …). */
    lang: string;
    /** Editor text; for `image` the asset id. */
    content: string;
    name: string;
  }
  let { lang, content, name }: Props = $props();

  const kind = $derived(previewKind(lang));
  const DEBOUNCE_MS = 250;

  // Debounced content. A kind switch (or a new file) lands immediately.
  let shown = $state(untrack(() => content));
  let lastKind = untrack(() => kind);
  let lastName = untrack(() => name);
  $effect(() => {
    const next = content;
    const k = kind;
    const n = name;
    if (k !== lastKind || n !== lastName) {
      lastKind = k;
      lastName = n;
      shown = next;
      return;
    }
    if (next === untrack(() => shown)) return;
    const t = setTimeout(() => (shown = next), DEBOUNCE_MS);
    return () => clearTimeout(t);
  });
</script>

<div class="wb-preview" data-testid="wb-preview" data-kind={kind ?? 'none'} aria-label={`Preview of ${name}`} role="region">
  {#if kind === 'markdown'}
    <MarkdownPreview content={shown} />
  {:else if kind === 'html'}
    <HtmlPreview content={shown} {name} />
  {:else if kind === 'svg'}
    <ImagePreview content={shown} {name} />
  {:else if kind === 'mermaid' || kind === 'd2'}
    <DiagramPreview {kind} content={shown} />
  {:else if kind === 'json'}
    <JsonPreview content={shown} />
  {:else}
    <EmptyState
      icon="eye"
      title="No preview for this language"
      body="Preview supports Markdown, HTML, SVG, Mermaid, D2 and JSON."
    />
  {/if}
</div>

<style>
  .wb-preview {
    block-size: 100%;
    min-block-size: 0;
    overflow: auto;
    background: var(--surface);
    color: var(--text);
  }
</style>
