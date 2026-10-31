///<reference types="svelte" />
;// MockupViewer — renders a single mockup attachment by type, isolating all
// untrusted/agent-controlled content from the parent DOM (design §8):
//   • image (incl. svg) → <img src={authed blob url}>  (svg rendered AS AN
//     IMAGE, never inlined — avoids SVG-borne script execution).
//   • html (text/html) → sandboxed <iframe srcdoc={content}>; sandbox does NOT
//     include allow-scripts by default. A per-mockup "Enable interactivity"
//     toggle adds allow-scripts and shows a visible warning.
//   • mermaid (text/vnd.mermaid or *.mmd) → mermaid.render() → SVG STRING, then
//     embedded inside a SANDBOXED iframe srcdoc (NOT inlined into the parent
//     DOM, since mermaid input is agent/user-controlled).
// The render box is exposed to a sibling MockupAnnotations overlay so pins sit
// exactly over the mockup.

import { announceShown } from '../../lib/announce.svelte';
import { onMount } from 'svelte';
import { renderMermaid } from '../../lib/render/mermaid';
import { authedBlobUrl, authedText } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import MockupAnnotations from './MockupAnnotations.svelte';
import type { ProductAttachment } from './types';
;

  interface Props {
    attachment: ProductAttachment;
    /** When given, render THIS source (html / mermaid) instead of fetching the
     *  attachment's bytes — the Design Arena owns the in-memory, autosaved source
     *  and re-renders the viewer as it changes. Ignored for images. */
    source?: string | null;
    /** The arena has its own toolbar; hide the viewer's file-name strip. */
    hideToolbar?: boolean;
    /** Arena "Enable interactivity" lives in its inspector — mirror it here. */
    allowScripts?: boolean;
  };;

  // Mermaid loads on the first diagram (lib/render/mermaid.ts: lazy, strict
  // security, auto-rendering off) — never with the Product page's chunk.

  // ── Derived type classification ─────────────────────────────────────────────
  type Kind = 'image' | 'html' | 'mermaid' | 'unknown';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { attachment, source = null, hideToolbar = false, allowScripts = $bindable(false) }: Props = $props()/*Ωignore_startΩ*/;allowScripts;/*Ωignore_endΩ*/;

  function classify(a: ProductAttachment): Kind {
    const mime = (a.mime || '').toLowerCase();
    const name = (a.filename || '').toLowerCase();
    if (mime === 'text/vnd.mermaid' || name.endsWith('.mmd')) return 'mermaid';
    if (mime === 'text/html') return 'html';
    // svg + raster images both render as <img>.
    if (mime === 'image/svg+xml' || mime.startsWith('image/')) return 'image';
    return 'unknown';
  }
  const kind = $derived(classify(attachment));

  // ── State ─────────────────────────────────────────────────────────────────
  // image: authed blob URL.
  let imgUrl = $state<string | null>(null);
  // html: raw content text rendered in the iframe srcdoc.
  let htmlContent = $state<string | null>(null);
  // mermaid: the rendered SVG string (embedded in a sandboxed iframe srcdoc).
  let mermaidSvg = $state<string | null>(null);

  let rendering = $state(false);
  let renderError = $state<string | null>(null);

  // The render-box element passed to the annotation overlay so pins anchor to it.
  let renderBox = $state<HTMLDivElement | null>(null);

  // Object URLs we created (revoke on unmount / re-render).
  let createdUrls: string[] = [];

  // Unique counter so mermaid render ids never collide across re-renders.
  let mermaidSeq = 0;

  // ── Helpers ─────────────────────────────────────────────────────────────────

  /** Fetch attachment bytes as text (authed; a 401 re-auths like any call). */
  function fetchText(id: string): Promise<string> {
    return authedText(`/product/attachments/${encodeURIComponent(id)}`);
  }

  /** Wrap an SVG string in a minimal sandboxed HTML doc for iframe srcdoc. */
  function svgDoc(svg: string): string {
    return `<!doctype html><html><head><meta charset="utf-8">` +
      `<style>html,body{margin:0;padding:8px;background:#fff;}` + // ui-guards: allow — rendered page canvas (the mockup’s own white page)
      `svg{max-width:100%;height:auto;display:block;margin:0 auto;}</style></head>` +
      `<body>${svg}</body></html>`;
  }

  // Generation counter: a blob URL that resolves AFTER revokeCreated() ran (the
  // attachment changed / the viewer unmounted mid-fetch) is revoked on the spot
  // instead of being pushed into a list nobody will revoke again.
  let gen = 0;
  function revokeCreated(): void {
    gen++;
    const stale = createdUrls.splice(0);
    for (const u of stale) URL.revokeObjectURL(u);
  }

  // ── Render pipeline — re-runs whenever the attachment OR the source changes ──
  $effect(() => {
    // Read the attachment id + source so this effect re-runs on change; the
    // returned cleanup cancels a pending debounced render.
    return scheduleRender(attachment, source);
  });

  /** Live edit of a diagram already on screen (the code view's keystrokes):
   *  debounce, and renderFor keeps the last good SVG meanwhile. Anything else
   *  renders now. Returns the cleanup that drops a still-pending render. */
  function scheduleRender(a: ProductAttachment, src: string | null): (() => void) | undefined {
    if (src !== null && renderedFor === a.id) {
      const t = setTimeout(() => void renderFor(a, src), LIVE_DEBOUNCE_MS);
      return () => clearTimeout(t);
    }
    void renderFor(a, src);
    return undefined;
  }

  $effect(() => {
    // Revoke object URLs on unmount.
    return () => revokeCreated();
  });

  /** Attachment id whose Mermaid diagram is on screen (plain, not reactive):
   *  a source edit of THAT diagram re-renders in place. */
  let renderedFor: string | null = null;
  const LIVE_DEBOUNCE_MS = 200;
  /** A mid-edit parse error, shown under the last good diagram. */
  let liveError = $state<string | null>(null);

  async function renderFor(a: ProductAttachment, src: string | null): Promise<void> {
    // Typing in the code view used to blank the diagram to "Rendering…" and
    // remount the annotation overlay per keystroke (refetching annotations and
    // dropping a half-placed pin). Keep the last good SVG while re-rendering.
    const live = src !== null && renderedFor === a.id && mermaidSvg !== null && classify(a) === 'mermaid';
    if (!live) {
      rendering = true;
      renderError = null;
      imgUrl = null;
      htmlContent = null;
      mermaidSvg = null;
      renderedFor = null;
    }
    liveError = null;
    revokeCreated();
    const myGen = gen;
    try {
      const k = classify(a);
      if (k === 'image') {
        const url = await authedBlobUrl(`/product/attachments/${encodeURIComponent(a.id)}`);
        if (myGen !== gen) {
          URL.revokeObjectURL(url); // superseded while in flight
          return;
        }
        createdUrls.push(url);
        imgUrl = url;
      } else if (k === 'html') {
        htmlContent = src ?? (await fetchText(a.id));
      } else if (k === 'mermaid') {
        const text = src ?? (await fetchText(a.id));
        if (!text.trim()) {
          mermaidSvg = '';
        } else {
          const id = `otto-mermaid-${Date.now()}-${mermaidSeq++}`;
          const { svg, error, stale } = await renderMermaid(id, text, { isStale: () => myGen !== gen });
          if (stale || myGen !== gen) return; // superseded while mermaid loaded/rendered
          if (svg === undefined) throw new Error(error ?? 'Diagram error');
          mermaidSvg = svg;
          renderedFor = a.id;
        }
      } else {
        renderError = `Unsupported mockup type: ${a.mime || 'unknown'}`;
      }
    } catch (e) {
      if (myGen !== gen) return; // a newer render owns the box
      const msg = e instanceof Error ? e.message : String(e);
      // A source override means the user is TYPING (code view) — a transient
      // Mermaid parse error is expected mid-edit; show it inline, don't toast.
      // Mid-edit, the last good diagram (and its pins) stays above the error.
      if (live) liveError = msg;
      else renderError = msg;
      if (src === null) toasts.error('Couldn’t render mockup', msg);
    } finally {
      if (myGen === gen) rendering = false;
    }
  }

  // Sandbox string for the HTML iframe. Without interactivity it is the bare
  // sandbox="" (most restrictive: scripts, forms, popups, same-origin all off).
  // With interactivity, only allow-scripts is added (still no allow-same-origin,
  // so the frame stays in an opaque origin and cannot reach the daemon/cookies).
  const htmlSandbox = $derived(allowScripts ? 'allow-scripts' : '');

  onMount(() => {
    // No-op; kept for symmetry / future hooks.
  });
