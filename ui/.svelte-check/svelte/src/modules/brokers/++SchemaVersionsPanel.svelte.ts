///<reference types="svelte" />
;// Schema version history + version diff panel. Shown inside SchemaTab when a
// subject is selected. Fetches all registered versions and lets the operator
// compare any two via the shared DiffView component (word-level diff).

import { SchemaVersionCache } from './schemaVersionCache';
import { api } from '../../lib/api/client';
import { toastError } from '../../lib/toastError';
import DiffView from '../../lib/components/DiffView.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { BrokerCluster } from '../../lib/api/types';
import type { SchemaVersion, SchemaVersionDetail, CompatCheckResp } from './types';
;

  interface Props {
    cluster: BrokerCluster;
    subject: string;
  };function $$render() {

  
  
  

  
  
  
  
  
  
  
  
  
  

  let { cluster, subject }: Props = $props();

  let versions = $state<SchemaVersion[]>([]);
  let loading = $state(true);
  /** Failed load — inline with Retry (it used to toast AND claim "No versions found."). */
  let loadError = $state<string | null>(null);
  // The two versions selected for the diff.
  let diffA = $state<SchemaVersionDetail | null>(null);
  let diffB = $state<SchemaVersionDetail | null>(null);
  let showDiff = $state(false);
  const cache = new SchemaVersionCache();
  let generation = 0;
  let listController: AbortController | null = null;
  const detailControllers: Record<'A' | 'B', AbortController | null> = { A:null, B:null };
  let detailErrors = $state<Record<'A' | 'B', string | null>>({ A:null, B:null });
  const detailError = $derived(detailErrors.A ?? detailErrors.B);
  let selectedA = $state<number | null>(null);
  let selectedB = $state<number | null>(null);
  const versionBase = () => `/brokers/clusters/${cluster.id}/schema-registry/subjects/${encodeURIComponent(subject)}/versions`;
  function cancelLoads() {
    listController?.abort(); detailControllers.A?.abort(); detailControllers.B?.abort();
  }
  async function selectVersion(side: 'A' | 'B', version: number, reveal = true): Promise<void> {
    detailControllers[side]?.abort();
    const controller = new AbortController(); detailControllers[side] = controller;
    const gen = generation, base = versionBase();
    if (side === 'A') { selectedA = version; diffA = null; } else { selectedB = version; diffB = null; }
    detailErrors[side] = null;
    try {
      const detail = cache.get(base, version) ?? await api.get<SchemaVersionDetail>(`${base}/${version}`, controller.signal);
      if (controller.signal.aborted || gen !== generation) return;
      cache.set(base, detail);
      if (side === 'A') diffA = detail; else diffB = detail;
      if (reveal) showDiff = true;
    } catch (e) {
      if (!controller.signal.aborted && gen === generation) detailErrors[side] = loadErrorText(e);
    }
  }
  function retryDetails() {
    if (selectedA !== null) void selectVersion('A', selectedA);
    if (selectedB !== null) void selectVersion('B', selectedB);
  }

  // Compatibility check against latest.
  let compatSchema = $state('');
  let compatLoading = $state(false);
  let compatResult = $state<CompatCheckResp | null>(null);

  $effect(() => {
    void cluster.id;
    void subject;
    loadVersions();
    return cancelLoads;
  });

  function loadVersions(): void {
    cancelLoads();
    const gen = ++generation;
    const controller = new AbortController(); listController = controller;
    loading = true;
    loadError = null;
    detailErrors = { A:null, B:null };
    versions = [];
    diffA = null; diffB = null;
    selectedA = null; selectedB = null;
    showDiff = false;
    compatResult = null;
    api.get<SchemaVersion[]>(versionBase(), controller.signal)
      .then((v) => {
        if (gen !== generation || controller.signal.aborted) return;
        versions = v;
        if (v.length >= 1) void selectVersion('B', v[v.length - 1].version, false);
        if (v.length >= 2) void selectVersion('A', v[v.length - 2].version, false);
      })
      .catch((e) => { if (gen === generation && !controller.signal.aborted) loadError = loadErrorText(e); })
      .finally(() => { if (gen === generation && !controller.signal.aborted) loading = false; });
  }

  function pretty(schema: string): string {
    try {
      return JSON.stringify(JSON.parse(schema), null, 2);
    } catch {
      return schema;
    }
  }

  async function checkCompat() {
    if (!compatSchema.trim()) return;
    compatLoading = true;
    compatResult = null;
    try {
      compatResult = await api.post<CompatCheckResp>(
        `/brokers/clusters/${cluster.id}/schema-registry/subjects/${encodeURIComponent(subject)}/compatibility`,
        { schema: compatSchema },
      );
    } catch (e) {
      toastError('Couldn’t check compatibility', e);
    } finally {
      compatLoading = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`svp`,});
  if(loading){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":4,"label":`schema versions`,}});} }
  } else if (loadError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`schema versions`,"error":loadError,"empty":true,"onretry":loadVersions,}});}
  } else if (versions.length === 0){
     { svelteHTML.createElement("p", { "class":`muted pad`,});     }
  }else{
    
     { svelteHTML.createElement("section", { "class":`version-list`,});
       { svelteHTML.createElement("h5", {}); versions.length;  }
       { svelteHTML.createElement("div", { "class":`vtable`,});
           for(let v of __sveltets_2_ensureArray(versions)){v.version;
           { svelteHTML.createElement("div", { "class":`vrow`,});
             { svelteHTML.createElement("span", { "class":`vnum`,}); v.version; }
             { svelteHTML.createElement("span", { "class":`vid muted`,});diffA?.version === v.version ? `#${diffA.id}` : diffB?.version === v.version ? `#${diffB.id}` : ''; }
             { svelteHTML.createElement("span", { "class":`vtype muted`,});diffA?.version === v.version ? diffA.schema_type : diffB?.version === v.version ? diffB.schema_type : ''; }
             { svelteHTML.createElement("div", { "class":`vbtns`,});
               { svelteHTML.createElement("button", {           "class":`btn small side-btn`,"aria-pressed":selectedA === v.version,"onclick":() => selectVersion('A', v.version),"aria-label":`Compare side A (before): version ${v.version}`,"title":`Compare side A (before): version ${v.version}`,});if(selectedA === v.version){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}  }
               { svelteHTML.createElement("button", {           "class":`btn small side-btn`,"aria-pressed":selectedB === v.version,"onclick":() => selectVersion('B', v.version),"aria-label":`Compare side B (after): version ${v.version}`,"title":`Compare side B (after): version ${v.version}`,});if(selectedB === v.version){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}  }
             }
           }
        }
       }
     }

    if(detailError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`selected schema versions`,"error":detailError,"empty":true,"onretry":retryDetails,}});}
    } else if ((selectedA !== null && !diffA) || (selectedB !== null && !diffB)){
       { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":2,"label":`selected versions`,}});} }
    }

    
    if(diffA && diffB && showDiff){
       { svelteHTML.createElement("section", { "class":`diff-section`,});
         { svelteHTML.createElement("h5", {}); diffA.version;  diffB.version; }
         { const $$_weiVffiD2C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD2C({ target: __sveltets_2_any(), props: {        "before":pretty(diffA.schema),"after":pretty(diffB.schema),"mode":`word`,"contextLines":3,}});}
       }
    } else if (diffA && diffB){
       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (showDiff = true),});
            diffA.version;  diffB.version;
         }
       }
    }

    
     { svelteHTML.createElement("section", { "class":`compat-section`,});
       { svelteHTML.createElement("h5", {});    }
       { svelteHTML.createElement("p", { "class":`muted small`,});            }
       { svelteHTML.createElement("textarea", {             "class":`compat-input`,"aria-label":`Candidate schema`,"dir":`ltr`,"bind:value":compatSchema,"placeholder":'{"type":"record","name":"...","fields":[...]}',"rows":5,});/*Ωignore_startΩ*/() => compatSchema = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("div", { "class":`compat-row`,});
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":checkCompat,"disabled":compatLoading || !compatSchema.trim(),});
          compatLoading ? 'Checking…' : 'Check';
         }
        if(compatResult){
           { svelteHTML.createElement("span", {   "class":`compat-result`,});compatResult.compatible;!compatResult.compatible;
            compatResult.compatible ? 'Compatible' : 'Incompatible';
           }
          if(compatResult.messages.length > 0){
             { svelteHTML.createElement("ul", { "class":`compat-msgs`,});
                  for(let m of __sveltets_2_ensureArray(compatResult.messages)){let i = 1;i; { svelteHTML.createElement("li", {});m; }}
             }
          }
        }
       }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SchemaVersionsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SchemaVersionsPanel__SvelteComponent_ = ReturnType<typeof SchemaVersionsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default SchemaVersionsPanel__SvelteComponent_;