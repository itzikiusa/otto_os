///<reference types="svelte" />
;
import { announce } from '../../../lib/announce.svelte';
// Per-cluster monitoring settings: enable, cadence, namespaces, probes
// (preset-fillable; JSON probes carry field mappings, prometheus probes
// carry include/exclude globs), exclusions, transport / concurrency /
// retention, and a "Test probes" dry run that shows what one pod parses to.
// Client-side validation mirrors the daemon's limits so most mistakes never
// round-trip.
import { rel } from '../../../lib/stores/now.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { toastError } from '../../../lib/toastError';
import { kindLabel } from '../../../lib/labels';
import { plural } from '../../../lib/plural';
import { untrack, tick } from 'svelte';
import { toasts } from '../../../lib/toast.svelte';
import { ctxMenu } from '../../../lib/contextmenu.svelte';
import { k8sApi } from '../../../lib/api/k8s';
import type {
    K8sCluster,
    K8sExclusion,
    K8sMapping,
    K8sMonitorConfig,
    K8sMonitorPreset,
    K8sMonitorStatus,
    K8sMonitorTestResp,
    K8sProbe,
    K8sProbeFormat,
    K8sUnit,
  } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import Modal from '../../../lib/components/Modal.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import Switch from '../../../lib/components/Switch.svelte';
