///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
// Inline S3 object preview body: text (pretty JSON tree, CSV/TSV table with
// a raw toggle, plain text/logs), images and PDFs. Images/PDFs stream the
// object through the size-capped `download?inline=true` route into a Blob
// and render from an object URL — SVG only ever via <img> (no script runs),
// PDFs via <object> (the viewer runs no document script). Object URLs are
// revoked on change and on destroy.
import { awsApi, awsDownloadBlob } from '../../lib/api/aws';
import Skeleton from '../../lib/components/Skeleton.svelte';
import JsonTree from '../../lib/components/JsonTree.svelte';
import { fmtBytes } from './util';
import type { S3Object, S3PreviewResp } from '../../lib/api/types';
;

  interface Props {
    accountId: string;
    bucket: string;
    obj: S3Object;
    data: S3PreviewResp | null;
    loading: boolean;
    error: string;
    onretry: () => void;
    ondownload: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { accountId, bucket, obj, data, loading, error, onretry, ondownload }: Props = $props();

  /** Mirrors the daemon's `INLINE_PREVIEW_CAP` (crates/otto-aws/src/s3.rs). */
  const INLINE_CAP = 25 * 1024 * 1024;

  const RASTER_TYPES = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/bmp', 'image/avif', 'image/x-icon', 'image/vnd.microsoft.icon']);
  const RASTER_EXT: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', bmp: 'image/bmp', avif: 'image/avif', ico: 'image/x-icon' };
  /** The blob as a raster image type (its own when allow-listed, else from the
   *  key's extension), or null — SVG and anything else are not rendered. */
  function rasterImageBlob(blob: Blob, objectKey: string): Blob | null {
    const own = blob.type.split(';')[0].trim().toLowerCase();
    const type = RASTER_TYPES.has(own) ? own : RASTER_EXT[objectKey.split('.').pop()?.toLowerCase() ?? ''];
    return type ? new Blob([blob], { type }) : null;
  }
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
        // Never trust the object's stored Content-Type for what we render: a
        // blob URL is same-origin with the app, so an "image" or "pdf" served
        // as text/html would run as a page. Re-type from the DETECTED kind —
        // PDF always as application/pdf, images only as a raster type.
        const typed = k === 'pdf' ? new Blob([blob], { type: 'application/pdf' }) : rasterImageBlob(blob, key);
        if (!typed) {
          mediaError = 'This object isn’t a previewable raster image.';
          return;
        }
        url = URL.createObjectURL(typed);
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
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (error) announce(`Couldn’t preview. ${error}`, { assertive: true }); });
  $effect(() => { if (mediaError) announce(`Couldn’t preview. ${mediaError}`, { assertive: true }); });
;
async () => {

if(loading){
   { const $$_notelekS0C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS0C({ target: __sveltets_2_any(), props: {  "rows":6,}});}
} else if (error){
   { svelteHTML.createElement("p", { "class":`pv-err`,});error;  { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onretry,});  } }
} else if (tooBig){
   { svelteHTML.createElement("div", { "class":`pv-note`,});
     { svelteHTML.createElement("p", {});    fmtBytes(size);         }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":ondownload,});  }
   }
} else if (kind === 'image' || kind === 'pdf'){
  if(mediaLoading){
     { svelteHTML.createElement("div", {}); { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":4,"label":`preview`,}});} }
  } else if (mediaError){
     { svelteHTML.createElement("p", { "class":`pv-err`,});mediaError;  { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => mediaTry++,});  } }
  } else if (mediaUrl && kind === 'image'){
     { svelteHTML.createElement("div", { "class":`pv-media`,}); { svelteHTML.createElement("img", {    "src":mediaUrl,"alt":`Preview of ${obj.key}`,});} }
  } else if (mediaUrl){
     { svelteHTML.createElement("object", {       "class":`pv-pdf`,"data":mediaUrl,"type":`application/pdf`,"title":`PDF preview of ${obj.key}`,});
       { svelteHTML.createElement("p", { "class":`pv-dim`,});       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":ondownload,});  } }
     }
  }
} else if (kind === 'binary'){
   { svelteHTML.createElement("div", { "class":`pv-note`,});
     { svelteHTML.createElement("p", {});     data?.content_type ?? 'unknown type';  }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":ondownload,});  }
   }
} else if (kind === 'json' && json !== undefined){
   { svelteHTML.createElement("div", { "class":`pv-body mono`,}); { const $$_eerTnosJ1C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ1C({ target: __sveltets_2_any(), props: {  "value":json,}});} }
} else if (kind === 'csv' && csv.length){
   { svelteHTML.createElement("div", { "class":`pv-tools`,});
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (csvRaw = !csvRaw),});csvRaw ? 'Table' : 'Raw'; }
     { svelteHTML.createElement("span", { "class":`pv-dim`,});csv.length >= 200 ? 'First 200 rows' : `${csv.length} rows`; }
   }
  if(csvRaw){
     { svelteHTML.createElement("pre", { "class":`pv-body pv-text`,});data?.text ?? ''; }
  }else{
     { svelteHTML.createElement("div", { "class":`pv-body`,});
       { svelteHTML.createElement("table", { "class":`pv-csv`,});
         { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {});    for(let h of __sveltets_2_ensureArray(csv[0])){let i = 1;i; { svelteHTML.createElement("th", {});h; }} } }
         { svelteHTML.createElement("tbody", {});
              for(let row of __sveltets_2_ensureArray(csv.slice(1))){let ri = 1;ri;
             { svelteHTML.createElement("tr", {});    for(let c of __sveltets_2_ensureArray(row)){let ci = 1;ci; { svelteHTML.createElement("td", {});c; }} }
          }
         }
       }
     }
  }
}else{
   { svelteHTML.createElement("pre", { "class":`pv-body pv-text`,});data?.text ?? ''; }
}
if(!loading && !error && data?.truncated && (kind === 'text' || kind === 'json' || kind === 'csv')){
   { svelteHTML.createElement("p", { "class":`pv-dim`,});        fmtBytes(size);      }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const S3Preview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type S3Preview__SvelteComponent_ = ReturnType<typeof S3Preview__SvelteComponent_>;
/*Ωignore_endΩ*/export default S3Preview__SvelteComponent_;