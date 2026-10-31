///<reference types="svelte" />
;// Bottom version strip: every committed version, oldest → newest, as a chip
// with its author (agents marked). Clicking chips selects up to two for
// Compare; versions are the undo model, so nothing here ever deletes.

import Icon from '../../lib/components/Icon.svelte';
import { announceShown } from '../../lib/announce.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import type { DesignVersion } from '../../lib/api/types';
import { stripOrder, versionAuthor } from './model';
;

  interface Props {
    versions: DesignVersion[];
    headId: string | null;
    approvedId: string | null;
    meId: string | null | undefined;
    selected: string[];
    loading?: boolean;
    error?: string | null;
    onselect: (id: string) => void;
    oncompare: () => void;
    onretry: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { versions, headId, approvedId, meId, selected, loading = false, error = null, onselect, oncompare, onretry }: Props =
    $props();

  const ordered = $derived(stripOrder(versions));
  let scroller = $state<HTMLDivElement | null>(null);

  // Keep the newest version in view when the strip grows.
  $effect(() => {
    void ordered.length;
    const el = scroller;
    if (el) queueMicrotask(() => el.querySelector('button.vchip:last-of-type')?.scrollIntoView({ inline: 'nearest', block: 'nearest' }));
  });

  /** What a screen reader hears when a refresh fails over loaded versions. */
  function refreshFailedText(err: string): string {
    return `Couldn’t refresh the versions. ${err}`;
  }

  function tip(v: DesignVersion): string {
    const when = new Date(v.created_at).toLocaleString();
    const msg = v.message ? ` — ${v.message}` : '';
    return `v${v.seq} · ${versionAuthor(v, meId)} · ${when}${msg}`;
  }

  function onKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const btns = scroller?.querySelectorAll<HTMLButtonElement>('button.vchip');
    const direction = scroller && getComputedStyle(scroller).direction === 'rtl' ? -1 : 1;
    const j = Math.max(0, Math.min((btns?.length ?? 1) - 1, i + (e.key === 'ArrowRight' ? direction : -direction)));
    btns?.[j]?.focus();
  }

  const hint = $derived(
    selected.length === 0
      ? 'Select versions to compare'
      : selected.length === 1
        ? 'Compare with the current version, or pick one more'
        : 'Compare the two selected versions',
  );
;
async () => {

 { svelteHTML.createElement("div", {   "class":`strip`,...__sveltets_2_empty({"data-testid":`design-version-strip`}),});
   { svelteHTML.createElement("span", { "class":`label`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":14,}});}  }
   { const $$_div1 = svelteHTML.createElement("div", {      "class":`chips scroll-thin`,"role":`group`,"aria-label":`Versions`,});scroller = $$_div1;
    if(loading && !versions.length){
       { svelteHTML.createElement("div", { "class":`strip-skel`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":1,"height":22,"label":`versions`,}});} }
    } else if (error && !versions.length){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         "what":`versions`,"variant":`compact`,loading,error,"empty":true,onretry,}});}
    }else{
          for(let v of __sveltets_2_ensureArray(ordered)){let i = 1;v.id;
        const author = versionAuthor(v, meId);
         { svelteHTML.createElement("button", {                 "class":`vchip`,"aria-pressed":selected.includes(v.id),"title":tip(v),"onclick":() => onselect(v.id),"onkeydown":(e) => onKey(e, i),...__sveltets_2_empty({"data-testid":`design-version-chip`}),...__sveltets_2_empty({"data-seq":v.seq}),});selected.includes(v.id);v.id === headId;
           { svelteHTML.createElement("span", {     "class":`av`,"aria-hidden":`true`,});v.author_kind === 'agent';v.author_kind === 'system'; }
           { svelteHTML.createElement("span", { "class":`v`,}); v.seq; }
           { svelteHTML.createElement("span", { "class":`who`,}); author;v.id === headId ? ', current' : '';  }
          if(v.id === approvedId){ { svelteHTML.createElement("span", {   "class":`appr`,"title":`Approved version`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} }}
         }
      }
      if(error){
        
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('span'),({ text: refreshFailedText(error), assertive: true })));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {     "class":`err`,"title":error,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}   }}
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":onretry,"disabled":loading,});loading ? 'Retrying…' : 'Retry'; }
      }
    }
   }
   { svelteHTML.createElement("span", { "class":`hint`,});hint; }
   { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":oncompare,"disabled":selected.length === 0 || versions.length < 2,...__sveltets_2_empty({"data-testid":`design-compare`}),});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`columns`,"size":12,}});} 
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const VersionStrip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type VersionStrip__SvelteComponent_ = ReturnType<typeof VersionStrip__SvelteComponent_>;
/*Ωignore_endΩ*/export default VersionStrip__SvelteComponent_;