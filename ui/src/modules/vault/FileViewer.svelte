<script lang="ts">
  // Non-markdown file viewer for the vault center pane — "prebuilt viewers"
  // for everything agents drop next to the notes: OpenAPI specs (json/yaml)
  // get a structured API view, JSON pretty-prints, CSV/TSV render as tables,
  // images/PDF display inline, and everything else falls back to
  // syntax-highlighted code. Content is loaded by the store (vault.openFile).
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import { formatHtml } from '../../lib/formatHtml';
  import { ensureHljs, highlightLine, langFromPath } from '../../lib/hl';
  import { ui } from '../../lib/stores/ui.svelte';
  import { parse as parseYaml } from 'yaml';
  import { renderD2 } from '../canvas/d2';
  import OpenApiView from './OpenApiView.svelte';
  import { RICH_TEXT_MAX, vault } from './vault.svelte';

  const path = $derived(vault.filePath ?? '');
  const name = $derived(path.split('/').pop() ?? path);
  const ext = $derived(name.includes('.') ? (name.split('.').pop()?.toLowerCase() ?? '') : '');

  const IMG = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'avif', 'bmp', 'ico']);
  const isImage = $derived(IMG.has(ext));
  const isPdf = $derived(ext === 'pdf');

  // Files past the rich-view budget (parse/pretty-print/highlight would freeze
  // the webview) render in a virtualized plain-text viewer instead.
  const isLarge = $derived(!isImage && !isPdf && vault.fileSize > RICH_TEXT_MAX);

  // .html renders in a sandboxed iframe (mockups/reference views agents drop in
  // the vault) with a Source toggle. No allow-same-origin: scripts run in an
  // opaque origin and can't reach the app or the daemon.
  const isHtml = $derived(ext === 'html' || ext === 'htm');

  // .d2 files render as diagrams (WASM, lazy) with a Source toggle — d2 is the
  // preferred format for data-model diagrams (sql_table), so agents drop these
  // next to the notes.
  const isD2 = $derived(ext === 'd2');
  let d2Svg = $state<string | null>(null);
  let d2Error = $state<string | null>(null);
  let d2Seq = 0;
  $effect(() => {
    const text = vault.fileText;
    const dark = ui.resolvedScheme === 'dark';
    d2Svg = null;
    d2Error = null;
    if (!isD2 || isLarge || text == null) return;
    const id = `vault-d2-file-${++d2Seq}`;
    void renderD2(id, text, { dark }).then(({ svg, error }) => {
      if (vault.fileText !== text) return; // switched files meanwhile
      d2Svg = svg ?? null;
      d2Error = error ?? null;
    });
  });

  // Re-render highlighted lines once hljs lazily arrives.
  let hlReady = $state(false);
  onMount(() => void ensureHljs().then(() => (hlReady = true)));

  /** json/yaml parsed (null = not parseable / not structured). */
  const parsed = $derived.by(() => {
    if (isLarge) return null;
    const text = vault.fileText;
    if (text == null) return null;
    try {
      if (ext === 'json') return JSON.parse(text) as unknown;
      if (ext === 'yaml' || ext === 'yml') return parseYaml(text) as unknown;
    } catch {
      /* malformed — fall through to the code view */
    }
    return null;
  });

  const openapi = $derived.by(() => {
    const p = parsed;
    if (p && typeof p === 'object' && ('openapi' in p || 'swagger' in p) && 'paths' in p) {
      return p as Record<string, unknown>;
    }
    return null;
  });
  /** OpenAPI/D2/HTML: toggle between the rendered view and the raw source. */
  let showRaw = $state(false);
  $effect(() => {
    void path;
    showRaw = false;
    bigPretty = null;
    bigPrettyError = '';
  });

  const prettyJson = $derived(
    ext === 'json' && parsed !== null ? JSON.stringify(parsed, null, 2) : null,
  );

  // -- CSV / TSV → table (quote-aware, bounded) --------------------------------
  const CSV_MAX_ROWS = 5000;
  function parseCsv(text: string, sep: string): string[][] {
    const rows: string[][] = [];
    let row: string[] = [];
    let cell = '';
    let inQ = false;
    for (let i = 0; i < text.length; i++) {
      const c = text[i];
      if (inQ) {
        if (c === '"') {
          if (text[i + 1] === '"') {
            cell += '"';
            i++;
          } else inQ = false;
        } else cell += c;
      } else if (c === '"') {
        inQ = true;
      } else if (c === sep) {
        row.push(cell);
        cell = '';
      } else if (c === '\n' || c === '\r') {
        if (c === '\r' && text[i + 1] === '\n') i++;
        row.push(cell);
        cell = '';
        rows.push(row);
        row = [];
        if (rows.length > CSV_MAX_ROWS) break;
      } else cell += c;
    }
    if (cell.length > 0 || row.length > 0) {
      row.push(cell);
      rows.push(row);
    }
    return rows;
  }

  const csvRows = $derived.by(() => {
    if (isLarge) return null;
    if (ext !== 'csv' && ext !== 'tsv') return null;
    const text = vault.fileText;
    if (!text) return null;
    const rows = parseCsv(text, ext === 'tsv' ? '\t' : ',');
    return rows.length > 0 ? rows : null;
  });
  const csvTruncated = $derived((csvRows?.length ?? 0) > CSV_MAX_ROWS);

  // -- highlighted code fallback -------------------------------------------------
  const CODE_MAX_LINES = 10_000;
  const codeHtml = $derived.by(() => {
    void hlReady;
    if (isImage || isPdf || isLarge) return null;
    if (openapi && !showRaw) return null;
    if (isD2 && !showRaw && d2Svg) return null;
    if (isHtml && !showRaw) return null;
    if (csvRows) return null;
    let text = prettyJson ?? vault.fileText;
    if (text == null) return null;
    if (isHtml) {
      // Agents save mockups minified (one huge line) — pretty-print for display.
      try {
        text = formatHtml(text);
      } catch {
        /* best-effort — show as saved */
      }
    }
    const lang = ext === 'json' ? 'json' : langFromPath(path);
    const lines = text.split('\n');
    const shown = lines.slice(0, CODE_MAX_LINES);
    return {
      html: shown.map((l) => highlightLine(l, lang)).join('\n'),
      truncated: lines.length > CODE_MAX_LINES,
      total: lines.length,
    };
  });

  // -- large files: virtualized plain text ---------------------------------------
  // Long lines (minified JSON is one giant line) are chunked so every row has a
  // bounded width; row count is capped so the sizer never exceeds the browser's
  // max element height (~33.5M px in WebKit).
  const BIG_LINE_CHUNK = 2000;
  const BIG_MAX_ROWS = 1_500_000;
  const BIG_ROW_H = 18;
  /** Pretty-printing is opt-in (a click) — parse+stringify of a huge JSON blocks
   * for seconds and roughly triples memory, so cap where we offer it. */
  const BIG_PRETTY_MAX = 64_000_000;
  let bigPretty = $state<string | null>(null);
  let bigPrettyError = $state('');
  let bigFormatting = $state(false);
  const canPrettyBig = $derived(
    isLarge && ext === 'json' && bigPretty === null && vault.fileSize <= BIG_PRETTY_MAX,
  );
  function formatBigJson(): void {
    const text = vault.fileText;
    if (text == null || bigFormatting) return;
    bigFormatting = true;
    bigPrettyError = '';
    // Let the button repaint to its "Formatting…" state before we block.
    setTimeout(() => {
      try {
        bigPretty = JSON.stringify(JSON.parse(text), null, 2);
      } catch (e) {
        bigPrettyError = e instanceof Error ? e.message : String(e);
      } finally {
        bigFormatting = false;
      }
    }, 30);
  }

  const bigLines = $derived.by(() => {
    if (!isLarge) return null;
    const text = bigPretty ?? vault.fileText;
    if (text == null) return null;
    const rows: string[] = [];
    let truncated = false;
    outer: for (const line of text.split('\n')) {
      if (line.length <= BIG_LINE_CHUNK) {
        rows.push(line);
        if (rows.length >= BIG_MAX_ROWS) {
          truncated = true;
          break;
        }
      } else {
        for (let i = 0; i < line.length; i += BIG_LINE_CHUNK) {
          rows.push(line.slice(i, i + BIG_LINE_CHUNK));
          if (rows.length >= BIG_MAX_ROWS) {
            truncated = true;
            break outer;
          }
        }
      }
    }
    return { rows, truncated };
  });
