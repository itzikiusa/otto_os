///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// Drawer "HTTP" tab (K-3): call an HTTP endpoint inside one pod or every pod
// of a workload — e.g. Spring Boot actuator loggers — through the daemon's
// kubectl-proxy gateway (port-forward fallback). Left: saved per-workload
// actions + built-in actuator presets. Right: method / port / path /
// headers / body with `{{var}}` inputs, a target pick and Run; results come
// back one row per pod. Mutating calls on a prod (or read-only) cluster ask
// for the target name typed back; the daemon enforces it too, and audits
// every call (`k8s.pod_http`, body hash only).
import { untrack } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { copyText } from '../../lib/clipboard';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ApiError } from '../../lib/api/client';
import { k8sApi } from '../../lib/api/k8s';
import { k8s } from '../../lib/stores/k8s.svelte';
import type { K8sPodAction, K8sPodHttpMethod, K8sPodHttpReq, K8sPodHttpResult } from '../../lib/api/types';
import { confirmProd, isProdEnv } from '../../lib/confirmProd';
import { clusterLabel } from './k8s-util';
import { ACTUATOR_PRESETS, LOG_LEVELS, defaultPort, fillTemplate, pathProblem, prettyBody, templateVars } from './podHttp';
;

  interface Props {
    clusterId: string;
    ns: string;
    /** Set for a pod drawer. */
    pod?: string;
    /** The workload this drawer is (or the pod belongs to), when known —
     *  `kind` is the pod-http workload kind (deployment, statefulset, …). */
    workload?: { kind: string; name: string } | null;
    /** Pod or workload manifest — for the default port guess. */
    manifest?: unknown;
    /** May the person send mutating methods (Edit + the `exec` op — the daemon's rule)? */
    canMutate: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { clusterId, ns, pod, workload = null, manifest = null, canMutate }: Props = $props();

  const METHODS: K8sPodHttpMethod[] = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];
  const cluster = $derived(k8s.clusters.find((c) => c.id === clusterId) ?? null);
  const guarded = $derived(isProdEnv(cluster?.environment) || !!(cluster as { read_only?: boolean } | null)?.read_only);

  let method = $state<K8sPodHttpMethod>('GET');
  let port = $state(untrack(() => defaultPort(manifest)));
  let path = $state('/actuator/health');
  let headersText = $state('');
  let body = $state('');
  let vars = $state<Record<string, string>>({ level: 'DEBUG' });
  let target = $state<'pod' | 'workload'>(untrack(() => (pod ? 'pod' : 'workload')));
  let loadedFrom = $state<string | null>(null);

  let running = $state(false);
  let runError = $state('');
  let results = $state<K8sPodHttpResult[] | null>(null);
  let openRow = $state<string | null>(null);

  // --- saved actions ---------------------------------------------------------
  let saved = $state<K8sPodAction[]>([]);
  let savedError = $state('');
  let savedLoading = $state(false);
  // Latest-wins: switching workload / namespace / cluster while a list is in
  // flight must not show the previous target's saved actions.
  let savedSeq = 0;
  /** ux-6-07: a refused copy says what to do next (it was silent). */
  async function copyBody(text: string): Promise<void> {
    if (await copyText(text)) toasts.success('Copied response body');
    else toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
  }
  async function loadSaved(): Promise<void> {
    const seq = ++savedSeq;
    if (!workload) return;
    savedLoading = true;
    try {
      const r = await k8sApi.podActions(clusterId, { namespace: ns, workload_kind: workload.kind, workload: workload.name });
      if (seq !== savedSeq) return;
      saved = r.actions;
      savedError = '';
    } catch (e) {
      if (seq !== savedSeq) return;
      savedError = loadErrorText(e);
    } finally {
      if (seq === savedSeq) savedLoading = false;
    }
  }
  $effect(() => {
    void clusterId;
    void ns;
    void workload?.name;
    untrack(() => void loadSaved());
  });

  const varNames = $derived(templateVars(path, method === 'GET' ? '' : body));
  const filledPath = $derived(fillTemplate(path, vars));
  const problem = $derived(pathProblem(filledPath) ?? (varNames.some((v) => !(vars[v] ?? '').trim()) ? 'Fill in every {{variable}}' : null));
  const mutating = $derived(method !== 'GET');
  const blockedByRole = $derived(mutating && !canMutate);
  const targetName = $derived(target === 'pod' && pod ? pod : (workload?.name ?? pod ?? ''));

  function parseHeaders(text: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of text.split('\n')) {
      const i = line.indexOf(':');
      if (i <= 0) continue;
      const k = line.slice(0, i).trim();
      if (k) out[k] = line.slice(i + 1).trim();
    }
    return out;
  }
  function headersToText(h: Record<string, string> | undefined): string {
    return Object.entries(h ?? {}).map(([k, v]) => `${k}: ${v}`).join('\n');
  }

  function applyPreset(p: { id: string; name: string; method: K8sPodHttpMethod; path: string; headers?: Record<string, string>; body?: string | null; port?: number }): void {
    method = p.method;
    path = p.path;
    headersText = headersToText(p.headers);
    body = p.body ?? '';
    if (p.port) port = p.port;
    loadedFrom = p.id;
    results = null;
    runError = '';
  }

  async function run(): Promise<void> {
    if (problem || running || blockedByRole) return;
    const req: K8sPodHttpReq = {
      namespace: ns,
      port: Number(port),
      method,
      path: filledPath,
      headers: parseHeaders(headersText),
      body: mutating && body.trim() ? fillTemplate(body, vars) : null,
    };
    if (target === 'pod' && pod) req.pod = pod;
    else if (workload) req.workload = workload;
    else if (pod) req.pod = pod;
    if (mutating && guarded) {
      const scope = req.pod ? `pod ${req.pod}` : `every pod of ${workload?.kind} ${workload?.name}`;
      const where = `${scope}${ns ? ` · namespace ${ns}` : ''} · ${clusterLabel(cluster)}${isProdEnv(cluster?.environment) ? '' : ' (read-only cluster)'}`;
      const ok = await confirmProd({
        env: cluster?.environment,
        where,
        verb: `Send ${method}`,
        what: `${method} ${req.path}`,
        typed: targetName,
        title: `${method} to pods?`,
        danger: true,
      });
      if (!ok) return;
      req.confirm_name = targetName;
    }
    running = true;
    runError = '';
    try {
      const r = await k8sApi.podHttp(clusterId, req);
      results = r.results;
      openRow = r.results.length === 1 ? r.results[0].pod : null;
      const failed = r.results.filter((x) => x.error || (x.status ?? 0) >= 400).length;
      if (mutating) {
        if (failed) toasts.warn(`${method} ${req.path}`, `${failed} of ${plural(r.results.length, 'pod')} failed`);
        else toasts.success(`${method} ${req.path}`, `${plural(r.results.length, 'pod')} answered`);
      }
    } catch (e) {
      runError = e instanceof ApiError && e.status === 409 ? `The daemon wants the target name confirmed: ${e.message}` : e instanceof Error ? e.message : String(e);
    } finally {
      running = false;
    }
  }

  async function saveAction(): Promise<void> {
    if (!workload) return;
    const existing = saved.find((s) => s.id === loadedFrom);
    const name = await confirmer.promptText(`Save this request for ${workload.kind} ${workload.name}. {{variables}} stay as placeholders.`, {
      title: existing ? 'Update saved action' : 'Save action',
      confirmLabel: 'Save',
      initial: existing?.name ?? ACTUATOR_PRESETS.find((p) => p.id === loadedFrom)?.name ?? '',
      placeholder: 'e.g. Billing logger → DEBUG',
    });
    if (!name) return;
    try {
      const a = await k8sApi.savePodAction(clusterId, {
        id: existing?.id,
        namespace: ns,
        workload_kind: workload.kind,
        workload: workload.name,
        name,
        method,
        port: Number(port),
        path,
        headers: parseHeaders(headersText),
        body_template: mutating && body.trim() ? body : null,
      });
      saved = [...saved.filter((s) => s.id !== a.id), a].sort((x, y) => x.name.localeCompare(y.name));
      loadedFrom = a.id;
      toasts.success('Action saved', a.name);
    } catch (e) {
      toastError("Couldn’t save the action", e);
    }
  }

  async function deleteAction(a: K8sPodAction): Promise<void> {
    const ok = await confirmer.ask(`Delete the saved action “${a.name}” for ${a.workload}?`, { title: 'Delete action', confirmLabel: 'Delete' });
    if (!ok) return;
    try {
      await k8sApi.deletePodAction(clusterId, a.id);
      saved = saved.filter((s) => s.id !== a.id);
      if (loadedFrom === a.id) loadedFrom = null;
    } catch (e) {
      toastError("Couldn’t delete the action", e);
    }
  }

  function statusTone(r: K8sPodHttpResult): string {
    if (r.error || r.status === null) return 'bad';
    if (r.status >= 500) return 'bad';
    if (r.status >= 400) return 'warn';
    return 'ok';
  }
  function shownBody(r: K8sPodHttpResult): string {
    if (r.body_base64) return `(binary body, ${r.body.length} base64 chars)`;
    return prettyBody(r.body);
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (runError) announce(runError, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", {   "class":`ph`,...__sveltets_2_empty({"data-testid":`k8s-pod-http`}),});
   { svelteHTML.createElement("aside", {   "class":`ph-side`,"aria-label":`Saved actions and presets`,});
    if(workload){
       { svelteHTML.createElement("div", { "class":`ph-sec`,});  workload.name; }
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`saved actions`,"variant":`compact`,"loading":savedLoading,"error":savedError,"empty":!saved.length,"onretry":() => void loadSaved(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", { "class":`dim small pad`,});         }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
        
           for(let a of __sveltets_2_ensureArray(saved)){a.id;
           { svelteHTML.createElement("div", { "class":`ph-item-row`,});
             { svelteHTML.createElement("button", {      "class":`ph-item`,"onclick":() => applyPreset({ ...a, body: a.body_template }),"title":`${a.method} :${a.port}${a.path}`,});loadedFrom === a.id;
               { svelteHTML.createElement("span", { "class":`m mono ${a.method.toLowerCase()}`,});a.method; } { svelteHTML.createElement("span", { "class":`ell`,});a.name; }
             }
            if(canMutate){ { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void deleteAction(a),"aria-label":`Delete ${a.name}`,"title":`Delete saved action`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }}
           }
        }
       LoadState}
    }
     { svelteHTML.createElement("div", { "class":`ph-sec`,});   }
       for(let p of __sveltets_2_ensureArray(ACTUATOR_PRESETS)){p.id;
       { svelteHTML.createElement("button", {        "class":`ph-item`,"onclick":() => applyPreset(p),"title":p.hint,...__sveltets_2_empty({"data-testid":`k8s-pod-http-preset-${p.id}`}),});loadedFrom === p.id;
         { svelteHTML.createElement("span", { "class":`m mono ${p.method.toLowerCase()}`,});p.method; } { svelteHTML.createElement("span", { "class":`ell`,});p.name; }
       }
    }
   }

   { svelteHTML.createElement("section", { "class":`ph-main`,});
     { svelteHTML.createElement("div", { "class":`ph-line`,});
       { svelteHTML.createElement("select", {     "class":`input meth`,"bind:value":method,"aria-label":`HTTP method`,});/*Ωignore_startΩ*/() => method = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let m of __sveltets_2_ensureArray(METHODS)){m; { svelteHTML.createElement("option", { "value":m,});m; }}
       }
       { svelteHTML.createElement("input", {              "class":`input port mono`,"type":`number`,"min":`1`,"max":`65535`,"bind:value":port,"aria-label":`Container port`,"title":`Container port`,});/*Ωignore_startΩ*/() => port = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input path mono`,"bind:value":path,"placeholder":`/actuator/health`,"aria-label":`Path`,...__sveltets_2_empty({"data-testid":`k8s-pod-http-path`}),"onkeydown":(e) => { if (e.key === 'Enter') void run(); },});/*Ωignore_startΩ*/() => path = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

    if(varNames.length){
       { svelteHTML.createElement("div", { "class":`ph-vars`,});
           for(let v of __sveltets_2_ensureArray(varNames)){v;
           { svelteHTML.createElement("label", { "class":`var`,});
             { svelteHTML.createElement("span", { "class":`dim small mono`,});v; }
            if(v === 'level'){
               { svelteHTML.createElement("select", {     "class":`input`,"bind:value":vars[v],"aria-label":`Log level`,});/*Ωignore_startΩ*/() => vars[v] = __sveltets_2_any(null);/*Ωignore_endΩ*/
                   for(let l of __sveltets_2_ensureArray(LOG_LEVELS)){l; { svelteHTML.createElement("option", { "value":l,});l === 'RESET' ? 'RESET (clear override)' : l; }}
               }
            }else{
               { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input mono`,"bind:value":vars[v],"placeholder":v === 'logger' ? 'com.example.service' : v,"aria-label":v,});/*Ωignore_startΩ*/() => vars[v] = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            }
           }
        }
       }
    }

     { svelteHTML.createElement("details", {   "class":`ph-more`,"open":!!headersText || mutating,});
       { svelteHTML.createElement("summary", {}); mutating ? ' & body' : ''; }
       { svelteHTML.createElement("textarea", {           "dir":`ltr`,"class":`input mono`,"rows":2,"bind:value":headersText,"placeholder":`Accept: application/json`,"aria-label":`Headers, one per line`,});/*Ωignore_startΩ*/() => headersText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
      if(mutating){
         { svelteHTML.createElement("textarea", {           "dir":`ltr`,"class":`input mono`,"rows":4,"bind:value":body,"placeholder":'{"configuredLevel":"DEBUG"}',"aria-label":`Request body`,});/*Ωignore_startΩ*/() => body = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
      }
     }

     { svelteHTML.createElement("div", { "class":`ph-run`,});
      if(pod && workload){
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Target`,});
           { svelteHTML.createElement("button", {     "aria-pressed":target === 'pod',"onclick":() => (target = 'pod'),});target === 'pod';  }
           { svelteHTML.createElement("button", {     "aria-pressed":target === 'workload',"onclick":() => (target = 'workload'),});target === 'workload';   workload.name; }
         }
      }else{
         { svelteHTML.createElement("span", { "class":`dim small`,});pod ? `Pod ${pod}` : workload ? `Every pod of ${workload.name}` : ''; }
      }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
      if(workload && canMutate){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void saveAction(),});  }}
       { svelteHTML.createElement("button", {          "class":`btn small primary`,"onclick":() => void run(),"disabled":!!problem || running || blockedByRole,"title":blockedByRole ? 'Sending a mutating request needs Edit on this cluster' : (problem ?? 'Send (Enter in the path)'),...__sveltets_2_empty({"data-testid":`k8s-pod-http-run`}),});mutating && guarded;
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":12,}});} running ? 'Sending…' : 'Send';
       }
     }
    if(problem && path){ { svelteHTML.createElement("div", { "class":`dim small`,});problem; }}
    if(mutating && guarded){ { svelteHTML.createElement("div", {   "class":`guard small`,"role":`note`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} isProdEnv(cluster?.environment) ? 'Production' : 'Read-only';           }}

    if(runError){
       { svelteHTML.createElement("div", { "class":`err`,});runError; }
    } else if (results && !results.length){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`box`,"title":`No pods matched`,"body":`The workload has no running pods to call.`,}});}
    } else if (results){
       { svelteHTML.createElement("ul", {   "class":`ph-results`,...__sveltets_2_empty({"data-testid":`k8s-pod-http-results`}),});
           for(let r of __sveltets_2_ensureArray(results)){r.pod;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {     "class":`res-row`,"aria-expanded":openRow === r.pod,"onclick":() => (openRow = openRow === r.pod ? null : r.pod),});
               { svelteHTML.createElement("span", { "class":`ph-status ${statusTone(r)} mono`,});r.status ?? 'ERR'; }
               { svelteHTML.createElement("span", { "class":`mono ell`,});r.pod; }
               { svelteHTML.createElement("span", { "class":`spacer`,}); }
               { svelteHTML.createElement("span", { "class":`dim small mono`,});r.duration_ms; r.via === 'port_forward' ? ' · port-forward' : ''; }
             }
            if(openRow === r.pod){
               { svelteHTML.createElement("div", { "class":`res-body`,});
                if(r.error){ { svelteHTML.createElement("div", { "class":`err small`,});r.error; }}
                if(r.body){
                   { svelteHTML.createElement("div", { "class":`res-tools`,});
                     { svelteHTML.createElement("span", { "class":`dim small`,});r.truncated ? 'Truncated at 256 KB' : ''; }
                     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copyBody(r.body),}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  }
                   }
                   { svelteHTML.createElement("pre", { "class":`mono`,});shownBody(r); }
                }
                if(Object.keys(r.headers).length){
                   { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", { "class":`small`,});  } { svelteHTML.createElement("pre", { "class":`mono`,});headersToText(r.headers); } }
                }
               }
            }
           }
        }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`dim small`,});                        }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PodHttpPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PodHttpPanel__SvelteComponent_ = ReturnType<typeof PodHttpPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default PodHttpPanel__SvelteComponent_;