<script lang="ts" module>
  // Files read for the panel, so switching tabs / opening the full view /
  // reopening a reference never re-reads (Reload clears one entry).
  import type { FsRead } from '../../../lib/api/types';
  const reads = new Map<string, Promise<FsRead>>();
  const READS_MAX = 24;
  export function forgetRead(path: string): void {
    reads.delete(path);
  }
  export type PreviewMode = 'preview' | 'source' | 'diff';
</script>

<script lang="ts">
  // What the side panel shows for one request, in one mode:
  //   • preview — the file RENDERED: markdown (the chat renderer), HTML in a
  //     `sandbox=""` srcdoc frame (no scripts, no same-origin, no forms), SVG
  //     and images as <img> from a blob (an <img> never runs SVG script),
  //     JSON pretty-printed, CSV/TSV as a table;
  //   • source — CodeView (line numbers, highlighting, the target line);
  //   • diff — this turn's change (InlineDiff `full`).
  // Content comes from the transcript when it has it (a Write's input, a code
  // block), else `GET /fs/read` (≤ 400 KiB; binary → empty + truncated), or the
  // session artifact route for images.
  import { getContext } from 'svelte';
  import { api, authedBlobUrl } from '../../../lib/api/client';
  import { langFromPath } from '../../../lib/hl';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Markdown from './Markdown.svelte';
  import CodeView from './CodeView.svelte';
  import InlineDiff from './InlineDiff.svelte';
  import { htmlDoc, parseDelimited, prettyJson, requestKind } from './preview';
  import { resolvePath } from './format';
  import { CONV_CTX, type ConvContext, type PreviewReq } from './context';

  interface Props {
    req: PreviewReq;
    mode: PreviewMode;
    wrap?: boolean;
    /** Bumped by the panel's Reload. */
    reloadKey?: number;
  }
  let { req, mode, wrap = false, reloadKey = 0 }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);
  const cwd = $derived(ctx.cwd ?? null);

  const absPath = $derived(req.kind === 'file' ? resolvePath(req.path, cwd) : req.kind === 'code' && req.file ? resolvePath(req.file, cwd) : null);
  const kind = $derived(requestKind(req));

  let content = $state<string | null>(null);
  let truncated = $state(false);
  let loading = $state(false);
  let error = $state<string | null>(null);
  /** A failed read can be retried; "images need an artifact" cannot. */
  let retryable = $state(false);
  /** Bumped by Retry — in BOTH the side and the full view (the reload
   *  toolbar exists only in the side view). */
  let retryKey = $state(0);
  function retry(): void {
    if (absPath) reads.delete(absPath);
    retryKey++;
  }
  let imageUrl = $state<string | null>(null);

  /** An artifact the session produced at this path (images are served there). */
  function artifactId(path: string): string | null {
    for (const a of ctx.conv.liveArtifacts) if (a.path === path) return a.id;
    const turns = ctx.conv.turns;
    for (let i = turns.length - 1; i >= 0; i--) {
      for (const b of turns[i].blocks) if (b.kind === 'artifact' && b.artifact.path === path) return b.artifact.id;
    }
    return null;
  }

  $effect(() => {
    void reloadKey;
    void retryKey;
    const r = req;
    const path = absPath;
    error = null;
    retryable = false;
    truncated = false;
    imageUrl = null;
    if (r.kind === 'code') {
      content = r.text;
      return;
    }
    if (r.kind !== 'file' || !path) {
      content = null;
      return;
    }
    if (r.content != null) {
      content = r.content;
      return;
    }
    let alive = true;
    let blob: string | null = null;
    content = null;
    if (kind === 'image') {
      const id = artifactId(path);
      if (!ctx.sessionId || !id) {
        error = 'Images preview only when the session produced them as an output.';
        return;
      }
      loading = true;
      authedBlobUrl(`/sessions/${encodeURIComponent(ctx.sessionId)}/artifacts/${encodeURIComponent(id)}`).then(
        (u) => {
          if (!alive) return URL.revokeObjectURL(u);
          blob = u;
          imageUrl = u;
          loading = false;
        },
        (e) => {
          if (!alive) return;
          error = e instanceof Error ? e.message : String(e);
          retryable = true;
          loading = false;
        },
      );
      return () => {
        alive = false;
        if (blob) URL.revokeObjectURL(blob);
      };
    }
    loading = true;
    let p = reads.get(path);
    if (!p) {
      p = api.get<FsRead>(`/fs/read?path=${encodeURIComponent(path)}`);
      reads.set(path, p);
      p.catch(() => reads.delete(path));
      while (reads.size > READS_MAX) {
        const k = reads.keys().next().value;
        if (k === undefined) break;
        reads.delete(k);
      }
    }
    p.then(
      (d) => {
        if (!alive) return;
        content = d.content;
        truncated = d.truncated;
        loading = false;
      },
      (e) => {
        if (!alive) return;
        error = e instanceof Error ? e.message : String(e);
        retryable = true;
        loading = false;
      },
    );
    return () => {
      alive = false;
    };
  });

  const binary = $derived(content === '' && truncated);
  const lang = $derived(
    req.kind === 'code' ? (req.lang ?? (req.file ? langFromPath(req.file) : null)) : absPath ? langFromPath(absPath) : null,
  );
  const line = $derived(req.kind === 'file' ? (req.line ?? null) : null);

  // Rendered views.
  const json = $derived(kind === 'json' && content != null ? prettyJson(content) : null);
  const table = $derived(
    (kind === 'csv' || kind === 'tsv') && content != null ? parseDelimited(content, kind === 'csv' ? ',' : '\t') : null,
  );
  const svgUrl = $derived.by(() => {
    if (kind !== 'svg' || content == null || mode !== 'preview') return null;
    return URL.createObjectURL(new Blob([content], { type: 'image/svg+xml' }));
  });
  $effect(() => {
    const u = svgUrl;
    return () => {
      if (u) URL.revokeObjectURL(u);
    };
  });
