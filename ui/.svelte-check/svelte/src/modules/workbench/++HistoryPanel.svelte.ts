///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
// History: the doc's full revision timeline (append-only — only "Delete
// forever" from the trash removes it). Pick a revision to see it, diff it
// against the current text or another revision, and restore it (restoring
// adds a new revision; the current text stays in history).
import { onDestroy, untrack } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import {
    getWorkbenchRevision,
    listWorkbenchRevisions,
    restoreWorkbenchRevision,
  } from '../../lib/api/workbench';
import type { WorkbenchDocFull, WorkbenchRevision, WorkbenchRevisionDetail } from '../../lib/api/types';
import RevisionDiff from './RevisionDiff.svelte';
;

  interface Props {
    ws: string;
    docId: string;
    /** The doc's newest revision seq — the timeline refreshes when it moves. */
    rev: number;
    /** The live editor buffer (diff target for "vs current"). */
    current: string;
    onrestored: (doc: WorkbenchDocFull) => void;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { ws, docId, rev, current, onrestored, onclose }: Props = $props();

  let revs: WorkbenchRevision[] = $state([]);
  let loading = $state(true);
  let error: string | null = $state(null);
  let selected: number | null = $state(null);
  let compareTo: number | 'current' = $state('current');
  let detail: WorkbenchRevisionDetail | null = $state(null);
  let other: WorkbenchRevisionDetail | null = $state(null);
  let detailError: string | null = $state(null);
  let view: 'diff' | 'text' = $state('diff');
  let restoring = $state(false);
  const PAGE_SIZE = 100;
  let pageCursor: number | undefined = $state(undefined);
  let pageTrail: (number | undefined)[] = $state([]);
  let compareInput = $state('');
  let compareError: string | null = $state(null);
  let alive = true;
  let loadGeneration = 0;
  let visit = 0;
  let ownerKey = '';

  function syncDocument(): void {
    const key = `${ws}\0${docId}`;
    if (key === ownerKey) return;
    ownerKey = key;
    visit++;
    selected = null;
    compareTo = 'current';
    compareInput = '';
    compareError = null;
    revs = [];
    pageCursor = undefined;
    pageTrail = [];
    restoring = false;
  }

  function invalidate(): void {
    alive = false;
    visit++;
    loadGeneration++;
  }
  onDestroy(invalidate);
  function close(): void {
    invalidate();
    onclose();
  }


  const KIND_LABEL: Record<string, string> = {
    create: 'Created',
    auto: 'Autosave',
    checkpoint: 'Saved',
    restore: 'Restored',
  };

  async function load(before?: number, trail: (number | undefined)[] = []): Promise<void> {
    syncDocument();
    if (!alive) return;
    const generation = ++loadGeneration;
    const workspace = ws, id = docId, originVisit = visit;
    const owns = () => alive && generation === loadGeneration && visit === originVisit && ws === workspace && docId === id;
    pageCursor = before;
    pageTrail = trail;
    loading = true;
    try {
      const list = await listWorkbenchRevisions(workspace, id, { limit: PAGE_SIZE, before_seq: before });
      if (!owns()) return;
      revs = [...list].sort((a, b) => b.seq - a.seq).slice(0, PAGE_SIZE);
      error = null;
    } catch (e) {
      if (owns()) error = loadErrorText(e);
    } finally {
      if (owns()) loading = false;
    }
  }

  async function older(): Promise<void> {
    if (loading || revs.length < PAGE_SIZE) return;
    return load(revs[revs.length - 1].seq, [...pageTrail, pageCursor]);
  }

  async function newer(): Promise<void> {
    if (loading || pageTrail.length === 0) return;
    return load(pageTrail[pageTrail.length - 1], pageTrail.slice(0, -1));
  }

  function compareRevision(): void {
    const seq = Number(compareInput);
    if (!Number.isSafeInteger(seq) || seq < 1) {
      compareError = 'Enter a positive revision number.';
      return;
    }
    compareError = null;
    compareTo = seq;
  }

  // Track document visits before asynchronous completions; a save refreshes
  // the newest page without retaining previously rendered metadata pages.
  $effect.pre(() => {
    void ws;
    void docId;
    untrack(syncDocument);
  });
  $effect(() => {
    void ws;
    void docId;
    void rev;
    untrack(() => void load());
  });

  $effect(() => {
    const seq = selected;
    const id = docId;
    detail = null;
    detailError = null;
    if (seq == null) return;
    let gone = false;
    getWorkbenchRevision(ws, id, seq)
      .then((d) => {
        if (!gone) detail = d;
      })
      .catch((e) => {
        if (!gone) detailError = loadErrorText(e);
      });
    return () => {
      gone = true;
    };
  });

  $effect(() => {
    const seq = compareTo;
    const id = docId;
    other = null;
    compareError = null;
    if (seq === 'current') return;
    let gone = false;
    getWorkbenchRevision(ws, id, seq)
      .then((d) => {
        if (!gone) other = d;
      })
      .catch((e) => {
        if (!gone) compareError = loadErrorText(e);
      });
    return () => {
      gone = true;
    };
  });

  function sizeLabel(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  async function restore(): Promise<void> {
    syncDocument();
    if (selected == null || restoring || !alive) return;
    const seq = selected, workspace = ws, id = docId, originVisit = visit;
    const owns = () => alive && visit === originVisit && ws === workspace && docId === id;
    restoring = true;
    try {
      const ok = await confirmer.ask(
        `The file’s content becomes revision ${seq}. Your current text is kept in history, so you can come back to it.`,
        { title: `Restore revision ${seq}?`, confirmLabel: 'Restore', danger: false },
      );
      if (!ok || !owns()) return;
      const doc = await restoreWorkbenchRevision(workspace, id, seq);
      if (!owns()) return;
      onrestored(doc);
      toasts.success(`Restored revision ${seq}`, 'Saved as a new revision.');
      selected = null;
      await load();
    } catch (e) {
      if (owns()) toasts.error("Couldn’t restore", loadErrorText(e));
    } finally {
      if (owns()) restoring = false;
    }
  }

  // Diff sides: the older revision on the left, newer on the right.
  const diffSides = $derived.by(() => {
    if (!detail) return null;
    if (compareTo === 'current') {
      return { from: detail.seq, to: undefined as number | undefined, before: detail.content, after: current, fromLabel: `Rev ${detail.seq}`, toLabel: 'Current' };
    }
    if (!other) return null;
    const [a, b] = detail.seq < other.seq ? [detail, other] : [other, detail];
    return { from: a.seq, to: b.seq, before: a.content, after: b.content, fromLabel: `Rev ${a.seq}`, toLabel: `Rev ${b.seq}` };
  });
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (compareError) announce(`Couldn’t compare revision: ${compareError}`, { assertive: true }); });
  // A failed revision load renders through LoadState, whose error line speaks.
  /** Re-run the detail load: the effect above refetches when `selected` changes. */
  function retryDetail(): void {
    const s = selected;
    selected = null;
    queueMicrotask(() => (selected = s));
  }
