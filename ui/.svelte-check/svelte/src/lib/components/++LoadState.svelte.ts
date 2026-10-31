///<reference types="svelte" />
;// The one "design every state" wrapper for a loaded list/page: loading,
// error, empty and loaded — so a failed load can never masquerade as an
// empty list ("No X yet") and a first load never flashes the empty state.
//
//   <LoadState what="scheduled tasks" loading={s.loading} error={s.error}
//              empty={s.tasks.length === 0} onretry={() => s.load()}>
//     {#snippet emptyView()}<EmptyState title="No scheduled tasks yet" … />{/snippet}
//     …the list…
//   </LoadState>
//
// Precedence:
//   • error + nothing to show  → inline "Couldn’t load {what}" + detail + Retry
//   • error + stale data       → the data, with a slim "Couldn’t refresh" bar + Retry
//   • loading + nothing yet    → Skeleton after a ~150 ms grace (the height is
//                                reserved at once; a fast load never flashes it)
//   • empty                    → `emptyView` (or nothing)
//   • else                     → children
//
// `variant` matches EmptyState: `page` for a whole pane, `panel` inside a
// card/list, `compact` for a narrow rail (one line + Retry).

import type { Snippet } from 'svelte';
import Icon from './Icon.svelte';
import Skeleton from './Skeleton.svelte';
import { announceEdge } from '../announce.svelte';
;

  interface Props {
    /** Noun for the headline: "Couldn’t load {what}". */
    what: string;
    loading?: boolean;
    /** Human cause of the last failed load (see `loadErrorText`); null/'' = ok. */
    error?: string | null;
    /** Loaded, nothing to show. */
    empty?: boolean;
    onretry?: () => void;
    variant?: 'page' | 'panel' | 'compact';
    /** Skeleton rows on first load. */
    rows?: number;
    emptyView?: Snippet;
    /** Secondary recovery next to Retry on the error view (e.g. "Reload Otto"). */
    errorActions?: Snippet;
    children?: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { what, loading = false, error = null, empty = false, onretry, variant = 'panel', rows = 4, emptyView, errorActions, children }: Props = $props();

  // First-load skeleton waits ~150 ms so a fast response never flashes it; the
  // block keeps its height meanwhile so nothing below jumps when it appears.
  let showSkeleton = $state(false);
  const firstLoad = $derived(loading && empty && !error);
  $effect(() => {
    if (!firstLoad) {
      showSkeleton = false;
      return;
    }
    const t = setTimeout(() => (showSkeleton = true), 150);
    return () => clearTimeout(t);
  });
  const skelRows = $derived(variant === 'compact' ? 2 : rows);
  const skelHeight = $derived(variant === 'compact' ? 24 : 36);
  // rows * (row + 8 px gap) + the list's 4 px block padding.
  const reserve = $derived(skelRows * (skelHeight + 8) + 4);

  // The stale bar is inserted together with its text, and WebKit/VoiceOver
  // don't read a status region that way — so a refresh failure over stale data
  // is spoken through the shell's announcer instead, once per distinct error
  // (a re-render with the same error stays quiet; a recovery re-arms it).
  const sayStale = announceEdge();
  $effect(() => {
    sayStale(staleAnnouncement(what, error, empty), { assertive: true });
  });
  /** What the stale bar says to a screen reader; null while it isn't shown. */
  function staleAnnouncement(noun: string, err: string | null, nothing: boolean): string | null {
    return err && !nothing ? `Couldn’t refresh ${noun}. ${err}` : null;
  }
;
async () => {

if(error && empty){
  if(variant === 'compact'){
     { svelteHTML.createElement("div", {     "class":`ls-compact`,"role":`alert`,...__sveltets_2_empty({"data-testid":`load-error`}),});
       { svelteHTML.createElement("span", { "class":`ls-compact-text`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}   what;  { svelteHTML.createElement("span", { "class":`ls-detail`,});error; } }
      if(onretry){ { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":onretry,"disabled":loading,});loading ? 'Retrying…' : 'Retry'; }}
      if(errorActions){;__sveltets_2_ensureSnippet(errorActions());}
     }
  }else{
     { svelteHTML.createElement("div", {      "class":`ls-error`,"role":`alert`,...__sveltets_2_empty({"data-testid":`load-error`}),});variant === 'page';
       { svelteHTML.createElement("div", { "class":`ls-icon`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":variant === 'page' ? 26 : 24,}});} }
       { svelteHTML.createElement("h3", {});  what; }
       { svelteHTML.createElement("p", { "class":`ls-detail`,});error; }
      if(onretry || errorActions){
         { svelteHTML.createElement("div", { "class":`ls-actions`,});
          if(onretry){
             { svelteHTML.createElement("button", {     "class":`btn ls-retry`,"onclick":onretry,"disabled":loading,});
               { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
              loading ? 'Retrying…' : 'Retry';
             }
          }
          if(errorActions){;__sveltets_2_ensureSnippet(errorActions());}
         }
      }
     }
  }
} else if (loading && empty){
   { svelteHTML.createElement("div", {        "class":`ls-loading`,"role":`status`,"aria-label":`Loading ${what}`,"style":`min-block-size:${reserve}px`,});variant === 'page';
    
    if(showSkeleton){ { svelteHTML.createElement("span", { "class":`sr-only`,}); what;  } { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {        "announce":false,"grace":false,"rows":skelRows,"height":skelHeight,}});}}
   }
} else if (empty){
  if(emptyView){;__sveltets_2_ensureSnippet(emptyView());}
}else{
  if(error){
     { svelteHTML.createElement("div", {   "class":`ls-stale`,...__sveltets_2_empty({"data-testid":`load-stale`}),});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
       { svelteHTML.createElement("span", {   "class":`ls-stale-text`,"title":error,});  what;       error; }
      if(onretry){ { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":onretry,"disabled":loading,});  }}
     }
  }
  if(children){;__sveltets_2_ensureSnippet(children());}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LoadState__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LoadState__SvelteComponent_ = ReturnType<typeof LoadState__SvelteComponent_>;
/*Ωignore_endΩ*/export default LoadState__SvelteComponent_;