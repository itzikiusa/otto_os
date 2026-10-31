///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import { sentenceCase } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import { toastError } from '../../lib/toastError';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
// EKS: clusters table (region switcher) + detail sheet with nodegroups and the
// raw describe-cluster JSON. "Open in Kubernetes" [aws_eks Edit + kubernetes
// Admin] imports a kubeconfig into an Otto-owned file (creates a k8s cluster
// row) and navigates to `#/kubernetes/<clusterId>`.
import { untrack } from 'svelte';
import { aws } from '../../lib/stores/aws.svelte';
import { awsApi, isLoginRequired } from '../../lib/api/aws';
import { auth } from '../../lib/stores/auth.svelte';
import { router } from '../../lib/router.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import JsonTree from '../../lib/components/JsonTree.svelte';
import ViewToolbar from './ViewToolbar.svelte';
import RegionPicker from './RegionPicker.svelte';
import RegionErrors from './RegionErrors.svelte';
import { ALL_REGIONS } from '../../lib/api/aws';
import { fmtAgo, fmtDate, awsErrorText } from './util';
import { awsLoadText } from './loadText';
import type { AwsAccount, EksClusterDetail, EksClusterSummary } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canImport = $derived(resourceAccess.can('aws_account', account.id, 'eks_import', 'aws_eks', 'edit') && auth.isRoot);
  // The view is remounted per account (AwsPage {#key}s on account+service), so
  // seeding the region from the initial account is intentional.
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  let filter = $state('');
  let auto = $state(false);
  let loading = $state(false);
  let error = $state('');
  const clusters = $derived(aws.eks[`${account.id}:${region}`] ?? null);
  const allRegions = $derived(region === ALL_REGIONS);
  const regionErrors = $derived(aws.regionErrors[`${account.id}:${region}`] ?? []);
  /** The region a cluster lives in (rows of an "All regions" list carry it). */
  const rowRegion = (c: EksClusterSummary): string => c.region ?? region;
  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = clusters ?? [];
    return q ? list.filter((c) => c.name.toLowerCase().includes(q) || (c.version ?? '').includes(q)) : list;
  });

  // Region-scoped, latest-wins: a slow region-A reply (or error) that lands
  // after a switch to B must not overwrite B's error or end B's spinner.
  let loadSeq = 0;
  async function load(): Promise<void> {
    const seq = ++loadSeq;
    const rg = region;
    loading = true;
    try {
      await aws.loadEks(account.id, rg);
      if (seq !== loadSeq) return;
      error = '';
    } catch (e) {
      if (seq !== loadSeq) return;
      error = awsLoadText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  $effect(() => {
    void region;
    untrack(() => {
      void aws.loadRegions();
      void load();
    });
  });

  let detail = $state<{ c: EksClusterSummary; d: EksClusterDetail | null; error: string } | null>(null);
  let detailRequest = 0;
  async function openDetail(c: EksClusterSummary): Promise<void> {
    const request = ++detailRequest;
    const target = { ...c, region: rowRegion(c) };
    detail = { c: target, d: null, error: '' };
    try {
      const d = await awsApi.eksCluster(account.id, target.name, target.region);
      if (request === detailRequest && detail) detail = { c: target, d, error: '' };
    } catch (e) {
      if (request === detailRequest && detail) detail = { c: target, d: null, error: awsLoadText(e) };
    }
  }

  let importing = $state<string | null>(null);
  async function openInK8s(c: EksClusterSummary): Promise<void> {
    if (!canImport) return;
    const ok = await confirmer.ask(
      `Import “${c.name}” into the Kubernetes console? Otto writes an Otto-owned kubeconfig (your ~/.kube/config is untouched) and links it to this AWS account for credentials.`,
      { title: 'Open in Kubernetes', confirmLabel: 'Import', danger: false },
    );
    if (!ok) return;
    importing = c.name;
    try {
      const k = await awsApi.eksImport(account.id, c.name, {}, rowRegion(c));
      toasts.success('Cluster imported', k.name);
      router.go(`kubernetes/${k.id}`);
    } catch (e) {
      toastError('Couldn’t import', e);
    } finally {
      importing = null;
    }
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
    }
  }

  function menu(e: MouseEvent | KeyboardEvent, c: EksClusterSummary): void {
    ctxMenu.show(e, [
      { label: 'Details', icon: 'info', action: () => void openDetail(c) },
      ...(c.arn ? [{ label: 'Copy ARN', icon: 'copy', action: () => void copy(c.arn!, 'ARN') }] : []),
      ...(c.endpoint ? [{ label: 'Copy endpoint', icon: 'copy', action: () => void copy(c.endpoint!, 'endpoint') }] : []),
      ...(canImport ? [{ separator: true }, { label: 'Open in Kubernetes…', icon: 'helm', action: () => void openInK8s(c) }] : []),
    ]);
  }

  const loginNeeded = $derived(isLoginRequired(new Error(error)));
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const detailError = $derived(detail?.error ?? '');
  $effect(() => { if (detailError) announce(`Couldn’t load the node groups. ${awsErrorText(detailError)}`, { assertive: true }); });
;
async () => {

  { const $$_rablooTweiV0C = __sveltets_2_ensureComponent(ViewToolbar); const $$_rablooTweiV0 = new $$_rablooTweiV0C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"title":`EKS`,"subtitle":clusters ? `${clusters.length} clusters` : '',filter,"filterPlaceholder":`Filter clusters…`,loading,auto,region,"onrefresh":load,}});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'filter';/*Ωignore_startΩ*/() => auto = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'auto';
    { const $$_rekciPnoigeR1C = __sveltets_2_ensureComponent(RegionPicker); const $$_rekciPnoigeR1 = new $$_rekciPnoigeR1C({ target: __sveltets_2_any(), props: {   account,"service":`eks`,region,"allowAll":true,}});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rekciPnoigeR1.$$bindings = 'region';}
 ViewToolbar}

 { svelteHTML.createElement("div", { "class":`tbl-wrap`,});
   { const $$_srorrEnoigeR1C = __sveltets_2_ensureComponent(RegionErrors); new $$_srorrEnoigeR1C({ target: __sveltets_2_any(), props: {  "errors":regionErrors,}});}
  if(loading && !clusters){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":5,"label":`EKS clusters`,}});} }
  } else if (error){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {     "what":`EKS clusters`,"error":awsErrorText(error),"empty":true,"onretry":() => void load(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(loginNeeded){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/ LoadState}
  } else if (shown.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {    "icon":`helm`,"title":filter ? 'No matching clusters' : allRegions ? 'No EKS clusters in any enabled region' : `No EKS clusters in ${region}`,}});}
  }else{
     { svelteHTML.createElement("table", { "class":`tbl`,});
       { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  }if(allRegions){ { svelteHTML.createElement("th", {});  }} { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`act`,}); } } }
       { svelteHTML.createElement("tbody", {});
           for(let c of __sveltets_2_ensureArray(shown)){`${c.region ?? ''}/${c.name}`;
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {          "class":`trow`,"tabindex":0,"onclick":() => void openDetail(c),"onkeydown":(e) => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); void openDetail(c); } },"oncontextmenu":(e) => menu(e, c),});
             { svelteHTML.createElement("td", { "class":`strong`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`helm`,"size":13,}});} c.name; }
            if(allRegions){ { svelteHTML.createElement("td", { "class":`mono`,});c.region ?? '—'; }}
             { svelteHTML.createElement("td", {}); { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":c.status === 'ACTIVE' ? 'ok' : 'warn',"label":sentenceCase(c.status),}});} }
             { svelteHTML.createElement("td", { "class":`mono`,});c.version ?? '—'; }
             { svelteHTML.createElement("td", {   "class":`mono dim hide-sm`,"title":c.endpoint ?? '',});c.endpoint ?? '—'; }
             { svelteHTML.createElement("td", {   "class":`dim hide-sm`,"title":fmtDate(c.created_at),});fmtAgo(c.created_at); }
             { svelteHTML.createElement("td", { "class":`act`,});
              if(canImport){
                 { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":(e) => { e.stopPropagation(); void openInK8s(c); },"disabled":importing === c.name,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`helm`,"size":12,}});} importing === c.name ? 'Importing…' : 'Open in Kubernetes';
                 }
              }
             }
           }}
        }
       }
     }
  }
 }