;
async () => {

 { svelteHTML.createElement("aside", {     "class":`wb-hist`,"aria-label":`History`,...__sveltets_2_empty({"data-testid":`wb-history`}),});
   { svelteHTML.createElement("header", { "class":`wb-side-head`,});
     { svelteHTML.createElement("h3", {});  }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void load(),"aria-label":`Refresh history`,"title":`Refresh history`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
     }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":close,"aria-label":`Close history`,"title":`Close history`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
     }
   }
   { svelteHTML.createElement("p", { "class":`wb-hist-note`,});            }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`history`,"variant":`compact`,loading,error,"empty":revs.length === 0,"onretry":() => void load(pageCursor, pageTrail),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("p", { "class":`wb-hist-note`,});           }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("ol", {   "class":`wb-revs`,"aria-label":`Revisions`,});
         for(let r of __sveltets_2_ensureArray(revs)){r.seq;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {            "class":`wb-rev`,...__sveltets_2_empty({"data-testid":`wb-rev-row`}),...__sveltets_2_empty({"data-seq":r.seq}),"aria-pressed":selected === r.seq,"onclick":() => (selected = selected === r.seq ? null : r.seq),});selected === r.seq;
             { svelteHTML.createElement("span", { "class":`wb-rev-top`,});
               { svelteHTML.createElement("span", { "class":`wb-rev-seq`,}); r.seq; }
               { svelteHTML.createElement("span", { "class":`wb-rev-kind k-${r.kind}`,});KIND_LABEL[r.kind] ?? r.kind;r.restored_from ? ` from #${r.restored_from}` : ''; }
               { svelteHTML.createElement("span", { "class":`wb-rev-when`,}); { const $$_emiTleR7C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR7C({ target: __sveltets_2_any(), props: {  "iso":r.updated_at,}});} }
             }
             { svelteHTML.createElement("span", { "class":`wb-rev-meta`,});
              sizeLabel(r.size);if(r.saves > 1){ r.saves; }
             }
           }
         }
      }
     }
   LoadState}

   { svelteHTML.createElement("div", {   "class":`wb-rev-actions`,"aria-label":`History pages`,});
     { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":loading || pageTrail.length === 0,"onclick":newer,});  }
     { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":loading || revs.length < PAGE_SIZE,"onclick":older,});  }
   }

  if(selected != null){
     { svelteHTML.createElement("section", {   "class":`wb-rev-detail`,"aria-label":`Revision ${selected}`,});
       { svelteHTML.createElement("div", { "class":`wb-rev-actions`,});
         { svelteHTML.createElement("div", {     "class":`seg`,"role":`group`,"aria-label":`Revision view`,});
           { svelteHTML.createElement("button", {     "aria-pressed":view === 'diff',"onclick":() => (view = 'diff'),});view === 'diff';  }
           { svelteHTML.createElement("button", {     "aria-pressed":view === 'text',"onclick":() => (view = 'text'),});view === 'text';  }
         }
        if(view === 'diff'){
           { svelteHTML.createElement("label", { "class":`wb-cmp`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("select", {   "bind:value":compareTo,"aria-label":`Compare with`,});/*Ωignore_startΩ*/() => compareTo = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":`current`,});  }
              if(compareTo !== 'current' && !revs.some((r) => r.seq === compareTo && r.seq !== selected)){
                 { svelteHTML.createElement("option", { "value":compareTo,}); compareTo; }
              }
                 for(let r of __sveltets_2_ensureArray(revs.filter((r) => r.seq !== selected))){r.seq;
                 { svelteHTML.createElement("option", { "value":r.seq,}); r.seq; }
              }
             }
           }
           { svelteHTML.createElement("label", { "class":`wb-cmp`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {              "dir":`auto`,"type":`text`,"inputmode":`numeric`,"size":6,"bind:value":compareInput,"aria-label":`Compare revision`,"onkeydown":(event) => { if (event.key === 'Enter') compareRevision(); },});/*Ωignore_startΩ*/() => compareInput = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":compareRevision,});  }
        }
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-testid":`wb-rev-restore`}),"disabled":restoring || !detail,"onclick":() => void restore(),"title":`Make this revision the current content (the current text stays in history)`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":12,}});}   
         }
       }
      if(compareError){
         { svelteHTML.createElement("p", { "class":`wb-err`,});   compareError; }
      }
      if(detailError){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`revision ${selected}`,"variant":`compact`,"empty":true,"error":detailError,"onretry":retryDetail,}});}
      } else if (!detail){
         { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {      "rows":6,"height":14,"label":`the revision`,}});}
      } else if (view === 'text'){
         { svelteHTML.createElement("pre", {   "class":`wb-rev-text`,...__sveltets_2_empty({"data-testid":`wb-rev-text`}),});detail.content; }
      } else if (diffSides){
         { const $$_ffiDnoisiveR2C = __sveltets_2_ensureComponent(RevisionDiff); new $$_ffiDnoisiveR2C({ target: __sveltets_2_any(), props: {   ws,docId,...diffSides,}});}
      }else{
         { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {      "rows":6,"height":14,"label":`the revision`,}});}
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HistoryPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HistoryPanel__SvelteComponent_ = ReturnType<typeof HistoryPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default HistoryPanel__SvelteComponent_;