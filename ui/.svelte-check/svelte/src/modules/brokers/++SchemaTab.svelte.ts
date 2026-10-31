///<reference types="svelte" />
;
import { api } from '../../lib/api/client';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Tabs from '../../lib/components/Tabs.svelte';
import { loadErrorText } from '../../lib/loadError';
import { TableWindow } from '../../lib/tableWindow.svelte';
import type { BrokerCluster, SchemaSubject } from '../../lib/api/types';
import SchemaVersionsPanel from './SchemaVersionsPanel.svelte';
;

  interface Props {
    cluster: BrokerCluster;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { cluster }: Props = $props();

  // Raw: a registry can hold thousands of subjects, each carrying its schema.
  let subjects = $state.raw<SchemaSubject[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let selected = $state.raw<SchemaSubject | null>(null);
  // Toggle to show version history / compat panel for the selected subject.
  let showVersions = $state(false);

  // Window the subject list (SC-03): thousands of subjects mount only the
  // visible slice between two spacers.
  const tw = new TableWindow();
  const win = $derived(tw.range(subjects.length));
  let listEl = $state<HTMLDivElement>();
  $effect(() => {
    void win;
    tw.measure(listEl, '.srow');
  });
  // ⌘F over every subject, not just the mounted slice.
  $effect(() =>
    tw.findRows(() => listEl, () => subjects, (s) => `${s.subject}\nv${s.version} · ${s.schema_type} · #${s.id}`, '.srow'),
  );

  function prettySchema(s: SchemaSubject): string {
    try {
      return JSON.stringify(JSON.parse(s.schema), null, 2);
    } catch {
      return s.schema;
    }
  }

  function load(): void {
    loading = true;
    error = null;
    selected = null;
    showVersions = false;
    api
      .get<SchemaSubject[]>(`/brokers/clusters/${cluster.id}/schema-registry/subjects`)
      .then((s) => {
        subjects = s;
        tw.reset(listEl);
        if (s.length > 0) selected = s[0];
      })
      .catch((e) => {
        error = loadErrorText(e);
      })
      .finally(() => (loading = false));
  }

  $effect(() => {
    void cluster.id;
    load();
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`schema-host`,});
 { svelteHTML.createElement("div", { "class":`schema`,});
  if(loading){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"label":`schema subjects`,}});} }
  } else if (error){
     { svelteHTML.createElement("div", { "class":`empty`,});
      if(cluster.schema_registry_url){
        
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`schema subjects`,loading,error,"empty":true,"onretry":load,}});}
      }else{
         { svelteHTML.createElement("p", {});error; }
         { svelteHTML.createElement("p", { "class":`muted small`,});
                     
         }
      }
     }
  }else{
     { const $$_div2 = svelteHTML.createElement("div", {        "class":`list`,"onscroll":tw.onscroll,});tw.active(subjects.length);listEl = $$_div2;tw.viewH= $$_div2.clientHeight;
      if(win.top){ { svelteHTML.createElement("div", {     "class":`tw-spacer`,"aria-hidden":`true`,"style":`height:${win.top}px`,}); }}
         for(let s of __sveltets_2_ensureArray((subjects.slice(win.start, win.end)))){s.subject;
         { svelteHTML.createElement("button", {      "class":`srow`,"onclick":() => { selected = s; showVersions = false; },});selected?.subject === s.subject;
           { svelteHTML.createElement("span", {   "class":`sn`,"title":s.subject,});s.subject; }
           { svelteHTML.createElement("span", { "class":`muted small`,}); s.version;  s.schema_type;  s.id; }
         }
      }
      if(win.bottom){ { svelteHTML.createElement("div", {     "class":`tw-spacer`,"aria-hidden":`true`,"style":`height:${win.bottom}px`,}); }}
      if(subjects.length === 0){ { svelteHTML.createElement("p", { "class":`muted pad`,});     }}
     }
     { svelteHTML.createElement("div", { "class":`view`,});
      if(selected){
         { svelteHTML.createElement("div", { "class":`view-head`,});
           { svelteHTML.createElement("span", { "class":`sn-big`,});selected.subject; }
           { svelteHTML.createElement("div", { "class":`view-tabs`,});
             { const $$_sbaT5C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT5C({ target: __sveltets_2_any(), props: {           "label":`Subject views`,"size":`s`,"tabs":[{ id: 'schema', label: 'Schema' }, { id: 'versions', label: 'Versions & Compat' }],"value":showVersions ? 'versions' : 'schema',"onchange":(id) => (showVersions = id === 'versions'),}});}
           }
         }
        if(showVersions){
           { const $$_lenaPsnoisreVamehcS3C = __sveltets_2_ensureComponent(SchemaVersionsPanel); new $$_lenaPsnoisreVamehcS3C({ target: __sveltets_2_any(), props: {    "cluster":cluster,"subject":selected.subject,}});}
        }else{
           { svelteHTML.createElement("pre", {   "class":`payload`,"dir":`ltr`,});prettySchema(selected); }
        }
      }
     }
  }
 }

 }

};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SchemaTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SchemaTab__SvelteComponent_ = ReturnType<typeof SchemaTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default SchemaTab__SvelteComponent_;