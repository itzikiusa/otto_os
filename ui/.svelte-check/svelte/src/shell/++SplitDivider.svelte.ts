///<reference types="svelte" />
;// The side-by-side split's divider: a 1px hairline (NSSplitView's thin
// divider) with a wider invisible grab area. Drag it, or focus it and use
// ←/→ (⇧ for a big step), Home/End for the limits and Enter — or a
// double-click — for 50/50. While dragging, the side pane's iframe stops
// taking pointer events (sidePane.dragging) so it can't swallow the moves.

import { sidePane } from '../lib/stores/sidePane.svelte';
import { leadingFromPointer, nudgeLeading, SPLIT_MAX, SPLIT_MIN } from '../lib/sidePane';
import { rafCoalesce, showDragOverlay } from '../lib/dragCursor';
import { splitter } from '../lib/paneResizer';
;

  interface Props {
    /** Accessible name of the two panes, e.g. "Agents and Connections". */
    label: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { label }: Props = $props();

  let el: HTMLDivElement | undefined = $state();

  const isRtl = (): boolean => !!el && getComputedStyle(el).direction === 'rtl';

  function onPointerDown(e: PointerEvent): void {
    if (e.button !== 0) return;
    const split = el?.parentElement;
    if (!split) return;
    e.preventDefault();
    el?.setPointerCapture(e.pointerId);
    sidePane.dragging = true;
    const rtl = isRtl();
    // Overlay cursor (not body.style — an inherited write restyles the whole
    // app) and one split write per frame (lib/dragCursor.ts).
    const hideOverlay = showDragOverlay('col-resize');
    const moves = rafCoalesce((ev: PointerEvent): void => {
      const r = split.getBoundingClientRect();
      sidePane.setLeading(leadingFromPointer(ev.clientX, r.left, r.right, rtl), false);
    });
    const move = (ev: PointerEvent): void => moves.push(ev);
    const up = (ev: PointerEvent): void => {
      moves.flush();
      el?.releasePointerCapture(ev.pointerId);
      el?.removeEventListener('pointermove', move);
      el?.removeEventListener('pointerup', up);
      el?.removeEventListener('pointercancel', up);
      hideOverlay();
      sidePane.dragging = false;
      sidePane.setLeading(sidePane.leading, true);
    };
    el?.addEventListener('pointermove', move);
    el?.addEventListener('pointerup', up);
    el?.addEventListener('pointercancel', up);
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const next = nudgeLeading(sidePane.leading, e.key, { shift: e.shiftKey, rtl: isRtl() });
    if (next === null) return;
    e.preventDefault();
    if (e.key === 'Enter') sidePane.resetSplit();
    else sidePane.setLeading(next);
  }

  const pct = $derived(Math.round(sidePane.leading * 100));
;
async () => {


  {const $$action_0 = __sveltets_2_ensureAction(splitter(svelteHTML.mapElementTag('div'),({ onkeydown: onKeyDown, onpointerdown: onPointerDown, ondblclick: () => sidePane.resetSplit() })));{ const $$_div0 = svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                        "class":`split-divider`,"role":`separator`,"aria-orientation":`vertical`,"aria-label":`Resize ${label}`,"aria-valuemin":Math.round(SPLIT_MIN * 100),"aria-valuemax":Math.round(SPLIT_MAX * 100),"aria-valuenow":pct,"aria-valuetext":`${pct}% · ${100 - pct}%`,"title":`Drag to resize · double-click for 50/50`,...__sveltets_2_empty({"data-testid":`split-divider`}),});el = $$_div0;sidePane.dragging; }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SplitDivider__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SplitDivider__SvelteComponent_ = ReturnType<typeof SplitDivider__SvelteComponent_>;
/*Ωignore_endΩ*/export default SplitDivider__SvelteComponent_;