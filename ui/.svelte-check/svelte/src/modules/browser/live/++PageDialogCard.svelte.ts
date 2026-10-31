///<reference types="svelte" />
;;
  // A JavaScript dialog (alert / confirm / prompt / beforeunload) the remote
  // page opened. The page is blocked until it's answered (the daemon
  // auto-dismisses after 60 s). Drawn in the live pane, attributed to the
  // page's host — it is the SITE talking, never Otto, so it is not an Otto
  // Modal and its buttons don't use Otto's action verbs for Otto things.
  interface Props {
    kind: 'alert' | 'confirm' | 'prompt' | 'beforeunload';
    message: string;
    defaultPrompt: string;
    url: string;
    onanswer: (accept: boolean, promptText?: string) => void;
    /** Watch-only viewers can see it but not answer. */
    canAnswer: boolean;
  };function $$render() {


  let { kind, message, defaultPrompt, url, onanswer, canAnswer }: Props = $props();

  let text = $state('');
  $effect(() => {
    text = defaultPrompt;
  });

  const host = $derived.by(() => {
    try {
      return new URL(url).host || 'This page';
    } catch {
      return 'This page';
    }
  });
  let cardEl = $state<HTMLElement | null>(null);
  $effect(() => {
    const _m = message;
    cardEl?.focus({ preventScroll: true });
  });

  function onkeydown(e: KeyboardEvent): void {
    if (!canAnswer) return;
    if (e.key === 'Escape' && kind !== 'alert') {
      e.preventDefault();
      onanswer(false);
    }
  }
;
async () => {

  { const $$_div0 = svelteHTML.createElement("div", {              "class":`dialog`,"role":`alertdialog`,"aria-modal":`false`,"aria-label":`${host} says`,"tabindex":-1,onkeydown,...__sveltets_2_empty({"data-testid":`live-page-dialog`}),});cardEl = $$_div0;
   { svelteHTML.createElement("p", { "class":`from`,}); { svelteHTML.createElement("span", { "dir":`ltr`,});host; }  }
  if(kind === 'beforeunload'){
     { svelteHTML.createElement("p", { "class":`msg`,});          }
  }else{
     { svelteHTML.createElement("p", { "class":`msg`,});message; }
  }
  if(kind === 'prompt'){
     { svelteHTML.createElement("input", {          "dir":`auto`,"class":`input answer`,"bind:value":text,"aria-label":`Answer to the page`,"disabled":!canAnswer,});/*Ωignore_startΩ*/() => text = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  }
   { svelteHTML.createElement("div", { "class":`actions`,});
    if(!canAnswer){
       { svelteHTML.createElement("span", { "class":`dim`,});       }
    } else if (kind === 'alert'){
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => onanswer(true),});  }
    } else if (kind === 'beforeunload'){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => onanswer(false),});  }
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => onanswer(true),});  }
    } else if (kind === 'prompt'){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => onanswer(false),});  }
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => onanswer(true, text),});  }
    }else{
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => onanswer(false),});  }
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => onanswer(true),});  }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PageDialogCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PageDialogCard__SvelteComponent_ = ReturnType<typeof PageDialogCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default PageDialogCard__SvelteComponent_;