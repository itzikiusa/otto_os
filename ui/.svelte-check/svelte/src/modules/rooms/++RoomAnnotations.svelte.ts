///<reference types="svelte" />
;
import type { Snippet } from 'svelte';
import type { RoomAction, RoomSnapshot, RoomPresentation, RoomAnnotationTool, RoomPoint } from '../../lib/api/room-types';
import { normalizedPoint, nextExpiry } from './room-state';

;type $$ComponentProps =  {room: RoomSnapshot; source: RoomPresentation; send: (action: RoomAction) => boolean; children: Snippet};function $$render() {

  
  
  
  let { room, source, send, children }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let tool = $state<RoomAnnotationTool>('pointer'), drawing = $state(false), points = $state<RoomPoint[]>([]);
  let strokeEpoch = 0, grantEpoch = 0, now = $state(Date.now());
  const grant = $derived(room.annotation_grants?.find(g => g.source_id === source.id && g.member_id === room.member_id));
  const allowed = $derived(room.annotations_enabled !== false && grant?.allowed && !grant.blocked);
  const owner = $derived(source.member_id === room.member_id);
  const host = $derived(room.host_member_id === room.member_id);
  // Expiry is parsed once per mark; `now` advances only when the earliest live
  // mark expires (one timer, re-armed when marks change), never on a fixed tick.
  const sourceMarks = $derived((room.annotations ?? []).filter(a => a.source_id === source.id && a.source_generation === source.generation && a.clear_epoch === source.clear_epoch).map(mark => ({mark, expires: Date.parse(mark.expires_at)})));
  const marks = $derived(sourceMarks.filter(m => m.expires > now).map(m => m.mark));
  $effect(() => {
    const at = nextExpiry(sourceMarks.map(m => m.expires), now);
    if (at === null) return;
    const timer = setTimeout(() => now = Date.now(), Math.max(0, at - Date.now()) + 16);
    return () => clearTimeout(timer);
  });
  // Keyboard placement (Point / Highlight): a focusable surface with a visible cursor.
  let kb = $state<{x: number, y: number} | null>(null), anchor = $state<RoomPoint | null>(null), kbNote = $state('');
  const keyboardTool = $derived(tool !== 'pen');
  $effect(() => { if (!allowed) { drawing = false; points = []; kb = null; anchor = null; kbNote = ''; } });
  $effect(() => { void tool; anchor = null; kbNote = ''; });
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  function kbFocus() { if (allowed && !kb) kb = {x: 0.5, y: 0.5}; }
  function kbBlur() { kb = null; anchor = null; kbNote = ''; }
  function kbSend(pts: RoomPoint[]) {
    if (!allowed) return;
    send({type: 'annotation', source_id: source.id, source_generation: source.generation, clear_epoch: source.clear_epoch, grant_epoch: grant?.epoch ?? 0, tool, points: pts});
  }
  function kbKey(event: KeyboardEvent) {
    if (!allowed || !kb || event.altKey || event.ctrlKey || event.metaKey) return;
    const step = event.shiftKey ? 0.1 : 0.02;
    const d = event.key === 'ArrowLeft' ? [-step, 0] : event.key === 'ArrowRight' ? [step, 0] : event.key === 'ArrowUp' ? [0, -step] : event.key === 'ArrowDown' ? [0, step] : null;
    if (d) { event.preventDefault(); kb = {x: clamp(kb.x + d[0]), y: clamp(kb.y + d[1])}; return; }
    if (event.key === 'Enter') {
      event.preventDefault();
      if (tool === 'pointer') { kbSend([{...kb}]); kbNote = 'Pointer placed.'; }
      else if (!anchor) { anchor = {...kb}; kbNote = 'First corner set. Move to the opposite corner and press Enter.'; }
      else { kbSend([anchor, {...kb}]); anchor = null; kbNote = 'Highlight placed.'; }
    } else if (event.key === 'Escape' && anchor) { event.preventDefault(); event.stopPropagation(); anchor = null; kbNote = 'Placement canceled.'; }
  }
  function position(event: PointerEvent) { return normalizedPoint(event.clientX, event.clientY, event.currentTarget instanceof SVGElement ? event.currentTarget.getBoundingClientRect() : {left: 0, top: 0, width: 0, height: 0}, source.width, source.height); }
  function begin(event: PointerEvent) {
    if (!allowed || event.button !== 0) return;
    const p = position(event); if (!p) return;
    event.preventDefault(); (event.currentTarget as SVGElement).setPointerCapture(event.pointerId);
    strokeEpoch = source.clear_epoch; grantEpoch = grant?.epoch ?? 0; points = [p]; drawing = true;
  }
  function move(event: PointerEvent) {
    if (!drawing || !allowed || points.length >= 64 || tool === 'pointer') return;
    const p = position(event); if (p) points = tool === 'highlight' ? [points[0], p] : [...points, p];
  }
  function finish() {
    if (drawing && allowed && points.length && (tool !== 'highlight' || points.length === 2)) send({type: 'annotation', source_id: source.id, source_generation: source.generation, clear_epoch: strokeEpoch, grant_epoch: grantEpoch, tool, points});
    drawing = false; points = [];
  }
  function polyline(value: RoomPoint[]) { return value.map(p => `${p.x * source.width},${p.y * source.height}`).join(' '); }
;
async () => {
 { svelteHTML.createElement("div", { "class":`media-stage`,});
  ;__sveltets_2_ensureSnippet(children());
 { svelteHTML.createElement("svg", {                "class":`annotation-overlay`,"viewBox":`0 0 ${source.width} ${source.height}`,"aria-label":`Annotations on ${source.title}`,"role":`img`,"onpointerdown":begin,"onpointermove":move,"onpointerup":finish,"onpointercancel":() => { drawing = false; points = []; },});allowed;
     for(let mark of __sveltets_2_ensureArray(marks)){mark.id;
    if(mark.tool === 'pointer' && mark.points[0]){ { svelteHTML.createElement("circle", {     "cx":mark.points[0].x * source.width,"cy":mark.points[0].y * source.height,"r":Math.max(6, source.width / 100),}); { svelteHTML.createElement("title", {});room.members?.find(m => m.id === mark.member_id)?.name ?? 'Participant';  } }
    } else if (mark.tool === 'highlight' && mark.points.length === 2){ { svelteHTML.createElement("rect", {          "class":`highlight`,"x":Math.min(mark.points[0].x, mark.points[1].x) * source.width,"y":Math.min(mark.points[0].y, mark.points[1].y) * source.height,"width":Math.abs(mark.points[1].x - mark.points[0].x) * source.width,"height":Math.abs(mark.points[1].y - mark.points[0].y) * source.height,});}}else{ { svelteHTML.createElement("polyline", {  "points":polyline(mark.points),});}}
  }
  if(drawing){ { svelteHTML.createElement("polyline", {  "points":polyline(points),});}}
  if(kb && keyboardTool){
    if(anchor){ { svelteHTML.createElement("rect", {          "class":`highlight kb-rect`,"x":Math.min(anchor.x, kb.x) * source.width,"y":Math.min(anchor.y, kb.y) * source.height,"width":Math.abs(kb.x - anchor.x) * source.width,"height":Math.abs(kb.y - anchor.y) * source.height,});}}
     { svelteHTML.createElement("circle", {        "class":`kb-cursor`,"cx":kb.x * source.width,"cy":kb.y * source.height,"r":Math.max(8, source.width / 80),});}
  }
 }
if(allowed && keyboardTool){
  
   { svelteHTML.createElement("div", {               "class":`place-surface`,"tabindex":0,"role":`application`,"aria-label":tool === 'pointer' ? `Place a pointer on ${source.title}` : `Place a highlight on ${source.title}`,"aria-describedby":`place-help-${source.id}`,"onfocus":kbFocus,"onblur":kbBlur,"onkeydown":kbKey,}); }
   { svelteHTML.createElement("span", {   "class":`sr-only`,"id":`place-help-${source.id}`,});         tool === 'pointer' ? 'Enter places the pointer.' : 'Enter sets the first corner, then Enter again completes the highlight.';     }
   { svelteHTML.createElement("span", {     "class":`sr-only`,"role":`status`,"aria-live":`polite`,});kbNote; }
}
 }
 { svelteHTML.createElement("div", { "class":`annotation-tools`,});
  if(allowed){ { svelteHTML.createElement("div", {     "class":`tools`,"role":`group`,"aria-label":`Annotation tool`,});
      for(let value of __sveltets_2_ensureArray((['pointer', 'highlight', 'pen']))){ { svelteHTML.createElement("button", {     "class":`btn small`,"aria-pressed":tool === value,"onclick":() => tool = value as RoomAnnotationTool,});value === 'pointer' ? 'Point' : value === 'highlight' ? 'Highlight' : 'Draw'; }}
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => send({type: 'undo_annotation', source_id: source.id}),});   }
   }
  } else if (!grant?.blocked && room.annotations_enabled !== false){ { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":grant?.requested,"onclick":() => send({type: 'request_annotation', source_id: source.id}),});grant?.requested ? 'Annotation requested' : 'Request annotation'; }
  }else{ { svelteHTML.createElement("span", {}); grant?.blocked ? 'revoked by host' : 'disabled'; }}
  if(owner || host){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => send({type: 'clear_annotations', source_id: source.id}),});  }}
 }

if(owner || host){ { svelteHTML.createElement("div", { "class":`requests`,});   for(let permission of __sveltets_2_ensureArray(room.annotation_grants?.filter(g => g.source_id === source.id && g.member_id !== room.member_id) ?? [])){permission.member_id;
  const member = room.members?.find(m => m.id === permission.member_id);
  if(owner && permission.requested && !permission.blocked){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => send({type: 'grant_annotation', source_id: source.id, member_id: permission.member_id, allowed: true}),}); member?.name ?? 'participant';   }}
  if(permission.allowed){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => host ? send({type: 'revoke_annotation', source_id: source.id, member_id: permission.member_id, blocked: true}) : send({type: 'grant_annotation', source_id: source.id, member_id: permission.member_id, allowed: false}),}); member?.name ?? 'participant';  }}
} }}

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomAnnotations__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomAnnotations__SvelteComponent_ = ReturnType<typeof RoomAnnotations__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomAnnotations__SvelteComponent_;