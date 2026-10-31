///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import type { BadgeTone } from '../../lib/status';
import { sentenceCase } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
// RDS (read-only): DB instances table (status pill, identifier, engine,
// class, AZ / Multi-AZ, storage, endpoint, created) with a region switcher
// and a right-side drawer (AwsDrawer) — Overview (key fields + tags),
// Metrics (CloudWatch via MetricsPanel), Raw JSON. No start/stop/reboot by
// design — the daemon has no RDS mutations either.
import { untrack } from 'svelte';
import { aws } from '../../lib/stores/aws.svelte';
import { awsApi, isLoginRequired } from '../../lib/api/aws';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import JsonTree from '../../lib/components/JsonTree.svelte';
import ViewToolbar from './ViewToolbar.svelte';
import RegionPicker from './RegionPicker.svelte';
import RegionErrors from './RegionErrors.svelte';
import { ALL_REGIONS } from '../../lib/api/aws';
import AwsDrawer from './AwsDrawer.svelte';
import MetricsPanel from './MetricsPanel.svelte';
import { fmtAgo, fmtDate, awsErrorText } from './util';
import { awsLoadText } from './loadText';
import { router } from '../../lib/router.svelte';
import type { AwsAccount, RdsInstance, RdsInstanceDetail } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  };;

  // Detail drawer.
  type DrawerTab = 'overview' | 'metrics' | 'raw';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { account, onsignin }: Props = $props();

  // Remounted per account (AwsPage {#key}s on account+service).
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  let filter = $state('');
  let auto = $state(false);
  let loading = $state(false);
  let error = $state('');
  const instances = $derived(aws.rds[`${account.id}:${region}`] ?? null);
  const allRegions = $derived(region === ALL_REGIONS);
  const regionErrors = $derived(aws.regionErrors[`${account.id}:${region}`] ?? []);
  /** The region an instance lives in (rows of an "All regions" list carry it). */
  const rowRegion = (i: RdsInstance): string => i.region ?? region;

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = instances ?? [];
    if (!q) return list;
    return list.filter((i) =>
      [i.identifier, i.engine, i.engine_version, i.class, i.az, i.endpoint, i.db_name, i.status, ...Object.values(i.tags ?? {})]
        .filter(Boolean)
        .some((v) => String(v).toLowerCase().includes(q)),
    );
  });

  // Region-scoped, latest-wins: a slow region-A reply (or error) that lands
  // after a switch to B must not overwrite B's error or end B's spinner.
  let loadSeq = 0;
  async function load(): Promise<void> {
    const seq = ++loadSeq;
    const rg = region;
    loading = true;
    try {
      await aws.loadRds(account.id, rg);
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

  const DRAWER_TABS: { id: DrawerTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'metrics', label: 'Metrics' },
    { id: 'raw', label: 'Raw JSON' },
  ];
  let drawerTab = $state<DrawerTab>('overview');
  let detail = $state<{ inst: RdsInstance; full: RdsInstanceDetail | null; error: string } | null>(null);
  const drawerInst = $derived.by(() => {
    const d = detail;
    if (!d) return null;
    return instances?.find((i) => i.identifier === d.inst.identifier && rowRegion(i) === d.inst.region) ?? d.inst;
  });

  function statusTone(status: string): BadgeTone {
    if (status === 'available') return 'ok';
    if (status === 'failed' || status === 'inaccessible-encryption-credentials' || status === 'storage-full') return 'bad';
    if (status === 'stopped' || status === 'deleting') return 'neutral';
    return 'warn';
  }

  let detailRequest = 0;
  async function openDetail(i: RdsInstance): Promise<void> {
    const request = ++detailRequest;
    const inst = { ...i, region: rowRegion(i) };
    if (detail?.inst.identifier !== inst.identifier || detail.inst.region !== inst.region) drawerTab = 'overview';
    detail = { inst, full: null, error: '' };
    try {
      const full = await awsApi.rdsInstance(account.id, inst.identifier, inst.region);
      if (request === detailRequest && detail) detail = { inst, full, error: '' };
    } catch (e) {
      if (request === detailRequest && detail)
        detail = { inst, full: null, error: awsLoadText(e) };
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

  function endpointOf(i: RdsInstance): string {
    if (!i.endpoint) return '';
    return i.port ? `${i.endpoint}:${i.port}` : i.endpoint;
  }

  function menu(e: MouseEvent | KeyboardEvent, i: RdsInstance): void {
    ctxMenu.show(e, [
      { label: 'Details', icon: 'info', action: () => void openDetail(i) },
      { label: 'Metrics', icon: 'chart', action: () => { void openDetail(i); drawerTab = 'metrics'; } },
      { label: 'Copy identifier', icon: 'copy', action: () => void copy(i.identifier, 'identifier') },
      ...(i.endpoint ? [{ label: `Copy endpoint ${endpointOf(i)}`, icon: 'copy', action: () => void copy(endpointOf(i), 'endpoint') }] : []),
    ]);
  }

  const loginNeeded = $derived(isLoginRequired(new Error(error)));
  const counts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const i of instances ?? []) c[i.status] = (c[i.status] ?? 0) + 1;
    return c;
  });
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const detailError = $derived(detail?.error ?? '');
  $effect(() => { if (detailError) announce(`Couldn’t load the DB instance details. ${awsErrorText(detailError)}`, { assertive: true }); });
