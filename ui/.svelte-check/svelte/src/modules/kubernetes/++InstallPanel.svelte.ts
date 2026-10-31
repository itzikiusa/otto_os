///<reference types="svelte" />
;// First-run installer for kubectl (full-page) and k9s (compact, inside a
// sheet). Admin-gated; progress comes from polling `/k8s/status` every
// 1.5 s while the job is `running` (the `k8s_install_updated` WS event also
// refetches). The kubectl panel auto-continues: once `status.kubectl.installed`
// flips, the page's `needsInstall` derived turns false and the module renders.

import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { liveQuery } from '../../lib/live';
import { k8s } from '../../lib/stores/k8s.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import type { K8sTool } from '../../lib/api/types';
;

  interface Props {
    tool: K8sTool;
    /** Rendered inside a sheet: no page chrome, no "continue" link. */
    compact?: boolean;
    oncontinue?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { tool, compact = false, oncontinue }: Props = $props();

  const POLL_MS = 1500;
  const job = $derived(k8s.status?.install[tool] ?? null);
  const installed = $derived(k8s.status?.[tool].installed ?? false);
  const canInstall = $derived(auth.can('kubernetes', 'admin'));
  let starting = $state(false);
  let logOpen = $state(false);

  const label = $derived(tool === 'kubectl' ? 'kubectl' : 'k9s');
  const blurb = $derived(
    tool === 'kubectl'
      ? 'Otto drives clusters through the kubectl CLI. It is not installed on this Mac yet — Otto can install it for you (Homebrew when available, otherwise a direct download into Otto’s bin directory; never sudo).'
      : 'k9s is a terminal UI for Kubernetes. Otto can install it on demand (Homebrew when available, otherwise the GitHub release tarball into Otto’s bin directory).',
  );

  // Refresh while running. The dependency is the job state only; the fetch is
  // untracked so the status refresh never re-arms the chain mid-tick.
  // `k8s_install_updated` already re-reads the status (k8s.applyEvent) while
  // the event socket is up — this is a 10 s safety net then; POLL_MS only
  // while it is down.
  $effect(() => {
    if (job?.state !== 'running') return;
    const p = liveQuery({ run: () => untrack(() => k8s.loadStatus()), on: [], fallbackMs: POLL_MS, safetyMs: 10_000, immediate: false });
    return () => p.stop();
  });

  // Announce the terminal states once.
  let announced = $state<string | null>(null);
  $effect(() => {
    const st = job?.state;
    if (!st || st === 'running' || st === 'idle') return;
    const key = `${st}:${job?.finished_at ?? ''}`;
    if (untrack(() => announced) === key) return;
    announced = key;
    if (st === 'done') toasts.success(`${label} installed`);
    else if (st === 'failed') toasts.error(`Couldn’t install ${label}`, job?.error ?? undefined);
  });

  async function start(): Promise<void> {
    starting = true;
    try {
      await k8s.install(tool);
      logOpen = true;
    } catch (e) {
      toastError('Couldn’t start the install', e);
    } finally {
      starting = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`install`,...__sveltets_2_empty({"data-testid":`k8s-install-panel`}),});compact;
   { svelteHTML.createElement("div", { "class":`install-head`,});
     { svelteHTML.createElement("div", { "class":`install-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":tool === 'kubectl' ? 'helm' : 'terminal',"size":compact ? 16 : 26,}});} }
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("h2", {});installed ? `${label} is installed` : `Otto needs ${label}`; }
      if(!installed){ { svelteHTML.createElement("p", { "class":`sub`,});blurb; }}
     }
   }

  if(installed){
     { svelteHTML.createElement("div", { "class":`install-ok`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`mono`,});k8s.status?.[tool].version ?? ''; }
      if(k8s.status?.[tool].path){ { svelteHTML.createElement("span", { "class":`dim mono`,});k8s.status?.[tool].path; }}
     }
  } else if (job?.state === 'running'){
     { svelteHTML.createElement("div", {       "class":`indeterminate`,"role":`progressbar`,"aria-label":`Installing ${label}`,"aria-busy":`true`,}); }
     { svelteHTML.createElement("div", { "class":`dim`,});          }
  } else if (job?.state === 'failed'){
     { svelteHTML.createElement("div", { "class":`fail`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("span", {});  label;job.error ? `: ${job.error}` : '';  }
     }
  }

  if(!installed){
     { svelteHTML.createElement("div", { "class":`actions`,});
      if(canInstall){
         { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":() => void start(),"disabled":starting || job?.state === 'running',});
          job?.state === 'running' ? 'Installing…' : job?.state === 'failed' ? 'Retry install' : `Install ${label}`;
         }
      }else{
         { svelteHTML.createElement("span", { "class":`dim`,});      label;      }
      }
       { svelteHTML.createElement("button", {     "class":`btn ghost`,"onclick":() => void k8s.loadStatus(),"title":`Re-check`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} 
       }
      if(!compact && oncontinue){
         { svelteHTML.createElement("button", {   "class":`btn ghost dim`,"onclick":oncontinue,});   }
      }
     }
  }

  if(job?.log_tail){
     { svelteHTML.createElement("details", {   "class":`log`,"bind:open":logOpen,});/*Ωignore_startΩ*/() => logOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/
       { svelteHTML.createElement("summary", {});  }
       { svelteHTML.createElement("pre", { "class":`mono`,});job.log_tail; }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const InstallPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type InstallPanel__SvelteComponent_ = ReturnType<typeof InstallPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default InstallPanel__SvelteComponent_;