;
async () => {

 { svelteHTML.createElement("div", {  "class":`viewer`,});hideToolbar;
  
   { svelteHTML.createElement("div", {   "class":`viewer-toolbar`,"hidden":hideToolbar,});
     { svelteHTML.createElement("span", {   "class":`vt-name`,"title":attachment.filename,});attachment.filename; }
     { svelteHTML.createElement("span", { "class":`vt-mime`,});attachment.mime; }
    if(kind === 'html'){
       { svelteHTML.createElement("label", {   "class":`vt-toggle`,"title":`Run scripts inside the sandboxed iframe`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":allowScripts,});/*Ωignore_startΩ*/() => allowScripts = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         
       }
    }
   }

  if(kind === 'html' && allowScripts){
     { svelteHTML.createElement("div", { "class":`warn`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":13,}});}
                 
              
     }
  }

  
   { svelteHTML.createElement("div", { "class":`render-wrap`,});
     { const $$_div2 = svelteHTML.createElement("div", {  "class":`render-box`,});renderBox = $$_div2;
      if(rendering){
         { svelteHTML.createElement("div", { "class":`render-msg`,});  }
      } else if (renderError){
         { svelteHTML.createElement("div", { "class":`render-msg err`,});renderError; }
      } else if (kind === 'image' && imgUrl){
        
         { svelteHTML.createElement("img", {      "class":`mockup-img`,"src":imgUrl,"alt":attachment.filename,});}
      } else if (kind === 'html' && htmlContent !== null){
         { svelteHTML.createElement("iframe", {         "class":`mockup-frame`,"title":attachment.filename,"sandbox":htmlSandbox,"srcdoc":htmlContent,}); }
      } else if (kind === 'mermaid' && mermaidSvg !== null){
        
         { svelteHTML.createElement("iframe", {        "class":`mockup-frame`,"title":attachment.filename,"sandbox":"","srcdoc":svgDoc(mermaidSvg),}); }
      }
     }

    
    if(liveError){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(liveError)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`live-err`,});liveError; }}
    }
    if(!rendering && !renderError && renderBox){
       { const $$_snoitatonnApukcoM2C = __sveltets_2_ensureComponent(MockupAnnotations); new $$_snoitatonnApukcoM2C({ target: __sveltets_2_any(), props: {    "attachmentId":attachment.id,"box":renderBox,}});}
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('allowScripts'), slots: {}, events: {} }}
const MockupViewer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MockupViewer__SvelteComponent_ = ReturnType<typeof MockupViewer__SvelteComponent_>;
/*Ωignore_endΩ*/export default MockupViewer__SvelteComponent_;