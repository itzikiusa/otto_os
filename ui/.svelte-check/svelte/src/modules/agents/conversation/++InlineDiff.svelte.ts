///<reference types="svelte" />
;// An edit's change, inline in the chat: one compact unified view per file —
// line number, +/− gutter, syntax-coloured code, tinted added/removed rows —
// capped at `maxLines` with "Show all". The git module's DiffViewer (file
// list, search, unified/split toolbar) is built for reviewing a branch; a
// chat edit is usually a few lines and wants none of that chrome.
// `full` is the side panel's variant (t3code's Diff panel): every file under
// a sticky header with its +/− and a fold, old AND new line numbers, no cap,
// optional wrapping, and a `focus` file scrolled into view.

import { tick } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { ensureHljs, highlightLine, langFromPath } from '../../../lib/hl';
import type { DiffResp } from '../../../lib/api/types';
;

  interface Props {
    diff: DiffResp;
    /** Rows shown before "Show all N lines" (compact only). */
    maxLines?: number;
    full?: boolean;
    wrap?: boolean;
    /** Full: the file to scroll to. */
    focus?: string | null;
    /** Full: shown relative to this folder. */
    cwd?: string | null;
  };;

  type Line = { origin: 'add' | 'del' | 'context'; old: number | null; new: number | null; html: string };;;
  type Row = { kind: 'file'; path: string } | { kind: 'gap'; text: string } | ({ kind: 'line' } & Line);;;

  // ── full (panel) ──────────────────────────────────────────────────────────
  type FileView = { path: string; add: number; del: number; hunks: { header: string; at: number | null; lines: Line[] }[] };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { diff, maxLines = 60, full = false, wrap = false, focus = null, cwd = null }: Props = $props();

  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });



  const rows = $derived.by(() => {
    if (full) return [] as Row[];
    const out: Row[] = [];
    const multi = diff.files.length > 1;
    for (const f of diff.files) {
      const lang = hlReady ? langFromPath(f.path) : null;
      if (multi) out.push({ kind: 'file', path: f.path });
      f.hunks.forEach((h, i) => {
        const first = h.lines.find((l) => l.new_line != null || l.old_line != null);
        const at = first?.new_line ?? first?.old_line;
        if (i > 0 || (at != null && at > 1)) out.push({ kind: 'gap', text: at != null ? `line ${at}` : h.header });
        for (const l of h.lines) {
          out.push({
            kind: 'line',
            origin: l.origin === 'add' || l.origin === 'del' ? l.origin : 'context',
            old: l.old_line,
            new: l.new_line,
            html: highlightLine(l.content || ' ', lang),
          });
        }
      });
    }
    return out;
  });
  const lineCount = $derived(full ? diff.files.reduce((n, f) => n + f.hunks.reduce((m, h) => m + h.lines.length, 0), 0) : rows.filter((r) => r.kind === 'line').length);
  let all = $state(false);
  const shown = $derived(all || rows.length <= maxLines ? rows : rows.slice(0, maxLines));

  const files = $derived.by(() => {
    if (!full) return [] as FileView[];
    return diff.files.map((f) => {
      const lang = hlReady ? langFromPath(f.path) : null;
      let add = 0;
      let del = 0;
      const hunks = f.hunks.map((h) => {
        const first = h.lines.find((l) => l.new_line != null || l.old_line != null);
        return {
          header: h.header,
          at: first?.new_line ?? first?.old_line ?? null,
          lines: h.lines.map((l): Line => {
            if (l.origin === 'add') add++;
            else if (l.origin === 'del') del++;
            return {
              origin: l.origin === 'add' || l.origin === 'del' ? l.origin : 'context',
              old: l.old_line,
              new: l.new_line,
              html: highlightLine(l.content || ' ', lang),
            };
          }),
        };
      });
      return { path: f.path, add: f.added ?? add, del: f.deleted ?? del, hunks };
    });
  });
  let folded = $state<Record<string, boolean>>({});
  const rel = (p: string): string => {
    if (!cwd) return p;
    const root = cwd.replace(/\/$/, '') + '/';
    return p.startsWith(root) ? p.slice(root.length) : p;
  };
  let rootEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const f = focus;
    if (!full || !f) return;
    void tick().then(() => rootEl?.querySelector(`[data-diff-file="${CSS.escape(f)}"]`)?.scrollIntoView({ block: 'start' }));
  });
