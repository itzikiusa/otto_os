///<reference types="svelte" />
;
  // Shared by the pane ⋯ menu (SessionView) and the Database pane's ✕
  // right-click (below) — one list, so the five rows can't drift apart.
  import { layout as layoutStore, type Preset as PresetOp } from '../../lib/stores/splitLayout.svelte';
  import type { MenuItem as PresetMenuItem } from '../../lib/contextmenu.svelte';

  /** The five layout presets as flat ctxMenu rows (the menu has no submenus);
   *  worded like their ⌘K commands in Splits.svelte ("Use grid layout"). */
  export function presetItems(): PresetMenuItem[] {
    const rows: [PresetOp, string][] = [
      ['cols', 'Use equal-columns layout'],
      ['rows', 'Use equal-rows layout'],
      ['one-two-below', 'Use one-above-two layout'],
      ['one-two-beside', 'Use one-beside-two layout'],
      ['grid', 'Use grid layout'],
    ];
    return rows.map(([p, label]) => ({ label, icon: 'split', action: () => layoutStore.applyPreset(p) }));
  }
;;

import { rowMenu } from '../../lib/rowMenu';
// One node of the split TREE (C2). A `split` lays its two children out with an
// 8px draggable gutter between them; a `leaf` renders the pane itself plus the
// C3a drop veil. The component mounts itself for a/b — the tree is arbitrarily
// deep, so the recursion lives here rather than in Splits.svelte.
//
// Tracks are `fr`, never `%`: `f% 8px (100−f)%` sums to 100 % + 8 px and would
// overflow 8 px per nesting level (the flat grid this replaced kept the gutter
// outside the tracks via `gap`, which a tree cannot do). Each is
// `minmax(0, Nfr)`: a bare `Nfr` has an `auto` minimum, so sizing the tracks
// measured the panes' content — with a live terminal inside, every output
// frame re-ran the grid's flex-fraction search (r3-12-01). A 0 minimum
// makes the split purely proportional; panes clip/scroll their own content.
import Self from './SplitNode.svelte';
import SessionView from './SessionView.svelte';
// The Database pane is rare; importing DatabasePage statically dragged
// CodeMirror + the DB views (~1.6 MB) into the Agents route — the default
// one — before first paint. Load it on demand from the page registry, which
// shares the chunk with the Database route (never fetched twice).
import { loadPage, loadedPage } from '../../shell/pages.svelte';
import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { splitter } from '../../lib/paneResizer';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import {
    layout,
    MIN_PANE_PX,
    type LayoutNode,
    type Side,
  } from '../../lib/stores/splitLayout.svelte';
