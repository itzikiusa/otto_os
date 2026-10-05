<script lang="ts">
  // Inline S3 object preview body: text (pretty JSON tree, CSV/TSV table with
  // a raw toggle, plain text/logs), images and PDFs. Images/PDFs stream the
  // object through the size-capped `download?inline=true` route into a Blob
  // and render from an object URL — SVG only ever via <img> (no script runs),
  // PDFs via <object> (the viewer runs no document script). Object URLs are
  // revoked on change and on destroy.
  import { awsApi, awsDownloadBlob } from '../../lib/api/aws';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import JsonTree from '../database/JsonTree.svelte';
  import { fmtBytes } from './util';
  import type { S3Object, S3PreviewResp } from '../../lib/api/types';

  interface Props {
    accountId: string;
    bucket: string;
    obj: S3Object;
    data: S3PreviewResp | null;
    loading: boolean;
    error: string;
    onretry: () => void;
    ondownload: () => void;
  }
  let { accountId, bucket, obj, data, loading, error, onretry, ondownload }: Props = $props();

  /** Mirrors the daemon's `INLINE_PREVIEW_CAP` (crates/otto-aws/src/s3.rs). */
  const INLINE_CAP = 25 * 1024 * 1024;

  const kind = $derived.by<'json' | 'csv' | 'text' | 'image' | 'pdf' | 'binary' | null>(() => {
    const d = data;
    if (!d) return null;
    if (d.kind === 'image' || d.kind === 'pdf') return d.kind;
    if (d.binary || d.kind === 'binary') return 'binary';
    const ct = (d.content_type ?? '').toLowerCase();
    const key = obj.key.toLowerCase();
    if (ct.includes('json') || key.endsWith('.json') || key.endsWith('.ndjson') || key.endsWith('.jsonl')) return 'json';
    if (ct.includes('csv') || key.endsWith('.csv') || key.endsWith('.tsv')) return 'csv';
    return 'text';
  });
  const size = $derived(data?.size ?? obj.size);
  const tooBig = $derived((kind === 'image' || kind === 'pdf') && size > INLINE_CAP);

  const json = $derived.by<unknown>(() => {
    if (kind !== 'json' || !data?.text) return undefined;
    try {
      return JSON.parse(data.text);
    } catch {
      return undefined;
    }
  });
  let csvRaw = $state(false);
  const csv = $derived.by<string[][]>(() => {
    if (kind !== 'csv' || !data?.text) return [];
    const sep = obj.key.toLowerCase().endsWith('.tsv') ? '\t' : ',';
    return data.text
      .split(/\r?\n/)
      .filter((l) => l.length)
      .slice(0, 200)
      .map((l) => splitCsvLine(l, sep));
  });

  /** One CSV line → cells, honouring "quoted, fields" and "" escapes. */
  function splitCsvLine(line: string, sep: string): string[] {
    if (sep === '\t' || !line.includes('"')) return line.split(sep);
    const out: string[] = [];
    let cur = '';
    let quoted = false;
    for (let i = 0; i < line.length; i++) {
      const c = line[i];
      if (quoted) {
        if (c === '"' && line[i + 1] === '"') {
          cur += '"';
          i++;
        } else if (c === '"') quoted = false;
        else cur += c;
      } else if (c === '"') quoted = true;
      else if (c === sep) {
        out.push(cur);
        cur = '';
      } else cur += c;
    }
    out.push(cur);
    return out;
  }

  // ── binary media (image / PDF) ──
  let mediaUrl = $state<string | null>(null);
  let mediaLoading = $state(false);
  let mediaError = $state('');
  let mediaTry = $state(0);

  $effect(() => {
    const k = kind;
    const key = obj.key;
    void mediaTry;
    if ((k !== 'image' && k !== 'pdf') || tooBig) return;
    const ctrl = new AbortController();
    let url: string | null = null;
    mediaLoading = true;
    mediaError = '';
    mediaUrl = null;
    awsDownloadBlob(awsApi.s3DownloadPath(accountId, bucket, key, true), undefined, ctrl.signal)
      .then(({ blob }) => {
        url = URL.createObjectURL(blob);
        mediaUrl = url;
      })
      .catch((e) => {
        if (e instanceof DOMException && e.name === 'AbortError') return;
        mediaError = e instanceof Error ? e.message : String(e);
      })
      .finally(() => {
        if (!ctrl.signal.aborted) mediaLoading = false;
      });
    return () => {
      ctrl.abort();
      if (url) URL.revokeObjectURL(url);
      mediaUrl = null;
    };
  });