;
async () => {

if(full){
   { const $$_div0 = svelteHTML.createElement("div", {      "class":`pdiff`,"dir":`ltr`,...__sveltets_2_empty({"data-lines":lineCount}),});wrap;rootEl = $$_div0;
       for(let f of __sveltets_2_ensureArray(files)){f.path;
       { svelteHTML.createElement("section", {    "class":`pdiff-file`,...__sveltets_2_empty({"data-diff-file":f.path}),});f.path === focus;
         { svelteHTML.createElement("button", {     "class":`pdiff-head`,"onclick":() => (folded[f.path] = !folded[f.path]),"aria-expanded":!folded[f.path],});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":folded[f.path] ? 'chevronRight' : 'chevronDown',"size":12,}});}
           { svelteHTML.createElement("span", {   "class":`pdiff-path mono`,"title":f.path,});rel(f.path); }
           { svelteHTML.createElement("span", { "class":`pdiff-stat mono`,}); { svelteHTML.createElement("span", { "class":`add`,}); f.add; }  { svelteHTML.createElement("span", { "class":`del`,}); f.del; } }
         }
        if(!folded[f.path]){
           { svelteHTML.createElement("div", { "class":`pdiff-body`,});
                for(let h of __sveltets_2_ensureArray(f.hunks)){let hi = 1;hi;
              if(hi > 0 || (h.at != null && h.at > 1)){
                 { svelteHTML.createElement("div", {   "class":`pdiff-gap mono`,"aria-hidden":`true`,}); h.at != null ? `line ${h.at}` : h.header; }
              }
                  for(let l of __sveltets_2_ensureArray(h.lines)){let li = 1;li;
                 { svelteHTML.createElement("div", { "class":`pdiff-row dline ${l.origin}`,});
                   { svelteHTML.createElement("span", { "class":`pdiff-no mono`,});l.old ?? ''; }
                   { svelteHTML.createElement("span", { "class":`pdiff-no mono`,});l.new ?? ''; }
                   { svelteHTML.createElement("span", {   "class":`pdiff-sign mono`,"aria-hidden":`true`,});l.origin === 'add' ? '+' : l.origin === 'del' ? '−' : ''; }
                   { svelteHTML.createElement("span", { "class":`pdiff-code mono hljs`,}); l.html; }
                 }
              }
            }
           }
        }
       }
    }
   }
}else{
   { svelteHTML.createElement("div", {     "class":`idiff diff-wrap`,"dir":`ltr`,...__sveltets_2_empty({"data-lines":lineCount}),});
     { svelteHTML.createElement("div", { "class":`idiff-scroll`,}); { svelteHTML.createElement("div", { "class":`idiff-inner`,});
          for(let r of __sveltets_2_ensureArray(shown)){let i = 1;i;
        if(r.kind === 'file'){
           { svelteHTML.createElement("div", { "class":`idiff-file mono`,});r.path; }
        } else if (r.kind === 'gap'){
           { svelteHTML.createElement("div", {   "class":`idiff-gap mono`,"aria-hidden":`true`,}); r.text; }
        }else{
           { svelteHTML.createElement("div", { "class":`idiff-row dline ${r.origin}`,});
             { svelteHTML.createElement("span", { "class":`idiff-no mono`,});(r.origin === 'del' ? r.old : r.new) ?? ''; }
             { svelteHTML.createElement("span", {   "class":`idiff-sign mono`,"aria-hidden":`true`,});r.origin === 'add' ? '+' : r.origin === 'del' ? '−' : ''; }
             { svelteHTML.createElement("span", { "class":`idiff-code mono hljs`,}); r.html; }
           }
        }
      }
     } }
    if(rows.length > maxLines){
       { svelteHTML.createElement("button", {     "class":`idiff-more`,"onclick":() => (all = !all),"aria-expanded":all,});
        all ? 'Show less' : `Show all ${lineCount} lines`;
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const InlineDiff__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type InlineDiff__SvelteComponent_ = ReturnType<typeof InlineDiff__SvelteComponent_>;
/*Ωignore_endΩ*/export default InlineDiff__SvelteComponent_;