</script>

<div class="file-view">
  <header>
    <nav class="crumbs" aria-label="File path">
      {#each path.split('/') as part, i (i)}
        {#if i < path.split('/').length - 1}
          <span class="c dim">{part}</span><span class="sep">/</span>
        {:else}
          <span class="c">{part}</span>
        {/if}
      {/each}
    </nav>
    {#if !isLarge && (openapi || (isD2 && d2Svg) || isHtml)}
      <button class="mode-btn" onclick={() => (showRaw = !showRaw)}>
        <Icon name={showRaw ? 'eye' : 'function'} size={13} />
        {showRaw ? (openapi ? 'Spec view' : isHtml ? 'Preview' : 'Diagram') : 'Source'}
      </button>
    {/if}
  </header>

  <div class="body">
    {#if vault.fileLoading}
      <div class="notice">Loading…</div>
    {:else if vault.fileError}
      <div class="notice err">{vault.fileError}</div>
    {:else if isImage && vault.fileBlobUrl}
      <div class="img-wrap">
        <img src={vault.fileBlobUrl} alt={name} />
      </div>
    {:else if isPdf && vault.fileBlobUrl}
      <iframe class="pdf" src={vault.fileBlobUrl} title={name}></iframe>
    {:else if bigLines}
      <div class="big-bar">
        <span class="dim">
          Large file ({(vault.fileSize / 1024 / 1024).toFixed(1)} MB) — plain text view
        </span>
        {#if canPrettyBig || bigFormatting}
          <button class="mode-btn" onclick={formatBigJson} disabled={bigFormatting}>
            <Icon name="function" size={13} />
            {bigFormatting ? 'Formatting…' : 'Pretty-print JSON'}
          </button>
        {/if}
        {#if bigPrettyError}
          <span class="err">Not valid JSON: {bigPrettyError}</span>
        {/if}
      </div>
      <VirtualList items={bigLines.rows} estimateHeight={BIG_ROW_H} class="big-lines">
        {#snippet row(line: string)}<div class="big-ln">{line}</div>{/snippet}
      </VirtualList>
      {#if bigLines.truncated}
        <div class="notice">Showing the first {BIG_MAX_ROWS.toLocaleString()} lines.</div>
      {/if}
    {:else if isHtml && !showRaw && vault.fileText != null}
      <iframe class="html-frame" sandbox="allow-scripts" srcdoc={vault.fileText} title={name}
      ></iframe>
    {:else if isD2 && !showRaw && d2Svg}
      <div class="d2-wrap">{@html d2Svg}</div>
    {:else if isD2 && !showRaw && d2Error}
      <div class="notice err">Diagram error: {d2Error}</div>
      <pre class="code"><code class="hljs">{vault.fileText}</code></pre>
    {:else if openapi && !showRaw}
      <OpenApiView spec={openapi} />
    {:else if csvRows}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              {#each csvRows[0] as h, i (i)}<th>{h}</th>{/each}
            </tr>
          </thead>
          <tbody>
            {#each csvRows.slice(1) as r, ri (ri)}
              <tr>
                {#each r as cell, ci (ci)}<td>{cell}</td>{/each}
              </tr>
            {/each}
          </tbody>
        </table>
        {#if csvTruncated}
          <div class="notice">Showing the first {CSV_MAX_ROWS} rows.</div>
        {/if}
      </div>
    {:else if codeHtml}
      <pre class="code"><code class="hljs">{@html codeHtml.html}</code></pre>
      {#if codeHtml.truncated}
        <div class="notice">Showing {CODE_MAX_LINES} of {codeHtml.total} lines.</div>
      {/if}
    {:else}
      <div class="notice">No preview available for this file.</div>
    {/if}
  </div>
</div>

<style>
  .file-view {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border);
  }
  .crumbs {
    display: flex;
    gap: 4px;
    font-size: 12.5px;
    overflow: hidden;
    white-space: nowrap;
  }
  .c.dim,
  .sep {
    color: var(--text-dim);
  }
  .mode-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 4px 9px;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .mode-btn:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .notice {
    padding: 14px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .notice.err {
    color: var(--status-exited);
  }
  .img-wrap {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    flex: 1;
  }
  .img-wrap img {
    max-width: 100%;
    max-height: 100%;
    border-radius: 8px;
  }
  .d2-wrap {
    /* d2 renders theme-aware (dark themeID in dark mode) — no forced bg. */
    display: flex;
    justify-content: center;
    padding: 20px;
    overflow: auto;
    flex: 1;
  }
  .d2-wrap :global(svg) {
    max-width: 100%;
    height: auto;
  }
  .pdf {
    flex: 1;
    border: none;
    min-height: 0;
  }
  .html-frame {
    flex: 1;
    border: none;
    min-height: 0;
    /* Mockups usually assume a light page; srcdoc iframes are transparent. */
    background: #fff;
  }
  .table-wrap {
    padding: 12px 14px;
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
  }
  th,
  td {
    border: 1px solid var(--border);
    padding: 4px 10px;
    text-align: start;
    white-space: nowrap;
    max-width: 380px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--surface-2);
  }
  .code {
    margin: 0;
    padding: 14px 18px 40px;
    font-size: 12px;
    line-height: 1.55;
    overflow: auto;
    flex: 1;
  }
  .big-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    font-size: 12.5px;
    border-bottom: 1px solid var(--border);
  }
  .big-bar .dim {
    color: var(--text-dim);
  }
  .big-bar .err {
    color: var(--status-exited);
  }
  .file-view :global(.big-lines) {
    flex: 1;
    min-height: 0;
    padding: 8px 14px 40px;
    font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
    font-size: 12px;
  }
  /* Let long rows extend the scroll width instead of being clipped to the pane. */
  .file-view :global(.big-lines .vlist-win) {
    inset-inline-end: auto;
    min-width: 100%;
  }
  .big-ln {
    white-space: pre;
    height: 18px;
    line-height: 18px;
  }
</style>
