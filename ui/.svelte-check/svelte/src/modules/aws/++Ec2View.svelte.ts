///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import type { BadgeTone } from '../../lib/status';
import { sentenceCase } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import { toastError } from '../../lib/toastError';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
// EC2: instances table (state pill, name, id, type, AZ, IPs, launch) with a
// state filter + region switcher; row actions start/stop/reboot [Edit] — stop
// and reboot need the instance id typed; a right-side drawer (AwsDrawer,
// same look as the Kubernetes ResourceDrawer) shows Overview (key fields +
// tags), Metrics (CloudWatch via MetricsPanel) and Raw JSON via JsonTree.
import { untrack } from 'svelte';
import { aws } from '../../lib/stores/aws.svelte';
import { awsApi, isLoginRequired } from '../../lib/api/aws';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { confirmProd } from '../../lib/confirmProd';
import { toasts } from '../../lib/toast.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import JsonTree from '../../lib/components/JsonTree.svelte';
import ViewToolbar from './ViewToolbar.svelte';
import AwsDrawer from './AwsDrawer.svelte';
import MetricsPanel from './MetricsPanel.svelte';
import RegionPicker from './RegionPicker.svelte';
import RegionErrors from './RegionErrors.svelte';
import { ALL_REGIONS } from '../../lib/api/aws';
import { fmtAgo, fmtDate, awsErrorText } from './util';
import { awsLoadText } from './loadText';
import type { AwsAccount, Ec2Action, Ec2Instance, Ec2InstanceDetail } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  };;

  // Detail drawer. The drawer's `inst` is refreshed from the list after a
  // power action so the state pill follows the instance.
  type DrawerTab = 'overview' | 'metrics' | 'raw';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canStart = $derived(resourceAccess.can('aws_account', account.id, 'ec2_start', 'aws_ec2', 'edit'));
  const canStop = $derived(resourceAccess.can('aws_account', account.id, 'ec2_stop', 'aws_ec2', 'edit'));
  const canReboot = $derived(resourceAccess.can('aws_account', account.id, 'ec2_reboot', 'aws_ec2', 'edit'));
  const canEdit = $derived(canStart || canStop || canReboot);
  const STATES = ['pending', 'running', 'stopping', 'stopped', 'shutting-down', 'terminated'];

  // The view is remounted per account (AwsPage {#key}s on account+service), so
  // seeding the region from the initial account is intentional.
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  let stateFilter = $state('');
  let filter = $state('');
  let auto = $state(false);
  let loading = $state(false);
  let error = $state('');
  const instances = $derived(aws.ec2[`${account.id}:${region}`] ?? null);
  const allRegions = $derived(region === ALL_REGIONS);
  const regionErrors = $derived(aws.regionErrors[`${account.id}:${region}`] ?? []);
  /** The region a row lives in (rows of an "All regions" list carry it). */
  const rowRegion = (i: Ec2Instance): string => i.region ?? region;

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    let list = instances ?? [];
    if (stateFilter) list = list.filter((i) => i.state === stateFilter);
    if (q)
      list = list.filter((i) =>
        [i.name, i.instance_id, i.type, i.private_ip, i.public_ip, i.az, ...Object.values(i.tags ?? {})]
          .filter(Boolean)
          .some((v) => String(v).toLowerCase().includes(q)),
      );
    return list;
  });

  // Region-scoped, latest-wins: a slow region-A reply (or error) that lands
  // after a switch to B must not overwrite B's error or end B's spinner.
  let loadSeq = 0;
  async function load(): Promise<void> {
    const seq = ++loadSeq;
    const rg = region;
    loading = true;
    try {
      await aws.loadEc2(account.id, rg);
      if (seq !== loadSeq) return;
      error = '';
    } catch (e) {
      if (seq !== loadSeq) return;
      error = awsLoadText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  // Load when the region changes (and on mount); `region` is the only dep.
  $effect(() => {
    void region;
    untrack(() => {
      void aws.loadRegions();
      void load();
    });
  });

  // Actions in flight per instance (disables its row buttons).
  let busy = $state<Record<string, boolean>>({});

  async function act(i: Ec2Instance, action: Ec2Action): Promise<void> {
    if (!resourceAccess.can('aws_account', account.id, `ec2_${action}`, 'aws_ec2', 'edit')) return;
    // Approval belongs to the target shown in the sheet, even if navigation or
    // a region change occurs before it resolves.
    const accountId = account.id;
    const targetRegion = rowRegion(i);
    const label = i.name ? `${i.name} (${i.instance_id})` : i.instance_id;
    const verb = action === 'start' ? 'Start' : action === 'stop' ? 'Stop' : 'Reboot';
    const ok = await confirmProd({
      env: account.environment,
      verb,
      title: `${verb} instance`,
      where: `EC2 ${label} · ${account.name} · ${targetRegion}`,
      // Start is reversible; stop / reboot interrupt a running host, so type the id.
      typed: action === 'start' ? undefined : i.instance_id,
      danger: action !== 'start',
    });
    if (!ok) return;
    busy = { ...busy, [i.instance_id]: true };
    try {
      const r = await awsApi.ec2Action(accountId, i.instance_id, action, targetRegion);
      toasts.success(`${verb} sent`, `${i.instance_id}: ${r.previous_state} → ${r.current_state}`);
      await load();
    } catch (e) {
      toastError(`Couldn’t ${action} ${label}`, e);
    } finally {
      busy = { ...busy, [i.instance_id]: false };
    }
  }

  /** Why a drawer power button is disabled (undefined when it isn't). */
  function powerHint(allowed: boolean, i: Ec2Instance, needs: 'stopped' | 'running'): string | undefined {
    if (!allowed) return 'You don’t have permission for this action on this account';
    if (busy[i.instance_id]) return 'An action is already in flight';
    if (i.state !== needs) return `Only available while the instance is ${needs} (it is ${i.state})`;
    return undefined;
  }

  const DRAWER_TABS: { id: DrawerTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'metrics', label: 'Metrics' },
    { id: 'raw', label: 'Raw JSON' },
  ];
  let drawerTab = $state<DrawerTab>('overview');
  let detail = $state<{ inst: Ec2Instance; full: Ec2InstanceDetail | null; error: string } | null>(null);
  const drawerInst = $derived.by(() => {
    const d = detail;
    if (!d) return null;
    return instances?.find((i) => i.instance_id === d.inst.instance_id) ?? d.inst;
  });
  function stateTone(state: string): BadgeTone {
    if (state === 'running') return 'ok';
    if (state === 'terminated') return 'bad';
    if (state === 'pending' || state === 'stopping' || state === 'shutting-down') return 'warn';
    return 'neutral';
  }
  async function openDetail(i: Ec2Instance): Promise<void> {
    if (detail?.inst.instance_id !== i.instance_id) drawerTab = 'overview';
    detail = { inst: i, full: null, error: '' };
    try {
      const d = await awsApi.ec2Instance(account.id, i.instance_id, rowRegion(i));
      if (detail?.inst.instance_id === i.instance_id) detail = { inst: i, full: d, error: '' };
    } catch (e) {
      if (detail?.inst.instance_id === i.instance_id)
        detail = { inst: i, full: null, error: awsLoadText(e) };
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

  function menu(e: MouseEvent | KeyboardEvent, i: Ec2Instance): void {
    const stopped = i.state === 'stopped';
    const running = i.state === 'running';
    ctxMenu.show(e, [
      { label: 'Details', icon: 'info', action: () => void openDetail(i) },
      { label: 'Copy instance id', icon: 'copy', action: () => void copy(i.instance_id, 'instance id') },
      ...(i.private_ip ? [{ label: `Copy private IP ${i.private_ip}`, icon: 'copy', action: () => void copy(i.private_ip!, 'IP') }] : []),
      ...(i.public_ip ? [{ label: `Copy public IP ${i.public_ip}`, icon: 'copy', action: () => void copy(i.public_ip!, 'IP') }] : []),
      ...(canEdit
        ? [
            { separator: true },
            { label: 'Start…', icon: 'play', disabled: !canStart || !stopped, action: () => void act(i, 'start') },
            { label: 'Reboot…', icon: 'refresh', disabled: !canReboot || !running, action: () => void act(i, 'reboot') },
            { label: 'Stop…', icon: 'stop', danger: true, disabled: !canStop || !running, action: () => void act(i, 'stop') },
          ]
        : []),
    ]);
  }

  const loginNeeded = $derived(isLoginRequired(new Error(error)));
  const counts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const i of instances ?? []) c[i.state] = (c[i.state] ?? 0) + 1;
    return c;
  });
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const detailError = $derived(detail?.error ?? '');
  $effect(() => { if (detailError) announce(`Couldn’t load the instance details. ${awsErrorText(detailError)}`, { assertive: true }); });
