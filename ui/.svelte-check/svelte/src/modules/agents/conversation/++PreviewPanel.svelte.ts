///<reference types="svelte" />
;// The chat's side panel (t3code's right-hand Diff panel): a file, a code
// block or a turn's diff next to the conversation, with Preview | Source |
// Diff tabs where they apply, Wrap, Reload, Open in Files, Copy path and a
// full view (Modal). The host (ConversationView) owns placement: beside the
// chat when the pane is wide, over it when narrow. Esc closes.

import { copyTextOrThrow } from '../../../lib/clipboard';
import { getContext, untrack } from 'svelte';
import { toastError } from '../../../lib/toastError';
import Icon from '../../../lib/components/Icon.svelte';
import Modal from '../../../lib/components/Modal.svelte';
import { openFile } from '../../../lib/stores/openfile.svelte';
import { toasts } from '../../../lib/toast.svelte';
import PreviewBody, { forgetRead, type PreviewMode } from './PreviewBody.svelte';
import { previewKind, previewKindForLang, requestKind, splitName } from './preview';
import { relPath, resolvePath } from './format';
import { onTabKey } from '../../../lib/tabKeys';
import { CONV_CTX, type ConvContext, type PreviewReq } from './context';
;

  interface Props {
    req: PreviewReq;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { req, onclose }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);
  const cwd = $derived(ctx.cwd ?? null);

  const path = $derived(req.kind === 'file' ? resolvePath(req.path, cwd) : req.kind === 'code' && req.file ? resolvePath(req.file, cwd) : null);
  const kind = $derived(requestKind(req));
  const modes = $derived.by(() => {
    const m: PreviewMode[] = [];
    if (req.kind === 'diff') return ['diff'] as PreviewMode[];
    if (kind !== 'code') m.push('preview');
    m.push('source');
    if (req.kind === 'file' && req.diff) m.push('diff');
    return m;
  });
  function initialMode(r: PreviewReq): PreviewMode {
    if (r.kind === 'diff') return 'diff';
    if (r.kind === 'file' && r.line) return 'source';
    if (r.kind === 'code') return (r.file ? previewKind(r.file) : previewKindForLang(r.lang)) !== 'code' ? 'preview' : 'source';
    return previewKind(r.path) !== 'code' ? 'preview' : 'source';
  }
  let mode = $state<PreviewMode>(untrack(() => initialMode(req)));
  let lastReq: PreviewReq | null = null;
  $effect(() => {
    // A new request resets the tab (the same request keeps what you picked).
    if (req !== lastReq) {
      lastReq = req;
      mode = initialMode(req);
    }
  });
  let wrap = $state(false);
  let reloadKey = $state(0);
  let full = $state(false);

  const title = $derived.by(() => {
    if (req.kind === 'diff') return { base: req.title, dir: '' };
    if (path) {
      const { dir, base } = splitName(relPath(path, cwd));
      return { base, dir };
    }
    return { base: req.kind === 'code' ? req.title : 'Preview', dir: '' };
  });
  const LABEL: Record<PreviewMode, string> = { preview: 'Preview', source: 'Source', diff: 'Diff' };

  function reload(): void {
    if (path) forgetRead(path);
    reloadKey++;
  }
  function openInFiles(): void {
    if (path) openFile.open(path, req.kind === 'file' ? (req.line ?? undefined) : undefined);
  }
  async function copyPath(): Promise<void> {
    if (!path) return;
    try {
      await copyTextOrThrow(path);
      toasts.info('Path copied', path);
    } catch (e) {
      toastError('Couldn’t copy to the clipboard', e);
    }
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape' && !full) {
      e.stopPropagation();
      onclose();
    }
  }

  // Narrow pane: the host lays the panel OVER the chat. Then the covered chat
  // (and its composer) must not stay reachable by Tab / screen readers: mark it
  // inert, move focus into the panel, and hand focus back when it closes.
  let panelEl = $state<HTMLElement | null>(null);
  let covering = $state(false);
  $effect(() => {
    const slot = panelEl?.parentElement;
    const host = slot?.parentElement;
    if (!slot || !host || typeof ResizeObserver === 'undefined') return;
    const measure = (): void => {
      const pos = getComputedStyle(slot).position;
      covering = pos === 'absolute' || pos === 'fixed';
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(host);
    return () => ro.disconnect();
  });
  $effect(() => {
    if (!covering || !panelEl) return;
    const slot = panelEl.parentElement;
    const host = slot?.parentElement;
    if (!slot || !host) return;
    const covered = Array.from(host.children).filter((c): c is HTMLElement => c !== slot && c instanceof HTMLElement && !c.inert);
    const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    for (const el of covered) el.inert = true;
    untrack(() => panelEl?.querySelector<HTMLElement>('button, [href], [tabindex]:not([tabindex="-1"])')?.focus());
    return () => {
      for (const el of covered) el.inert = false;
      if (returnTo?.isConnected) returnTo.focus();
    };
  });
  const canFiles = $derived(!!path && !!ctx.sessionId && !ctx.readonly);
  const fullWidth = $derived(typeof window === 'undefined' ? 1100 : Math.max(480, Math.min(1400, window.innerWidth - 80)));
;
async () => {


 { const $$_aside0 = svelteHTML.createElement("aside", {        "class":`pv`,"aria-label":`Preview: ${title.base}`,...__sveltets_2_empty({"data-preview":req.kind}),"onkeydown":onKey,});panelEl = $$_aside0;
   { svelteHTML.createElement("header", { "class":`pv-head`,});
     { svelteHTML.createElement("span", {   "class":`pv-icon`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":req.kind === 'diff' ? 'split' : req.kind === 'code' ? 'format' : 'file',"size":13,}});} }
     { svelteHTML.createElement("span", {   "class":`pv-title`,"title":path ?? title.base,});
       { svelteHTML.createElement("span", { "class":`pv-base`,});title.base; }
      if(title.dir){ { svelteHTML.createElement("span", { "class":`pv-dir mono`,});title.dir; }}
     }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (full = true),"aria-label":`Full view`,"title":`Full view`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":13,}});} }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":onclose,"aria-label":`Close preview`,"title":`Close preview`,"aria-keyshortcuts":`Escape`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});} }
   }
   { svelteHTML.createElement("div", { "class":`pv-bar`,});
    if(modes.length > 1){
       { svelteHTML.createElement("div", {     "class":`pv-seg`,"role":`tablist`,"aria-label":`View`,});
           for(let m of __sveltets_2_ensureArray(modes)){m;
           { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":mode === m,"tabindex":mode === m ? 0 : -1,"onclick":() => (mode = m),"onkeydown":onTabKey,...__sveltets_2_empty({"data-mode":m}),});mode === m;LABEL[m]; }
        }
       }
    }else{
       { svelteHTML.createElement("span", { "class":`pv-only`,});LABEL[modes[0]]; }
    }
     { svelteHTML.createElement("span", { "class":`pv-grow`,}); }
     { svelteHTML.createElement("button", {        "class":`pv-tool`,"aria-pressed":wrap,"onclick":() => (wrap = !wrap),"title":`Wrap long lines`,});wrap;  }
    if(req.kind === 'file' && req.content == null){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":reload,"aria-label":`Reload from disk`,"title":`Reload from disk`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} }
    }
    if(path){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void copyPath(),"aria-label":`Copy path`,"title":`Copy path`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
    }
    if(canFiles){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":openInFiles,"aria-label":`Open in Files`,"title":`Open in Files`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});} }
    }
   }
  if(req.kind === 'file' && req.content != null){
     { svelteHTML.createElement("div", { "class":`pv-src-note`,});                }
  }
   { svelteHTML.createElement("div", { "class":`pv-body`,});
     { const $$_ydoBweiverP2C = __sveltets_2_ensureComponent(PreviewBody); new $$_ydoBweiverP2C({ target: __sveltets_2_any(), props: {    req,mode,wrap,reloadKey,}});}
   }
 }

if(full){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":title.dir ? `${title.base} — ${title.dir}` : title.base,"width":fullWidth,"onclose":() => (full = false),}});
     { svelteHTML.createElement("div", { "class":`pv-full`,});
      if(modes.length > 1){
         { svelteHTML.createElement("div", {     "class":`pv-seg`,"role":`tablist`,"aria-label":`View`,});
             for(let m of __sveltets_2_ensureArray(modes)){m;
             { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":mode === m,"tabindex":mode === m ? 0 : -1,"onclick":() => (mode = m),"onkeydown":onTabKey,});mode === m;LABEL[m]; }
          }
         }
      }
       { svelteHTML.createElement("div", { "class":`pv-full-body`,}); { const $$_ydoBweiverP3C = __sveltets_2_ensureComponent(PreviewBody); new $$_ydoBweiverP3C({ target: __sveltets_2_any(), props: {    req,mode,wrap,reloadKey,}});} }
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PreviewPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PreviewPanel__SvelteComponent_ = ReturnType<typeof PreviewPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default PreviewPanel__SvelteComponent_;