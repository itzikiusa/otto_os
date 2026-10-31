///<reference types="svelte" />
;// A revision diff: the daemon's line-diff stats (+N −M) over the shared
// DiffView (which renders `before` → `after` client-side).

import DiffView from '../../lib/components/DiffView.svelte';
import { diffWorkbenchRevisions } from '../../lib/api/workbench';
;

  interface Props {
    ws: string;
    docId: string;
    /** Older side (a revision seq). */
    from: number;
    /** Newer side: a seq, or undefined = the current content. */
    to?: number;
    before: string;
    after: string;
    fromLabel: string;
    toLabel: string;
  };function $$render() {

  
  
  
  

  let { ws, docId, from, to, before, after, fromLabel, toLabel }: Props = $props();

  let stats: { added: number; removed: number } | null = $state(null);

  $effect(() => {
    const key = `${ws}/${docId}/${from}/${to ?? 'cur'}`;
    void after; // re-count when the current text changes (debounced by save)
    stats = null;
    let gone = false;
    diffWorkbenchRevisions(ws, docId, from, to)
      .then((d) => {
        if (!gone && key === `${ws}/${docId}/${from}/${to ?? 'cur'}`) stats = { added: d.added, removed: d.removed };
      })
      .catch(() => {
        /* stats are decorative; the diff below still renders */
      });
    return () => {
      gone = true;
    };
  });
;
async () => {

 { svelteHTML.createElement("div", {   "class":`wb-rd`,...__sveltets_2_empty({"data-testid":`wb-rev-diff`}),});
   { svelteHTML.createElement("div", { "class":`wb-rd-head`,});
     { svelteHTML.createElement("span", { "class":`wb-rd-side`,});fromLabel; }
     { svelteHTML.createElement("span", { "aria-hidden":`true`,});  }
     { svelteHTML.createElement("span", { "class":`wb-rd-side`,});toLabel; }
    if(stats){
       { svelteHTML.createElement("span", { "class":`wb-rd-stats`,});
         { svelteHTML.createElement("span", { "class":`add`,}); stats.added; }
         { svelteHTML.createElement("span", { "class":`del`,}); stats.removed; }
       }
    }
   }
  if(before === after){
     { svelteHTML.createElement("p", { "class":`wb-rd-same`,});  }
  }else{
     { svelteHTML.createElement("div", { "class":`wb-rd-body`,}); { const $$_weiVffiD2C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD2C({ target: __sveltets_2_any(), props: {      before,after,"mode":`line`,"contextLines":3,}});} }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RevisionDiff__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RevisionDiff__SvelteComponent_ = ReturnType<typeof RevisionDiff__SvelteComponent_>;
/*Ωignore_endΩ*/export default RevisionDiff__SvelteComponent_;