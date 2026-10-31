///<reference types="svelte" />
;// Shared service-view toolbar: title, client-side filter box, refresh +
// auto-refresh (10 s; 30 s across all regions) toggle, plus a slot for view-specific controls (region
// switcher, state filter…). `/` focuses the filter from anywhere in the view.

import type { Snippet } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import { pollWhileVisible } from '../../lib/poll';
import { AUTO_REFRESH_MS, effectiveRefreshMs } from './refresh';
;

  interface Props {
    title: string;
    subtitle?: string;
    filter?: string;
    filterPlaceholder?: string;
    loading?: boolean;
    auto?: boolean;
    /** The view's region — `ALL_REGIONS` floors the auto-refresh cadence. */
    region?: string;
    /** Return the load's promise so auto-refresh chains after it settles. */
    onrefresh?: () => void | Promise<unknown>;
    children?: Snippet;
    actions?: Snippet;
  };function $$render() {

  
  
  
  
  
  
  

  let {
    title,
    subtitle = '',
    filter = $bindable(''),
    filterPlaceholder = 'Filter…',
    loading = false,
    auto = $bindable(false),
    region,
    onrefresh,
    children,
    actions,
  }: Props = $props()/*Ωignore_startΩ*/;filter;auto;/*Ωignore_endΩ*/;

  let filterEl = $state<HTMLInputElement | null>(null);

  // Auto-refresh: re-armed whenever the toggle flips or the cadence changes;
  // stopped on unmount. A chain (next tick 10 s after the previous refresh
  // SETTLES), paused while the window is hidden — an interval stacked slow
  // `aws` CLI refreshes. All-regions mode spawns ~17 processes per tick, so
  // its cadence is floored at 30 s.
  const refreshMs = $derived(effectiveRefreshMs(AUTO_REFRESH_MS, region));
  $effect(() => {
    if (!auto || !onrefresh) return;
    const fn = onrefresh;
    const ms = refreshMs;
    const poller = pollWhileVisible(async () => { await fn(); }, { ms, immediate: false });
    return () => poller.stop();
  });

  function onKey(e: KeyboardEvent): void {
    if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (t?.closest('.cm-editor')) return;
    e.preventDefault();
    filterEl?.focus();
  }
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKey,});}

 { svelteHTML.createElement("div", { "class":`vt`,});
   { svelteHTML.createElement("div", { "class":`vt-title`,});
     { svelteHTML.createElement("h2", {});title; }
    if(subtitle){ { svelteHTML.createElement("span", { "class":`sub`,});subtitle; }}
   }
  if(children){ { svelteHTML.createElement("div", { "class":`vt-extra`,});;__sveltets_2_ensureSnippet(children()); }}
   { svelteHTML.createElement("label", { "class":`vt-filter`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
     { const $$_input2 = svelteHTML.createElement("input", {           "dir":`ltr`,"bind:value":filter,"type":`search`,"placeholder":filterPlaceholder,"aria-label":filterPlaceholder,});filterEl = $$_input2;/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }
  if(actions){;__sveltets_2_ensureSnippet(actions());}
  if(onrefresh){
     { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => onrefresh(),"title":`Refresh`,"aria-label":`Refresh`,"disabled":loading,});
      if(loading){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 14px`,"aria-hidden":`true`,}); }}else{ { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}}
     }
     { svelteHTML.createElement("label", {   "class":`auto`,"title":`Auto-refresh every ${refreshMs / 1000} s${refreshMs > AUTO_REFRESH_MS ? ' (all regions)' : ''}`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":auto,});/*Ωignore_startΩ*/() => auto = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", {});  }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('filter', 'auto'), slots: {}, events: {} }}
const ViewToolbar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ViewToolbar__SvelteComponent_ = ReturnType<typeof ViewToolbar__SvelteComponent_>;
/*Ωignore_endΩ*/export default ViewToolbar__SvelteComponent_;