;
async () => {

  { const $$_rablooTweiV0C = __sveltets_2_ensureComponent(ViewToolbar); const $$_rablooTweiV0 = new $$_rablooTweiV0C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"title":`RDS`,"subtitle":instances ? `${instances.length} instances · ${counts.available ?? 0} available` : '',filter,"filterPlaceholder":`Filter identifier, engine, endpoint, tag…`,loading,auto,region,"onrefresh":load,}});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'filter';/*Ωignore_startΩ*/() => auto = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'auto';
    { const $$_rekciPnoigeR1C = __sveltets_2_ensureComponent(RegionPicker); const $$_rekciPnoigeR1 = new $$_rekciPnoigeR1C({ target: __sveltets_2_any(), props: {   account,"service":`rds`,region,"allowAll":true,}});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rekciPnoigeR1.$$bindings = 'region';}
 ViewToolbar}

 { svelteHTML.createElement("div", { "class":`body`,});
 { svelteHTML.createElement("div", { "class":`tbl-wrap`,});
   { const $$_srorrEnoigeR2C = __sveltets_2_ensureComponent(RegionErrors); new $$_srorrEnoigeR2C({ target: __sveltets_2_any(), props: {  "errors":regionErrors,}});}
  if(loading && !instances){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"label":`RDS instances`,}});} }
  } else if (error){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`DB instances`,"error":awsErrorText(error),"empty":true,"onretry":() => void load(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(loginNeeded){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/ LoadState}
  } else if (shown.length === 0){
     { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {    "icon":`db`,"title":filter ? 'No matching instances' : allRegions ? 'No DB instances in any enabled region' : `No DB instances in ${region}`,}});}
  }else{
     { svelteHTML.createElement("table", { "class":`tbl`,});
       { svelteHTML.createElement("thead", {});
         { svelteHTML.createElement("tr", {});
           { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  }if(allRegions){ { svelteHTML.createElement("th", {});  }} { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`hide-md`,});  }
           { svelteHTML.createElement("th", { "class":`hide-md num`,});  } { svelteHTML.createElement("th", { "class":`hide-md`,});  } { svelteHTML.createElement("th", { "class":`hide-md`,});  } { svelteHTML.createElement("th", { "class":`act`,}); }
         }
       }
       { svelteHTML.createElement("tbody", {});
           for(let i of __sveltets_2_ensureArray(shown)){`${i.region ?? ''}/${i.identifier}`;
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {             "class":`trow`,"tabindex":0,"onclick":() => void openDetail(i),"onkeydown":(e) => { if (e.key === 'Enter') void openDetail(i); },"oncontextmenu":(e) => menu(e, i),});detail?.inst.identifier === i.identifier && detail.inst.region === rowRegion(i);
             { svelteHTML.createElement("td", {}); { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":statusTone(i.status),"label":sentenceCase(i.status),}});} }
             { svelteHTML.createElement("td", {   "class":`strong mono`,"title":i.identifier,});i.identifier; }
            if(allRegions){ { svelteHTML.createElement("td", { "class":`mono`,});i.region ?? '—'; }}
             { svelteHTML.createElement("td", { "class":`hide-sm`,});i.engine ?? '—';if(i.engine_version){ { svelteHTML.createElement("span", { "class":`dim`,}); i.engine_version; }} }
             { svelteHTML.createElement("td", { "class":`mono hide-sm`,});i.class ?? '—'; }
             { svelteHTML.createElement("td", { "class":`mono hide-md`,});i.az ?? '—';if(i.multi_az){' '; { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Multi-AZ`,}});}} }
             { svelteHTML.createElement("td", { "class":`mono hide-md num`,});i.storage_gb != null ? `${i.storage_gb} GB` : '—'; }
             { svelteHTML.createElement("td", {   "class":`mono hide-md`,"title":endpointOf(i),});endpointOf(i) || '—'; }
             { svelteHTML.createElement("td", {   "class":`dim hide-md`,"title":fmtDate(i.created),});fmtAgo(i.created); }
             { svelteHTML.createElement("td", { "class":`act`,});
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":(e) => menu(e, i),"aria-label":`Actions for ${i.identifier}`,"title":`Actions`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
             }
           }}
        }
       }
     }
  }
 }

if(detail && drawerInst){
  const d = detail;
  const inst = drawerInst;
   { const $$_rewarDswA1C = __sveltets_2_ensureComponent(AwsDrawer); new $$_rewarDswA1C({ target: __sveltets_2_any(), props: {                   children:() => { return __sveltets_2_any(0); },"kind":`db instance`,"name":inst.identifier,"id":endpointOf(inst),"status":inst.status,"statusTone":statusTone(inst.status),"tabs":DRAWER_TABS,"tab":drawerTab,"ontab":(t) => (drawerTab = t as DrawerTab),"onclose":() => (detail = null),}});
    if(drawerTab === 'overview'){
       { svelteHTML.createElement("div", { "class":`dt`,});
         { svelteHTML.createElement("div", { "class":`logs-link`,});
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => router.go(`aws/${account.id}/logs/${encodeURIComponent(`/aws/rds/instance/${inst.identifier}/`)}/${encodeURIComponent(rowRegion(inst))}`),"title":`Open this instance’s exported log groups in CloudWatch Logs (needs log exports enabled)`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`text`,"size":12,}});}  }
         }
         { svelteHTML.createElement("dl", { "class":`kv`,});
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});inst.engine ?? '—'; inst.engine_version ?? ''; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.class ?? '—'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.az ?? '—';if(inst.multi_az){  { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Multi-AZ`,}});}} }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});inst.storage_gb != null ? `${inst.storage_gb} GB` : '—';if(inst.storage_type){  { svelteHTML.createElement("span", { "class":`dim`,}); inst.storage_type;  }} }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});endpointOf(inst) || '—'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.db_name ?? '—'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.master_username ?? '—'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});inst.publicly_accessible ? 'yes' : 'no'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});fmtDate(inst.created);  { svelteHTML.createElement("span", { "class":`dim`,}); fmtAgo(inst.created);  } }
         }
         { svelteHTML.createElement("h3", {});  }
        if(Object.keys(inst.tags ?? {}).length === 0){
           { svelteHTML.createElement("p", { "class":`dim`,});  }
        }else{
           { svelteHTML.createElement("div", { "class":`tags`,});
               for(let [k, v] of __sveltets_2_ensureArray((Object.entries(inst.tags).sort(([a], [b]) => a.localeCompare(b))))){k;
               { svelteHTML.createElement("span", {   "class":`kv`,"title":`${k}=${v}`,}); { svelteHTML.createElement("strong", {});k; } v; }
            }
           }
        }
       }
    } else if (drawerTab === 'metrics'){
      `${account.id}/${rowRegion(inst)}/${inst.identifier}`; {
         { const $$_lenaPscirteM2C = __sveltets_2_ensureComponent(MetricsPanel); new $$_lenaPscirteM2C({ target: __sveltets_2_any(), props: {         "accountId":account.id,"namespace":`AWS/RDS`,"dimValue":inst.identifier,"region":rowRegion(inst),onsignin,}});}
      }
    }else{
       { svelteHTML.createElement("div", { "class":`dt`,});
        if(d.error){
           { svelteHTML.createElement("p", { "class":`err`,});awsErrorText(d.error); }
        } else if (!d.full){
           { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {  "rows":5,}});}
        }else{
           { svelteHTML.createElement("div", { "class":`raw mono`,}); { const $$_eerTnosJ4C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ4C({ target: __sveltets_2_any(), props: {  "value":d.full.raw,}});} }
        }
       }
    }
   AwsDrawer}
}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RdsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RdsView__SvelteComponent_ = ReturnType<typeof RdsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default RdsView__SvelteComponent_;