import { collectorLine, rbacMessage } from './monitor-util';
import { revalidateAfterRemove, validateMonitorConfig } from './monitor-settings';
;

  interface Props {
    cluster: K8sCluster;
    canEdit: boolean;
    onsaved?: (cfg: K8sMonitorConfig, status: K8sMonitorStatus | null) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { cluster, canEdit, onsaved }: Props = $props();

  let cfg = $state<K8sMonitorConfig | null>(null);
  let status = $state<K8sMonitorStatus | null>(null);
  let presets = $state<K8sMonitorPreset[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  let toggling = $state(false);
  let toggleError = $state('');
  // The switch updates the persisted config only; other fields remain drafts.
  let savedConfig: K8sMonitorConfig | null = null;
  let loadGeneration = 0;
  let running = $state(false);
  let errors = $state<Record<string, string>>({});
  let settingsEl = $state<HTMLDivElement | null>(null);
  /** UX-5-09: the id of a field's inline error (its `aria-describedby`). */
  const errId = (key: string): string => `mon-${key.replace(/\./g, '-')}-err`;
  /** `aria-invalid` + `aria-describedby` for the field validated under `key`. */
  function invalid(key: string): { 'aria-invalid'?: 'true'; 'aria-describedby'?: string } {
    return errors[key] ? { 'aria-invalid': 'true', 'aria-describedby': errId(key) } : {};
  }
  const errorCount = $derived(Object.keys(errors).length);
  let nsText = $state('');
  let testOpen = $state(false);
  let testBusy = $state(false);
  let testResult = $state<K8sMonitorTestResp | null>(null);
  let testError = $state('');
  let testPod = $state('');

  const FORMATS: K8sProbeFormat[] = ['json', 'prometheus', 'health'];
  const UNITS: K8sUnit[] = ['number', 'bytes', 'bytes_human', 'duration_human', 'percent'];
  const NEEDS_NS = $derived(!cluster.default_namespace);
  // Metadata refreshes replace the cluster object without changing the form's
  // owner. Reload only on an identity change, preserving unsaved settings.
  const clusterId = $derived(cluster.id);

  async function load(): Promise<void> {
    const generation = ++loadGeneration;
    const id = cluster.id;
    loading = true;
    toggling = false;
    saving = false;
    testBusy = false;
    testOpen = false;
    toggleError = '';
    savedConfig = null;
    cfg = null;
    try {
      const r = await k8sApi.monitor(id);
      if (generation !== loadGeneration || id !== cluster.id) return;
      savedConfig = structuredClone(r.config);
      cfg = r.config;
      status = r.status;
      presets = r.presets;
      nsText = r.config.namespaces.join(', ');
      loadError = '';
    } catch (e) {
      if (generation === loadGeneration && id === cluster.id) loadError = loadErrorText(e);
    } finally {
      if (generation === loadGeneration && id === cluster.id) loading = false;
    }
  }

  async function setEnabled(enabled: boolean): Promise<void> {
    if (!savedConfig || !cfg || saving || toggling || testBusy) return;
    const id = cluster.id;
    const generation = loadGeneration;
    toggling = true;
    toggleError = '';
    try {
      const r = await k8sApi.monitorSave(id, { ...savedConfig, enabled });
      if (id !== cluster.id || generation !== loadGeneration || !cfg) return;
      savedConfig = structuredClone(r.config);
      cfg.enabled = r.config.enabled;
      status = r.status;
      onsaved?.(r.config, r.status);
    } catch (e) {
      if (id === cluster.id && generation === loadGeneration) toggleError = loadErrorText(e);
    } finally {
      if (id === cluster.id && generation === loadGeneration) toggling = false;
    }
  }

  $effect(() => {
    void clusterId;
    untrack(() => void load());
  });

  function validate(c: K8sMonitorConfig): Record<string, string> {
    return validateMonitorConfig(c, NEEDS_NS);
  }
  /** UC-6-05: errors are keyed by row index, so a removal re-runs the
   *  validation (only while errors show) instead of leaving a message on the
   *  row that slid into the removed one's slot. */
  function revalidate(): void {
    if (cfg) errors = revalidateAfterRemove(errors, cfg, NEEDS_NS);
  }

  function syncNamespaces(): void {
    if (!cfg) return;
    cfg.namespaces = nsText
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  async function save(): Promise<void> {
    if (!cfg || toggling || testBusy) return;
    syncNamespaces();
    errors = validate(cfg);
    if (Object.keys(errors).length) {
      // Inline only (no toast): move to the first invalid field.
      await tick();
      const first = settingsEl?.querySelector<HTMLElement>('[aria-invalid="true"]');
      first?.scrollIntoView({ block: 'center' });
      first?.focus();
      return;
    }
    const id = cluster.id;
    const generation = loadGeneration;
    saving = true;
    try {
      const r = await k8sApi.monitorSave(id, cfg);
      if (id !== cluster.id || generation !== loadGeneration) return;
      savedConfig = structuredClone(r.config);
      cfg = r.config;
      status = r.status;
      nsText = r.config.namespaces.join(', ');
      toasts.success('Monitoring saved', r.config.enabled ? 'The collector picks the change up within 15 s.' : 'Monitoring is off for this cluster.');
      onsaved?.(r.config, r.status);
    } catch (e) {
      if (id === cluster.id && generation === loadGeneration) toastError('Couldn’t save the monitor settings', e);
    } finally {
      if (id === cluster.id && generation === loadGeneration) saving = false;
    }
  }

  async function runNow(): Promise<void> {
    running = true;
    try {
      status = await k8sApi.monitorRun(cluster.id);
      toasts.success('Cycle finished', collectorLine(status, true, rel));
      if (cfg) onsaved?.(cfg, status);
    } catch (e) {
      toastError('Couldn’t run the monitor cycle', e);
    } finally {
      running = false;
    }
  }

  async function runTest(): Promise<void> {
    if (saving || toggling || testBusy) return;
    const id = cluster.id;
    const generation = loadGeneration;
    testBusy = true;
    testError = '';
    testResult = null;
    try {
      // Test what is on screen: save first so the daemon sees the same probes.
      if (cfg) {
        syncNamespaces();
        const r = await k8sApi.monitorSave(id, cfg);
        if (id !== cluster.id || generation !== loadGeneration) return;
        savedConfig = structuredClone(r.config);
        cfg = r.config;
        status = r.status;
        onsaved?.(r.config, r.status);
      }
      const result = await k8sApi.monitorTest(id, { pod: testPod.trim() || undefined });
      if (id === cluster.id && generation === loadGeneration) testResult = result;
    } catch (e) {
      if (id === cluster.id && generation === loadGeneration) testError = loadErrorText(e);
    } finally {
      if (id === cluster.id && generation === loadGeneration) testBusy = false;
    }
  }

  // --- probe editing -----------------------------------------------------------
  function addProbe(): void {
    if (!cfg) return;
    cfg.probes = [...cfg.probes, { name: `probe${cfg.probes.length + 1}`, port: null, path: '/metrics', format: 'prometheus', mappings: [], include: [], exclude: [], timeout_ms: 3000 }];
  }
  function removeProbe(i: number): void {
    if (!cfg) return;
    cfg.probes = cfg.probes.filter((_, j) => j !== i);
    revalidate();
  }
  function applyPreset(p: K8sMonitorPreset): void {
    if (!cfg) return;
    cfg.probes = p.probes.map((x) => ({ ...x, mappings: [...(x.mappings ?? [])], include: [...(x.include ?? [])], exclude: [...(x.exclude ?? [])] }));
    toasts.info('Preset applied', `${p.probes.length} probe(s) from “${p.title}”. Adjust ports if your services differ.`);
  }
  function presetMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      presets.map((p) => ({ label: p.title, icon: 'layers', action: () => applyPreset(p) })),
    );
  }
  function formatMenu(e: MouseEvent, p: K8sProbe): void {
    ctxMenu.show(
      e,
      FORMATS.map((f) => ({ label: f, action: () => (p.format = f) })),
    );
  }
  function addMapping(p: K8sProbe): void {
    p.mappings = [...(p.mappings ?? []), { field: '', metric: '', label: null, unit: 'number' }];
  }
  function removeMapping(p: K8sProbe, j: number): void {
    p.mappings = (p.mappings ?? []).filter((_, k) => k !== j);
    revalidate();
  }
  function unitMenu(e: MouseEvent, m: K8sMapping): void {
    ctxMenu.show(
      e,
      UNITS.map((u) => ({ label: u, action: () => (m.unit = u) })),
    );
  }
  function globs(list: string[] | undefined): string {
    return (list ?? []).join(', ');
  }
  function setGlobs(p: K8sProbe, key: 'include' | 'exclude', text: string): void {
    p[key] = text
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  // --- exclusions ---------------------------------------------------------------
  function addExclusion(kind: K8sExclusion['kind']): void {
    if (!cfg) return;
    const x: K8sExclusion = kind === 'label' ? { kind: 'label', selector: '' } : { kind, match: '' };
    cfg.exclusions = [...cfg.exclusions, x];
  }
  function exclusionMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Pod name glob', action: () => addExclusion('pod') },
      { label: 'Workload glob (kind:name)', action: () => addExclusion('workload') },
      { label: 'Namespace glob', action: () => addExclusion('namespace') },
      { label: 'Label selector', action: () => addExclusion('label') },
    ]);
  }
  function removeExclusion(i: number): void {
    if (!cfg) return;
    cfg.exclusions = cfg.exclusions.filter((_, j) => j !== i);
    revalidate();
  }
  function transportMenu(e: MouseEvent): void {
    if (!cfg) return;
    const c = cfg;
    ctxMenu.show(e, [
      { label: 'auto — try the API-server proxy, else port-forward', action: () => (c.transport = 'auto') },
      { label: 'proxy — pods/proxy through the API server', action: () => (c.transport = 'proxy') },
      { label: 'port_forward — kubectl port-forward per pod', action: () => (c.transport = 'port_forward') },
    ]);
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const errorSummary = $derived(errorCount ? (errors.probes ?? `Fix the ${plural(errorCount, 'highlighted field')} to save.`) : '');
  $effect(() => { if (errorSummary) announce(errorSummary, { assertive: true }); });
  $effect(() => { if (toggleError) announce(`Couldn’t update monitoring: ${toggleError} Try the switch again.`, { assertive: true }); });
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {    "class":`settings`,...__sveltets_2_empty({"data-testid":`k8s-monitor-settings`}),});settingsEl = $$_div0;
  if(!cfg){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`monitoring settings`,loading,"error":loadError,"empty":true,"onretry":() => void load(),"rows":6,}});}
  }else{
     { svelteHTML.createElement("section", { "class":`card block`,});
       { svelteHTML.createElement("div", { "class":`row between`,});
        
         { svelteHTML.createElement("label", { "class":`toggle`,});
           { svelteHTML.createElement("span", {   "class":`sw-slot`,...__sveltets_2_empty({"data-testid":`k8s-monitor-enabled`}),}); { const $$_hctiwS5C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS5C({ target: __sveltets_2_any(), props: {        "checked":cfg.enabled,"onchange":(v) => void setEnabled(v),"label":`Monitoring`,"disabled":!canEdit || saving || toggling || testBusy,}});} }
           { svelteHTML.createElement("span", { "class":`strong`,});toggling ? 'Updating monitoring…' : `Monitoring ${cfg.enabled ? 'on' : 'off'}`; }
         }
         { svelteHTML.createElement("div", { "class":`row`,});
          if(canEdit){
             { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void runNow(),"disabled":running || saving,"title":`Run one collection cycle now`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} running ? 'Running…' : 'Run once'; }
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => { testOpen = true; testResult = null; testError = ''; },"disabled":!cfg.probes.length,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}   }
             { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => void save(),"disabled":saving || toggling || testBusy,...__sveltets_2_empty({"data-testid":`k8s-monitor-save`}),});saving ? 'Saving…' : 'Save'; }
          }
         }
       }
       { svelteHTML.createElement("div", { "class":`dim status`,});collectorLine(status, cfg.enabled, rel); }
       { svelteHTML.createElement("div", { "class":`dim`,});            }
      if(errorCount){ { svelteHTML.createElement("p", { "class":`err field-error`,});errors.probes ?? `Fix the ${plural(errorCount, 'highlighted field')} to save.`; }}
      if(toggleError){ { svelteHTML.createElement("p", { "class":`error`,});   toggleError;     }}
      if(rbacMessage(status?.metrics_server)){
         { svelteHTML.createElement("div", { "class":`rbac`,}); { svelteHTML.createElement("strong", {});  }       { svelteHTML.createElement("code", {});rbacMessage(status?.metrics_server); } }
      }
       { svelteHTML.createElement("div", { "class":`grid3`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {               "class":`input`,"type":`number`,"min":`15`,"max":`3600`,"bind:value":cfg.interval_secs,...invalid('interval_secs'),"disabled":!canEdit,...__sveltets_2_empty({"data-testid":`k8s-monitor-interval`}),});/*Ωignore_startΩ*/() => cfg.interval_secs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(errors.interval_secs){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId('interval_secs'),});errors.interval_secs; }}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {             "class":`input`,"type":`number`,"min":`1`,"max":`32`,"bind:value":cfg.concurrency,...invalid('concurrency'),"disabled":!canEdit,});/*Ωignore_startΩ*/() => cfg.concurrency = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(errors.concurrency){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId('concurrency'),});errors.concurrency; }}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   { svelteHTML.createElement("span", { "class":`dim`,});      } }
           { svelteHTML.createElement("input", {             "class":`input`,"type":`number`,"min":`1`,"max":`90`,"bind:value":cfg.retention_days,...invalid('retention_days'),"disabled":!canEdit,});/*Ωignore_startΩ*/() => cfg.retention_days = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(errors.retention_days){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId('retention_days'),});errors.retention_days; }}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});     { svelteHTML.createElement("span", { "class":`dim`,});    } }
           { svelteHTML.createElement("input", {                 "class":`input`,"type":`number`,"min":`100`,"max":`10000`,"step":`100`,"bind:value":cfg.series_cap,...invalid('series_cap'),"disabled":!canEdit,...__sveltets_2_empty({"data-testid":`k8s-monitor-series-cap`}),});/*Ωignore_startΩ*/() => cfg.series_cap = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(errors.series_cap){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId('series_cap'),});errors.series_cap; }}
         }
       }
       { svelteHTML.createElement("div", { "class":`grid2`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {}); if(!NEEDS_NS){ { svelteHTML.createElement("span", { "class":`dim`,});  cluster.default_namespace;  }} }
           { svelteHTML.createElement("input", {               "dir":`auto`,"class":`input`,"placeholder":NEEDS_NS ? 'required: e.g. groove, groove-jobs' : cluster.default_namespace ?? '',"bind:value":nsText,...invalid('namespaces'),"onblur":syncNamespaces,"disabled":!canEdit,...__sveltets_2_empty({"data-testid":`k8s-monitor-namespaces`}),});/*Ωignore_startΩ*/() => nsText = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(errors.namespaces){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId('namespaces'),});errors.namespaces; }}
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("button", {     "class":`input picker`,"onclick":transportMenu,"disabled":!canEdit,});cfg.transport;  { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`dot`,"size":12,}});} }
         }
       }
       { svelteHTML.createElement("label", { "class":`toggle small`,});
         { svelteHTML.createElement("span", {   "class":`sw-slot`,...__sveltets_2_empty({"data-testid":`k8s-monitor-metrics-server`}),}); { const $$_hctiwS4C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS4C({ target: __sveltets_2_any(), props: {        "checked":cfg.metrics_server,"onchange":(v) => { if (cfg) cfg.metrics_server = v; },"label":`Probe metrics-server every cycle`,"disabled":!canEdit,}});} }
         { svelteHTML.createElement("span", {});     { svelteHTML.createElement("span", { "class":`dim`,});                  } }
       }
       { svelteHTML.createElement("label", { "class":`toggle small`,});
         { svelteHTML.createElement("span", {   "class":`sw-slot`,...__sveltets_2_empty({"data-testid":`k8s-monitor-request-labels`}),}); { const $$_hctiwS4C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS4C({ target: __sveltets_2_any(), props: {        "checked":cfg.request_labels ?? false,"onchange":(v) => { if (cfg) cfg.request_labels = v; },"label":`Keep request path labels`,"disabled":!canEdit,}});} }
         { svelteHTML.createElement("span", {});     { svelteHTML.createElement("span", { "class":`dim`,});  { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }                                   } }
       }
     }

     { svelteHTML.createElement("section", { "class":`card block`,});
       { svelteHTML.createElement("div", { "class":`row between`,});
         { svelteHTML.createElement("h3", {});  { svelteHTML.createElement("span", { "class":`dim`,}); cfg.probes.length;  } }
        if(canEdit){
           { svelteHTML.createElement("div", { "class":`row`,});
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":presetMenu,...__sveltets_2_empty({"data-testid":`k8s-monitor-preset`}),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`layers`,"size":12,}});}  }
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":addProbe,"disabled":cfg.probes.length >= 10,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
           }
        }
       }
       { svelteHTML.createElement("p", { "class":`dim help`,});                   { svelteHTML.createElement("b", {});  }         { svelteHTML.createElement("b", {});  }          { svelteHTML.createElement("b", {});  }    { svelteHTML.createElement("code", {});  }  }
      if(!cfg.probes.length){
         { svelteHTML.createElement("div", { "class":`dim`,});          }
      }
          for(let p of __sveltets_2_ensureArray(cfg.probes)){let i = 1;i;
         { svelteHTML.createElement("div", {   "class":`probe`,...__sveltets_2_empty({"data-testid":`k8s-monitor-probe`}),});
           { svelteHTML.createElement("div", { "class":`grid-probe`,});
             { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {         "dir":`auto`,"class":`input`,"bind:value":p.name,...invalid(`probe.${i}.name`),"disabled":!canEdit,});/*Ωignore_startΩ*/() => p.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}if(errors[`probe.${i}.name`]){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId(`probe.${i}.name`),});errors[`probe.${i}.name`]; }} }
             { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {           "class":`input`,"type":`number`,"placeholder":`first`,"bind:value":p.port,...invalid(`probe.${i}.port`),"disabled":!canEdit,});/*Ωignore_startΩ*/() => p.port = __sveltets_2_any(null);/*Ωignore_endΩ*/}if(errors[`probe.${i}.port`]){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId(`probe.${i}.port`),});errors[`probe.${i}.port`]; }} }
             { svelteHTML.createElement("label", { "class":`field wide`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {         "dir":`ltr`,"class":`input mono`,"bind:value":p.path,...invalid(`probe.${i}.path`),"disabled":!canEdit,});/*Ωignore_startΩ*/() => p.path = __sveltets_2_any(null);/*Ωignore_endΩ*/}if(errors[`probe.${i}.path`]){ { svelteHTML.createElement("p", {   "class":`err field-error`,"id":errId(`probe.${i}.path`),});errors[`probe.${i}.path`]; }} }
             { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("button", {     "class":`input picker`,"onclick":(e) => formatMenu(e, p),"disabled":!canEdit,});p.format; } }
             { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {            "class":`input`,"type":`number`,"min":`100`,"max":`30000`,"bind:value":p.timeout_ms,"disabled":!canEdit,});/*Ωignore_startΩ*/() => p.timeout_ms = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
            if(canEdit){ { svelteHTML.createElement("button", {       "class":`icon-btn del`,"onclick":() => removeProbe(i),"title":`Remove probe`,"aria-label":`Remove probe`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});} }}
           }
          if(p.format === 'json'){
             { svelteHTML.createElement("div", { "class":`sub-block`,});
               { svelteHTML.createElement("div", { "class":`row between`,}); { svelteHTML.createElement("span", { "class":`dim small`,});  }if(canEdit){ { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => addMapping(p),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }} }
                  for(let m of __sveltets_2_ensureArray(p.mappings ?? [])){let j = 1;j;
                 { svelteHTML.createElement("div", { "class":`map`,});
                   { svelteHTML.createElement("input", {             "dir":`ltr`,"class":`input mono`,"placeholder":`memory_stats.sys`,"bind:value":m.field,...invalid(`probe.${i}.map.${j}`),"disabled":!canEdit,"title":`Dotted path; numbers index arrays`,});/*Ωignore_startΩ*/() => m.field = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                   { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Metric name`,"class":`input mono`,"placeholder":`metric name`,"bind:value":m.metric,"disabled":!canEdit,});/*Ωignore_startΩ*/() => m.metric = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                   { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Metric label`,"class":`input mono`,"placeholder":`or label`,"bind:value":m.label,"disabled":!canEdit,});/*Ωignore_startΩ*/() => m.label = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                   { svelteHTML.createElement("button", {     "class":`input picker`,"onclick":(e) => unitMenu(e, m),"disabled":!canEdit,});m.unit ?? 'number'; }
                  if(canEdit){ { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => removeMapping(p, j),"aria-label":`Remove mapping`,"title":`Remove mapping`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }}
                  if(errors[`probe.${i}.map.${j}`]){ { svelteHTML.createElement("p", {   "class":`err field-error span`,"id":errId(`probe.${i}.map.${j}`),});errors[`probe.${i}.map.${j}`]; }}
                 }
              }
             }
          } else if (p.format === 'prometheus'){
             { svelteHTML.createElement("div", { "class":`sub-block grid2`,});
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   { svelteHTML.createElement("span", { "class":`dim`,});   } } { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input mono`,"value":globs(p.include),"oninput":(e) => setGlobs(p, 'include', e.currentTarget.value),"placeholder":`http_*, process_*`,"disabled":!canEdit,});} }
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input mono`,"value":globs(p.exclude),"oninput":(e) => setGlobs(p, 'exclude', e.currentTarget.value),"placeholder":`*_bucket`,"disabled":!canEdit,});} }
             }
          }
         }
      }
     }

     { svelteHTML.createElement("section", { "class":`card block`,});
       { svelteHTML.createElement("div", { "class":`row between`,});
         { svelteHTML.createElement("h3", {});  { svelteHTML.createElement("span", { "class":`dim`,}); cfg.exclusions.length;  } }
        if(canEdit){ { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":exclusionMenu,...__sveltets_2_empty({"data-testid":`k8s-monitor-add-exclusion`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }}
       }
       { svelteHTML.createElement("p", { "class":`dim help`,});             { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }     { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }  }
          for(let x of __sveltets_2_ensureArray(cfg.exclusions)){let i = 1;i;
         { svelteHTML.createElement("div", { "class":`ex`,});
           { svelteHTML.createElement("span", { "class":`chip`,});kindLabel(x.kind); }
          if(x.kind === 'label'){
             { svelteHTML.createElement("input", {             "dir":`ltr`,"aria-label":`Label selector`,"class":`input mono`,"placeholder":`app=frb,tier!=web`,"bind:value":x.selector,...invalid(`ex.${i}`),"disabled":!canEdit,});/*Ωignore_startΩ*/() => x.selector = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          }else{
             { svelteHTML.createElement("input", {             "dir":`ltr`,"aria-label":`Name pattern`,"class":`input mono`,"placeholder":x.kind === 'workload' ? 'cronjob:*' : '*-confsrv-*',"bind:value":x.match,...invalid(`ex.${i}`),"disabled":!canEdit,});/*Ωignore_startΩ*/() => x.match = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          }
          if(canEdit){ { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => removeExclusion(i),"aria-label":`Remove exclusion`,"title":`Remove exclusion`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }}
          if(errors[`ex.${i}`]){ { svelteHTML.createElement("p", {   "class":`err field-error span`,"id":errId(`ex.${i}`),});errors[`ex.${i}`]; }}
         }
      }
     }
  }
 }

if(testOpen && cfg){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Test probes`,"width":720,"onclose":() => (testOpen = false),}});
     { svelteHTML.createElement("div", { "class":`test`,});
       { svelteHTML.createElement("div", { "class":`row`,});
         { svelteHTML.createElement("input", {          "dir":`ltr`,"aria-label":`Test pod name`,"class":`input mono`,"placeholder":`pod name (blank = first running pod)`,"bind:value":testPod,});/*Ωignore_startΩ*/() => testPod = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":() => void runTest(),"disabled":testBusy || saving || toggling,});testBusy ? 'Testing…' : 'Run'; }
       }
       { svelteHTML.createElement("p", { "class":`dim help`,});                      }
      if(testError){ { svelteHTML.createElement("div", { "class":`error`,});testError; }}
      if(testResult){
         { svelteHTML.createElement("div", { "class":`dim small`,});  { svelteHTML.createElement("b", { "class":`mono`,});testResult.namespace; testResult.pod; }    { svelteHTML.createElement("b", {});testResult.transport; }    { svelteHTML.createElement("b", {});testResult.metrics_server.split(':')[0]; } }
           for(let pr of __sveltets_2_ensureArray(testResult.probes)){pr.name;
           { svelteHTML.createElement("div", {    "class":`tp`,...__sveltets_2_empty({"data-testid":`k8s-monitor-test-probe`}),});!pr.ok;
             { svelteHTML.createElement("div", { "class":`row between`,});
               { svelteHTML.createElement("span", {}); { svelteHTML.createElement("b", {});pr.name; } if(pr.port){ { svelteHTML.createElement("span", { "class":`dim`,}); pr.port; }} }
               { svelteHTML.createElement("span", { "class":`dim small`,});pr.ok ? `HTTP ${pr.status} · ${pr.ms} ms · ${plural(pr.sample_count ?? 0, 'sample')}` : (pr.error ?? `HTTP ${pr.status}`);if(pr.parse_errors){' '; plural(pr.parse_errors, 'parse error');}if(pr.capped){' '; } }
             }
            if(pr.labels && Object.keys(pr.labels).length){
               { svelteHTML.createElement("div", { "class":`row wrap`,});   for(let [k, v] of __sveltets_2_ensureArray(Object.entries(pr.labels))){k; { svelteHTML.createElement("span", { "class":`chip accent`,});k; v; }} }
            }
            if(pr.samples?.length){
               { svelteHTML.createElement("table", { "class":`samples`,});
                 { svelteHTML.createElement("tbody", {});
                      for(let s of __sveltets_2_ensureArray(pr.samples)){let i = 1;i;
                     { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", { "class":`mono`,});s.metric; } { svelteHTML.createElement("td", { "class":`mono dim`,});Object.entries(s.labels).map(([k, v]) => `${k}="${v}"`).join(' '); } { svelteHTML.createElement("td", { "class":`mono num`,});s.value; } }
                  }
                 }
               }
            } else if (pr.ok && pr.body_preview){
               { svelteHTML.createElement("pre", { "class":`preview`,});pr.body_preview; }
            }
           }
        }
      }
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MonitorSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MonitorSettings__SvelteComponent_ = ReturnType<typeof MonitorSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default MonitorSettings__SvelteComponent_;