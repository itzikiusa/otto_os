///<reference types="svelte" />
;
  // Box bodies load on demand, one chunk per kind: the Home chunk no longer
  // carries every kind's dependencies (the DB box alone reaches ResultsGrid →
  // CodeMirror). One promise per kind, shared by every tile on every space;
  // `bodies` is reactive so a tile renders the moment its kind resolves and a
  // second tile of a loaded kind renders synchronously (no skeleton flash).
  import type { Component } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  // Bodies take different subsets of { box, viewId, zoomed, zoomedAway, tick, active }.
  type BoxBody = Component<any>;
  const BODY_LOADERS: Record<string, () => Promise<{ default: BoxBody }>> = {
    sessions: () => import('./boxes/SessionsBox.svelte'),
    'mission-control': () => import('./boxes/MissionControlBox.svelte'),
    'db-dashboard': () => import('./boxes/DbDashboardBox.svelte'),
    k8s: () => import('./boxes/K8sBox.svelte'),
    insights: () => import('./boxes/InsightsBox.svelte'),
    usage: () => import('./boxes/UsageBox.svelte'),
    classrooms: () => import('./boxes/ClassroomsBox.svelte'),
  };
  const bodies = new SvelteMap<string, BoxBody>();
  const inflight = new Map<string, Promise<void>>();
  function loadBody(kind: string): Promise<void> {
    const load = BODY_LOADERS[kind];
    if (!load) return Promise.resolve();
    let p = inflight.get(kind);
    if (!p) {
      p = load().then(
        (m) => void bodies.set(kind, m.default),
        (e: unknown) => {
          inflight.delete(kind); // let Retry re-import
          throw e;
        },
      );
      inflight.set(kind, p);
    }
    return p;
  }
;;