;
async () => {

  { const $$_rablooTweiV0C = __sveltets_2_ensureComponent(ViewToolbar); const $$_rablooTweiV0 = new $$_rablooTweiV0C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"title":`EC2`,"subtitle":instances ? `${instances.length} instances · ${counts.running ?? 0} running` : '',filter,"filterPlaceholder":`Filter name, id, IP, tag…`,loading,auto,region,"onrefresh":load,}});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'filter';/*Ωignore_startΩ*/() => auto = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'auto';
    { const $$_rekciPnoigeR1C = __sveltets_2_ensureComponent(RegionPicker); const $$_rekciPnoigeR1 = new $$_rekciPnoigeR1C({ target: __sveltets_2_any(), props: {   account,"service":`ec2`,region,"allowAll":true,}});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rekciPnoigeR1.$$bindings = 'region';}
   { svelteHTML.createElement("label", { "class":`sel`,});
     { svelteHTML.createElement("span", { "class":`lbl`,});  }
     { svelteHTML.createElement("select", {   "bind:value":stateFilter,"aria-label":`State filter`,});/*Ωignore_startΩ*/() => stateFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/
       { svelteHTML.createElement("option", {"value":"",});  }
         for(let s of __sveltets_2_ensureArray(STATES)){s; { svelteHTML.createElement("option", { "value":s,});s;counts[s] ? ` (${counts[s]})` : ''; }}
     }
   }
 ViewToolbar}

 { svelteHTML.createElement("div", { "class":`body`,});
 { svelteHTML.createElement("div", { "class":`tbl-wrap`,});
   { const $$_srorrEnoigeR2C = __sveltets_2_ensureComponent(RegionErrors); new $$_srorrEnoigeR2C({ target: __sveltets_2_any(), props: {  "errors":regionErrors,}});}
  if(loading && !instances){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"label":`EC2 instances`,}});} }
  } else if (error){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`instances`,"error":awsErrorText(error),"empty":true,"onretry":() => void load(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(loginNeeded){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/ LoadState}
  } else if (shown.length === 0){
     { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {    "icon":`box`,"title":filter || stateFilter ? 'No matching instances' : allRegions ? 'No instances in any enabled region' : `No instances in ${region}`,}});}
  }else{
     { svelteHTML.createElement("table", { "class":`tbl`,});
       { svelteHTML.createElement("thead", {});
         { svelteHTML.createElement("tr", {});
           { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  }if(allRegions){ { svelteHTML.createElement("th", {});  }} { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  }
           { svelteHTML.createElement("th", { "class":`hide-md`,});  } { svelteHTML.createElement("th", { "class":`hide-md`,});  } { svelteHTML.createElement("th", { "class":`hide-md`,});  } { svelteHTML.createElement("th", { "class":`act`,}); }
         }
       }
       { svelteHTML.createElement("tbody", {});
           for(let i of __sveltets_2_ensureArray(shown)){`${i.region ?? ''}/${i.instance_id}`;
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {             "class":`trow`,"tabindex":0,"onclick":() => void openDetail(i),"onkeydown":(e) => { if (e.key === 'Enter') void openDetail(i); },"oncontextmenu":(e) => menu(e, i),});detail?.inst.instance_id === i.instance_id;
             { svelteHTML.createElement("td", {}); { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":stateTone(i.state),"label":sentenceCase(i.state),}});} }
             { svelteHTML.createElement("td", {   "class":`strong`,"title":i.name ?? '',});i.name ?? '—'; }
             { svelteHTML.createElement("td", { "class":`mono`,});i.instance_id; }
            if(allRegions){ { svelteHTML.createElement("td", { "class":`mono`,});i.region ?? '—'; }}
             { svelteHTML.createElement("td", { "class":`mono hide-sm`,});i.type; }
             { svelteHTML.createElement("td", { "class":`mono hide-sm`,});i.az ?? '—'; }
             { svelteHTML.createElement("td", { "class":`mono hide-md`,});i.private_ip ?? '—'; }
             { svelteHTML.createElement("td", { "class":`mono hide-md`,});i.public_ip ?? '—'; }
             { svelteHTML.createElement("td", {   "class":`dim hide-md`,"title":fmtDate(i.launch_time),});fmtAgo(i.launch_time); }
             { svelteHTML.createElement("td", { "class":`act`,});
               { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":(e) => menu(e, i),"aria-label":`Actions for ${i.instance_id}`,"title":`Actions`,"disabled":busy[i.instance_id],}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
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
   { const $$_rewarDswA1C = __sveltets_2_ensureComponent(AwsDrawer); new $$_rewarDswA1C({ target: __sveltets_2_any(), props: {                   children:() => { return __sveltets_2_any(0); },"kind":`instance`,"name":inst.name ?? inst.instance_id,"id":inst.instance_id,"status":inst.state,"statusTone":stateTone(inst.state),"tabs":DRAWER_TABS,"tab":drawerTab,"ontab":(t) => (drawerTab = t as DrawerTab),"onclose":() => (detail = null),}});
    if(drawerTab === 'overview'){
       { svelteHTML.createElement("div", { "class":`dt`,});
         { svelteHTML.createElement("div", { "class":`dt-top`,});
           { svelteHTML.createElement("span", { "class":`mono`,});inst.type; }
           { svelteHTML.createElement("span", { "class":`mono dim`,});inst.az ?? ''; }
          if(canEdit){
             { svelteHTML.createElement("span", { "class":`spacer`,}); }
             { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void act(inst, 'start'),"disabled":!canStart || inst.state !== 'stopped' || busy[inst.instance_id],"title":powerHint(canStart, inst, 'stopped'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  }
             { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void act(inst, 'reboot'),"disabled":!canReboot || inst.state !== 'running' || busy[inst.instance_id],"title":powerHint(canReboot, inst, 'running'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}  }
             { svelteHTML.createElement("button", {       "class":`btn small danger`,"onclick":() => void act(inst, 'stop'),"disabled":!canStop || inst.state !== 'running' || busy[inst.instance_id],"title":powerHint(canStop, inst, 'running'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}  }
          }
         }
         { svelteHTML.createElement("dl", { "class":`kv`,});
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.instance_id; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.type; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.az ?? '—'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.private_ip ?? '—'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});inst.public_ip ?? '—'; }
           { svelteHTML.createElement("dt", {});   } { svelteHTML.createElement("dd", { "class":`mono`,});inst.vpc_id ?? '—';  inst.subnet_id ?? '—'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});inst.platform ?? 'linux'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});fmtDate(inst.launch_time);  { svelteHTML.createElement("span", { "class":`dim`,}); fmtAgo(inst.launch_time);  } }
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
      `${account.id}/${rowRegion(inst)}/${inst.instance_id}`; {
         { const $$_lenaPscirteM2C = __sveltets_2_ensureComponent(MetricsPanel); new $$_lenaPscirteM2C({ target: __sveltets_2_any(), props: {           "accountId":account.id,"namespace":`AWS/EC2`,"dimValue":inst.instance_id,"region":rowRegion(inst),"instanceType":inst.type,onsignin,}});}
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
const Ec2View__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Ec2View__SvelteComponent_ = ReturnType<typeof Ec2View__SvelteComponent_>;
/*Ωignore_endΩ*/export default Ec2View__SvelteComponent_;