///<reference types="svelte" />
;
  // Files read for the panel, so switching tabs / opening the full view /
  // reopening a reference never re-reads (Reload clears one entry).
  import type { FsRead } from '../../../lib/api/types';
  const reads = new Map<string, Promise<FsRead>>();
  const READS_MAX = 24;
  export function forgetRead(path: string): void {
    reads.delete(path);
  }
  export type PreviewMode = 'preview' | 'source' | 'diff';
;;// What the side panel shows for one request, in one mode:
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
import Skeleton from '../../../lib/components/Skeleton.svelte';
import { loadErrorText } from '../../../lib/loadError';
import Markdown from './Markdown.svelte';
import CodeView from './CodeView.svelte';
import InlineDiff from './InlineDiff.svelte';
import { htmlDoc, parseDelimited, prettyJson, requestKind } from './preview';
import { resolvePath } from './format';
import { CONV_CTX, type ConvContext, type PreviewReq } from './context';
;

  interface Props {
    req: PreviewReq;
    mode: PreviewMode;
    wrap?: boolean;
    /** Bumped by the panel's Reload. */
    reloadKey?: number;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
        error = loadErrorText(e);
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
;
async () => {



 { svelteHTML.createElement("div", {     "class":`pb`,...__sveltets_2_empty({"data-preview-mode":mode}),...__sveltets_2_empty({"data-preview-kind":kind}),});
  if(mode === 'diff'){
    if(req.kind === 'diff'){
       { const $$_ffiDenilnI1C = __sveltets_2_ensureComponent(InlineDiff); new $$_ffiDenilnI1C({ target: __sveltets_2_any(), props: {       "diff":req.diff,"full":true,wrap,"focus":req.focus ?? null,cwd,}});}
    } else if (req.kind === 'file' && req.diff){
       { const $$_ffiDenilnI1C = __sveltets_2_ensureComponent(InlineDiff); new $$_ffiDenilnI1C({ target: __sveltets_2_any(), props: {     "diff":req.diff,"full":true,wrap,cwd,}});}
    }else{
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`split`,"title":`No change recorded`,"body":`This conversation has no edit to this file.`,}});}
    }
  } else if (error){
     { svelteHTML.createElement("div", { "class":`pb-err`,});
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {                 "icon":`warning`,"tone":`error`,"title":`Couldn’t open the file`,"body":error,"actionLabel":retryable ? 'Retry' : undefined,"actionIcon":`refresh`,"actionKind":`secondary`,"onaction":retryable ? retry : undefined,}});}
     }
  } else if (loading || (content == null && !imageUrl)){
    
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":0,"label":`the file`,}});}
     { svelteHTML.createElement("div", {   "class":`pb-loading`,"aria-hidden":`true`,});
       { svelteHTML.createElement("div", { "class":`sk`,}); }
       { svelteHTML.createElement("div", { "class":`sk short`,}); }
       { svelteHTML.createElement("div", { "class":`sk`,}); }
       { svelteHTML.createElement("div", { "class":`sk mid`,}); }
     }
  } else if (binary){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`file`,"title":`Binary file`,"body":`This file is not text, so there is no preview or source to show.`,}});}
  } else if (mode === 'preview' && kind === 'image' && imageUrl){
     { svelteHTML.createElement("div", { "class":`pb-img`,}); { svelteHTML.createElement("img", {    "src":imageUrl,"alt":absPath ?? 'Image',});} }
  } else if (mode === 'preview' && content != null && kind === 'markdown'){
     { svelteHTML.createElement("div", { "class":`pb-md`,}); { const $$_nwodkraM2C = __sveltets_2_ensureComponent(Markdown); new $$_nwodkraM2C({ target: __sveltets_2_any(), props: {  "md":content,}});} }
  } else if (mode === 'preview' && content != null && kind === 'html'){
     { svelteHTML.createElement("div", { "class":`pb-frame-wrap`,});
       { svelteHTML.createElement("div", { "class":`pb-note`,});          }
       { svelteHTML.createElement("iframe", {     "class":`pb-frame`,"title":`HTML preview of ${absPath ?? 'the file'}`,"sandbox":"","srcdoc":htmlDoc(content),}); }
     }
  } else if (mode === 'preview' && svgUrl){
     { svelteHTML.createElement("div", { "class":`pb-img checker`,}); { svelteHTML.createElement("img", {    "src":svgUrl,"alt":absPath ?? 'SVG',});} }
  } else if (mode === 'preview' && json != null){
     { const $$_weiVedoC1C = __sveltets_2_ensureComponent(CodeView); new $$_weiVedoC1C({ target: __sveltets_2_any(), props: {     "text":json,"lang":`json`,wrap,}});}
  } else if (mode === 'preview' && table){
     { svelteHTML.createElement("div", { "class":`pb-table-wrap`,});
       { svelteHTML.createElement("table", { "class":`pb-table`,});
        if(table.rows.length){
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "class":`pb-rn`,});  }    for(let h of __sveltets_2_ensureArray(table.rows[0])){let i = 1;i; { svelteHTML.createElement("th", {});h; }} } }
        }
         { svelteHTML.createElement("tbody", {});
              for(let r of __sveltets_2_ensureArray(table.rows.slice(1))){let ri = 1;ri;
             { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", { "class":`pb-rn`,});ri + 1; }    for(let c of __sveltets_2_ensureArray(r)){let ci = 1;ci; { svelteHTML.createElement("td", {});c; }} }
          }
         }
       }
      if(table.cut){ { svelteHTML.createElement("div", { "class":`pb-note`,});   table.rows.length;         }}
     }
  } else if (content != null){
     { const $$_weiVedoC1C = __sveltets_2_ensureComponent(CodeView); new $$_weiVedoC1C({ target: __sveltets_2_any(), props: {     "text":content,lang,line,wrap,}});}
  }
  if(truncated && !binary && content){
     { svelteHTML.createElement("div", { "class":`pb-note trunc`,});         }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PreviewBody__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PreviewBody__SvelteComponent_ = ReturnType<typeof PreviewBody__SvelteComponent_>;
/*Ωignore_endΩ*/export default PreviewBody__SvelteComponent_;