if(detail){
  const d = detail;
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":d.c.name,"width":820,"onclose":() => (detail = null),}});
     { svelteHTML.createElement("div", { "class":`dt`,});
       { svelteHTML.createElement("div", { "class":`dt-top`,});
         { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "tone":d.c.status === 'ACTIVE' ? 'ok' : 'warn',"label":sentenceCase(d.c.status),}});}
         { svelteHTML.createElement("span", { "class":`mono`,}); d.c.version ?? '?'; }
         { svelteHTML.createElement("span", {   "class":`mono dim ell`,"title":d.c.arn ?? '',});d.c.arn ?? ''; }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => router.go(`aws/${account.id}/logs/${encodeURIComponent(`/aws/eks/${d.c.name}/cluster`)}/${encodeURIComponent(rowRegion(d.c))}`),"title":`Open the control-plane log group in CloudWatch Logs (needs control-plane logging enabled on the cluster)`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`text`,"size":12,}});}   }
        if(canImport){
           { svelteHTML.createElement("span", { "class":`spacer`,}); }
           { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => void openInK8s(d.c),"disabled":importing === d.c.name,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`helm`,"size":12,}});}    }
        }
       }
       { svelteHTML.createElement("h3", {});  }
      if(d.error){
         { svelteHTML.createElement("p", { "class":`err`,});awsErrorText(d.error); }
      } else if (!d.d){
         { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {  "rows":3,}});}
      } else if (d.d.nodegroups.length === 0){
         { svelteHTML.createElement("p", { "class":`dim`,});        }
      }else{
         { svelteHTML.createElement("table", { "class":`tbl inner`,});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`num`,});  } { svelteHTML.createElement("th", { "class":`num`,});  } { svelteHTML.createElement("th", { "class":`num`,});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
           { svelteHTML.createElement("tbody", {});
               for(let ng of __sveltets_2_ensureArray(d.d.nodegroups)){ng.name;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", { "class":`strong`,});ng.name; }
                 { svelteHTML.createElement("td", {}); { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":ng.status === 'ACTIVE' ? 'ok' : 'warn',"label":sentenceCase(ng.status),}});} }
                 { svelteHTML.createElement("td", { "class":`num mono`,});ng.desired ?? '—'; }
                 { svelteHTML.createElement("td", { "class":`num mono`,});ng.min ?? '—'; }
                 { svelteHTML.createElement("td", { "class":`num mono`,});ng.max ?? '—'; }
                 { svelteHTML.createElement("td", { "class":`mono`,});ng.instance_types.join(', '); }
                 { svelteHTML.createElement("td", { "class":`mono dim`,});ng.ami_type ?? '—'; }
               }
            }
           }
         }
      }
      if(d.d){
         { svelteHTML.createElement("h3", {});  }
         { svelteHTML.createElement("div", { "class":`raw mono`,}); { const $$_eerTnosJ3C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ3C({ target: __sveltets_2_any(), props: {  "value":d.d.cluster,}});} }
      }
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const EksView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type EksView__SvelteComponent_ = ReturnType<typeof EksView__SvelteComponent_>;
/*Ωignore_endΩ*/export default EksView__SvelteComponent_;