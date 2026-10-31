///<reference types="svelte" />
;// JSON result view: one JSON object PER ROW (not one big array) so row
// boundaries are unmistakable — each row is its own bordered, numbered,
// copyable block. Same data the server returned; only the rendering differs.
// Disclosure follows the same expansion plan as the Vertical view (node
// budget + sticky per-path toggles + Expand/Collapse all), so what is open
// here is open there. ⌘F searches inside closed branches and undrawn
// records too (json-find.ts).

import { tick, untrack } from 'svelte';
import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import Icon from '../../lib/components/Icon.svelte';
import { registerFindProvider } from '../../lib/findProviders';
import JsonTree from '../../lib/components/JsonTree.svelte';
import type { QueryResult, DbEngine } from '../../lib/api/types';
import type { EditFlow } from './EditFlow.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import {
    CHUNK,
    estimateNodes,
    newExpansionState,
    planExpansion,
    setExpansionMode,
    type ExpansionState,
  } from '../../lib/jsonTree/expansion-plan';
import { ALT_BATCH, cellStr, copyWithToast, prettyJson } from './results-format';
import { LineIndex, revealSteps, type FindTop, type RevealState } from '../../lib/jsonTree/json-find';
;

  interface Props {
    result: QueryResult;
    /** Filtered/sorted rows as objects, capped; `idx` is the ORIGINAL liveRows index. */
    objRows: { obj: Record<string, unknown>; idx: number }[];
    /** Column names with duplicates disambiguated (`name (2)`, …). */
    uniqueColNames: string[];
    engine: DbEngine | null;
    flow: EditFlow;
    mini: boolean;
    /** Records still drawable below the current batch (excludes the hard-capped tail). */
    altRemaining: number;
    viewTruncated: boolean;
    viewCap: number;
    /** Total rows in the view (the "Showing first N of M rows" note). */
    totalRows: number;
    onshowmore: () => void;
    /** Shared nested-document expansion state (ResultsGrid's `$state`); a
     *  local one is used when absent so the view works standalone. */
    expansion?: ExpansionState;
    /** "Compare two records" request from a record header (mounted by ResultsGrid). */
    oncompare?: (left: number, right: number) => void;
    /** Every record the view may draw (≤ the view cap; `objRows` is its drawn
     *  head) so ⌘F finds matches past the batch. Falls back to `objRows`. */
    findRows?: Record<string, unknown>[];
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    result,
    objRows,
    uniqueColNames,
    engine,
    flow,
    mini,
    altRemaining,
    viewTruncated,
    viewCap,
    totalRows,
    onshowmore,
    expansion,
    oncompare,
    findRows,
  }: Props = $props();
  // `uniqueColNames` / `engine` are accepted for prop parity with VerticalView
  // (the JSON view renders `obj`, already keyed by those names) and unused.

  // ── Expansion plans (see VerticalView — same state, same planner) ───────────
  const localExpansion = $state(newExpansionState());
  const exp = $derived(expansion ?? localExpansion);
  const plans = $derived.by<Set<string>[]>(() => {
    void exp.version;
    return objRows.map((r) => planExpansion(r.obj, exp));
  });
  const resettable = $derived.by(() => {
    void exp.version;
    return exp.mode !== 'budget' || exp.overrides.size > 0;
  });
  const EXPAND_ALL_WARN = 20_000;
  function expandAll(): void {
    const total = objRows.reduce((n, r) => n + estimateNodes(r.obj, EXPAND_ALL_WARN), 0);
    if (total >= EXPAND_ALL_WARN) {
      toasts.info('Expanding everything', `${objRows.length} records with ~${total.toLocaleString()}+ fields — this can take a moment`);
    }
    setExpansionMode(exp, 'all');
  }
  /** Expand / Collapse / Reset also drop what a find revealed. */
  function setMode(mode: ExpansionState['mode']): void {
    reveals.clear();
    setExpansionMode(exp, mode);
  }

  // ── ⌘F over closed branches (json-find.ts) ──────────────────────────────────
  // Rows = every line of every findable record with everything open; built on
  // demand, cached per record list until the find bar closes. Revealing one
  // opens its path in THAT record only (draws more records first when it
  // sits past the batch).
  let viewEl = $state<HTMLElement | null>(null);
  // Keyed by the record's ORIGINAL row idx (not its draw position), so a sort
  // or filter leaves a reveal on the record it was made in.
  const reveals = new SvelteMap<number, RevealState>();
  $effect(() => {
    void result;
    untrack(() => reveals.clear());
  });
  const topsOf = (obj: Record<string, unknown>): FindTop[] => Object.entries(obj).map(([k, v]) => [k, k, v]);
  let findCache: { src: unknown; idx: LineIndex } | null = null;
  function findIndex(): { recs: Record<string, unknown>[]; idx: LineIndex } {
    const recs = findRows ?? objRows.map((r) => r.obj);
    const src = findRows ?? objRows;
    if (findCache?.src !== src) findCache = { src, idx: new LineIndex(recs.length, (i) => topsOf(recs[i]), 'json') };
    return { recs, idx: findCache.idx };
  }
  async function revealLine(i: number): Promise<void> {
    const { recs, idx } = findIndex();
    const at = idx.locate(i);
    if (!at) return;
    if (at.rec >= objRows.length) {
      for (let n = objRows.length; n <= at.rec; n += ALT_BATCH) onshowmore();
      await tick();
    }
    const key = objRows[at.rec]?.idx;
    if (key === undefined) return;
    let r = reveals.get(key);
    if (!r) {
      r = { opens: new SvelteSet(), shown: new SvelteMap(), strs: new SvelteSet() };
      reveals.set(key, r);
    }
    const steps = revealSteps(topsOf(recs[at.rec]), at.line, CHUNK, true);
    for (const p of steps.opens) r.opens.add(p);
    for (const [p, n] of steps.shown) if ((r.shown.get(p) ?? 0) < n) r.shown.set(p, n);
    if (at.line.long) r.strs.add(at.line.path);
    await tick();
  }
  function lineElement(i: number): Element | null {
    const at = findIndex().idx.locate(i);
    if (!at || !viewEl) return null;
    // A clipped string may hide the match: report it unmounted so it's revealed.
    if (at.line.long && !reveals.get(objRows[at.rec]?.idx ?? -1)?.strs.has(at.line.path)) return null;
    return viewEl.querySelector(`[data-jrec="${at.rec}"] [data-jpath="${CSS.escape(at.line.path)}"]`);
  }
  $effect(() =>
    registerFindProvider({
      root: () => viewEl,
      count: () => findIndex().idx.count(),
      text: (i) => findIndex().idx.text(i),
      lowered: true,
      release: () => (findCache = null),
      reveal: revealLine,
      rowElement: lineElement,
    }),
  );

  // ── Record menu (⋯): insert / export / compare — the same set as Vertical ──
  const canEdit = $derived(flow.editable && !flow.reviewSql);
  function recordId(obj: Record<string, unknown>, ri: number): string {
    const id = obj._id;
    if (id === undefined || id === null) return String(ri + 1);
    const s = cellStr(id);
    const m = s.match(/^ObjectId\("(.+)"\)$/);
    return (m ? m[1] : s).replace(/[^\w.-]+/g, '_').slice(0, 64);
  }
  // Local copy of ResultsGrid's `download()` (see VerticalView) — flagged for
  // the reviewer to fold into one helper.
  function download(text: string, name: string, mime: string): void {
    const blob = new Blob([text], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1500);
  }
  let comparePick = $state<number | null>(null);
  $effect(() => {
    void result;
    comparePick = null;
  });
  function recordMenu(e: MouseEvent, obj: Record<string, unknown>, idx: number, ri: number): void {
    const items: MenuItem[] = [];
    if (canEdit) {
      items.push(
        { label: 'Insert document…', icon: 'plus', action: () => flow.openDocEditor(-1) },
        { label: 'Replace document (JSON)…', icon: 'edit', action: () => flow.openDocEditor(idx) },
        { separator: true },
      );
    }
    items.push({ label: flow.rawDocLabel, icon: 'eye', action: () => flow.openRawDoc(idx) });
    items.push({
      label: 'Export…',
      icon: 'arrowDown',
      action: () =>
        download(prettyJson(obj), `${flow.editTable ?? flow.copyTarget?.table ?? 'record'}-${recordId(obj, ri)}.json`, 'application/json'),
    });
    if (oncompare) {
      items.push({
        label: comparePick === null ? 'Compare with…' : comparePick === idx ? 'Cancel compare' : 'Compare with the picked record',
        icon: 'split',
        action: () => {
          if (comparePick === idx) comparePick = null;
          else if (comparePick === null) {
            comparePick = idx;
            toasts.info('Compare', 'Pick a second record from its ⋯ menu');
          } else {
            const left = comparePick;
            comparePick = null;
            oncompare(left, idx);
          }
        },
      });
    }
    ctxMenu.show(e, items);
  }
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {  "class":`alt-view`,});viewEl = $$_div0;
  if(!mini){
     { svelteHTML.createElement("div", {  "class":`vv-tools`,...__sveltets_2_empty({"data-find-skip":true}),});
       { svelteHTML.createElement("button", {     "class":`vv-tool`,"onclick":expandAll,"title":`Open every nested field of the drawn records`,});  }
       { svelteHTML.createElement("button", {     "class":`vv-tool`,"onclick":() => setMode('none'),"title":`Close every nested field`,});  }
       { svelteHTML.createElement("button", {       "class":`vv-tool`,"onclick":() => setMode('budget'),"title":`Back to the default: open what fits the node budget`,"disabled":!resettable,});  }
     }
  }
  if(viewTruncated){ { svelteHTML.createElement("div", {  "class":`alt-note dim`,...__sveltets_2_empty({"data-find-skip":true}),});  viewCap;  totalRows;  }}
      for(let { obj, idx } of __sveltets_2_ensureArray(objRows)){let ri = 1;ri;
     { svelteHTML.createElement("div", {    "class":`jrec`,...__sveltets_2_empty({"data-jrec":ri}),});comparePick === idx;
       { svelteHTML.createElement("div", {  "class":`jrec-head mono`,...__sveltets_2_empty({"data-find-skip":true}),});
         { svelteHTML.createElement("span", { "class":`jrec-n`,}); ri + 1; }
        if(comparePick === idx){ { svelteHTML.createElement("span", { "class":`jrec-tag`,});  }}
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(canEdit){
           { svelteHTML.createElement("button", {       "class":`jrec-copy`,"title":`Edit this document (opens a review before running)`,"aria-label":`Edit document`,"onclick":() => flow.openDocEditor(idx),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} }
        }
         { svelteHTML.createElement("button", {       "class":`jrec-copy`,"title":`Copy this row as JSON`,"aria-label":`Copy row JSON`,"onclick":() => copyWithToast(prettyJson(obj)),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} }
        if(!mini){
           { svelteHTML.createElement("button", {       "class":`jrec-copy jrec-more`,"title":`Record actions`,"aria-label":`Record actions`,"onclick":(e) => recordMenu(e, obj, idx, ri),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
        }
       }
      
       { svelteHTML.createElement("div", { "class":`alt-json mono`,}); { const $$_eerTnosJ3C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ3C({ target: __sveltets_2_any(), props: {        "value":obj,"path":"","plan":plans[ri] ?? new Set<string>(),"expansion":exp,"reveal":reveals.get(idx),}});} }
     }
  }
  if(altRemaining > 0){
     { svelteHTML.createElement("button", {    "class":`alt-more`,"onclick":onshowmore,...__sveltets_2_empty({"data-find-skip":true}),});
       Math.min(ALT_BATCH, altRemaining);   altRemaining;  
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const JsonView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type JsonView__SvelteComponent_ = ReturnType<typeof JsonView__SvelteComponent_>;
/*Ωignore_endΩ*/export default JsonView__SvelteComponent_;