</script>

<div class="pb" data-preview-mode={mode} data-preview-kind={kind}>
  {#if mode === 'diff'}
    {#if req.kind === 'diff'}
      <InlineDiff diff={req.diff} full {wrap} focus={req.focus ?? null} {cwd} />
    {:else if req.kind === 'file' && req.diff}
      <InlineDiff diff={req.diff} full {wrap} {cwd} />
    {:else}
      <EmptyState icon="split" title="No change recorded" body="This conversation has no edit to this file." />
    {/if}
  {:else if error}
    <div class="pb-err">
      <EmptyState
        icon="warning"
        tone="error"
        title="Couldn’t open the file"
        body={error}
        actionLabel={retryable ? 'Retry' : undefined}
        actionIcon="refresh"
        actionKind="secondary"
        onaction={retryable ? retry : undefined}
      />
    </div>
  {:else if loading || (content == null && !imageUrl)}
    <div class="pb-loading" aria-busy="true" aria-label="Loading the file">
      <div class="sk"></div>
      <div class="sk short"></div>
      <div class="sk"></div>
      <div class="sk mid"></div>
    </div>
  {:else if binary}
    <EmptyState icon="file" title="Binary file" body="This file is not text, so there is no preview or source to show." />
  {:else if mode === 'preview' && kind === 'image' && imageUrl}
    <div class="pb-img"><img src={imageUrl} alt={absPath ?? 'Image'} /></div>
  {:else if mode === 'preview' && content != null && kind === 'markdown'}
    <div class="pb-md"><Markdown md={content} /></div>
  {:else if mode === 'preview' && content != null && kind === 'html'}
    <div class="pb-frame-wrap">
      <div class="pb-note">Sandboxed preview — scripts, forms and network-origin access are off.</div>
      <iframe class="pb-frame" title="HTML preview of {absPath ?? 'the file'}" sandbox="" srcdoc={htmlDoc(content)}></iframe>
    </div>
  {:else if mode === 'preview' && svgUrl}
    <div class="pb-img checker"><img src={svgUrl} alt={absPath ?? 'SVG'} /></div>
  {:else if mode === 'preview' && json != null}
    <CodeView text={json} lang="json" {wrap} />
  {:else if mode === 'preview' && table}
    <div class="pb-table-wrap">
      <table class="pb-table">
        {#if table.rows.length}
          <thead><tr><th class="pb-rn">#</th>{#each table.rows[0] as h, i (i)}<th>{h}</th>{/each}</tr></thead>
        {/if}
        <tbody>
          {#each table.rows.slice(1) as r, ri (ri)}
            <tr><td class="pb-rn">{ri + 1}</td>{#each r as c, ci (ci)}<td>{c}</td>{/each}</tr>
          {/each}
        </tbody>
      </table>
      {#if table.cut}<div class="pb-note">Showing the first {table.rows.length} rows — the Source tab has them all.</div>{/if}
    </div>
  {:else if content != null}
    <CodeView text={content} {lang} {line} {wrap} />
  {/if}
  {#if truncated && !binary && content}
    <div class="pb-note trunc">Showing the first 400 KiB of a larger file.</div>
  {/if}
</div>

<style>
  .pb {
    min-width: 0;
    min-height: 100%;
    display: flex;
    flex-direction: column;
  }
  .pb-md {
    padding: 16px 20px 24px;
    --prose-measure: 90ch;
  }
  .pb-loading {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px;
  }
  .sk {
    height: 12px;
    width: 90%;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .sk.short {
    width: 55%;
  }
  .sk.mid {
    width: 72%;
  }
  @media (prefers-reduced-motion: no-preference) {
    .sk {
      animation: otto-pulse 1.4s ease-in-out infinite;
    }
  }
  .pb-err {
    padding: 12px;
  }
  .pb-img {
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 16px;
  }
  .pb-img img {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  /* A transparent SVG reads on a quiet checkerboard, in either scheme. */
  .pb-img.checker img {
    background-color: var(--surface);
    background-image:
      linear-gradient(45deg, var(--surface-2) 25%, transparent 25%),
      linear-gradient(-45deg, var(--surface-2) 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, var(--surface-2) 75%),
      linear-gradient(-45deg, transparent 75%, var(--surface-2) 75%);
    background-size: 16px 16px;
    background-position: 0 0, 0 8px, 8px -8px, -8px 0;
  }
  .pb-frame-wrap {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 320px;
  }
  .pb-frame {
    flex: 1;
    width: 100%;
    min-height: 320px;
    border: 0;
    /* The page's own colours: a document preview, not app chrome. */
    background: var(--surface);
  }
  .pb-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }
  .pb-note.trunc {
    border-bottom: 0;
    border-top: 1px solid var(--border);
  }
  .pb-table-wrap {
    overflow: auto;
    padding: 12px;
  }
  .pb-table {
    border-collapse: separate;
    border-spacing: 0;
    font-size: var(--fs-xs);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .pb-table th,
  .pb-table td {
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    border-inline-end: 1px solid var(--border);
    text-align: start;
    white-space: nowrap;
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pb-table th {
    position: sticky;
    top: 0;
    background: var(--surface-2);
    font-weight: 600;
  }
  .pb-table tbody tr:nth-child(even) {
    background: color-mix(in srgb, var(--text) 3%, transparent);
  }
  .pb-rn {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    text-align: end;
  }
</style>