import { rowMenu } from '../../lib/rowMenu';
// One widget on the Home desktop: a quiet header (kind icon + title; the
// refresh / zoom / menu controls surface on hover or focus), the kind’s live
// body, and — in the grid — a drag grip for reordering plus a corner handle
// for resizing in grid units. The same component renders the zoomed
// (full-page) widget, minus the grid affordances. Widgets are opaque content
// cards resting on the ambient backdrop (elevation 1, --shadow-card).
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { router } from '../../lib/router.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { home, COLS, GAP_PX, MIN_H, MAX_H, MIN_W, ROW_PX, type HomeBox } from './home.svelte';
import { kindDef } from '../../lib/home/kinds';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
;

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed?: boolean;
    /** Index within the view (for the move-left/right menu). */
    index?: number;
    count?: number;
    /** Drag-reorder plumbing (grid only). */
    ondragbox?: (id: string) => void;
    ondropon?: (id: string) => void;
    /** False while the box’s space is not the one on screen (every space
     *  stays mounted): the body pauses its poller and keeps its data. */
    active?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { box, viewId, zoomed = false, index = 0, count = 1, ondragbox, ondropon, active = true }: Props = $props();

  const def = $derived(kindDef(box.kind));
  const Body = $derived(bodies.get(box.kind) ?? null);
  // The grid copy of the zoomed box (it stays mounted, hidden, under the zoom):
  // a body with a heavy scene (Classrooms' WebGL school) releases it, so only
  // the zoomed copy holds a live graphics context.
  const zoomedAway = $derived(!zoomed && home.zoomedId === box.id);
  const hasLoader = $derived(box.kind in BODY_LOADERS);
  let loadFailed = $state<string | null>(null);
  let loadAttempt = $state(0);
  $effect(() => {
    const kind = box.kind;
    void loadAttempt;
    if (bodies.has(kind)) return;
    let live = true;
    loadFailed = null;
    loadBody(kind).catch((e) => {
      if (live) loadFailed = loadErrorText(e);
    });
    return () => {
      live = false;
    };
  });
  function retryLoad(): void {
    loadAttempt += 1;
  }
  // Bumped by the header’s refresh button; each box body re-fetches on change.
  let tick = $state(0);

  // ── Resize (grid units) ──────────────────────────────────────────────────
  // Pointer-capture drag on the corner handle. The column width is measured
  // from the tile’s own box (cols = current span) so no grid metrics need to
  // be threaded down; rows are the fixed ROW_PX unit.
  let el = $state<HTMLElement | null>(null);
  let resizing = $state(false);
  function onResizeStart(e: PointerEvent): void {
    if (!el || viewport.isPhone) return;
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const rect = el.getBoundingClientRect();
    const colPx = (rect.width - GAP_PX * (box.w - 1)) / box.w + GAP_PX;
    const rowPx = ROW_PX + GAP_PX;
    const startX = e.clientX;
    const startY = e.clientY;
    const w0 = box.w;
    const h0 = box.h;
    const dir = getComputedStyle(el).direction === 'rtl' ? -1 : 1;
    resizing = true;
    home.interacting = true;
    const move = (ev: PointerEvent): void => {
      const w = Math.min(COLS, Math.max(MIN_W, Math.round(w0 + ((ev.clientX - startX) * dir) / colPx)));
      const h = Math.min(MAX_H, Math.max(MIN_H, Math.round(h0 + (ev.clientY - startY) / rowPx)));
      home.resizeBox(viewId, box.id, w, h);
    };
    const up = (): void => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      target.removeEventListener('pointercancel', up);
      resizing = false;
      home.interacting = false;
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  }

  // Keyboard resize on the handle: arrows nudge one grid unit.
  function onResizeKey(e: KeyboardEvent): void {
    const dir = el && getComputedStyle(el).direction === 'rtl' ? -1 : 1;
    const d: Record<string, [number, number]> = { ArrowRight: [dir, 0], ArrowLeft: [-dir, 0], ArrowDown: [0, 1], ArrowUp: [0, -1] };
    const v = d[e.key];
    if (!v) return;
    e.preventDefault();
    home.resizeBox(viewId, box.id, box.w + v[0], box.h + v[1]);
  }

  // ── Drag reorder (HTML5 DnD on the header handle) ───────────────────────
  let dragOver = $state(false);
  function onDragStart(e: DragEvent): void {
    e.dataTransfer?.setData('text/plain', box.id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
    home.interacting = true;
    ondragbox?.(box.id);
  }
  function onDragEnd(): void {
    home.interacting = false;
  }
  function onDragOver(e: DragEvent): void {
    if (!ondropon) return;
    e.preventDefault();
    dragOver = true;
  }
  function onDrop(e: DragEvent): void {
    e.preventDefault();
    dragOver = false;
    ondropon?.(box.id);
  }

  function menu(e: MouseEvent | KeyboardEvent): void {
    const others = home.views.filter((v) => v.id !== viewId);
    ctxMenu.show(e, [
      { label: 'Refresh', icon: 'refresh', action: () => (tick += 1) },
      { label: zoomed ? 'Exit zoom' : 'Zoom in', icon: zoomed ? 'minimize' : 'maximize', action: () => home.toggleZoom(box.id) },
      { label: `Open ${def.label}`, icon: 'external', action: () => router.go(def.route) },
      { separator: true },
      ...(zoomed
        ? []
        : [
            { label: 'Move left', icon: 'chevronLeft', disabled: index <= 0, action: () => home.moveBox(viewId, box.id, index - 1) },
            { label: 'Move right', icon: 'chevronRight', disabled: index >= count - 1, action: () => home.moveBox(viewId, box.id, index + 1) },
            { label: 'Fill width', icon: 'maximize', disabled: box.w === COLS, action: () => home.resizeBox(viewId, box.id, COLS, box.h) },
            { label: 'Reset size', icon: 'square', action: () => home.resizeBox(viewId, box.id, def.w, def.h) },
          ]),
      ...others.map((v) => ({
        label: `Move to “${v.name}”`,
        icon: 'grid',
        action: () => {
          home.moveBoxToView(viewId, box.id, v.id);
        },
      })),
      { separator: true },
      { label: 'Remove widget', icon: 'trash', danger: true, action: () => home.removeBox(viewId, box.id) }, // ui-guards: allow — instant, no confirm (the widget re-adds from Add widget)
    ]);
  }
;
async () => {




  {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('section')));{ const $$_section0 = svelteHTML.createElement("section", __sveltets_2_union($$action_0), {                       "class":`hbox`,...__sveltets_2_empty({"data-kind":box.kind}),...__sveltets_2_empty({"data-box-id":box.id}),"aria-label":def.label,"oncontextmenu":menu,"ondragover":onDragOver,"ondragleave":() => (dragOver = false),"ondrop":onDrop,});zoomed;resizing;dragOver;el = $$_section0;__sveltets_2_ensureType(String, Number, zoomed || viewport.isPhone ? null : `span ${box.w}`);__sveltets_2_ensureType(String, Number, zoomed ? null : `span ${box.h}`);
   { svelteHTML.createElement("header", {   "class":`hb-head`,"ondblclick":() => home.toggleZoom(box.id),});
    if(!zoomed && !viewport.isPhone){
       { svelteHTML.createElement("span", {           "class":`grip`,"draggable":`true`,"ondragstart":onDragStart,"ondragend":onDragEnd,"title":`Drag to reorder`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`grip`,"size":12,}});} }
    }
     { svelteHTML.createElement("span", { "class":`hb-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":def.icon,"size":13,}});} }
     { svelteHTML.createElement("span", { "class":`hb-title ellipsis`,});def.label; }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (tick += 1),"title":`Refresh ${def.label}`,"aria-label":`Refresh ${def.label}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => home.toggleZoom(box.id),"title":zoomed ? 'Exit zoom' : 'Zoom in',"aria-label":zoomed ? 'Exit zoom' : 'Zoom in',"aria-keyshortcuts":zoomed ? 'Escape' : undefined,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":zoomed ? 'minimize' : 'maximize',"size":12,}});}
     }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":menu,"title":`${def.label} widget options`,"aria-label":`${def.label} widget options`,"aria-haspopup":`menu`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
   }
   { svelteHTML.createElement("div", { "class":`hb-body`,});
    if(Body){
       { const $$_ydoB2C = __sveltets_2_ensureComponent(Body); new $$_ydoB2C({ target: __sveltets_2_any(), props: {      box,viewId,zoomed,zoomedAway,tick,active,}});}
    } else if (loadFailed){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`this widget`,"variant":`compact`,"error":loadFailed,"empty":true,"onretry":retryLoad,}});}
    } else if (hasLoader){
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {  "rows":3,}});}
    }
   }
  if(!zoomed && !viewport.isPhone){
     { svelteHTML.createElement("div", {                       "class":`resize`,"role":`slider`,"tabindex":0,"aria-label":`Resize ${def.label} (${box.w} × ${box.h})`,"aria-valuetext":`${box.w} columns by ${box.h} rows`,"aria-valuenow":box.w,"aria-valuemin":MIN_W,"aria-valuemax":COLS,"title":`Drag to resize · arrows to nudge`,"onpointerdown":onResizeStart,"onkeydown":onResizeKey,}); }
  }
 }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HomeBox__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HomeBox__SvelteComponent_ = ReturnType<typeof HomeBox__SvelteComponent_>;
/*Ωignore_endΩ*/export default HomeBox__SvelteComponent_;