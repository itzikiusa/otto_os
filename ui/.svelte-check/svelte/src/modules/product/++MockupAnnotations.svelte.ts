///<reference types="svelte" />
;// MockupAnnotations — an absolutely-positioned overlay over the mockup render
// box plus a side list of notes. Two modes:
//   • Annotate: overlay has pointer-events:auto and captures clicks; a click
//     drops a pin at relative (x_pct, y_pct) = (offsetX/clientWidth,
//     offsetY/clientHeight), clamped to 0..1, opening an inline editor.
//   • Interact: overlay has pointer-events:none so the iframe beneath receives
//     input (relevant when HTML interactivity is enabled).
// Coordinates are relative, so pins survive resize. Pins render at
//   left:{x_pct*100}% top:{y_pct*100}%.

import { tick } from 'svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import LoadState from '../../lib/components/LoadState.svelte';
import { product } from '../../lib/stores/product.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import Icon from '../../lib/components/Icon.svelte';
import type { MockupAnnotation } from './types';
;

  interface Props {
    attachmentId: string;
    // The viewer's render-box element — the overlay is rendered as its child and
    // covers it exactly (so percentage coordinates map 1:1).
    box: HTMLElement;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const { attachmentId, box }: Props = $props();

  // ── State ─────────────────────────────────────────────────────────────────
  let notes = $state<MockupAnnotation[]>([]);
  let loading = $state(false);
  /** Why the last annotations load failed — shown inline with Retry, never as
   *  "No annotations yet" (which invited re-adding pins that already exist). */
  let loadError = $state<string | null>(null);
  /** Bumped per load: a response for a previous attachment is dropped. */
  let loadSeq = 0;
  let mode = $state<'annotate' | 'interact'>('annotate');

  // Inline editor for a pending (un-saved) pin.
  let pending = $state<{ x_pct: number; y_pct: number } | null>(null);
  let pendingBody = $state('');
  let saving = $state(false);

  // Geometry of the render box, tracked so the absolute overlay stays aligned
  // and percentage-positioned pins survive resize. Relative to .render-wrap
  // (the box's offset parent).
  let geom = $state({ left: 0, top: 0, width: 0, height: 0 });

  function measure(): void {
    if (!box) return;
    geom = {
      left: box.offsetLeft,
      top: box.offsetTop,
      width: box.offsetWidth,
      height: box.offsetHeight,
    };
  }

  // ── Load on mount / attachment change ───────────────────────────────────────
  $effect(() => {
    // Re-run when the attachment id changes.
    attachmentId;
    void loadNotes();
  });

  // Keep the overlay aligned to the box (initial + on resize).
  $effect(() => {
    if (!box) return;
    measure();
    const ro = new ResizeObserver(() => measure());
    ro.observe(box);
    window.addEventListener('resize', measure);
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', measure);
    };
  });

  async function loadNotes(): Promise<void> {
    const seq = ++loadSeq;
    loading = true;
    loadError = null;
    try {
      const next = await product.listAnnotations(attachmentId);
      if (seq === loadSeq) notes = next;
    } catch (e) {
      if (seq === loadSeq) loadError = loadErrorText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  /** Svelte action: focus the node when it mounts (replaces the `autofocus`
   *  attribute, which the a11y linter flags). */
  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  const clamp = (v: number): number => Math.max(0, Math.min(1, v));

  /** Annotate-mode click on the overlay → open the inline editor at the spot. */
  function onOverlayClick(e: MouseEvent): void {
    if (mode !== 'annotate') return;
    const el = e.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const w = el.clientWidth || rect.width || 1;
    const h = el.clientHeight || rect.height || 1;
    const x = clamp((e.clientX - rect.left) / w);
    const y = clamp((e.clientY - rect.top) / h);
    pending = { x_pct: x, y_pct: y };
    pendingBody = '';
    viaButton = false;
  }

  async function savePending(): Promise<void> {
    if (!pending || saving) return;
    const body = pendingBody.trim();
    if (!body) {
      pending = null;
      return;
    }
    saving = true;
    try {
      const created = await product.addAnnotation(attachmentId, {
        x_pct: pending.x_pct,
        y_pct: pending.y_pct,
        body,
      });
      notes = [...notes, created];
      pending = null;
      pendingBody = '';
      if (viaButton) {
        viaButton = false;
        void tick().then(() => addBtn?.focus());
      }
    } catch (e) {
      toastError('Couldn’t add note', e);
    } finally {
      saving = false;
    }
  }

  function cancelPending(): void {
    pending = null;
    pendingBody = '';
  }

  // ── Keyboard path: "Add annotation" + arrow-key pin placement ───────────────
  // A pin can be started without a mouse: the button drops the pending pin at the
  // centre of what is currently VISIBLE of the render box, the arrow keys on the
  // pin nudge it (⇧ for a big step) in the same normalised coordinates a click
  // produces, and the note saves through the same savePending() path.
  let addBtn = $state<HTMLButtonElement | null>(null);
  /** The pending pin was started from the keyboard button → give focus back to it. */
  let viaButton = false;
  const PIN_STEP = 0.01;
  const PIN_STEP_BIG = 0.05;

  /** The scroll container the overlay lives in (the render box's offset parent). */
  function scroller(): HTMLElement | null {
    return (box?.offsetParent as HTMLElement | null) ?? null;
  }

  function startAtCentre(): void {
    if (mode !== 'annotate') mode = 'annotate';
    const wrap = scroller();
    const w = geom.width || 1;
    const h = geom.height || 1;
    // Centre of the visible part of the box (the box can be taller than the pane).
    const cx = wrap ? wrap.scrollLeft + wrap.clientWidth / 2 - geom.left : w / 2;
    const cy = wrap ? wrap.scrollTop + wrap.clientHeight / 2 - geom.top : h / 2;
    pending = { x_pct: clamp(cx / w), y_pct: clamp(cy / h) };
    pendingBody = '';
    viaButton = true;
  }

  function onPinKey(e: KeyboardEvent): void {
    if (!pending) return;
    const step = e.shiftKey ? PIN_STEP_BIG : PIN_STEP;
    let { x_pct, y_pct } = pending;
    if (e.key === 'ArrowLeft') x_pct -= step;
    else if (e.key === 'ArrowRight') x_pct += step;
    else if (e.key === 'ArrowUp') y_pct -= step;
    else if (e.key === 'ArrowDown') y_pct += step;
    else if (e.key === 'Escape') {
      e.preventDefault();
      closeEditor();
      return;
    } else return;
    e.preventDefault();
    pending = { x_pct: clamp(x_pct), y_pct: clamp(y_pct) };
  }

  /** Esc / Cancel: drop the pending pin; a keyboard-started one returns to Add. */
  function closeEditor(): void {
    const back = viaButton;
    viaButton = false;
    cancelPending();
    if (back) void tick().then(() => addBtn?.focus());
  }

  // ── Editor placement ───────────────────────────────────────────────────────
  // The editor is placed SEPARATELY from the pin: its measured size is clamped
  // into the visible part of the scroll container (with an inset), its width and
  // height are capped to the room available (it scrolls inside), and it opens
  // above the pin only when it does not fit below.
  let editorEl = $state<HTMLElement | null>(null);
  let editorStyle = $state('');
  const EDITOR_W = 220;
  const EDITOR_GAP = 14;
  const EDITOR_INSET = 8;
  const EDITOR_MIN_H = 96;

  function placeEditor(): void {
    const wrap = scroller();
    if (!pending || !editorEl || !wrap) return;
    const minX = wrap.scrollLeft - geom.left + EDITOR_INSET;
    const maxX = wrap.scrollLeft + wrap.clientWidth - geom.left - EDITOR_INSET;
    const minY = wrap.scrollTop - geom.top + EDITOR_INSET;
    const maxY = wrap.scrollTop + wrap.clientHeight - geom.top - EDITOR_INSET;
    const pinX = pending.x_pct * geom.width;
    const pinY = pending.y_pct * geom.height;
    const width = Math.max(120, Math.min(EDITOR_W, maxX - minX));
    // Natural (uncapped) height: the content height, plus the 1px borders.
    const natural = editorEl.scrollHeight + 2;
    const below = maxY - (pinY + EDITOR_GAP);
    const above = pinY - EDITOR_GAP - minY;
    let top: number;
    let height: number;
    if (natural <= below) {
      height = natural;
      top = pinY + EDITOR_GAP;
    } else if (natural <= above) {
      height = natural;
      top = pinY - EDITOR_GAP - natural;
    } else if (below >= above) {
      height = Math.max(EDITOR_MIN_H, below);
      top = pinY + EDITOR_GAP;
    } else {
      height = Math.max(EDITOR_MIN_H, above);
      top = pinY - EDITOR_GAP - height;
    }
    height = Math.min(height, Math.max(EDITOR_MIN_H, maxY - minY));
    // Final clamp into the visible region.
    top = Math.max(minY, Math.min(top, maxY - height));
    const left = Math.max(minX, Math.min(pinX - width / 2, maxX - width));
    editorStyle = `left:${Math.round(left)}px; top:${Math.round(top)}px; width:${Math.round(width)}px; max-height:${Math.round(height)}px`;
  }

  $effect(() => {
    // Re-place when the pin moves, the box resizes or the note grows.
    void pending;
    void geom;
    void pendingBody;
    if (!pending) {
      editorStyle = '';
      return;
    }
    if (!editorEl) return;
    placeEditor();
  });
  $effect(() => {
    if (!pending) return;
    const wrap = scroller();
    if (!wrap) return;
    const again = (): void => placeEditor();
    wrap.addEventListener('scroll', again, { passive: true });
    window.addEventListener('resize', again);
    return () => {
      wrap.removeEventListener('scroll', again);
      window.removeEventListener('resize', again);
    };
  });

  async function toggleResolved(n: MockupAnnotation): Promise<void> {
    try {
      const updated = await product.patchAnnotation(n.id, { resolved: !n.resolved });
      notes = notes.map((x) => (x.id === n.id ? updated : x));
    } catch (e) {
      toastError('Couldn’t update note', e);
    }
  }

  async function removeNote(n: MockupAnnotation): Promise<void> {
    const ok = await confirmer.ask('Delete this annotation?', {
      title: 'Delete annotation',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await product.deleteAnnotation(n.id);
      notes = notes.filter((x) => x.id !== n.id);
    } catch (e) {
      toastError('Couldn’t delete note', e);
    }
  }

;
async () => {


  { svelteHTML.createElement("div", {         "class":`overlay`,"role":`presentation`,"style":`left:${geom.left}px; top:${geom.top}px; width:${geom.width}px; height:${geom.height}px`,"onclick":onOverlayClick,});mode === 'annotate';
      for(let n of __sveltets_2_ensureArray(notes)){let i = 1;n.id;
     { svelteHTML.createElement("div", {        "class":`pin`,"style":`left:${n.x_pct * 100}%; top:${n.y_pct * 100}%`,"title":n.body,});n.resolved;
      i + 1;
     }
  }

  if(pending){
    
     { svelteHTML.createElement("button", {             "type":`button`,"class":`pin pending`,"style":`left:${pending.x_pct * 100}%; top:${pending.y_pct * 100}%`,"aria-label":`New annotation position. Arrow keys move it, Shift for larger steps; Escape cancels.`,"onkeydown":onPinKey,"onclick":(e) => e.stopPropagation(),});
      notes.length + 1;
     }
    
     { const $$_div1 = svelteHTML.createElement("div", {                 "class":`editor`,"style":editorStyle,"role":`dialog`,"tabindex":-1,"aria-label":`New annotation`,"onclick":(e) => e.stopPropagation(),"onkeydown":(e) => {
        e.stopPropagation();
        if (e.key === 'Escape') {
          e.preventDefault();
          closeEditor();
        }
      },});!editorStyle;editorEl = $$_div1;
       {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('textarea')));{ svelteHTML.createElement("textarea", __sveltets_2_union($$action_0), {           "dir":`auto`,"bind:value":pendingBody,"aria-label":`Annotation note`,"placeholder":`e.g. Make this button the primary action`,"rows":3,});/*Ωignore_startΩ*/() => pendingBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }}
       { svelteHTML.createElement("div", { "class":`editor-actions`,});
         { svelteHTML.createElement("button", {   "class":`mini ghost`,"onclick":closeEditor,});  }
         { svelteHTML.createElement("button", {     "class":`mini primary`,"onclick":savePending,"disabled":saving || !pendingBody.trim(),});
          saving ? 'Saving…' : 'Add';
         }
       }
     }
  }
 }


 { svelteHTML.createElement("div", { "class":`side`,});
   { svelteHTML.createElement("div", { "class":`side-head`,});
     { svelteHTML.createElement("div", {     "class":`mode-toggle`,"role":`group`,"aria-label":`Annotation mode`,});
       { svelteHTML.createElement("button", {        "class":`mt`,"aria-pressed":mode === 'annotate',"onclick":() => (mode = 'annotate'),});mode === 'annotate';
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`pin`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {        "class":`mt`,"aria-pressed":mode === 'interact',"onclick":() => { mode = 'interact'; cancelPending(); },});mode === 'interact';
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});} 
       }
     }
    if(mode === 'annotate'){
       { const $$_button2 = svelteHTML.createElement("button", {        "class":`mini add-note`,"onclick":startAtCentre,"title":`Drop a pin in the middle of the visible mockup, then move it with the arrow keys (⇧ for bigger steps)`,});addBtn = $$_button2;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`note-list`,});
    if(loadError){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`annotations`,"variant":`compact`,"error":loadError,"empty":notes.length === 0,"onretry":() => void loadNotes(),}});}
    }
    if(loading){
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {      "rows":3,"height":28,"label":`annotations`,}});}
    } else if (loadError && notes.length === 0){
      
    } else if (notes.length === 0){
       { svelteHTML.createElement("div", { "class":`note-empty`,});
          mode === 'annotate' ? ' Click the mockup to drop a pin.' : '';
       }
    }else{
          for(let n of __sveltets_2_ensureArray(notes)){let i = 1;n.id;
         { svelteHTML.createElement("div", {  "class":`note`,});n.resolved;
           { svelteHTML.createElement("span", { "class":`note-num`,});i + 1; }
           { svelteHTML.createElement("span", { "class":`note-body`,});n.body; }
           { svelteHTML.createElement("div", { "class":`note-actions`,});
             { svelteHTML.createElement("button", {         "class":`note-btn`,"onclick":() => toggleResolved(n),"title":n.resolved ? 'Reopen' : 'Resolve',"aria-label":n.resolved ? 'Reopen' : 'Resolve',});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
             }
             { svelteHTML.createElement("button", {         "class":`note-btn danger`,"onclick":() => removeNote(n),"title":`Delete`,"aria-label":`Delete`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
             }
           }
         }
      }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MockupAnnotations__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MockupAnnotations__SvelteComponent_ = ReturnType<typeof MockupAnnotations__SvelteComponent_>;
/*Ωignore_endΩ*/export default MockupAnnotations__SvelteComponent_;