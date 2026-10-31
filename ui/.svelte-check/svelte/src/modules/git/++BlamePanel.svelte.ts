///<reference types="svelte" />
;// `git blame` for one file, opened from a diff header's ⋯ menu (via
// `gitBridge`). The daemon returns one row per RUN of consecutive lines
// with the run's source text, so each row shows who/when next to the code
// itself (syntax-highlighted, deferred). Clicking the commit selects it in
// the graph; "Blame before this change" re-blames the file at the commit's
// parent (following renames via porcelain `previous`), with Back to return.

import type { BlameLine, BlameResp } from '../../lib/api/types';
import { api } from '../../lib/api/client';
import { gitBridge } from './gitBridge.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { TableWindow } from '../../lib/tableWindow.svelte';
import { ensureHljs, langFromPath } from '../../lib/hl';
import { DeferredHighlighter } from './diff-highlight.svelte';
;

  interface Props {
    repoId: string;
    path: string;
    /** Revision to blame; the daemon defaults to HEAD. */
    rev?: string;
    onclose: () => void;
  };;
  /** One display row per SOURCE line (uniform height, which `TableWindow`
   *  needs); the run's commit shows on its first line only. An older daemon
   *  without `text` gets one row per run with its line range. */
  interface Row {
    run: BlameLine;
    first: boolean;
    n: string;
    code: string | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, path, rev, onclose }: Props = $props();

  /** "Blame before this change" hops, newest last; empty = the props target. */
  let hops = $state<{ rev: string; path: string }[]>([]);
  // A new file/rev from the parent starts a fresh trail.
  $effect(() => {
    void path;
    void rev;
    hops = [];
  });
  const target = $derived(hops.at(-1) ?? { rev: rev ?? 'HEAD', path });

  const hl = new DeferredHighlighter();
  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
    return () => hl.clear();
  });
  const lang = $derived(hlReady ? langFromPath(target.path) : null);

  let blame = $state.raw<BlameResp | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let retryRev = $state(0);

  $effect(() => {
    void retryRev;
    const id = repoId;
    const p = target.path;
    const r = target.rev;
    loading = true;
    error = null;
    // Switching files aborts the previous blame; a late response is dropped.
    const ctl = new AbortController();
    api
      .get<BlameResp>(
        `/repos/${id}/blame?path=${encodeURIComponent(p)}&rev=${encodeURIComponent(r)}`,
        ctl.signal,
      )
      .then((b) => {
        if (!ctl.signal.aborted) blame = b;
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted) return;
        blame = null;
        error = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) loading = false;
      });
    return () => ctl.abort();
  });

  // Windowed: a generated or long-lived file blames to 20k+ runs, and every
  // run was a real <tr> (≈670 ms to mount). Past 400 runs only the rows in
  // view (+ overscan) render between two spacer rows (`TableWindow`).
  const tw = new TableWindow(400, 15);
  let bodyEl = $state<HTMLDivElement | undefined>();
  const lines = $derived(blame?.lines ?? []);

  const rows = $derived.by((): Row[] => {
    const out: Row[] = [];
    for (const run of lines) {
      const text = run.text ?? [];
      if (text.length === 0) out.push({ run, first: true, n: range(run.line_start, run.count), code: null });
      else text.forEach((t, k) => out.push({ run, first: k === 0, n: String(run.line_start + k), code: t }));
    }
    return out;
  });
  const win = $derived(tw.range(rows.length));
  $effect(() => {
    void win;
    tw.measure(bodyEl);
  });
  // ⌘F over every blame run, not just the mounted slice.
  $effect(() =>
    tw.findRows(
      () => bodyEl,
      () => rows,
      (r) => [r.n, r.first ? `${r.run.author}\n${r.run.short_sha}\n${day(r.run.at)}` : '', r.code ?? r.run.summary].join('\n'),
    ),
  );
  // A new blame starts at the top.
  $effect(() => {
    void blame;
    tw.reset(bodyEl);
  });
  /** Widest line range, so the gutter keeps one width while rows window in
   *  and out (auto table layout sizes columns from the MOUNTED rows only). */
  const gutterCh = $derived.by(() => {
    let max = 0;
    for (const l of lines) max = Math.max(max, l.line_start + l.count - 1);
    // Per-line numbers; a text-less (older daemon) run shows "a–b".
    return lines.some((l) => !l.text?.length) ? String(max).length * 2 + 1 : String(max).length;
  });

  function day(at: string): string {
    const d = new Date(at);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString();
  }

  function blameBefore(l: BlameLine): void {
    if (!l.previous) return;
    hops = [...hops, { rev: l.previous.sha, path: l.previous.path }];
  }

  /** "12" for a single line, "12–15" for a run. */
  function range(start: number, count: number): string {
    return count > 1 ? `${start}–${start + count - 1}` : `${start}`;
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`bl`,"aria-label":`Blame`,});
   { svelteHTML.createElement("header", { "class":`bl-head`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":13,}});}
     { svelteHTML.createElement("span", {   "class":`bl-title mono`,"title":target.path,});target.path; }
    if(blame){ { svelteHTML.createElement("span", { "class":`chip mono`,});blame.rev.length === 40 ? blame.rev.slice(0, 8) : blame.rev; }}
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(hops.length > 0){
       { svelteHTML.createElement("button", {         "class":`btn small ghost`,"onclick":() => (hops = hops.slice(0, -1)),"title":`Back to the previous blame`,"aria-label":`Back to the previous blame`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":12,}});} 
       }
    }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onclose,"aria-label":`Close blame`,"title":`Close blame`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
     }
   }

   { const $$_div1 = svelteHTML.createElement("div", {     "class":`bl-body`,"onscroll":tw.onscroll,});bodyEl = $$_div1;tw.viewH= $$_div1.clientHeight;
    if(loading){
       { svelteHTML.createElement("div", { "class":`bl-pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"height":22,}});} }
    } else if (error){
       { svelteHTML.createElement("div", { "class":`bl-pad`,}); { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {       "what":`blame`,error,"empty":true,"onretry":() => retryRev++,"variant":`compact`,}});} }
    } else if (!blame || lines.length === 0){
       { svelteHTML.createElement("p", { "class":`bl-msg`,});           }
    }else{
       { svelteHTML.createElement("table", {   "class":`bl-table`,"style":`--bl-gutter:${gutterCh}ch`,});
         { svelteHTML.createElement("tbody", {});
          if(win.top){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":4,"style":`height:${win.top}px`,}); } }}
              for(let r of __sveltets_2_ensureArray((rows.slice(win.start, win.end)))){let j = 1;`${r.run.sha}-${r.n}-${win.start + j}`;
            const l = r.run;
             { svelteHTML.createElement("tr", { });r.first;
               { svelteHTML.createElement("td", { "class":`bl-gutter mono`,});r.n; }
               { svelteHTML.createElement("td", { "class":`bl-who`,});
                if(r.first){
                   { svelteHTML.createElement("button", {       "class":`bl-commit`,"onclick":() => gitBridge.focusCommit(repoId, l.sha),"title":l.summary,});
                     { svelteHTML.createElement("span", { "class":`bl-author`,});l.author; }
                     { svelteHTML.createElement("span", { "class":`mono bl-sha`,});l.short_sha; }
                     { svelteHTML.createElement("span", { "class":`bl-date`,});day(l.at); }
                   }
                }
               }
               { svelteHTML.createElement("td", { "class":`bl-act`,});
                if(r.first && l.previous){
                   { svelteHTML.createElement("button", {         "class":`icon-btn bl-before`,"onclick":() => blameBefore(l),"title":`Blame before this change (${l.short_sha}^)`,"aria-label":`Blame before this change (${l.short_sha})`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});}
                   }
                }
               }
              if(r.code === null){
                 { svelteHTML.createElement("td", {   "class":`bl-code bl-summary`,"title":l.summary,});l.summary; }
              }else{
                 { svelteHTML.createElement("td", {   "class":`bl-code mono`,"title":l.summary,}); hl.html(r.code, lang); }
              }
             }
          }
          if(win.bottom){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":4,"style":`height:${win.bottom}px`,}); } }}
         }
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BlamePanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BlamePanel__SvelteComponent_ = ReturnType<typeof BlamePanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default BlamePanel__SvelteComponent_;