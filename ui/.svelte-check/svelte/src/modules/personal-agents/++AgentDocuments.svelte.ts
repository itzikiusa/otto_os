///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { personalAgentsApi } from '../../lib/api/personalAgents';
import { listVaults } from '../../lib/api/vault';
import type { PersonalAgentDocument, Vault } from '../../lib/api/types';
import Markdown from '../agents/conversation/Markdown.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { loadErrorText } from '../../lib/loadError';
import { guardUnsaved } from '../../lib/leaveGuard';
import { router } from '../../lib/router.svelte';
import { focusFirstField, refocusEditTrigger } from '../../lib/inlineEdit';
;

  interface Props {
    agentId: string;
    workspaceId: string;
    kind: 'memory' | 'context';
    editable?: boolean;
    sharedWorkspace?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { agentId, workspaceId, kind, editable = false, sharedWorkspace = false }: Props = $props();
  let document = $state<PersonalAgentDocument | null>(null);
  let draft = $state('');
  let editing = $state(false);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state('');
  let picker = $state<string | null>(null);
  let vaults = $state<Vault[] | null>(null);
  let vaultId = $state('');
  let generation = 0;
  const label = $derived(kind === 'memory' ? 'Memory' : 'Context');
  $effect(() => router.guard(() => !saving));
  $effect(() => guardUnsaved(() => editing && draft !== (document?.content ?? ''), { what: 'this agent’s document' }));

  async function load() {
    const seq = ++generation;
    loading = true;
    error = '';
    try {
      const result = await personalAgentsApi.document(agentId, kind);
      if (seq !== generation) return;
      document = result;
      draft = result.content;
      editing = false;
    } catch (e) {
      if (seq === generation) error = `Couldn’t load the ${kind}. ${loadErrorText(e)}`;
    } finally { if (seq === generation) loading = false; }
  }
  $effect(() => {
    agentId; kind;
    void load();
    return () => { generation++; };
  });
  async function save() {
    if (!document || !editable || saving) return;
    const seq = generation;
    saving = true;
    error = '';
    try {
      const result = await personalAgentsApi.saveDocument(agentId, kind, { content: draft, version: document.version });
      if (seq !== generation) return;
      document = result;
      draft = result.content;
      editing = false;
      void refocusEditTrigger(`doc-${kind}`);
    } catch (e) {
      if (seq === generation) error = `Couldn’t save the ${kind}. ${loadErrorText(e)}`;
    } finally { if (seq === generation) saving = false; }
  }
  async function chooseVault() {
    try { vaults = await listVaults(workspaceId); vaultId = String(vaults[0]?.id ?? ''); }
    catch (e) { error = `Couldn’t list Vaults. ${loadErrorText(e)}`; }
  }
  function addReference(path: string) {
    draft += `${draft && !draft.endsWith('\n') ? '\n' : ''}\n- Reference: ${path}\n`;
    picker = null;
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`document`,...__sveltets_2_empty({"data-testid":`agent-document`}),});
   { svelteHTML.createElement("h2", {});label; }
   { svelteHTML.createElement("p", { "class":`hint`,});kind === 'memory'
    ? 'Notes the agent reads and updates between runs. Saving edits preserves the rest of your workspace.'
    : 'Background notes and references you maintain. New runs and new chats receive a snapshot; existing chats keep their current context.'; }
  if(document?.path){ { svelteHTML.createElement("code", { "class":`path`,});document.path; }}
  if(kind === 'memory' && sharedWorkspace){ { svelteHTML.createElement("p", { "class":`hint`,});          }}
  if(loading){ { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":3,"label":label.toLowerCase(),}});}
  }else{
    if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: error, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`error`,});error; }}}
    if(document){
      if(editing){
         {const $$action_0 = __sveltets_2_ensureAction(focusFirstField(svelteHTML.mapElementTag('textarea')));{ svelteHTML.createElement("textarea", __sveltets_2_union($$action_0), {          "dir":`auto`,"aria-label":`${label} Markdown`,"bind:value":draft,"disabled":saving,"spellcheck":`false`,});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }}
        if(kind === 'context'){
           { svelteHTML.createElement("div", { "class":`actions`,});
             { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":saving,"onclick":() => (picker = ''),});   }
             { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":saving,"onclick":chooseVault,});   }
           }
          if(vaults){
             { svelteHTML.createElement("div", { "class":`actions`,});
               { svelteHTML.createElement("select", {   "aria-label":`Vault`,"bind:value":vaultId,});/*Ωignore_startΩ*/() => vaultId = __sveltets_2_any(null);/*Ωignore_endΩ*/   for(let vault of __sveltets_2_ensureArray(vaults)){vault.id; { svelteHTML.createElement("option", { "value":String(vault.id),});vault.name; }} }
               { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":!vaultId,"onclick":() => (picker = vaults?.find((v) => String(v.id) === vaultId)?.root_path ?? ''),});  }
              if(!vaults.length){ { svelteHTML.createElement("span", { "class":`hint`,});     }}
             }
          }
        }
         { svelteHTML.createElement("div", { "class":`actions`,});
           { svelteHTML.createElement("button", {       "type":`button`,"class":`btn primary`,"disabled":saving,"onclick":save,});saving ? 'Saving…' : 'Save'; }
           { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":saving,"onclick":() => { draft = document?.content ?? ''; editing = false; error = ''; void refocusEditTrigger(`doc-${kind}`); },});  }
          if(error){ { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":saving,"onclick":load,});   }}
         }
      }else{
        if(document.content){ { const $$_nwodkraM1C = __sveltets_2_ensureComponent(Markdown); new $$_nwodkraM1C({ target: __sveltets_2_any(), props: {  "md":document.content,}});}}else{ { svelteHTML.createElement("p", { "class":`hint`,}); label.toLowerCase();  }}
         { svelteHTML.createElement("div", { "class":`actions`,});
          if(editable){ { svelteHTML.createElement("button", {         "type":`button`,"class":`btn`,...__sveltets_2_empty({"data-edit-for":`doc-${kind}`}),"aria-label":`Edit ${label.toLowerCase()}`,"onclick":() => (editing = true),}); label.toLowerCase(); }}
           { svelteHTML.createElement("button", {     "type":`button`,"class":`btn`,"onclick":load,});  }
         }
      }
    }else{ { svelteHTML.createElement("button", {     "type":`button`,"class":`btn`,"onclick":load,});  }}
  }
 }
if(picker !== null){ { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {        "title":`Choose context reference`,"start":picker,"files":true,"onpick":addReference,"onclose":() => (picker = null),}});}}

};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentDocuments__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentDocuments__SvelteComponent_ = ReturnType<typeof AgentDocuments__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentDocuments__SvelteComponent_;