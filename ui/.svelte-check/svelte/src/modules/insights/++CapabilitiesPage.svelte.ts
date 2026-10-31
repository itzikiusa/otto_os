///<reference types="svelte" />
;// Capability & Health Registry (B3) — one page answering "what can Otto do
// right now, what's degraded, and how do I fix it?". Root-only: backed by
// GET /capabilities (5 s cached on the server) and GET /support-bundle.

import { capabilitiesApi, depKindLabel, featureLabel, settingsRoute, statusLabel } from './capabilities';
import type { ModuleCapability } from './capabilities';
import { router } from '../../lib/router.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { CapabilityStatus } from './capabilities';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  /** Capability status → the shared status tone (green / amber / neutral). */
  function tone(s: CapabilityStatus): 'success' | 'warning' | 'neutral' {
    return s === 'ready' ? 'success' : s === 'degraded' ? 'warning' : 'neutral';
  }

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let caps: ModuleCapability[] = $state([]);
  let loading = $state(true);
  /** Last load failure — shown inline (with Retry) instead of a toast plus a
   *  generic "no capability data" page that hid the actual reason. */
  let loadErr = $state('');
  /** Which feature is expanded (showing dep breakdown). */
  let expanded = $state<Set<string>>(new Set());

  // ---------------------------------------------------------------------------
  // Load on mount
  // ---------------------------------------------------------------------------

  let loaded = false;
  $effect(() => {
    if (loaded) return;
    loaded = true;
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadErr = '';
    try {
      caps = await capabilitiesApi.list();
    } catch (e) {
      loadErr = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // UI helpers
  // ---------------------------------------------------------------------------

  function toggle(feature: string): void {
    const next = new Set(expanded);
    if (next.has(feature)) next.delete(feature);
    else next.add(feature);
    expanded = next;
  }

  /** Count of ready / degraded / missing_setup modules. */
  const summary = $derived({
    ready: caps.filter((c) => c.status === 'ready').length,
    degraded: caps.filter((c) => c.status === 'degraded').length,
    missing: caps.filter((c) => c.status === 'missing_setup').length,
  });

  /** Sort: degraded first, then missing_setup, then ready. */
  const sorted = $derived(
    [...caps].sort((a, b) => {
      const order = { degraded: 0, missing_setup: 1, ready: 2 };
      return (order[a.status] ?? 99) - (order[b.status] ?? 99);
    }),
  );
;
async () => {


 { svelteHTML.createElement("div", { "class":`caps`,});

  if(!loading && caps.length > 0){
    
     { svelteHTML.createElement("div", { "class":`summary-row`,});
      if(summary.degraded > 0){
         { const $$_egdaBsutatS2C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS2C({ target: __sveltets_2_any(), props: {    "tone":`warning`,"label":`${summary.degraded} degraded`,}});}
      }
      if(summary.missing > 0){
         { const $$_egdaBsutatS2C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS2C({ target: __sveltets_2_any(), props: {    "tone":`neutral`,"label":`${summary.missing} not set up`,}});}
      }
      if(summary.ready > 0){
         { const $$_egdaBsutatS2C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS2C({ target: __sveltets_2_any(), props: {    "tone":`success`,"label":`${summary.ready} ready`,}});}
      }
     }
  }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"what":`capabilities`,"variant":`page`,loading,"error":loadErr || null,"empty":caps.length === 0,"onretry":load,"rows":5,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {         "icon":`gauge`,"variant":`page`,"title":`No capability data`,"body":`The daemon reported no capability information. The Health view needs the root account.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`cap-list`,});
         for(let cap of __sveltets_2_ensureArray(sorted)){cap.feature;
        const open = expanded.has(cap.feature);
         { svelteHTML.createElement("div", {  "class":`cap-card`,});cap.status === 'degraded';
           { svelteHTML.createElement("div", { "class":`cap-head`,});
             { svelteHTML.createElement("button", {       "class":`cap-toggle`,"aria-expanded":open,"aria-controls":`deps-${cap.feature}`,"onclick":() => toggle(cap.feature),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
               { svelteHTML.createElement("span", {   "class":`feature-label`,"title":featureLabel(cap.feature),});featureLabel(cap.feature); }
               { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "tone":tone(cap.status),"label":statusLabel(cap.status),}});}
              if(cap.deps.length > 0){
                 { svelteHTML.createElement("span", { "class":`dep-count dim`,});
                  cap.deps.length; cap.deps.length !== 1 ? 'checks' : 'check';
                 }
              }
             }
            
             { svelteHTML.createElement("span", { "class":`fix-slot`,});
              if(cap.status !== 'ready'){
                
                 { svelteHTML.createElement("button", {     "class":`btn small`,"title":`Open the settings that fix ${featureLabel(cap.feature)}`,"onclick":() => router.go(settingsRoute(cap.feature)),});
                   { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});}
                    
                 }
              }
             }
           }

          
          if(cap.reasons.length > 0){
             { svelteHTML.createElement("ul", { "class":`cap-issues`,});
                  for(let reason of __sveltets_2_ensureArray(cap.reasons)){let i = 1;reason;
                 { svelteHTML.createElement("li", { "class":`issue-row`,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}
                   { svelteHTML.createElement("div", { "class":`issue-text`,});
                     { svelteHTML.createElement("span", { "class":`issue-reason`,});reason; }
                    if(cap.fixes[i]){
                       { svelteHTML.createElement("span", { "class":`issue-fix dim`,});cap.fixes[i]; }
                    }
                   }
                 }
              }
             }
          }

          
          if(open && cap.deps.length > 0){
             { svelteHTML.createElement("ul", {   "class":`dep-list`,"id":`deps-${cap.feature}`,});
                 for(let dep of __sveltets_2_ensureArray(cap.deps)){dep.name + dep.kind;
                 { svelteHTML.createElement("li", { "class":`dep-row`,});
                   { svelteHTML.createElement("span", {        "class":`dep-ok`,"role":`img`,"aria-label":dep.ok ? 'OK' : 'Not OK',"title":dep.ok ? 'OK' : 'Not OK',});!dep.ok;
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":dep.ok ? 'check' : 'x',"size":12,}});}
                   }
                   { svelteHTML.createElement("span", { "class":`dep-kind dim`,});depKindLabel(dep.kind); }
                   { svelteHTML.createElement("span", { "class":`dep-name`,});dep.name; }
                  if(dep.detail){
                     { svelteHTML.createElement("span", {   "class":`dep-detail dim`,"title":dep.detail,});dep.detail; }
                  }
                 }
              }
             }
          }
         }
      }
     }
   LoadState}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CapabilitiesPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CapabilitiesPage__SvelteComponent_ = ReturnType<typeof CapabilitiesPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default CapabilitiesPage__SvelteComponent_;