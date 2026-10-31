///<reference types="svelte" />
;// Non-markdown file viewer for the vault center pane — "prebuilt viewers"
// for everything agents drop next to the notes: OpenAPI specs (json/yaml)
// get a structured API view, JSON pretty-prints, CSV/TSV render as tables,
// images/PDF display inline, and everything else falls back to
// syntax-highlighted code. Content is loaded by the store (vault.openFile).

import { onMount } from 'svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import VirtualList from '../../lib/components/VirtualList.svelte';
import { formatHtml } from '../../lib/formatHtml';
import { ensureHljs, highlightLine, langFromPath } from '../../lib/hl';
import { ui } from '../../lib/stores/ui.svelte';
import { parse as parseYaml } from 'yaml';
import { renderD2 } from '../canvas/d2';
import OpenApiView from './OpenApiView.svelte';
import { RICH_TEXT_MAX, vault } from './vault.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
    void renderD2(id, text, { dark, isStale: () => vault.fileText !== text }).then(({ svg, error }) => {
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
  // V6: a 5k-row × N-column table used to mount every cell at once. Mount a
  // page of rows and grow on demand (a real <table> keeps column sizing and
  // the sticky header, which a fixed-height row window would break).
  const CSV_PAGE = 200;
  let csvShown = $state(CSV_PAGE);
  $effect(() => {
    void path;
    csvShown = CSV_PAGE;
  });
  const csvBody = $derived(csvRows ? csvRows.slice(1, 1 + csvShown) : []);
  const csvHidden = $derived(Math.max(0, (csvRows?.length ?? 1) - 1 - csvShown));

  // -- highlighted code fallback -------------------------------------------------
  const CODE_MAX_LINES = 10_000;
  /** Past this many lines the code view is windowed and highlights only the
   *  rows on screen (V6: 10k lines were highlighted eagerly into one <pre>). */
  const CODE_VIRTUAL_FROM = 1500;
  const CODE_ROW_H = 20;
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
    if (lines.length > CODE_VIRTUAL_FROM) {
      return { html: null, lines, lang, truncated: false, total: lines.length };
    }
    const shown = lines.slice(0, CODE_MAX_LINES);
    return {
      html: shown.map((l) => highlightLine(l, lang)).join('\n'),
      lines: null,
      lang,
      truncated: lines.length > CODE_MAX_LINES,
      total: lines.length,
    };
  });

  const codeLines = $derived(codeHtml?.lines ?? null);
  const codeLang = $derived(codeHtml?.lang ?? null);

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
;
async () => {

 { svelteHTML.createElement("div", { "class":`file-view`,});
   { svelteHTML.createElement("header", {});
     { svelteHTML.createElement("nav", {   "class":`crumbs`,"aria-label":`File path`,});
          for(let part of __sveltets_2_ensureArray(path.split('/'))){let i = 1;i;
        if(i < path.split('/').length - 1){
           { svelteHTML.createElement("span", { "class":`c dim`,});part; } { svelteHTML.createElement("span", { "class":`sep`,});  }
        }else{
           { svelteHTML.createElement("span", { "class":`c`,});part; }
        }
      }
     }
    if(!isLarge && (openapi || (isD2 && d2Svg) || isHtml)){
       { svelteHTML.createElement("button", {   "class":`mode-btn`,"onclick":() => (showRaw = !showRaw),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":showRaw ? 'eye' : 'function',"size":13,}});}
        showRaw ? (openapi ? 'Spec view' : isHtml ? 'Preview' : 'Diagram') : 'Source';
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`body`,});
    if(vault.fileLoading){
        { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {  "what":`the file`,"loading":true,"empty":true,}});}
    } else if (vault.fileError){
       { svelteHTML.createElement("div", { "class":`notice err`,});vault.fileError; }
    } else if (isImage && vault.fileBlobUrl){
       { svelteHTML.createElement("div", { "class":`img-wrap`,});
         { svelteHTML.createElement("img", {    "src":vault.fileBlobUrl,"alt":name,});}
       }
    } else if (isPdf && vault.fileBlobUrl){
       { svelteHTML.createElement("iframe", {     "class":`pdf`,"src":vault.fileBlobUrl,"title":name,}); }
    } else if (bigLines){
       { svelteHTML.createElement("div", { "class":`big-bar`,});
         { svelteHTML.createElement("span", { "class":`dim`,});
            (vault.fileSize / 1024 / 1024).toFixed(1);     
         }
        if(canPrettyBig || bigFormatting){
           { svelteHTML.createElement("button", {     "class":`mode-btn`,"onclick":formatBigJson,"disabled":bigFormatting,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`function`,"size":13,}});}
            bigFormatting ? 'Formatting…' : 'Pretty-print JSON';
           }
        }
        if(bigPrettyError){
           { svelteHTML.createElement("span", { "class":`err`,});   bigPrettyError; }
        }
       }
       { const $$_tsiLlautriV2C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV2 = new $$_tsiLlautriV2C({ target: __sveltets_2_any(), props: {       "items":bigLines.rows,"estimateHeight":BIG_ROW_H,"class":`big-lines`,"findText":(line: string) => line,row:(line: string) => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", { "class":`big-ln`,});line; }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV2.$$prop_def;/*Ωignore_endΩ*/
        
       VirtualList}
      if(bigLines.truncated){
         { svelteHTML.createElement("div", { "class":`notice`,});   BIG_MAX_ROWS.toLocaleString();  }
      }
    } else if (isHtml && !showRaw && vault.fileText != null){
       { svelteHTML.createElement("iframe", {        "class":`html-frame`,"sandbox":`allow-scripts`,"srcdoc":vault.fileText,"title":name,}); }
    } else if (isD2 && !showRaw && d2Svg){
       { svelteHTML.createElement("div", { "class":`d2-wrap`,}); d2Svg; }
    } else if (isD2 && !showRaw && d2Error){
       { svelteHTML.createElement("div", { "class":`notice err`,});  d2Error; }
       { svelteHTML.createElement("pre", { "class":`code`,}); { svelteHTML.createElement("code", { "class":`hljs`,});vault.fileText; } }
    } else if (openapi && !showRaw){
       { const $$_weiVipAnepO2C = __sveltets_2_ensureComponent(OpenApiView); new $$_weiVipAnepO2C({ target: __sveltets_2_any(), props: {  "spec":openapi,}});}
    } else if (csvRows){
       { svelteHTML.createElement("div", { "class":`table-wrap`,});
         { svelteHTML.createElement("table", {});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {});
                  for(let h of __sveltets_2_ensureArray(csvRows[0])){let i = 1;i; { svelteHTML.createElement("th", {});h; }}
             }
           }
           { svelteHTML.createElement("tbody", {});
                for(let r of __sveltets_2_ensureArray(csvBody)){let ri = 1;ri;
               { svelteHTML.createElement("tr", {});
                    for(let cell of __sveltets_2_ensureArray(r)){let ci = 1;ci; { svelteHTML.createElement("td", {});cell; }}
               }
            }
           }
         }
        if(csvHidden > 0){
           { svelteHTML.createElement("button", {   "class":`mode-btn more-rows`,"onclick":() => (csvShown += CSV_PAGE * 5),});
             Math.min(csvHidden, CSV_PAGE * 5).toLocaleString();   csvHidden.toLocaleString(); 
           }
        }
        if(csvTruncated){
           { svelteHTML.createElement("div", { "class":`notice`,});   CSV_MAX_ROWS;  }
        }
       }
    } else if (codeLines){
       { const $$_tsiLlautriV2C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV2 = new $$_tsiLlautriV2C({ target: __sveltets_2_any(), props: {       "items":codeLines,"estimateHeight":CODE_ROW_H,"class":`code-lines hljs`,"findText":(line: string) => line,row:(line: string) => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", { "class":`code-ln`,}); highlightLine(line, codeLang); }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV2.$$prop_def;/*Ωignore_endΩ*/
        
       VirtualList}
    } else if (codeHtml){
       { svelteHTML.createElement("pre", { "class":`code`,}); { svelteHTML.createElement("code", { "class":`hljs`,}); codeHtml.html; } }
      if(codeHtml.truncated){
         { svelteHTML.createElement("div", { "class":`notice`,}); CODE_MAX_LINES;  codeHtml.total;  }
      }
    }else{
       { svelteHTML.createElement("div", { "class":`notice`,});      }
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FileViewer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FileViewer__SvelteComponent_ = ReturnType<typeof FileViewer__SvelteComponent_>;
/*Ωignore_endΩ*/export default FileViewer__SvelteComponent_;