///<reference types="svelte" />
;// ListPane — the chrome shared by every "220px list + detail" tab on the
// Product page (ChatTab/RefineTab's chat lists, MockupsTab's mockup list):
// a bordered sidebar-style column with a title + action-buttons header and a
// scrollable body. The body renders `empty` INSTEAD OF `children` only when
// `isEmpty` is true and an `empty` snippet was actually passed — callers that
// fold loading/error states into their own `children` markup (e.g. RefineTab,
// which must keep its discovery-run picker visible even with zero threads)
// simply omit `isEmpty`/`empty` and keep full control.

import type { Snippet } from 'svelte';
;

  interface Props {
    title: string;
    width?: number;
    /** When true (and `empty` is given), render `empty` instead of `children`. */
    isEmpty?: boolean;
    actions?: Snippet;
    children?: Snippet;
    empty?: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { title, width = 220, isEmpty = false, actions, children, empty }: Props = $props();
;
async () => {

 { svelteHTML.createElement("aside", {   "class":`list-pane`,"style":`width: ${width}px`,});
   { svelteHTML.createElement("div", { "class":`pane-head`,});
     { svelteHTML.createElement("span", { "class":`pane-title`,});title; }
    if(actions){
       { svelteHTML.createElement("div", { "class":`pane-actions`,});;__sveltets_2_ensureSnippet(actions()); }
    }
   }
   { svelteHTML.createElement("div", { "class":`pane-body`,});
    if(isEmpty && empty){
      ;__sveltets_2_ensureSnippet(empty());
    } else if (children){
      ;__sveltets_2_ensureSnippet(children());
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ListPane__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ListPane__SvelteComponent_ = ReturnType<typeof ListPane__SvelteComponent_>;
/*Ωignore_endΩ*/export default ListPane__SvelteComponent_;