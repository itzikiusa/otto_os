///<reference types="svelte" />
;
  import type { DesignArtifact as DA } from '../../lib/api/types';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  export interface CompareSide {
    artifact: DA;
    versionId: string;
  }
;;

import { onTabKey } from './tabKeys';
// Compare two versions — of this artifact, or this artifact against a
// reference from the library. Side by side renders both with the same stage
// the editor uses; Changes is a text diff of the sources (JSON pretty-printed
// first). Restore puts the left version's content back as a NEW version
// (the view confirms and saves) — history never rewinds.
import { untrack } from 'svelte';
import { loadErrorText } from '../../lib/loadError';
import Modal from '../../lib/components/Modal.svelte';
import DiffView from '../../lib/components/DiffView.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { fetchContent, listVersions } from '../../lib/api/design';
import type { DesignArtifact, DesignVersion } from '../../lib/api/types';
import ArtifactStage from './ArtifactStage.svelte';
import { isJsonFormat, isTextFormat, versionAuthor } from './model';
;

  interface Props {
    left: CompareSide;
    right: CompareSide;
    /** The artifact open in the view — only its versions can be restored. */
    currentId: string;
    headId: string | null;
    meId: string | null | undefined;
    onclose: () => void;
    onrestore: (versionId: string, seq: number) => void;
  };;

  interface Loaded {
    key: string;
    text: string | null;
    blobUrl: string | null;
    error: string | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { left: initialLeft, right: initialRight, currentId, headId, meId, onclose, onrestore }: Props = $props();

  let left = $state<CompareSide>(untrack(() => initialLeft));
  let right = $state<CompareSide>(untrack(() => initialRight));
  let mode = $state<'side' | 'changes'>('side');
  let innerH = $state(800);

  // Version lists per artifact (for the pickers), fetched once each.
  let versionsOf = $state<Record<string, DesignVersion[]>>({});
  $effect(() => {
    for (const id of [left.artifact.id, right.artifact.id]) {
      if (untrack(() => versionsOf[id])) continue;
      void listVersions(id).then(
        (v) => (versionsOf = { ...versionsOf, [id]: v }),
        () => (versionsOf = { ...versionsOf, [id]: [] }),
      );
    }
  });

  let loadedL = $state<Loaded | null>(null);
  let loadedR = $state<Loaded | null>(null);

  function loader(side: () => CompareSide, set: (l: Loaded) => void, get: () => Loaded | null) {
    return () => {
      const s = side();
      const key = `${s.artifact.id}:${s.versionId}`;
      if (untrack(get)?.key === key) return;
      const asText = isTextFormat(s.artifact.format);
      let canceled = false;
      void fetchContent(s.artifact.id, { version: s.versionId, asText }).then(
        (c) => {
          if (canceled) {
            if (c.blobUrl) URL.revokeObjectURL(c.blobUrl);
            return;
          }
          const prev = untrack(get);
          if (prev?.blobUrl) URL.revokeObjectURL(prev.blobUrl);
          set({ key, text: c.text, blobUrl: c.blobUrl, error: null });
        },
        (e) => {
          if (!canceled) set({ key, text: null, blobUrl: null, error: loadErrorText(e) });
        },
      );
      return () => {
        canceled = true;
      };
    };
  }
  $effect(loader(() => left, (l) => (loadedL = l), () => loadedL));
  $effect(loader(() => right, (l) => (loadedR = l), () => loadedR));
  $effect(() => () => {
    if (loadedL?.blobUrl) URL.revokeObjectURL(loadedL.blobUrl);
    if (loadedR?.blobUrl) URL.revokeObjectURL(loadedR.blobUrl);
  });

  const textBoth = $derived(isTextFormat(left.artifact.format) && isTextFormat(right.artifact.format));
  function pretty(format: string, t: string | null): string {
    if (t == null) return '';
    if (!isJsonFormat(format)) return t;
    try {
      return JSON.stringify(JSON.parse(t), null, 2);
    } catch {
      return t;
    }
  }

  function seqOf(side: CompareSide): number | null {
    return versionsOf[side.artifact.id]?.find((v) => v.id === side.versionId)?.seq ?? null;
  }
  function labelOf(side: CompareSide, v: DesignVersion): string {
    const cur = side.artifact.id === currentId && v.id === headId ? ', current' : '';
    return `v${v.seq} (${versionAuthor(v, meId)}${cur})`;
  }

  const canRestore = $derived(left.artifact.id === currentId && left.versionId !== headId && seqOf(left) != null);
  const paneH = $derived(Math.max(260, Math.round(innerH * 0.58)));
;
async () => {



 { svelteHTML.createElement("svelte:window", {  "bind:innerHeight":innerH,});/*Ωignore_startΩ*/() => innerH = __sveltets_2_any(null);/*Ωignore_endΩ*/}

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Compare versions`,"width":1180,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(canRestore){
       { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => onrestore(left.versionId, seqOf(left)!),...__sveltets_2_empty({"data-testid":`design-restore`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}  seqOf(left);
       }
    }
     { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onclose,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`cmp`,...__sveltets_2_empty({"data-testid":`design-compare-modal`}),});
     { svelteHTML.createElement("div", { "class":`modes`,});
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`tablist`,"aria-label":`Compare mode`,});
         { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":mode === 'side',"tabindex":mode === 'side' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mode = 'side'),});mode === 'side';   }
         { svelteHTML.createElement("button", {              "role":`tab`,"aria-selected":mode === 'changes',"tabindex":mode === 'changes' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mode = 'changes'),"disabled":!textBoth,"title":textBoth ? undefined : 'Changes compare text sources; images and models compare side by side',});mode === 'changes';  }
       }
     }
     { svelteHTML.createElement("div", { "class":`pickers`,});
         for(let p of __sveltets_2_ensureArray(([{ s: left, set: (v: string) => (left = { ...left, versionId: v }), n: 'Left' }, { s: right, set: (v: string) => (right = { ...right, versionId: v }), n: 'Right' }]))){p.n;
         { svelteHTML.createElement("label", { "class":`picker`,});
           { svelteHTML.createElement("span", {   "class":`title`,"title":p.s.artifact.title,});p.s.artifact.id === currentId ? '' : `${p.s.artifact.title} · `; }
           { svelteHTML.createElement("select", {       "class":`input`,"aria-label":`${p.n} version`,"value":p.s.versionId,"onchange":(e) => p.set((e.currentTarget as HTMLSelectElement).value),});
               for(let v of __sveltets_2_ensureArray(versionsOf[p.s.artifact.id] ?? [])){v.id; { svelteHTML.createElement("option", { "value":v.id,});labelOf(p.s, v); }}
            if(!(versionsOf[p.s.artifact.id] ?? []).length){ { svelteHTML.createElement("option", { "value":p.s.versionId,});  }}
           }
         }
      }
     }
    if(mode === 'side' || !textBoth){
       { svelteHTML.createElement("div", {  "class":`panes`,});__sveltets_2_ensureType(String, Number, `${paneH}px`);
            for(let p of __sveltets_2_ensureArray(([{ s: left, l: loadedL }, { s: right, l: loadedR }]))){let i = 1;i;
           { svelteHTML.createElement("div", { "class":`pane`,});
            if(!p.l){
               { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":6,"height":16,"label":`this version`,}});}
            } else if (p.l.error){
               { svelteHTML.createElement("p", { "class":`msg err`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}     p.l.error; }
            }else{
              p.l.key; {
                  { const $$_egatStcafitrA4C = __sveltets_2_ensureComponent(ArtifactStage); new $$_egatStcafitrA4C({ target: __sveltets_2_any(), props: {      "artifact":p.s.artifact,"source":p.l.text,"blobUrl":p.l.blobUrl,"readonly":true,"compact":true,}});}
              }
            }
           }
        }
       }
    }else{
       { svelteHTML.createElement("div", {  "class":`diff`,});__sveltets_2_ensureType(String, Number, `${paneH}px`);
        if(loadedL && loadedR && !loadedL.error && !loadedR.error){
           { const $$_weiVffiD3C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD3C({ target: __sveltets_2_any(), props: {        "before":pretty(left.artifact.format, loadedL.text),"after":pretty(right.artifact.format, loadedR.text),"mode":`split`,"contextLines":4,}});}
        }else{
           { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":8,"height":16,"label":`the versions`,}});}
        }
       }
    }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CompareModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CompareModal__SvelteComponent_ = ReturnType<typeof CompareModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default CompareModal__SvelteComponent_;