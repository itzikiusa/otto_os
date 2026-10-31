///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
// k9s-style resource table: sticky header row + `VirtualList` body sharing
// one CSS grid template, health-colored status pill, keyboard-navigable rows
// (listbox/option semantics — the whole table is one selection widget).
import { untrack } from 'svelte';
import VirtualList from '../../lib/components/VirtualList.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import type { K8sResourceKind, K8sRow } from '../../lib/api/types';
import { columnsFor, gridTemplate } from './columns';
import { healthTone, kindDef, kubectlErrorSummary } from './k8s-util';
;

  interface Props {
    kind: K8sResourceKind;
    rows: K8sRow[];
    /** Rows before the text filter (for the "n of m" footer). */
    total: number;
    hasMetrics: boolean;
    allNamespaces: boolean;
    loading: boolean;
    error: string;
    selected: { ns: string; name: string } | null;
    onselect: (row: K8sRow) => void;
    onopen: (row: K8sRow) => void;
    onmenu: (e: MouseEvent | KeyboardEvent, row: K8sRow) => void;
    onretry: () => void;
    /** The text filter hiding rows, named in the filtered-empty state. */
    filter?: string;
    /** Filtered-empty "Clear filter" action (UX-5-16). */
    onclearfilter?: () => void;
    /** K-1: the view these rows belong to — the remembered scroll offset is
     *  restored once per key when its rows first render. */
    scrollKey?: string;
    initialScroll?: (key: string) => number;
    onscrolled?: (key: string, top: number) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { kind, rows, total, hasMetrics, allNamespaces, loading, error, selected, onselect, onopen, onmenu, onretry, filter = '', onclearfilter, scrollKey = '', initialScroll, onscrolled }: Props = $props();

  let bodyEl = $state<HTMLDivElement | null>(null);
  let restoredFor: string | null = null;
  $effect(() => {
    const key = scrollKey;
    const el = bodyEl;
    const n = rows.length;
    if (!el || !n || restoredFor === key) return;
    restoredFor = key;
    const top = untrack(() => initialScroll?.(key) ?? 0);
    const list = el.querySelector<HTMLElement>('.vlist');
    if (list && top > 0) list.scrollTop = top;
  });
  let scrollTimer: ReturnType<typeof setTimeout> | null = null;
  function onBodyScroll(e: Event): void {
    const t = e.target as HTMLElement | null;
    if (!t?.classList.contains('vlist') || !onscrolled) return;
    const key = scrollKey;
    const top = t.scrollTop;
    if (scrollTimer) clearTimeout(scrollTimer);
    scrollTimer = setTimeout(() => onscrolled(key, top), 200);
  }
  const errSummary = $derived(kubectlErrorSummary(error));

  const ROW_H = 30;
  const cols = $derived(columnsFor(kind, rows, hasMetrics, allNamespaces));
  const template = $derived(gridTemplate(cols));
  const isSel = (r: K8sRow): boolean => !!selected && selected.name === r.name && selected.ns === r.namespace;

  function rowKey(e: KeyboardEvent, r: K8sRow, i: number): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      onopen(r);
    } else if (e.key === ' ') {
      e.preventDefault();
      onselect(r);
    } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      onmenu(e, r);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const j = e.key === 'ArrowDown' ? i + 1 : i - 1;
      const next = rows[j];
      if (!next) return;
      onselect(next);
      // Focus the neighbouring row when it's rendered (VirtualList keeps a
      // window around the viewport, so it usually is).
      (e.currentTarget as HTMLElement).parentElement
        ?.querySelector<HTMLElement>(`[data-i="${j}"]`)
        ?.focus();
    }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  // Only the stale-banner case (rows still shown); the empty-table error is LoadState's.
  const staleError = $derived(rows.length && error ? error : '');
  $effect(() => { if (staleError) announce(`Couldn’t refresh — showing the last good load. ${staleError}`, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", {   "class":`rt`,...__sveltets_2_empty({"data-testid":`k8s-resource-table`}),});
  
   { svelteHTML.createElement("div", {     "class":`rt-head`,"aria-hidden":`true`,"style":`grid-template-columns:${template}`,});
       for(let c of __sveltets_2_ensureArray(cols)){c.key;
       { svelteHTML.createElement("div", {  "class":`rt-hcell`,});c.num;c.label; }
    }
   }

  if(error && !rows.length){
     { svelteHTML.createElement("div", {   "class":`rt-state`,...__sveltets_2_empty({"data-testid":`k8s-table-error`}),});
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {      "what":kindDef(kind).label.toLowerCase(),"error":errSummary,"empty":true,onretry,}});}
      if(errSummary !== error.trim()){
         { svelteHTML.createElement("details", { "class":`rt-raw`,});
           { svelteHTML.createElement("summary", {});   }
           { svelteHTML.createElement("pre", {});error; }
         }
      }
     }
  } else if (loading && !rows.length){
     { svelteHTML.createElement("div", { "class":`rt-state`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":8,"height":26,}});} }
  } else if (!rows.length){
     { svelteHTML.createElement("div", { "class":`rt-state`,});
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {            "icon":`box`,"title":total ? (filter ? `No ${kindDef(kind).label.toLowerCase()} match “${filter}”` : 'No rows match the filter') : `No ${kindDef(kind).label.toLowerCase()} here`,"body":total ? `${total} hidden by the filter.` : 'Try another namespace, or "All namespaces".',"actionLabel":total && onclearfilter ? 'Clear filter' : undefined,"actionKind":`secondary`,"onaction":total ? onclearfilter : undefined,}});}
     }
  }else{
     { const $$_div1 = svelteHTML.createElement("div", {          "class":`rt-body`,"onscrollcapture":onBodyScroll,"role":`listbox`,"aria-label":`${kindDef(kind).label} rows`,"aria-multiselectable":`false`,});bodyEl = $$_div1;
       { const $$_tsiLlautriV2C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV2 = new $$_tsiLlautriV2C({ target: __sveltets_2_any(), props: {     "items":rows,"estimateHeight":ROW_H,"class":`rt-vlist`,row:(r, i) => { async ()/*Ωignore_positionΩ*/ => {
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                         "class":`rt-row`,"role":`option`,"aria-selected":isSel(r),"tabindex":0,...__sveltets_2_empty({"data-i":i}),...__sveltets_2_empty({"data-testid":`k8s-row`}),"style":`grid-template-columns:${template};height:${ROW_H}px`,"onclick":() => onselect(r),"ondblclick":() => onopen(r),"onkeydown":(e) => rowKey(e, r, i),"oncontextmenu":(e) => onmenu(e, r),});isSel(r);
               for(let c of __sveltets_2_ensureArray(cols)){c.key;
              const v = c.value(r);
               { svelteHTML.createElement("div", {     "class":`rt-cell`,"title":v,});c.num;c.mono;
                if(c.status){
                  const tone = healthTone(r.health, r.status);
                   { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },tone,"dot":true,"live":tone === 'info',}}); { svelteHTML.createElement("span", { "class":`st-txt`,});v; } Badge}
                }else{
                  v;
                }
               }
            }
           }}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV2.$$prop_def;/*Ωignore_endΩ*/
        
       VirtualList}
     }
    if(error){
       { svelteHTML.createElement("div", { "class":`rt-stale`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}         error; }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ResourceTable__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ResourceTable__SvelteComponent_ = ReturnType<typeof ResourceTable__SvelteComponent_>;
/*Ωignore_endΩ*/export default ResourceTable__SvelteComponent_;