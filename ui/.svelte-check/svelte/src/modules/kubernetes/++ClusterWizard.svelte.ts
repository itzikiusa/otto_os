///<reference types="svelte" />
;// Add / edit cluster sheet. Add = two steps: (1) pick contexts from
// `/k8s/discover` (multi-select), paste a kubeconfig, or jump to AWS/EKS;
// (2) name (single pick), default namespace, environment → save, then a
// best-effort connectivity test toasts per cluster. Edit = step 2 only,
// PATCHing the row.

import { untrack } from 'svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { radioKey } from '../../lib/radioKey';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { router } from '../../lib/router.svelte';
import { k8s } from '../../lib/stores/k8s.svelte';
import { k8sApi } from '../../lib/api/k8s';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { Environment, K8sCluster, K8sDiscoveredContext } from '../../lib/api/types';
;

  interface Props {
    existing?: K8sCluster | null;
    onclose: () => void;
  };;

  type Mode = 'contexts' | 'paste' | 'eks';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { existing = null, onclose }: Props = $props();

  const ENVS: Environment[] = ['dev', 'staging', 'prod'];

  // Initial values are snapshotted from the prop on purpose (the sheet is
  // remounted per open) — hence `untrack`.
  const init = untrack(() => existing);
  let step = $state<1 | 2>(init ? 2 : 1);
  let mode = $state<Mode>('contexts');
  let busy = $state(false);
  let error = $state('');

  // step 1 — contexts
  let contexts: K8sDiscoveredContext[] = $state([]);
  let discovering = $state(false);
  let discoverError = $state('');
  let ctxFilter = $state('');
  let picked = $state<Set<string>>(new Set());
  const ctxKey = (c: K8sDiscoveredContext): string => `${c.kubeconfig_path}::${c.name}`;
  const filteredContexts = $derived(
    contexts.filter((c) => {
      const q = ctxFilter.trim().toLowerCase();
      return !q || c.name.toLowerCase().includes(q) || c.cluster.toLowerCase().includes(q) || (c.server ?? '').toLowerCase().includes(q);
    }),
  );
  const pickedList = $derived(contexts.filter((c) => picked.has(ctxKey(c))));

  // step 1 — paste
  let yamlText = $state('');
  let pasteContext = $state('');

  // step 2
  let name = $state(init?.name ?? '');
  let contextName = $state(init?.context_name ?? '');
  let kubeconfigPath = $state(init?.kubeconfig_path ?? '');
  let defaultNs = $state(init?.default_namespace ?? '');
  let knownNsText = $state((init?.known_namespaces ?? []).join(', '));
  const knownNsList = (): string[] => [...new Set(knownNsText.split(/[,\s]+/).map((s) => s.trim().toLowerCase()).filter(Boolean))];
  let env = $state<Environment>(init?.environment ?? 'dev');

  $effect(() => {
    if (existing || !auth.isRoot) return;
    untrack(() => void discover());
  });

  async function discover(): Promise<void> {
    discovering = true;
    discoverError = '';
    try {
      const r = await k8sApi.discover();
      contexts = r.contexts;
    } catch (e) {
      discoverError = e instanceof Error ? e.message : String(e);
    } finally {
      discovering = false;
    }
  }

  function togglePick(c: K8sDiscoveredContext): void {
    const k = ctxKey(c);
    const next = new Set(picked);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    picked = next;
  }

  const canNext = $derived(
    mode === 'contexts' ? pickedList.length > 0 : mode === 'paste' ? yamlText.trim().length > 0 : false,
  );

  function next(): void {
    error = '';
    if (mode === 'contexts' && pickedList.length === 1) {
      const c = pickedList[0];
      name = c.name;
      contextName = c.name;
      kubeconfigPath = c.kubeconfig_path;
      defaultNs = c.namespace ?? '';
    } else if (mode === 'paste') {
      name = name || pasteContext || '';
    }
    step = 2;
  }

  const singleName = $derived(existing !== null || mode === 'paste' || pickedList.length <= 1);
  const canSave = $derived(!busy && (singleName ? name.trim().length > 0 : true));

  async function testAndToast(c: K8sCluster): Promise<void> {
    try {
      const r = await k8sApi.testCluster(c.id);
      if (r.ok) toasts.success(`Connected to ${c.name}`, `${r.server_version ?? ''} · ${r.latency_ms} ms`.trim());
      else toasts.warn(`Saved ${c.name}, but couldn’t reach it`, r.message);
      void k8s.loadCapabilities(c.id, true);
    } catch (e) {
      toasts.warn(`Saved ${c.name}, but couldn’t test it`, loadErrorText(e));
    }
  }

  async function save(): Promise<void> {
    if(!existing && !auth.isRoot)return;
    busy = true;
    error = '';
    try {
      if (existing) {
        const c = await k8s.updateCluster(existing.id, {
          name: name.trim(),
          known_namespaces: knownNsList(),
          ...(auth.isRoot ? {context_name:contextName.trim() || existing.context_name,default_namespace:defaultNs.trim().toLowerCase() || null,environment:env} : {}),
        });
        toasts.success('Cluster updated', c.name);
        onclose();
        return;
      }
      const created: K8sCluster[] = [];
      if (mode === 'paste') {
        created.push(
          await k8s.importCluster({
            name: name.trim(),
            kubeconfig_yaml: yamlText,
            context_name: pasteContext.trim() || null,
            default_namespace: defaultNs.trim().toLowerCase() || null,
            environment: env,
          }),
        );
      } else if (pickedList.length === 1) {
        created.push(
          await k8s.createCluster({
            name: name.trim(),
            source: 'kubeconfig',
            kubeconfig_path: kubeconfigPath || null,
            context_name: contextName.trim(),
            default_namespace: defaultNs.trim().toLowerCase() || null,
            environment: env,
            known_namespaces: knownNsList(),
          }),
        );
      } else {
        for (const c of pickedList) {
          created.push(
            await k8s.createCluster({
              name: c.name,
              source: 'kubeconfig',
              kubeconfig_path: c.kubeconfig_path,
              context_name: c.name,
              default_namespace: defaultNs.trim().toLowerCase() || c.namespace || null,
              environment: env,
            }),
          );
        }
      }
      onclose();
      // Best-effort tests after the sheet closes; each toasts its outcome.
      for (const c of created) void testAndToast(c);
      if (created.length === 1) router.go(`kubernetes/${encodeURIComponent(created[0].id)}`);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":existing ? 'Edit cluster' : step === 1 ? 'Add cluster' : 'Cluster details',"width":580,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(step === 2 && !existing){
       { svelteHTML.createElement("button", {     "class":`btn ghost`,"disabled":busy,"onclick":() => (step = 1),});  }
    }
     { svelteHTML.createElement("span", { "class":`spacer`,}); }
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":onclose,});  }
    if(step === 1){
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!canNext || busy,"onclick":next,});  }
    }else{
       { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":!canSave,"onclick":() => void save(),...__sveltets_2_empty({"data-testid":`k8s-wizard-save`}),});
        busy ? 'Saving…' : existing ? 'Save' : 'Save & test';
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`wiz`,...__sveltets_2_empty({"data-testid":`k8s-cluster-wizard`}),});
    if(!auth.isRoot){ { svelteHTML.createElement("p", { "class":`hint`,});            }}
    if(step === 1){
       { svelteHTML.createElement("div", {     "class":`segmented modes`,"role":`tablist`,"aria-label":`Cluster source`,});
         { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":mode === 'contexts',"tabindex":mode === 'contexts' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mode = 'contexts'),});mode === 'contexts';  }
         { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":mode === 'paste',"tabindex":mode === 'paste' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mode = 'paste'),});mode === 'paste';  }
         { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":mode === 'eks',"tabindex":mode === 'eks' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mode = 'eks'),});mode === 'eks';  }
       }

      if(mode === 'contexts'){
         { svelteHTML.createElement("p", { "class":`hint`,});    { svelteHTML.createElement("span", { "class":`mono`,});  }   { svelteHTML.createElement("span", { "class":`mono`,});  }               }
         { svelteHTML.createElement("div", { "class":`ctx-tools`,});
           { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input`,"placeholder":`Filter contexts…`,"bind:value":ctxFilter,"aria-label":`Filter contexts`,});/*Ωignore_startΩ*/() => ctxFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {         "class":`btn ghost`,"onclick":() => void discover(),"disabled":discovering,"aria-label":`Rescan`,"title":`Rescan`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
         }
        if(discovering){
           { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":3,"height":40,}});}
        } else if (discoverError){
           { svelteHTML.createElement("div", { "class":`err`,});discoverError; }
        } else if (!contexts.length){
           { svelteHTML.createElement("div", { "class":`hint`,});           }
        }else{
           { svelteHTML.createElement("div", {     "class":`ctx-list`,"role":`group`,"aria-label":`Kubeconfig contexts`,});
               for(let c of __sveltets_2_ensureArray(filteredContexts)){ctxKey(c);
              const on = picked.has(ctxKey(c));
               { svelteHTML.createElement("label", {  "class":`ctx`,});on;
                 { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":on,"onchange":() => togglePick(c),});}
                 { svelteHTML.createElement("span", { "class":`ctx-main`,});
                   { svelteHTML.createElement("span", { "class":`ctx-name`,});c.name; }
                   { svelteHTML.createElement("span", { "class":`ctx-meta mono`,});c.cluster;if(c.server){' '; c.server;}if(c.namespace){' ';  c.namespace;} }
                   { svelteHTML.createElement("span", { "class":`ctx-meta mono dim`,});c.kubeconfig_path; }
                 }
               }
            }
           }
        }
      } else if (mode === 'paste'){
         { svelteHTML.createElement("p", { "class":`hint`,});               { svelteHTML.createElement("span", { "class":`mono`,});  }  }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`k8s-yaml`,});  }
           { svelteHTML.createElement("textarea", {             "dir":`ltr`,"id":`k8s-yaml`,"class":`input mono`,"rows":10,"bind:value":yamlText,"placeholder":`apiVersion: v1&#10;kind: Config&#10;…`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => yamlText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`k8s-paste-ctx`,});   { svelteHTML.createElement("span", { "class":`dim`,});       } }
           { svelteHTML.createElement("input", {        "dir":`ltr`,"id":`k8s-paste-ctx`,"class":`input mono`,"bind:value":pasteContext,});/*Ωignore_startΩ*/() => pasteContext = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }else{
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {               "icon":`cloud`,"title":`EKS clusters come from the AWS module`,"body":`Pick an account → EKS → “Open in Kubernetes”. Otto runs aws eks update-kubeconfig into its own kubeconfig and links the cluster to that account.`,"actionLabel":`Go to AWS`,"actionIcon":`external`,"actionKind":`secondary`,"onaction":() => { onclose(); router.go('aws'); },}});}
      }
    }else{
      if(!existing && !singleName){
         { svelteHTML.createElement("div", { "class":`picked-summary`,});
           { svelteHTML.createElement("strong", {});pickedList.length;  }        
           { svelteHTML.createElement("ul", {});   for(let c of __sveltets_2_ensureArray(pickedList)){ctxKey(c); { svelteHTML.createElement("li", { "class":`mono`,});c.name; }} }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`k8s-name`,});  }
           { svelteHTML.createElement("input", {            "dir":`auto`,"id":`k8s-name`,"class":`input`,"bind:value":name,"placeholder":`prod-eu-1`,...__sveltets_2_empty({"data-testid":`k8s-wizard-name`}),});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
        if(existing || mode === 'contexts'){
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", { "for":`k8s-ctx`,});  }
             { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`k8s-ctx`,"class":`input mono`,"bind:value":contextName,"readonly":!existing || !auth.isRoot,});/*Ωignore_startΩ*/() => contextName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(kubeconfigPath){ { svelteHTML.createElement("span", { "class":`hint mono`,});kubeconfigPath; }}
           }
        }
      }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`k8s-ns`,});   { svelteHTML.createElement("span", { "class":`dim`,});    } }
         { svelteHTML.createElement("input", {                  "dir":`ltr`,"id":`k8s-ns`,"disabled":!auth.isRoot,"class":`input mono`,"bind:value":defaultNs,"placeholder":`default`,"autocapitalize":`off`,"autocorrect":`off`,"spellcheck":false,});/*Ωignore_startΩ*/() => defaultNs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`k8s-known-ns`,});  { svelteHTML.createElement("span", { "class":`dim`,});                 } }
         { svelteHTML.createElement("input", {                  "dir":`ltr`,"id":`k8s-known-ns`,"class":`input mono`,"bind:value":knownNsText,"placeholder":`koala-staging, koala-jobs`,"autocapitalize":`off`,"autocorrect":`off`,"spellcheck":false,...__sveltets_2_empty({"data-testid":`k8s-known-ns`}),});/*Ωignore_startΩ*/() => knownNsText = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("div", {     "class":`env-row`,"role":`radiogroup`,"aria-label":`Environment`,});
             for(let e of __sveltets_2_ensureArray(ENVS)){e;
             { svelteHTML.createElement("button", {                   "type":`button`,"class":`env-chip`,"role":`radio`,"aria-checked":env === e,"tabindex":env === e ? 0 : -1,"onkeydown":radioKey,"disabled":!auth.isRoot,"onclick":() => (env = e),});env === e;e === 'prod';e; }
          }
         }
        if(env === 'prod'){ { svelteHTML.createElement("span", { "class":`hint danger`,});                    }}
       }
    }

    if(error){ { svelteHTML.createElement("div", { "class":`err`,});error; }}
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ClusterWizard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ClusterWizard__SvelteComponent_ = ReturnType<typeof ClusterWizard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ClusterWizard__SvelteComponent_;