</script>

{#if loading}
  <Skeleton rows={6} />
{:else if error}
  <p class="pv-err" role="alert">{error} <button class="btn small" onclick={onretry}>Retry preview</button></p>
{:else if tooBig}
  <div class="pv-note">
    <p>Too large to preview ({fmtBytes(size)}) — in-app preview is capped at 25 MB.</p>
    <button class="btn small" onclick={ondownload}>Download</button>
  </div>
{:else if kind === 'image' || kind === 'pdf'}
  {#if mediaLoading}
    <div><Skeleton rows={4} label="preview" /></div>
  {:else if mediaError}
    <p class="pv-err" role="alert">{mediaError} <button class="btn small" onclick={() => mediaTry++}>Retry preview</button></p>
  {:else if mediaUrl && kind === 'image'}
    <div class="pv-media"><img src={mediaUrl} alt={`Preview of ${obj.key}`} /></div>
  {:else if mediaUrl}
    <object class="pv-pdf" data={mediaUrl} type="application/pdf" title={`PDF preview of ${obj.key}`}>
      <p class="pv-dim">This PDF can’t be shown inline. <button class="btn small" onclick={ondownload}>Download</button></p>
    </object>
  {/if}
{:else if kind === 'binary'}
  <div class="pv-note">
    <p>No preview for this type ({data?.content_type ?? 'unknown type'}).</p>
    <button class="btn small" onclick={ondownload}>Download</button>
  </div>
{:else if kind === 'json' && json !== undefined}
  <div class="pv-body mono"><JsonTree value={json} /></div>
{:else if kind === 'csv' && csv.length}
  <div class="pv-tools">
    <button class="btn small" aria-pressed={csvRaw} onclick={() => (csvRaw = !csvRaw)}>{csvRaw ? 'Table' : 'Raw'}</button>
    <span class="pv-dim">{csv.length >= 200 ? 'First 200 rows' : `${csv.length} rows`}</span>
  </div>
  {#if csvRaw}
    <pre class="pv-body pv-text">{data?.text ?? ''}</pre>
  {:else}
    <div class="pv-body">
      <table class="pv-csv">
        <thead><tr>{#each csv[0] as h, i (i)}<th>{h}</th>{/each}</tr></thead>
        <tbody>
          {#each csv.slice(1) as row, ri (ri)}
            <tr>{#each row as c, ci (ci)}<td>{c}</td>{/each}</tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{:else}
  <pre class="pv-body pv-text">{data?.text ?? ''}</pre>
{/if}
{#if !loading && !error && data?.truncated && (kind === 'text' || kind === 'json' || kind === 'csv')}
  <p class="pv-dim">Truncated — showing the first 64 KB of {fmtBytes(size)}. Download for the whole object.</p>
{/if}

<style>
  .pv-dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
    margin: 0;
  }
  .pv-err {
    color: var(--danger);
    font-size: var(--fs-m);
  }
  .pv-note {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .pv-note p {
    margin: 0;
  }
  .pv-tools {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pv-body {
    font-size: var(--fs-s);
    overflow: auto;
    max-height: 60vh;
  }
  .pv-text {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: var(--font-mono);
    background: var(--bg);
    padding: 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
  }
  .pv-media {
    display: grid;
    place-items: center;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px;
    max-height: 70vh;
    overflow: auto;
  }
  .pv-media img {
    max-width: 100%;
    max-height: calc(70vh - 16px);
    object-fit: contain;
  }
  .pv-pdf {
    inline-size: 100%;
    block-size: 70vh;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
  }
  .pv-csv {
    border-collapse: collapse;
    font-size: var(--fs-s);
    font-family: var(--font-mono);
  }
  .pv-csv th,
  .pv-csv td {
    border: 1px solid var(--border);
    padding: 2px 6px;
    white-space: nowrap;
  }
  .pv-csv th {
    position: sticky;
    top: 0;
    background: var(--surface-2);
  }
</style>
