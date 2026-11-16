<script lang="ts">
  // One file preview for every surface that shows a file — the IDE editor's
  // Preview / Split and Threads' file viewer — so Markdown, HTML, images, JSON,
  // Mermaid and tables read the same everywhere (and no surface grows its own
  // renderer). The kind comes from the file name (filePreviewKind.ts):
  //   markdown  lib/render/markdown (vault renderer) + ```mermaid diagrams,
  //             links/images resolved against the file's folder
  //   html      sandboxed srcdoc frame, scripts off until enabled per file
  //   svg       as an image (data: URL <img> — never inlined into Otto's DOM)
  //   image/pdf the bytes from `mediaUrl` (the host's authenticated file route)
  //   json      the lazily rendered JsonTree; invalid JSON → an inline message
  //   mermaid   the strict Mermaid bridge
  //   csv/tsv/jsonl  a read-only table / one JSON tree per line
  // The host lazy-loads this component: its renderers stay out of the page's
  // first-paint bundle, and Mermaid loads only when a diagram shows.
  import { onDestroy } from 'svelte';
  import EmptyState from './EmptyState.svelte';
  import JsonTree from './JsonTree.svelte';
  import LoadState from './LoadState.svelte';
  import { filePreviewKind, type FilePreviewKind } from './filePreviewKind';
  import { loadErrorText } from '../loadError';
  import { parseJsonWithPos, svgDataUrl } from '../render/previewText';
  import HtmlFrame from './preview/HtmlFrame.svelte';
  import ImageView from './preview/ImageView.svelte';
  import MarkdownView from './preview/MarkdownView.svelte';
  import MermaidView from './preview/MermaidView.svelte';
  import PreviewNotice from './preview/PreviewNotice.svelte';
  import TableView from './preview/TableView.svelte';

  interface Props {
    /** Absolute path of the file (names the kind; relative links resolve from its folder). */
    path: string;
    /** Override the kind detected from `path`. */
    kind?: FilePreviewKind | null;
    /** The file's text (the live buffer in an editor). Unused for image/pdf. */
    text?: string;
    /** Project root: a Markdown link starting with `/` resolves against it. */
    root?: string | null;
    /** A URL for a file's bytes (images, PDF, Markdown images). A `blob:` URL it
     *  returns is owned (revoked) by the preview. Null → bytes aren't reachable. */
    mediaUrl?: ((absPath: string) => string | Promise<string>) | null;
    /** Why `mediaUrl` is null (shown instead of the image), e.g. a remote project. */
    mediaUnavailable?: string;
    /** A relative Markdown link to another file was followed. */
    onOpenLink?: ((absPath: string, anchor: string | null) => void) | null;
    /** Markdown Split sync: keep this heading (0-based, -1 = top) in view. */
    headingOrdinal?: number | null;
  }
  let {
    path,
    kind = undefined,
    text = '',
    root = null,
    mediaUrl = null,
    mediaUnavailable = 'This file’s bytes can’t be fetched here.',
    onOpenLink = null,
    headingOrdinal = null,
  }: Props = $props();

  const k = $derived(kind === undefined ? filePreviewKind(path) : kind);
  const name = $derived(path.slice(path.lastIndexOf('/') + 1) || path);
  const binary = $derived(k === 'image' || k === 'pdf');
  const json = $derived(k === 'json' ? parseJsonWithPos(text) : null);

  // Bytes for image / PDF, fetched once per path (latest-wins).
  let media = $state<{ url: string | null; error: string | null; loading: boolean }>({ url: null, error: null, loading: false });
  let attempt = $state(0);
  let owned: string | null = null;
  function release(): void {
    if (owned) URL.revokeObjectURL(owned);
    owned = null;
  }
  onDestroy(release);

  $effect(() => {
    void attempt;
    const kk = k;
    const p = path;
    const load = mediaUrl;
    release();
    if (kk !== 'image' && kk !== 'pdf') {
      media = { url: null, error: null, loading: false };
      return;
    }
    if (!load) {
      media = { url: null, error: mediaUnavailable, loading: false };
      return;
    }
    media = { url: null, error: null, loading: true };
    let live = true;
    void Promise.resolve()
      .then(() => load(p))
      .then(async (u) => {
        let url = u;
        if (kk === 'pdf') {
          // Re-type the bytes: the frame only ever renders them as a PDF.
          const bytes = await (await fetch(u)).blob();
          if (u.startsWith('blob:')) URL.revokeObjectURL(u);
          url = URL.createObjectURL(new Blob([bytes], { type: 'application/pdf' }));
        }
        if (!live) {
          if (url.startsWith('blob:')) URL.revokeObjectURL(url);
          return;
        }
        if (url.startsWith('blob:')) owned = url;
        media = { url, error: null, loading: false };
      })
      .catch((e: unknown) => {
        if (live) media = { url: null, error: loadErrorText(e), loading: false };
      });
    return () => {
      live = false;
    };
  });
</script>

<div class="file-preview" role="region" aria-label={`Preview of ${name}`} data-testid="file-preview" data-kind={k ?? 'none'} data-preview-scroll>
  {#if !k}
    <EmptyState variant="panel" icon="eye" title="No preview for this file type" body="Preview shows Markdown, HTML, SVG, images, PDF, JSON, Mermaid and CSV/TSV files." />
  {:else if binary}
    {#if media.url && k === 'image'}
      <ImageView src={media.url} {name} />
    {:else if media.url}
      <iframe class="fp-pdf" src={media.url} title={`Preview of ${name}`} data-testid="file-preview-pdf"></iframe>
    {:else}
      <LoadState what={name} variant="panel" loading={media.loading} error={media.error} empty onretry={mediaUrl ? () => attempt++ : undefined} />
    {/if}
  {:else if !text.trim()}
    <EmptyState variant="panel" icon="file" title="Empty file" body="Nothing to preview yet — the preview follows the editor as you type." />
  {:else if k === 'markdown'}
    <MarkdownView {path} content={text} {root} {mediaUrl} {onOpenLink} {headingOrdinal} />
  {:else if k === 'html'}
    <HtmlFrame {path} content={text} {name} />
  {:else if k === 'svg'}
    <ImageView src={svgDataUrl(text)} {name} />
  {:else if k === 'json' && json}
    {#if json.ok}
      <div class="fp-json" data-testid="file-preview-json"><JsonTree value={json.value} /></div>
    {:else}
      <PreviewNotice title="Invalid JSON" message={json.error ?? 'Parse error'} line={json.line} col={json.col} />
    {/if}
  {:else if k === 'mermaid'}
    <MermaidView content={text} />
  {:else if k === 'csv' || k === 'tsv' || k === 'jsonl'}
    <TableView content={text} kind={k} />
  {/if}
</div>

<style>
  .file-preview {
    display: flex;
    flex-direction: column;
    flex: 1;
    block-size: 100%;
    min-block-size: 0;
    min-inline-size: 0;
    overflow: auto;
    background: var(--bg);
    color: var(--text);
  }
  .fp-json {
    padding: 8px 12px;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    overflow-x: auto;
  }
  .fp-pdf {
    flex: 1;
    inline-size: 100%;
    min-block-size: 320px;
    border: 0;
  }
</style>
