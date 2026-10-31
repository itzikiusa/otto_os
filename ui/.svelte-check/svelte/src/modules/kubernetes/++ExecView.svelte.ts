///<reference types="svelte" />
;
import { NO_WORKSPACE } from '../../lib/labels';
// Drawer "Terminal" tab: opens a `kubectl exec -it` terminal session for the pod
// (`POST …/exec`, Edit) and renders it inline with `<LazyTerminal preferDom>`
// (agent-TUI renderer; shells in a pod redraw prompts constantly). The
// session is killed when the view unmounts — it lives only in this drawer.
import { untrack } from 'svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { api } from '../../lib/api/client';
import { k8sApi } from '../../lib/api/k8s';
import { ws } from '../../lib/stores/workspace.svelte';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { k8s } from '../../lib/stores/k8s.svelte';
import { confirmProd, isProdEnv } from '../../lib/confirmProd';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import { clusterLabel } from './k8s-util';
import type { K8sContainer, SessionStatus } from '../../lib/api/types';
;

  interface Props {
    clusterId: string;
    ns: string;
    pod: string;
    containers: K8sContainer[];
    /** Open the shell immediately (the `s` shortcut). */
    autoOpen?: boolean;
    /** Called once `autoOpen` was acted on, so the owner can consume the flag —
     *  a later remount (switching back to the Terminal tab) must not open a
     *  second shell unasked. */
    onautoopened?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { clusterId, ns, pod, containers, autoOpen = false, onautoopened }: Props = $props();

  $effect(() => { void resourceAccess.load('k8s_cluster', clusterId, `namespace:${ns}`); });
  const canExec = $derived(resourceAccess.can('k8s_cluster', clusterId, 'exec', 'kubernetes', 'edit', `namespace:${ns}`));
  let container = $state('');
  let sessionId = $state<string | null>(null);
  let status = $state<SessionStatus | null>(null);
  let opening = $state(false);
  /** False once the view unmounted: an exec POST that resolves after that
   *  must delete its session — nothing is left to show (or close) it, and a
   *  leaked `kubectl exec -it` PTY would run on the daemon until it exits. */
  let alive = true;
  let error = $state('');

  const running = $derived(containers.filter((c) => !c.init));
  const cluster = $derived(k8s.clusters.find((c) => c.id === clusterId) ?? null);
  const isProd = $derived(isProdEnv(cluster?.environment));

  async function open(): Promise<void> {
    if (!canExec || opening) return;
    const wsId = ws.currentId;
    if (!wsId) {
      error = `${NO_WORKSPACE} — the exec session is attached to it.`;
      return;
    }
    // A shell in a production pod can change anything in it — ask first.
    if (isProd) {
      const ok = await confirmProd({
        env: cluster?.environment,
        where: `pod “${pod}” · namespace ${ns} · ${cluster ? clusterLabel(cluster) : 'this cluster'}`,
        verb: 'Open shell',
        what: 'An interactive shell. Anything you type runs inside the live container.',
        title: 'Open a production shell?',
        danger: true,
      });
      if (!ok || !alive) return;
    }
    opening = true;
    error = '';
    try {
      const s = await k8sApi.exec(clusterId, {
        workspace_id: wsId,
        ns,
        pod,
        container: container || null,
      });
      if (!alive) {
        void api.del(`/sessions/${s.id}`).catch(() => {});
        return;
      }
      sessionId = s.id;
    } catch (e) {
      if (alive) error = e instanceof Error ? e.message : String(e);
    } finally {
      opening = false;
    }
  }

  async function close(): Promise<void> {
    const id = sessionId;
    sessionId = null;
    status = null;
    if (!id) return;
    try {
      await api.del(`/sessions/${id}`);
    } catch {
      /* best-effort — the terminal session is torn down when the daemon notices anyway */
    }
  }

  // Auto-open once when asked (read untracked: a later `autoOpen` flip must
  // not re-run this and close a live shell); kill the session on unmount.
  $effect(() => {
    alive = true;
    if (untrack(() => autoOpen)) {
      untrack(() => {
        onautoopened?.();
        void open();
      });
    }
    return () => {
      alive = false;
      untrack(() => void close());
    };
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`exec`,});
  if(!canExec){
     { svelteHTML.createElement("div", { "class":`note`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":13,}});}        { svelteHTML.createElement("b", {});  }  }
  } else if (sessionId){
     { svelteHTML.createElement("div", { "class":`exec-bar`,});
       { svelteHTML.createElement("span", { "class":`mono`,});pod;container ? ` · ${container}` : ''; }
      if(cluster){ { const $$_egdaBvnE2C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE2C({ target: __sveltets_2_any(), props: {  "env":cluster.environment,}});}}
       { svelteHTML.createElement("span", { "class":`dim`,});status ?? ''; }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void close(),});  }
     }
     { svelteHTML.createElement("div", { "class":`term`,});
      sessionId; {
         { const $$_lanimreTyzaL2C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL2C({ target: __sveltets_2_any(), props: {     sessionId,"preferDom":true,"autoFocus":true,"restartable":true,"onrestart":() => { void close().then(open); },"onstatus":(s) => (status = s),}});}
      }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`launch`,});
       { svelteHTML.createElement("p", { "class":`dim`,});  { svelteHTML.createElement("span", { "class":`mono`,});   pod;   }            if(cluster){  { const $$_egdaBvnE3C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE3C({ target: __sveltets_2_any(), props: {  "env":cluster.environment,}});}} }
      if(running.length > 1){
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", { "class":`lbl`,});  }
           { svelteHTML.createElement("select", {   "class":`input`,"bind:value":container,});/*Ωignore_startΩ*/() => container = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",}); running[0]?.name;  }
               for(let c of __sveltets_2_ensureArray(running)){c.name; { svelteHTML.createElement("option", { "value":c.name,});c.name; }}
           }
         }
      }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":() => void open(),"disabled":opening,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":13,}});} opening ? 'Opening…' : 'Open shell';
       }
      if(error){ { svelteHTML.createElement("div", { "class":`err`,});error; }}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ExecView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ExecView__SvelteComponent_ = ReturnType<typeof ExecView__SvelteComponent_>;
/*Ωignore_endΩ*/export default ExecView__SvelteComponent_;