;

  interface Props {
    node: LayoutNode;
    /** Nesting depth — informational (styling hook), the tree recurses on `node`. */
    depth?: number;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { node, depth = 0 }: Props = $props();

  /** Bumped by the Database pane's Retry: re-keys the `{#await}` so a fresh
   *  `loadPage('database')` runs (a failed load is forgotten there). */
  let dbLoad = $state(0);

  // ── split: gutter drag + keyboard resize ─────────────────────────────────
  let el = $state<HTMLDivElement | null>(null);

  /** Pointer drag on THIS node's gutter: the fraction is measured against the
   *  node's own rect, then guarded so neither side drops below MIN_PANE_PX. The
   *  guard is skipped when the node itself is narrower than two minimum panes —
   *  it cannot satisfy it, and clamping there would pin the gutter mid-drag. */
  function startDrag(e: PointerEvent): void {
    if (node.kind !== 'split' || !el) return;
    e.preventDefault();
    const rect = el.getBoundingClientRect();
    const { key, axis } = node;
    const rtl = getComputedStyle(el).direction === 'rtl';
    const move = (ev: PointerEvent): void => {
      const size = axis === 'col' ? rect.width : rect.height;
      if (size <= 0) return;
      const along =
        axis === 'col' ? (rtl ? rect.right - ev.clientX : ev.clientX - rect.left) : ev.clientY - rect.top;
      let frac = along / size;
      if (size >= 2 * MIN_PANE_PX) {
        frac = Math.min(1 - MIN_PANE_PX / size, Math.max(MIN_PANE_PX / size, frac));
      }
      layout.setFrac(key, frac);
    };
    const up = (): void => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  /** Keyboard resize (a11y — pointer-only otherwise): arrows nudge 2 %, Home/End
   *  jump to the 10/90 bounds clampFrac allows. */
  function gutterKeydown(e: KeyboardEvent): void {
    if (node.kind !== 'split') return;
    const horizontal = node.axis === 'col';
    const rtl = el ? getComputedStyle(el).direction === 'rtl' : false;
    const dec = horizontal ? (rtl ? e.key === 'ArrowRight' : e.key === 'ArrowLeft') : e.key === 'ArrowUp';
    const inc = horizontal ? (rtl ? e.key === 'ArrowLeft' : e.key === 'ArrowRight') : e.key === 'ArrowDown';
    let next: number | null = null;
    if (dec) next = node.frac - 0.02;
    else if (inc) next = node.frac + 0.02;
    else if (e.key === 'Home') next = 0.1;
    else if (e.key === 'End') next = 0.9;
    if (next === null) return;
    e.preventDefault();
    layout.setFrac(node.key, next);
  }

  // ── leaf: index-based workspace calls + the C3a drop veil ─────────────────
  const idx = $derived(node.kind === 'leaf' ? layout.leaves.findIndex((l) => l.key === node.key) : -1);
  const paneCount = $derived(ws.panes.length);

  /** Drop zone under the cursor: the outer quarter on each side moves the pane
   *  there, the middle swaps the two sessions. */
  let zone = $state<Side | 'centre' | null>(null);
  const ZONE_LABEL: Record<Side | 'centre', string> = {
    centre: 'Swap',
    left: 'Move left',
    right: 'Move right',
    up: 'Move up',
    down: 'Move down',
  };

  function zoneAt(e: DragEvent): Side | 'centre' {
    const target = e.currentTarget as HTMLElement;
    const r = target.getBoundingClientRect();
    const rtl = getComputedStyle(target).direction === 'rtl';
    const rawPx = r.width > 0 ? (e.clientX - r.left) / r.width : 0.5;
    const px = rtl ? 1 - rawPx : rawPx;
    const py = r.height > 0 ? (e.clientY - r.top) / r.height : 0.5;
    const edge = Math.min(px, 1 - px, py, 1 - py);
    if (edge > 0.25) return 'centre';
    if (edge === px) return 'left';
    if (edge === 1 - px) return 'right';
    if (edge === py) return 'up';
    return 'down';
  }

  function onVeilOver(e: DragEvent): void {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    zone = zoneAt(e);
  }

  function onVeilDrop(e: DragEvent): void {
    e.preventDefault();
    const from = layout.dragKey;
    const z = zone;
    zone = null;
    layout.dragKey = null;
    if (!from || node.kind !== 'leaf' || from === node.key || !z) return;
    if (z === 'centre') layout.swap(from, node.key);
    else layout.move(from, node.key, z);
    // `activeSessionId` follows the focused leaf and only focusPane routes to
    // navigateToSession — keep the URL + navigator highlight in step.
    ws.focusPane(layout.focusedIndex);
  }

;
async () => {



if(node.kind === 'split'){
   { const $$_div0 = svelteHTML.createElement("div", {            "class":`split-node`,...__sveltets_2_empty({"data-axis":node.axis}),...__sveltets_2_empty({"data-key":node.key}),...__sveltets_2_empty({"data-depth":depth}),"style":node.axis === 'col'
      ? `grid-template-columns: minmax(0, ${node.frac}fr) 8px minmax(0, ${1 - node.frac}fr); grid-template-rows: minmax(0, 1fr);`
      : `grid-template-rows: minmax(0, ${node.frac}fr) 8px minmax(0, ${1 - node.frac}fr); grid-template-columns: minmax(0, 1fr);`,});el = $$_div0;
     { const $$_fleS1C = __sveltets_2_ensureComponent(Self); new $$_fleS1C({ target: __sveltets_2_any(), props: {    "node":node.a,"depth":depth + 1,}});}
    
     {const $$action_0 = __sveltets_2_ensureAction(splitter(svelteHTML.mapElementTag('div'),({ onkeydown: gutterKeydown, onpointerdown: startDrag, ondblclick: () => node.kind === 'split' && layout.setFrac(node.key, 0.5) })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                   "class":`gutter`,"role":`separator`,"aria-orientation":node.axis === 'col' ? 'vertical' : 'horizontal',"aria-label":`Resize split (arrow keys; double-click resets)`,"title":`Drag to resize · double-click to split evenly`,"aria-valuenow":Math.round(node.frac * 100),"aria-valuemin":10,"aria-valuemax":90,}); }}
     { const $$_fleS1C = __sveltets_2_ensureComponent(Self); new $$_fleS1C({ target: __sveltets_2_any(), props: {    "node":node.b,"depth":depth + 1,}});}
   }
}else{
   { svelteHTML.createElement("div", {     "class":`leaf`,...__sveltets_2_empty({"data-pane-key":node.key}),...__sveltets_2_empty({"data-session":node.session}),});
    if(node.session === DB_PANE_ID){
       { svelteHTML.createElement("div", {          "class":`db-pane`,"role":`group`,"aria-label":`Database`,"onpointerdown":() => ws.focusPane(idx),});ws.focusedPane === idx && paneCount > 1;
        if(paneCount > 1){
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {            "class":`db-pane-close`,"title":`Close pane — right-click for layout presets`,"aria-label":`Close pane`,"onclick":() => ws.closePane(idx),"oncontextmenu":(e) => ctxMenu.show(e, presetItems()),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }}
        }
        if(loadedPage('database')){
          const DatabasePage = loadedPage('database')!;
           { const $$_egaPesabataD2C = __sveltets_2_ensureComponent(DatabasePage); new $$_egaPesabataD2C({ target: __sveltets_2_any(), props: {}});}
        }else{
          
          dbLoad; {
               { 
               { svelteHTML.createElement("div", { "class":`db-pane-loading`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":3,"label":`the Database pane`,}});} }
            try { await (loadPage('database'));} catch($$_e) { 
               { svelteHTML.createElement("div", { "class":`db-pane-loading`,});
                 { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`the Database pane`,"variant":`panel`,"empty":true,"error":`Otto may have been updated in the background. Retry, or reload Otto to pick up the new version.`,"onretry":() => dbLoad++,errorActions:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => location.reload(),});  }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
                  
                 LoadState}
               }
            }}
          }
        }
       }
    }else{
       { const $$_weiVnoisseS1C = __sveltets_2_ensureComponent(SessionView); new $$_weiVnoisseS1C({ target: __sveltets_2_any(), props: {                   "sessionId":node.session,"focused":ws.focusedPane === idx && paneCount > 1,"showClose":paneCount > 1,"showGrip":paneCount > 1,"dragKey":node.key,"ondragpane":(phase) => (layout.dragKey = phase === 'start' ? node.key : null),"onfocus":() => ws.focusPane(idx),"closeTitle":ws.closeTabTitle(node.session, 'session'),"onclosepane":() => void ws.requestCloseTab(node.session),}});}
    }
    
     { svelteHTML.createElement("div", {                "role":`presentation`,"class":`drop-veil`,...__sveltets_2_empty({"data-zone":zone ?? ''}),"hidden":!layout.dragKey || layout.dragKey === node.key,"ondragover":onVeilOver,"ondragleave":() => (zone = null),"ondrop":onVeilDrop,});zone !== null;
      if(zone){
         { svelteHTML.createElement("span", {   "class":`drop-ind`,...__sveltets_2_empty({"data-ind":zone}),}); }
         { svelteHTML.createElement("span", { "class":`drop-cap`,});ZONE_LABEL[zone]; }
      }
     }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SplitNode__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SplitNode__SvelteComponent_ = ReturnType<typeof SplitNode__SvelteComponent_>;
/*Ωignore_endΩ*/export default SplitNode__SvelteComponent_;